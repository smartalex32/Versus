//! Folder browser navigation capability regressions.

use super::*;

#[test]
fn reveal_difference_expands_only_target_ancestors_and_queues_shallow_metadata() {
    let roots = [PathBuf::from("left-root"), PathBuf::from("right-root")];
    let mut browser = FolderBrowser::new(roots);
    browser.reveal_difference(PathBuf::from("one/two/changed.txt"));
    for side in 0..2 {
        assert!(browser.expanded[side].contains(&PathBuf::from("one")));
        assert!(browser.expanded[side].contains(&PathBuf::from("one/two")));
        assert!(browser.jobs[side].contains_key(&PathBuf::from("one")));
        assert!(browser.jobs[side].contains_key(&PathBuf::from("one/two")));
        assert!(!browser.jobs[side].contains_key(&PathBuf::from("one/two/changed.txt")));
    }
    assert_eq!(
        browser.pending_reveal,
        [
            Some(PathBuf::from("one/two/changed.txt")),
            Some(PathBuf::from("one/two/changed.txt"))
        ]
    );
    assert_eq!(
        browser.selected,
        [
            Some(PathBuf::from("one/two/changed.txt")),
            Some(PathBuf::from("one/two/changed.txt"))
        ]
    );
}

#[test]
fn single_source_enumerates_only_its_own_root() {
    let root = std::env::temp_dir().join(format!("versus-single-source-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("only-left.txt"), "content").unwrap();
    let mut browser = FolderBrowser::new_with_sources([Some(root.clone()), None]);
    for _ in 0..50 {
        browser.poll(&egui::Context::default());
        if browser.listings[0].contains_key(std::path::Path::new("")) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        browser.listings[0]
            .get(std::path::Path::new(""))
            .is_some_and(|items| items
                .iter()
                .any(|item| item.relative_path == PathBuf::from("only-left.txt")))
    );
    assert!(browser.listings[1].is_empty());
    assert!(browser.jobs[1].is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn changing_one_source_preserves_the_other_tree_state() {
    let mut browser = FolderBrowser::new_with_sources([
        Some(PathBuf::from("left")),
        Some(PathBuf::from("right")),
    ]);
    browser.expanded[0].insert(PathBuf::from("kept"));
    browser.expanded[1].insert(PathBuf::from("replaced"));
    browser.set_sources([
        Some(PathBuf::from("left")),
        Some(PathBuf::from("new-right")),
    ]);
    assert!(browser.expanded[0].contains(&PathBuf::from("kept")));
    assert!(browser.expanded[1].is_empty());
    assert_eq!(browser.source_roots()[0], Some(PathBuf::from("left")));
    assert_eq!(browser.source_roots()[1], Some(PathBuf::from("new-right")));
}

#[test]
fn changing_sources_discards_old_overlay_and_search_results() {
    let mut browser = FolderBrowser::new_with_sources([
        Some(PathBuf::from("left")),
        Some(PathBuf::from("right")),
    ]);
    browser
        .overlays
        .insert(PathBuf::from("old.txt"), DirectoryEntryState::Different);
    browser.overlay_paths.insert(PathBuf::from("old.txt"));
    let (sender, receiver) = mpsc::channel();
    browser.overlay_job = Some(OverlayJob {
        receiver,
        cancel: Arc::new(AtomicBool::new(false)),
        signatures: BTreeMap::new(),
    });
    browser.set_sources([
        Some(PathBuf::from("new-left")),
        Some(PathBuf::from("right")),
    ]);
    assert!(browser.overlay_job.is_none());
    assert!(browser.retired_overlay_jobs.is_empty());
    assert!(browser.overlays.is_empty());
    assert!(browser.overlay_paths.is_empty());
    drop(sender);
}
