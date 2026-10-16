use super::*;
use std::fs;
fn temp(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("versus-search-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}
fn replace_file(path: &Path, text: &str) {
    let replacement = path.with_extension("replacement");
    fs::write(&replacement, text).unwrap();
    #[cfg(windows)]
    fs::remove_file(path).unwrap();
    fs::rename(replacement, path).unwrap();
}
#[test]
fn regex_preview_is_grouped_and_staged_without_saving() {
    let root = temp("regex");
    let file = root.join("a.txt");
    fs::write(&file, "a1\na2\n").unwrap();
    fs::write(root.join("bin"), [0, 1, 2]).unwrap();
    let query = SearchQuery {
        pattern: "a(\\d)".into(),
        case_sensitive: true,
        whole_word: false,
        regex: true,
    };
    let copies = BTreeMap::new();
    let results = search_roots(
        &[Some(root.clone()), None],
        SearchScope::Left,
        &query,
        None,
        &copies,
    )
    .unwrap();
    assert_eq!(results.files[0].hits.len(), 2);
    assert_eq!(results.skipped_binary.len(), 1);
    let selected = results.files[0].hits.iter().map(|hit| hit.id).collect();
    let preview = preview_replace(&results, &selected, &query, "b$1", &copies).unwrap();
    let mut copies = BTreeMap::new();
    let changes = apply_preview(&mut copies, &preview).unwrap();
    assert_eq!(changes[0].after, "b1\nb2\n");
    assert_eq!(fs::read_to_string(&file).unwrap(), "a1\na2\n");
    let _ = fs::remove_dir_all(root);
}
#[test]
fn both_scopes_keep_side_identity_but_preview_deduplicates_the_file() {
    let root = temp("both");
    let file = root.join("same.txt");
    fs::write(&file, "needle").unwrap();
    let query = SearchQuery::plain("needle");
    let results = search_roots(
        &[Some(root.clone()), Some(root.clone())],
        SearchScope::Both,
        &query,
        None,
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(results.files.len(), 2);
    assert_eq!(results.files[0].side, 0);
    assert_eq!(results.files[1].side, 1);
    let selected = results
        .files
        .iter()
        .flat_map(|file| file.hits.iter())
        .map(|hit| hit.id)
        .collect();
    let preview = preview_replace(&results, &selected, &query, "done", &BTreeMap::new()).unwrap();
    assert_eq!(preview.changes.len(), 1);
    assert_eq!(preview.changes[0].ranges, vec![0..6]);
    assert_eq!(preview.changes[0].edits, vec![(0..6, "done".into())]);
    let _ = fs::remove_dir_all(root);
}
#[test]
fn stale_multifile_preview_does_not_partially_mutate_working_copies() {
    let root = temp("atomic-preview");
    let first = root.join("first.txt");
    let second = root.join("second.txt");
    fs::write(&first, "replace").unwrap();
    fs::write(&second, "replace").unwrap();
    let query = SearchQuery::plain("replace");
    let results = search_roots(
        &[Some(root.clone()), None],
        SearchScope::Left,
        &query,
        None,
        &BTreeMap::new(),
    )
    .unwrap();
    let selected = results
        .files
        .iter()
        .flat_map(|file| file.hits.iter())
        .map(|hit| hit.id)
        .collect();
    let preview = preview_replace(&results, &selected, &query, "done", &BTreeMap::new()).unwrap();
    let mut working = BTreeMap::new();
    let mut stale = load_working_file(Some(second.as_path())).unwrap();
    stale.text = "changed after preview".into();
    working.insert(second.clone(), stale);
    assert!(apply_preview(&mut working, &preview).is_err());
    assert!(!working.contains_key(&first));
    assert_eq!(working[&second].text, "changed after preview");
    let _ = fs::remove_dir_all(root);
}
#[test]
fn streaming_emits_matching_files_before_a_later_root_fails() {
    let root = temp("streaming-early-result");
    let file = root.join("match.txt");
    fs::write(&file, "needle").unwrap();
    let missing = root.join("missing");
    let mut events = Vec::new();
    let error = search_roots_streaming(
        &[Some(file.clone()), Some(missing)],
        SearchScope::Both,
        &SearchQuery::plain("needle"),
        None,
        &BTreeMap::new(),
        |event| {
            events.push(event);
            true
        },
    )
    .unwrap_err();
    assert_eq!(error.path, Some(root.join("missing")));
    assert!(matches!(
        events.as_slice(),
        [SearchEvent::File(SearchFileResults { path, side: 0, .. })] if path == &file
    ));
    let _ = fs::remove_dir_all(root);
}
#[test]
fn streaming_stops_when_receiver_declines_an_event() {
    let root = temp("streaming-receiver-gone");
    fs::write(root.join("first.txt"), "needle").unwrap();
    fs::write(root.join("second.txt"), "needle").unwrap();
    let mut events = Vec::new();
    search_roots_streaming(
        &[Some(root.clone()), None],
        SearchScope::Left,
        &SearchQuery::plain("needle"),
        None,
        &BTreeMap::new(),
        |event| {
            events.push(event);
            false
        },
    )
    .unwrap();
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], SearchEvent::File(_)));
    let _ = fs::remove_dir_all(root);
}
#[test]
fn streaming_emits_bounded_hit_batches_before_a_large_file_finishes() {
    let root = temp("streaming-hit-batches");
    let file = root.join("many.txt");
    fs::write(&file, "needle\n".repeat(SEARCH_HIT_BATCH_SIZE * 2 + 3)).unwrap();
    let mut events = Vec::new();
    search_roots_streaming(
        &[Some(file.clone()), None],
        SearchScope::Left,
        &SearchQuery::plain("needle"),
        None,
        &BTreeMap::new(),
        |event| {
            let continue_search = !matches!(event, SearchEvent::File(_));
            events.push(event);
            continue_search
        },
    )
    .unwrap();
    assert!(matches!(
        events.as_slice(),
        [SearchEvent::File(SearchFileResults { hits, .. })]
            if hits.len() == 1
    ));
    // The receiver stopped at the first match, proving the remaining
    // matches were not required before an event was delivered.
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, SearchEvent::Complete { .. }))
    );
    let _ = fs::remove_dir_all(root);
}
#[test]
fn streaming_aggregates_batches_and_can_cancel_between_them() {
    let root = temp("streaming-batch-cancel");
    let file = root.join("many.txt");
    fs::write(&file, "needle\n".repeat(SEARCH_HIT_BATCH_SIZE * 2 + 3)).unwrap();
    let cancellation = AtomicBool::new(false);
    let mut events = Vec::new();
    search_roots_streaming(
        &[Some(file), None],
        SearchScope::Left,
        &SearchQuery::plain("needle"),
        Some(&cancellation),
        &BTreeMap::new(),
        |event| {
            if matches!(event, SearchEvent::File(_)) {
                cancellation.store(true, Ordering::Relaxed);
            }
            events.push(event);
            true
        },
    )
    .unwrap();
    assert!(matches!(
        events.as_slice(),
        [SearchEvent::File(SearchFileResults { hits, .. }), SearchEvent::Complete { cancelled: true }]
            if hits.len() == 1
    ));

    let full = search_roots(
        &[Some(root.join("many.txt")), None],
        SearchScope::Left,
        &SearchQuery::plain("needle"),
        None,
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        full.files[0]
            .hits
            .iter()
            .map(|hit| hit.id)
            .collect::<Vec<_>>(),
        (1..=(SEARCH_HIT_BATCH_SIZE * 2 + 3) as u64).collect::<Vec<_>>()
    );
    let _ = fs::remove_dir_all(root);
}
#[test]
fn streaming_assigns_lines_incrementally_for_dense_utf8_matches() {
    let root = temp("streaming-utf8-lines");
    let file = root.join("lines.txt");
    fs::write(&file, "β needle\nneedle β needle\nneedle").unwrap();
    let mut events = Vec::new();
    search_roots_streaming(
        &[Some(file), None],
        SearchScope::Left,
        &SearchQuery::plain("needle"),
        None,
        &BTreeMap::new(),
        |event| {
            events.push(event);
            true
        },
    )
    .unwrap();
    let hits: Vec<_> = events
        .into_iter()
        .flat_map(|event| match event {
            SearchEvent::File(file) => file.hits,
            SearchEvent::Hits { hits, .. } => hits,
            _ => Vec::new(),
        })
        .collect();
    assert_eq!(
        hits.iter().map(|hit| hit.line).collect::<Vec<_>>(),
        vec![1, 2, 2, 3]
    );
    assert_eq!(
        hits.iter().map(|hit| hit.id).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    let _ = fs::remove_dir_all(root);
}
#[test]
fn streaming_preserves_side_specific_unique_ids_and_dirty_overrides() {
    let root = temp("streaming-sides");
    let file = root.join("same.txt");
    fs::write(&file, "disk needle").unwrap();
    let mut dirty = load_working_file(Some(file.as_path())).unwrap();
    dirty.text = "working needle".into();
    // An external atomic replacement can give the new on-disk file a
    // different resolved identity. Keep unsaved text selected by its
    // native path rather than silently searching the replacement.
    dirty.identity = Some(root.join("retired-identity"));
    replace_file(&file, "replacement needle");
    let mut overrides = BTreeMap::new();
    overrides.insert(file.clone(), dirty);
    let mut events = Vec::new();
    search_roots_streaming(
        &[Some(file.clone()), Some(file.clone())],
        SearchScope::Both,
        &SearchQuery::plain("needle"),
        None,
        &overrides,
        |event| {
            events.push(event);
            true
        },
    )
    .unwrap();
    let files: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SearchEvent::File(file) => Some(file),
            _ => None,
        })
        .collect();
    assert_eq!(files.len(), 2);
    assert_eq!((files[0].side, files[1].side), (0, 1));
    assert_eq!(files[0].working_file.text, "working needle");
    assert_ne!(files[0].hits[0].id, files[1].hits[0].id);
    assert!(matches!(
        events.last(),
        Some(SearchEvent::Complete { cancelled: false })
    ));
    let _ = fs::remove_dir_all(root);
}
#[test]
fn clean_override_is_reloaded_from_disk_for_a_fresh_search() {
    let root = temp("streaming-fresh-disk");
    let file = root.join("content.txt");
    fs::write(&file, "old needle").unwrap();
    let clean = load_working_file(Some(file.as_path())).unwrap();
    replace_file(&file, "new needle");
    let mut overrides = BTreeMap::new();
    overrides.insert(file.clone(), clean);
    let results = search_roots(
        &[Some(file.clone()), None],
        SearchScope::Left,
        &SearchQuery::plain("needle"),
        None,
        &overrides,
    )
    .unwrap();
    assert_eq!(results.files[0].working_file.text, "new needle");
    let _ = fs::remove_dir_all(root);
}
#[test]
fn preview_only_validates_files_with_selected_replacement_targets() {
    let root = temp("selected-side-preview");
    fs::create_dir_all(&root).unwrap();
    let paths = [root.join("left.txt"), root.join("right.txt")];
    for path in &paths {
        fs::write(path, "match").unwrap();
    }
    let query = SearchQuery::plain("match");
    let results = search_roots(
        &paths.clone().map(Some),
        SearchScope::Both,
        &query,
        None,
        &BTreeMap::new(),
    )
    .unwrap();
    let mut dirty = load_working_file(Some(&paths[1])).unwrap();
    dirty.text = "new unsaved text".into();
    let overrides = BTreeMap::from([(paths[1].clone(), dirty)]);
    let left = BTreeSet::from([results.files[0].hits[0].id]);
    let preview = preview_replace(&results, &left, &query, "changed", &overrides).unwrap();
    assert_eq!(preview.changes.len(), 1);
    assert_eq!(preview.changes[0].path, paths[0]);
    assert_eq!(preview.changes[0].after, "changed");
    let right = BTreeSet::from([results.files[1].hits[0].id]);
    assert!(preview_replace(&results, &right, &query, "changed", &overrides).is_err());
    assert_eq!(fs::read_to_string(&paths[0]).unwrap(), "match");
    assert_eq!(fs::read_to_string(&paths[1]).unwrap(), "match");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn preview_uses_fresh_search_snapshot_instead_of_a_clean_editor_cache() {
    let root = temp("preview-fresh-clean-cache");
    let file = root.join("content.txt");
    fs::write(&file, "old needle").unwrap();
    let clean_cache = load_working_file(Some(file.as_path())).unwrap();
    replace_file(&file, "new needle");
    let mut overrides = BTreeMap::new();
    overrides.insert(file.clone(), clean_cache);
    let query = SearchQuery::plain("needle");
    let results = search_roots(
        &[Some(file.clone()), None],
        SearchScope::Left,
        &query,
        None,
        &overrides,
    )
    .unwrap();
    let selected = results.files[0].hits.iter().map(|hit| hit.id).collect();
    let preview = preview_replace(&results, &selected, &query, "done", &overrides).unwrap();
    assert_eq!(preview.changes[0].before, "new needle");
    assert_eq!(preview.changes[0].after, "new done");
    assert_eq!(
        preview.bases.get(&file),
        Some(&results.files[0].working_file)
    );
    let _ = fs::remove_dir_all(root);
}
#[test]
fn streaming_reports_cancellation_as_completion() {
    let root = temp("streaming-cancelled");
    let cancelled = AtomicBool::new(true);
    let mut events = Vec::new();
    search_roots_streaming(
        &[Some(root.clone()), None],
        SearchScope::Left,
        &SearchQuery::plain("needle"),
        Some(&cancelled),
        &BTreeMap::new(),
        |event| {
            events.push(event);
            true
        },
    )
    .unwrap();
    assert_eq!(events, vec![SearchEvent::Complete { cancelled: true }]);
    let _ = fs::remove_dir_all(root);
}
#[test]
fn aliased_roots_share_unsaved_copy_and_coalesce_selected_matches() {
    let root = temp("aliases");
    let file = root.join("same.txt");
    fs::write(&file, "a b").unwrap();
    let alias = root.join("../").join(root.file_name().unwrap());
    let query = SearchQuery {
        pattern: "[ab]".into(),
        case_sensitive: true,
        whole_word: false,
        regex: true,
    };
    let mut copies = BTreeMap::new();
    let mut working = load_working_file(Some(file.as_path())).unwrap();
    working.text = "a b".into();
    copies.insert(file.clone(), working);
    let results = search_roots(
        &[Some(root.clone()), Some(alias)],
        SearchScope::Both,
        &query,
        None,
        &copies,
    )
    .unwrap();
    assert_eq!(results.files.len(), 2);
    assert_eq!(results.files[0].side, 0);
    assert_eq!(results.files[1].side, 1);
    let selected = results
        .files
        .iter()
        .flat_map(|file| file.hits.iter())
        .filter(|hit| {
            (hit.side == 0 && hit.matched == "a") || (hit.side == 1 && hit.matched == "b")
        })
        .map(|hit| hit.id)
        .collect();
    let preview = preview_replace(&results, &selected, &query, "x", &copies).unwrap();
    assert_eq!(preview.changes.len(), 1);
    assert_eq!(preview.changes[0].after, "x x");
    apply_preview(&mut copies, &preview).unwrap();
    assert_eq!(copies[&file].text, "x x");
    let _ = fs::remove_dir_all(root);
}
#[cfg(unix)]
#[test]
fn lexical_aliases_with_different_symlink_parents_stay_separate() {
    use std::os::unix::fs::symlink;
    let root = temp("physical-aliases");
    let external = root.join("external");
    let link = root.join("link");
    fs::create_dir_all(external.join("nested")).unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("outside.txt"), "left").unwrap();
    fs::write(external.join("outside.txt"), "right").unwrap();
    symlink(external.join("nested"), &link).unwrap();
    // `link/..` is the external parent physically but normalizes to the
    // same lexical spelling as `root/outside.txt`.
    let left = link.join("../outside.txt");
    let right = root.join("outside.txt");
    let query = SearchQuery::plain("t");
    let results = search_roots(
        &[Some(left), Some(right)],
        SearchScope::Both,
        &query,
        None,
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(results.files.len(), 2);
    let selected = results
        .files
        .iter()
        .flat_map(|file| file.hits.iter())
        .map(|hit| hit.id)
        .collect();
    let preview = preview_replace(&results, &selected, &query, "T", &BTreeMap::new()).unwrap();
    assert_eq!(preview.changes.len(), 2);
    let _ = fs::remove_dir_all(root);
}
