use std::{
    fs,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::{SystemTime, UNIX_EPOCH},
};
use versus::core::*;

fn sandbox(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "versus-core-{name}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn text_comparison_handles_bom_line_endings_and_whitespace_options() {
    let options = FileCompareOptions::default();
    let same = compare_texts("\u{feff}one\r\ntwo\r\n", "one\ntwo\n", &options);
    assert!(same.hunks.is_empty());
    let different = compare_texts("one value\n", "one   value\n", &options);
    assert_eq!(different.hunks.len(), 1);
    let ignoring = compare_texts(
        "one value\n",
        "one   value\n",
        &FileCompareOptions {
            ignore_whitespace: true,
            ..options
        },
    );
    assert!(ignoring.hunks.is_empty());
}

#[test]
fn file_comparison_identifies_binary_and_large_files() {
    let root = sandbox("files");
    let left = root.join("left");
    let right = root.join("right");
    fs::write(&left, [0, 1, 2]).unwrap();
    fs::write(&right, [0, 1, 3]).unwrap();
    assert_eq!(
        compare_files(&left, &right, &FileCompareOptions::default())
            .unwrap()
            .kind,
        FileDiffKind::Binary
    );
    fs::write(&left, "abcdef").unwrap();
    fs::write(&right, "abcdeg").unwrap();
    assert_eq!(
        compare_files(
            &left,
            &right,
            &FileCompareOptions {
                text_size_limit: 2,
                ..FileCompareOptions::default()
            }
        )
        .unwrap()
        .kind,
        FileDiffKind::TooLarge
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_comparison_covers_equal_changed_added_deleted_empty_and_unicode_text() {
    let root = sandbox("text-files");
    let left = root.join("left");
    let right = root.join("right");
    fs::write(&left, "").unwrap();
    fs::write(&right, "").unwrap();
    assert!(
        compare_files(&left, &right, &FileCompareOptions::default())
            .unwrap()
            .equal
    );
    fs::write(&left, "café\nkeep\nold\n").unwrap();
    fs::write(&right, "café\nchanged\nnew\n").unwrap();
    let changed = compare_files(&left, &right, &FileCompareOptions::default()).unwrap();
    assert_eq!(changed.kind, FileDiffKind::Text);
    assert!(!changed.equal);
    assert!(!changed.text.unwrap().hunks.is_empty());
    fs::write(&left, "one\ntwo\nthree\n").unwrap();
    fs::write(&right, "one\nthree\n").unwrap();
    assert_eq!(
        compare_files(&left, &right, &FileCompareOptions::default())
            .unwrap()
            .text
            .unwrap()
            .hunks[0]
            .kind,
        HunkKind::Removed
    );
    fs::write(&left, "one\nthree\n").unwrap();
    fs::write(&right, "one\ntwo\nthree\n").unwrap();
    assert_eq!(
        compare_files(&left, &right, &FileCompareOptions::default())
            .unwrap()
            .text
            .unwrap()
            .hunks[0]
            .kind,
        HunkKind::Added
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn buffered_comparison_observes_cancellation() {
    let root = sandbox("buffer-cancel");
    let left = root.join("left");
    let right = root.join("right");
    fs::write(&left, vec![b'x'; 256 * 1024]).unwrap();
    fs::write(&right, vec![b'x'; 256 * 1024]).unwrap();
    let cancellation = AtomicBool::new(true);
    assert_eq!(
        buffered_files_equal_cancellable(&left, &right, 16, Some(&cancellation)).unwrap(),
        None
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn directory_comparison_reports_states_and_does_not_follow_symlinks() {
    let root = sandbox("directories");
    let left = root.join("left");
    let right = root.join("right");
    fs::create_dir_all(left.join("nested")).unwrap();
    fs::create_dir_all(&right).unwrap();
    fs::write(left.join("same"), "equal").unwrap();
    fs::write(right.join("same"), "equal").unwrap();
    fs::write(left.join("changed"), "left").unwrap();
    fs::write(right.join("changed"), "right").unwrap();
    fs::write(left.join("left-only"), "x").unwrap();
    fs::create_dir(right.join("changed-dir")).unwrap();
    fs::write(left.join("changed-dir"), "file").unwrap();
    let diff = compare_directories(&left, &right, &DirectoryCompareOptions::default()).unwrap();
    assert!(
        diff.entries
            .iter()
            .any(|e| e.relative_path == PathBuf::from("same")
                && e.state == DirectoryEntryState::Same)
    );
    assert!(
        diff.entries
            .iter()
            .any(|e| e.relative_path == PathBuf::from("changed")
                && e.state == DirectoryEntryState::Different)
    );
    assert!(
        diff.entries
            .iter()
            .any(|e| e.relative_path == PathBuf::from("left-only")
                && e.state == DirectoryEntryState::LeftOnly)
    );
    assert!(
        diff.entries
            .iter()
            .any(|e| e.relative_path == PathBuf::from("changed-dir")
                && e.state == DirectoryEntryState::TypeMismatch)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn directory_ignore_options_apply_only_to_bounded_valid_utf8_text() {
    let root = sandbox("directory-ignore-options");
    let left = root.join("left");
    let right = root.join("right");
    fs::create_dir_all(&left).unwrap();
    fs::create_dir_all(&right).unwrap();
    fs::write(left.join("text.txt"), "one\tvalue\r\ntwo\r\n").unwrap();
    fs::write(right.join("text.txt"), "one value\ntwo\n").unwrap();
    fs::write(left.join("real-change.txt"), "left\n").unwrap();
    fs::write(right.join("real-change.txt"), "right\n").unwrap();
    fs::write(left.join("binary.bin"), [0, 1, 2]).unwrap();
    fs::write(right.join("binary.bin"), [0, 1, 3]).unwrap();
    fs::write(left.join("invalid-utf8.txt"), [0xff, b' ']).unwrap();
    fs::write(right.join("invalid-utf8.txt"), [0xff, b'\t']).unwrap();

    let exact = compare_directories(&left, &right, &DirectoryCompareOptions::default()).unwrap();
    assert_eq!(
        exact
            .entries
            .iter()
            .find(|entry| entry.relative_path == PathBuf::from("text.txt"))
            .unwrap()
            .state,
        DirectoryEntryState::Different
    );
    let ignored = compare_directories(
        &left,
        &right,
        &DirectoryCompareOptions {
            ignore_whitespace: true,
            ignore_line_endings: true,
            ..DirectoryCompareOptions::default()
        },
    )
    .unwrap();
    for (name, expected) in [
        ("text.txt", DirectoryEntryState::Same),
        ("real-change.txt", DirectoryEntryState::Different),
        ("binary.bin", DirectoryEntryState::Different),
        ("invalid-utf8.txt", DirectoryEntryState::Different),
    ] {
        assert_eq!(
            ignored
                .entries
                .iter()
                .find(|entry| entry.relative_path == PathBuf::from(name))
                .unwrap()
                .state,
            expected,
            "{name}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn directory_ignores_preserve_equal_large_text_and_oversize_byte_comparison() {
    let root = sandbox("directory-ignore-large");
    let left = root.join("left");
    let right = root.join("right");
    fs::create_dir_all(&left).unwrap();
    fs::create_dir_all(&right).unwrap();
    let text = "engineering value\r\n".repeat(32 * 1024);
    for index in 0..8 {
        let name = format!("same-{index}.txt");
        fs::write(left.join(&name), &text).unwrap();
        fs::write(right.join(&name), &text).unwrap();
    }
    // These would normalize identically, but exceed the bounded text limit.
    fs::write(
        left.join("large.txt"),
        vec![b' '; DEFAULT_TEXT_SIZE_LIMIT as usize + 1],
    )
    .unwrap();
    fs::write(
        right.join("large.txt"),
        vec![b'\t'; DEFAULT_TEXT_SIZE_LIMIT as usize + 1],
    )
    .unwrap();
    let result = compare_directories(
        &left,
        &right,
        &DirectoryCompareOptions {
            ignore_whitespace: true,
            ignore_line_endings: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(result.entries.len(), 9);
    for entry in result.entries {
        assert_eq!(
            entry.state,
            if entry.relative_path == PathBuf::from("large.txt") {
                DirectoryEntryState::Different
            } else {
                DirectoryEntryState::Same
            }
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn exact_text_comparison_preserves_line_endings_and_final_newline() {
    let strict = FileCompareOptions {
        ignore_line_endings: false,
        ..FileCompareOptions::default()
    };
    assert!(!compare_texts("line\n", "line", &strict).hunks.is_empty());
    assert!(
        !compare_texts("line\r\n", "line\n", &strict)
            .hunks
            .is_empty()
    );
    let ignore_endings = FileCompareOptions {
        ignore_line_endings: true,
        ..strict
    };
    assert!(
        compare_texts("line\r\n", "line", &ignore_endings)
            .hunks
            .is_empty()
    );
    for options in [strict, ignore_endings] {
        let result = compare_texts("one\rtwo\r", "one\rchanged\r", &options);
        assert_eq!(result.left_lines, ["one", "two"]);
        assert_eq!(result.right_lines, ["one", "changed"]);
        assert_eq!(result.hunks[0].left_range, 1..2);
        assert_eq!(result.hunks[0].right_range, 1..2);
    }
}

#[test]
fn directory_progress_counts_distinct_entries_and_does_not_finish_when_cancelled() {
    let root = sandbox("directory-progress");
    let left = root.join("left");
    let right = root.join("right");
    fs::create_dir_all(&left).unwrap();
    fs::create_dir_all(&right).unwrap();
    fs::write(left.join("same.txt"), "same").unwrap();
    fs::write(right.join("same.txt"), "same").unwrap();
    fs::write(left.join("left.txt"), "left").unwrap();
    let progress = Arc::new(ComparisonProgress::default());
    let diff = compare_directories(
        &left,
        &right,
        &DirectoryCompareOptions {
            progress: Some(progress.clone()),
            ..DirectoryCompareOptions::default()
        },
    )
    .unwrap();
    assert_eq!(diff.entries.len(), 2);
    let complete = progress.snapshot();
    assert_eq!(complete.stage, ProgressStage::Finished);
    assert_eq!(complete.completed, 2);
    assert_eq!(complete.total, Some(2));

    let cancelled = Arc::new(AtomicBool::new(true));
    let cancelled_progress = Arc::new(ComparisonProgress::default());
    assert!(
        compare_directories(
            &left,
            &right,
            &DirectoryCompareOptions {
                cancellation: Some(cancelled),
                progress: Some(cancelled_progress.clone()),
                ..DirectoryCompareOptions::default()
            },
        )
        .unwrap()
        .cancelled
    );
    assert_ne!(cancelled_progress.snapshot().stage, ProgressStage::Finished);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn directory_comparison_reports_symlinks_without_traversing_them() {
    use std::os::unix::fs::symlink;

    let root = sandbox("symlink");
    let left = root.join("left");
    let right = root.join("right");
    let outside = root.join("outside");
    fs::create_dir_all(&left).unwrap();
    fs::create_dir_all(&right).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("not-in-comparison"), "data").unwrap();
    symlink(&outside, left.join("linked")).unwrap();
    symlink(&outside, right.join("linked")).unwrap();

    let diff = compare_directories(&left, &right, &DirectoryCompareOptions::default()).unwrap();
    assert!(diff.entries.iter().any(|entry| {
        entry.relative_path == PathBuf::from("linked")
            && entry.kind == DirectoryEntryKind::Symlink
            && entry.state == DirectoryEntryState::Same
    }));
    assert!(
        !diff
            .entries
            .iter()
            .any(|entry| entry.relative_path == PathBuf::from("linked/not-in-comparison"))
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancellation_stops_directory_scan() {
    let root = sandbox("cancel");
    let left = root.join("left");
    let right = root.join("right");
    fs::create_dir_all(&left).unwrap();
    fs::create_dir_all(&right).unwrap();
    let cancellation = Arc::new(AtomicBool::new(true));
    assert!(
        compare_directories(
            &left,
            &right,
            &DirectoryCompareOptions {
                buffer_size: 4,
                cancellation: Some(cancellation),
                ..DirectoryCompareOptions::default()
            }
        )
        .unwrap()
        .cancelled
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn three_way_merge_marks_conflicts_and_applies_choices() {
    let options = FileCompareOptions::default();
    let diff = compare_three_texts("one\ntwo\n", "one\nleft\n", "one\nright\n", &options).unwrap();
    assert_eq!(diff.hunks[0].kind, MergeHunkKind::Conflict);
    assert!(diff.merged.contains("<<<<<<< LEFT"));
    assert_eq!(
        render_three_way(&diff, &[Some(MergeChoice::Right)]),
        "one\nright\n"
    );
    let same = compare_three_texts("a\n", "b\n", "b\n", &options).unwrap();
    assert_eq!(same.hunks[0].kind, MergeHunkKind::Identical);
}

#[test]
fn three_way_merge_preserves_prepend_middle_and_append_insert_positions() {
    let options = FileCompareOptions::default();
    for (left, expected) in [
        ("start\na\nb\n", "start\na\nb\n"),
        ("a\nmiddle\nb\n", "a\nmiddle\nb\n"),
        ("a\nb\nend\n", "a\nb\nend\n"),
    ] {
        let diff = compare_three_texts("a\nb\n", left, "a\nb\n", &options).unwrap();
        assert_eq!(diff.hunks[0].kind, MergeHunkKind::LeftOnly);
        assert_eq!(diff.merged, expected);
    }
}

#[test]
fn adjacent_merge_edits_do_not_conflict() {
    let options = FileCompareOptions::default();
    let diff = compare_three_texts("a\nb\n", "left\nb\n", "a\nright\n", &options).unwrap();
    assert!(
        diff.hunks
            .iter()
            .all(|hunk| hunk.kind != MergeHunkKind::Conflict)
    );
    assert_eq!(diff.merged, "left\nright\n");
}

#[test]
fn same_anchor_insertion_and_replacement_are_conflicts_without_panicking() {
    let options = FileCompareOptions::default();
    for (left, right) in [
        ("left\nb\n", "insert\na\nb\n"),
        ("insert\na\nb\n", "right\nb\n"),
    ] {
        let result =
            std::panic::catch_unwind(|| compare_three_texts("a\nb\n", left, right, &options));
        let diff = result.expect("merge must not panic").unwrap();
        assert!(
            diff.hunks
                .iter()
                .any(|hunk| hunk.kind == MergeHunkKind::Conflict)
        );
    }
}

#[test]
fn small_three_way_inputs_never_panic_and_unchanged_side_uses_changed_text() {
    let options = FileCompareOptions::default();
    let inputs = ["", "a\n", "b\n", "a\na\n", "a\nb\n", "b\na\n", "b\nb\n"];
    for base in inputs {
        for changed in inputs {
            let result =
                std::panic::catch_unwind(|| compare_three_texts(base, changed, base, &options));
            let diff = result.expect("merge must not panic").unwrap();
            assert_eq!(diff.merged, changed, "base={base:?}, changed={changed:?}");
        }
    }
}

#[test]
fn safe_save_requires_overwrite_and_applies_requested_line_ending() {
    let root = sandbox("save");
    let target = root.join("out.txt");
    save_text_safely(
        &target,
        "a\nb\n",
        &SaveOptions {
            overwrite: false,
            line_ending: LineEnding::Crlf,
        },
    )
    .unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "a\r\nb\r\n");
    assert!(save_text_safely(&target, "changed", &SaveOptions::default()).is_err());
    save_text_safely(
        &target,
        "changed",
        &SaveOptions {
            overwrite: true,
            ..SaveOptions::default()
        },
    )
    .unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "changed");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn safe_save_requires_overwrite_for_dangling_symlink() {
    use std::os::unix::fs::symlink;

    let root = sandbox("dangling-save");
    let target = root.join("dangling");
    symlink(root.join("missing"), &target).unwrap();
    assert!(save_text_safely(&target, "data", &SaveOptions::default()).is_err());
    fs::remove_dir_all(root).unwrap();
}
