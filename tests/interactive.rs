use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{SystemTime, UNIX_EPOCH},
};

use versus::core::*;

fn sandbox(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "versus-interactive-{name}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn protected_ranges_are_independent_and_keep_original_line_numbers_for_optional_diff() {
    let left = (0..800)
        .map(|line| format!("left-{line}\n"))
        .collect::<String>();
    let right = (0..400)
        .map(|line| format!("right-{line}\n"))
        .collect::<String>();
    let mut workspace = TwoWayWorkspace::new(left, right);

    workspace
        .add_ignored_range(TwoWaySide::Left, 50..750)
        .unwrap();
    workspace
        .add_ignored_range(TwoWaySide::Right, 100..300)
        .unwrap();
    assert_eq!(workspace.ignored_ranges(TwoWaySide::Left), &[50..750]);
    assert_eq!(workspace.ignored_ranges(TwoWaySide::Right), &[100..300]);
    assert_eq!(workspace.comparison_lines(TwoWaySide::Left).len(), 100);
    assert_eq!(workspace.comparison_lines(TwoWaySide::Right).len(), 200);
    assert!(
        workspace
            .comparison_lines(TwoWaySide::Left)
            .iter()
            .all(|(line, _)| *line < 50 || *line >= 750)
    );

    // Calling the optional comparison utility consumes only already-loaded text
    // and preserves source numbers after independently omitted ranges.
    let cancelled = AtomicBool::new(false);
    let comparison = compare_loaded_text(
        [
            workspace.text(TwoWaySide::Left),
            workspace.text(TwoWaySide::Right),
        ],
        &cancelled,
        &FileViewOptions::default(),
        [
            workspace.ignored_ranges(TwoWaySide::Left),
            workspace.ignored_ranges(TwoWaySide::Right),
        ],
    )
    .unwrap()
    .unwrap();
    assert!(comparison.rows.iter().all(|row| {
        row.left
            .as_ref()
            .is_none_or(|(number, _)| !(51..=750).contains(number))
            && row
                .right
                .as_ref()
                .is_none_or(|(number, _)| !(101..=300).contains(number))
    }));

    // Protected content is never an automatic replacement target, but remains
    // readable as a source for a deliberate transfer.
    assert!(matches!(
        workspace.transfer(
            TwoWaySide::Right,
            0..1,
            120..121,
            TransferPlacement::Replace,
        ),
        Err(TwoWayMergeError::ProtectedRange {
            side: TwoWaySide::Left,
            ..
        })
    ));
    workspace
        .transfer(TwoWaySide::Left, 50..51, 0..0, TransferPlacement::Insert)
        .unwrap();
    assert!(workspace.text(TwoWaySide::Right).starts_with("left-50\n"));
}

#[test]
fn transfers_are_atomic_undoable_redoable_and_never_mutate_the_source() {
    let mut workspace = TwoWayWorkspace::new("one\ntwo\nthree\n", "alpha\nbeta\n");
    let original_left = workspace.text(TwoWaySide::Left).to_owned();
    let original_right = workspace.text(TwoWaySide::Right).to_owned();

    workspace
        .transfer(TwoWaySide::Left, 1..3, 1..1, TransferPlacement::Insert)
        .unwrap();
    assert_eq!(workspace.text(TwoWaySide::Left), original_left);
    assert_eq!(
        workspace.text(TwoWaySide::Right),
        "alpha\ntwo\nthree\nbeta\n"
    );

    workspace
        .transfer(TwoWaySide::Right, 0..1, 0..1, TransferPlacement::Replace)
        .unwrap();
    assert_eq!(
        workspace.text(TwoWaySide::Right),
        "alpha\ntwo\nthree\nbeta\n"
    );
    assert_eq!(workspace.text(TwoWaySide::Left), "alpha\ntwo\nthree\n");

    assert!(workspace.undo());
    assert_eq!(workspace.text(TwoWaySide::Left), original_left);
    assert_eq!(
        workspace.text(TwoWaySide::Right),
        "alpha\ntwo\nthree\nbeta\n"
    );
    assert!(workspace.undo());
    assert_eq!(workspace.text(TwoWaySide::Right), original_right);
    assert!(workspace.redo());
    assert!(workspace.redo());
    assert_eq!(workspace.text(TwoWaySide::Left), "alpha\ntwo\nthree\n");
    assert_eq!(
        workspace.text(TwoWaySide::Right),
        "alpha\ntwo\nthree\nbeta\n"
    );
}

#[test]
fn chunks_support_empty_identical_unrelated_and_manual_boundary_changes() {
    let empty = TwoWayWorkspace::new("", "");
    assert_eq!(empty.line_count(TwoWaySide::Left), 0);
    assert!(empty.chunks(TwoWaySide::Left).is_empty());

    let identical = TwoWayWorkspace::new("same\n\nparagraph\n", "same\n\nparagraph\n");
    assert_eq!(
        identical.text(TwoWaySide::Left),
        identical.text(TwoWaySide::Right)
    );
    assert!(!identical.chunks(TwoWaySide::Left).is_empty());

    let mut unrelated = TwoWayWorkspace::new("fn left() {}\n\nbody\n", "zero\none\ntwo\n");
    let first = unrelated.create_chunk(TwoWaySide::Left, 0..2).unwrap();
    let (first_half, second_half) = unrelated.split_chunk(TwoWaySide::Left, first, 1).unwrap();
    unrelated
        .adjust_chunk(TwoWaySide::Left, second_half, 1..3)
        .unwrap();
    let combined = unrelated
        .combine_chunks(TwoWaySide::Left, first_half, second_half)
        .unwrap();
    let manual = unrelated
        .chunks(TwoWaySide::Left)
        .into_iter()
        .find(|chunk| chunk.id == combined)
        .unwrap();
    assert_eq!(manual.kind, ChunkKind::Manual);
    assert_eq!(manual.range, 0..3);

    // Each boundary operation is one history item. Rewinding combination leaves
    // the edited chunks available, rather than corrupting a later transfer.
    assert!(unrelated.undo());
    assert!(
        unrelated
            .chunks(TwoWaySide::Left)
            .iter()
            .any(|chunk| chunk.id == first_half)
    );
    assert!(unrelated.redo());
    unrelated
        .transfer_chunk(TwoWaySide::Left, combined, 1..1, TransferPlacement::Insert)
        .unwrap();
    assert_eq!(
        unrelated.text(TwoWaySide::Right),
        "zero\nfn left() {}\n\nbody\none\ntwo\n"
    );
}

#[test]
fn working_files_preserve_crlf_and_utf16_and_refuse_external_overwrite() {
    let root = sandbox("working-file");
    let crlf = root.join("crlf.txt");
    fs::write(&crlf, b"one\r\ntwo\r\n").unwrap();
    let mut file = load_working_file(Some(&crlf)).unwrap();
    file.text = "one\r\nthree\r\n".into();
    save_working_file(&mut file).unwrap();
    assert_eq!(fs::read(&crlf).unwrap(), b"one\r\nthree\r\n");

    let utf16 = root.join("utf16.txt");
    fs::write(&utf16, [0xff, 0xfe, b'a', 0, b'\r', 0, b'\n', 0]).unwrap();
    let mut unicode = load_working_file(Some(&utf16)).unwrap();
    unicode.text = "β\r\n".into();
    save_working_file(&mut unicode).unwrap();
    let bytes = fs::read(&utf16).unwrap();
    assert_eq!(&bytes[..2], &[0xff, 0xfe]);
    assert_eq!(load_working_file(Some(&utf16)).unwrap().text, "β\r\n");

    fs::write(&crlf, b"edited outside\n").unwrap();
    file.text.push_str("four\r\n");
    assert!(save_working_file(&mut file).is_err());
    assert_eq!(fs::read(&crlf).unwrap(), b"edited outside\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn search_preview_honors_regex_and_plain_literals_and_never_writes_disk() {
    let root = sandbox("search");
    let file = root.join("content.txt");
    fs::write(&file, "cost $5\nitem-12\nitem-34\n").unwrap();
    let plain = SearchQuery::plain("$5");
    let results = search_roots(
        &[Some(root.clone()), None],
        SearchScope::Left,
        &plain,
        None,
        &BTreeMap::new(),
    )
    .unwrap();
    let selected: BTreeSet<_> = results.files[0].hits.iter().map(|hit| hit.id).collect();
    let preview = preview_replace(&results, &selected, &plain, "$6", &BTreeMap::new()).unwrap();
    assert_eq!(preview.changes[0].after, "cost $6\nitem-12\nitem-34\n");

    let regex = SearchQuery {
        pattern: r"item-(\d+)".into(),
        case_sensitive: true,
        whole_word: true,
        regex: true,
    };
    let found = search_roots(
        &[Some(root.clone()), None],
        SearchScope::Left,
        &regex,
        None,
        &BTreeMap::new(),
    )
    .unwrap();
    let selected = found.files[0].hits.iter().map(|hit| hit.id).collect();
    let captured = preview_replace(&found, &selected, &regex, "id:$1", &BTreeMap::new()).unwrap();
    let mut working = BTreeMap::new();
    let changes = apply_preview(&mut working, &captured).unwrap();
    assert_eq!(changes[0].after, "cost $5\nid:12\nid:34\n");
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "cost $5\nitem-12\nitem-34\n"
    );

    // A caller can atomically undo a batch by applying the preview's captured
    // preimage; all changes remain in memory until an explicit save.
    working.get_mut(&file).unwrap().text = changes[0].before.clone();
    assert_eq!(working[&file].text, "cost $5\nitem-12\nitem-34\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn lazy_directory_listing_stays_shallow_and_does_not_follow_symlinks() {
    let root = sandbox("lazy-directory");
    let left = root.join("left");
    let right = root.join("right");
    fs::create_dir_all(left.join("child/grandchild")).unwrap();
    fs::create_dir_all(&right).unwrap();
    fs::write(left.join("visible.txt"), "left").unwrap();
    fs::write(left.join("child/grandchild/hidden.txt"), "deep").unwrap();
    fs::write(right.join("right.txt"), "right").unwrap();

    let top = directory_entries_at(&left, &right, Path::new("")).unwrap();
    assert_eq!(top.len(), 3);
    assert!(
        top.iter()
            .all(|entry| entry.relative_path.components().count() == 1)
    );
    assert!(
        top.iter()
            .all(|entry| entry.relative_path != PathBuf::from("child/grandchild"))
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let outside = root.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("secret"), "no traversal").unwrap();
        symlink(&outside, left.join("linked")).unwrap();
        let listed = directory_entries_at(&left, &right, Path::new("")).unwrap();
        let link = listed
            .iter()
            .find(|entry| entry.relative_path == PathBuf::from("linked"))
            .unwrap();
        assert_eq!(
            link.left.as_ref().unwrap().kind,
            DirectoryEntryKind::Symlink
        );
        assert!(
            directory_entries_at(&left, &right, Path::new("linked"))
                .unwrap()
                .is_empty()
        );
    }
    fs::remove_dir_all(root).unwrap();
}
