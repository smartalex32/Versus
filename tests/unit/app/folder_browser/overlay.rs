//! Folder browser overlay capability regressions.

use super::*;

#[test]
fn setting_identical_sources_preserves_overlay_state() {
    let sources = [Some(PathBuf::from("left")), Some(PathBuf::from("right"))];
    let mut browser = FolderBrowser::new_with_sources(sources.clone());
    browser
        .overlays
        .insert(PathBuf::from("kept.txt"), DirectoryEntryState::Different);
    browser.set_sources(sources);
    assert_eq!(
        browser.overlays.get(std::path::Path::new("kept.txt")),
        Some(&DirectoryEntryState::Different)
    );
}

#[test]
fn search_and_error_apis_do_not_change_working_state() {
    let mut browser = FolderBrowser::new_with_sources([None, None]);
    browser.open_search();
    assert!(browser.search_state.open);
    browser.error = Some("metadata failure".into());
    assert_eq!(browser.error_details(), ["metadata failure"]);
    browser.clear_error();
    assert!(browser.error_details().is_empty());
    assert!(browser.working.is_empty());
}

#[test]
fn overlay_compares_staged_text_without_saving_to_disk() {
    let root = std::env::temp_dir().join(format!("versus-overlay-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let left_root = root.join("left");
    let right_root = root.join("right");
    fs::create_dir_all(&left_root).unwrap();
    fs::create_dir_all(&right_root).unwrap();
    let left_path = left_root.join("item.txt");
    let right_path = right_root.join("item.txt");
    fs::write(&left_path, "left\n").unwrap();
    fs::write(&right_path, "right\n").unwrap();
    let mut left = load_working_file(Some(left_path.as_path())).unwrap();
    let right = load_working_file(Some(right_path.as_path())).unwrap();
    left.text = "right\n".into();
    let relative = PathBuf::from("item.txt");
    let results = compare_overlay_files(
        [left_root, right_root],
        [relative.clone()].into_iter().collect(),
        [(relative.clone(), [Some(left), Some(right)])]
            .into_iter()
            .collect(),
        [false, false, false],
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(results[&relative], DirectoryEntryState::Same);
    assert_eq!(fs::read_to_string(left_path).unwrap(), "left\n");
    let _ = fs::remove_dir_all(root);
}
#[test]
fn working_copy_overlay_keeps_a_non_file_peer_as_type_mismatch() {
    let root = std::env::temp_dir().join(format!("versus-overlay-mismatch-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let roots = [root.join("left"), root.join("right")];
    fs::create_dir_all(&roots[0]).unwrap();
    fs::create_dir_all(roots[1].join("item")).unwrap();
    fs::write(roots[0].join("item"), "text\n").unwrap();
    let mut left = load_working_file(Some(&roots[0].join("item"))).unwrap();
    left.text = "edited\n".into();
    let relative = PathBuf::from("item");
    let states = compare_overlay_files(
        roots,
        [relative.clone()].into_iter().collect(),
        [(relative.clone(), [Some(left), None])]
            .into_iter()
            .collect(),
        [false; 3],
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(states[&relative], DirectoryEntryState::TypeMismatch);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn overlay_cache_ignores_show_only_but_rechecks_after_save() {
    let path = PathBuf::from("item.txt");
    let file = WorkingFile {
        identity: None,
        path: Some(path.clone()),
        text: "edited\n".into(),
        encoding: TextEncoding::Utf8 { bom: false },
        fingerprint: FileFingerprint::Bytes(b"saved\n".to_vec()),
    };
    let files = [Some(file), None];
    let revisions = [(files[0].as_ref().unwrap().identity_key(), 1)]
        .into_iter()
        .collect();
    let dirty = overlay_signature(
        &path,
        Some(&files),
        &revisions,
        &[false, false, false],
        true,
    );
    let show_only = overlay_signature(&path, Some(&files), &revisions, &[true, false, false], true);
    let saved = overlay_signature(
        &path,
        Some(&files),
        &revisions,
        &[false, false, false],
        false,
    );
    assert_eq!(dirty, show_only);
    assert_ne!(dirty, saved);
}

#[test]
fn disabling_run_diff_cancels_overlay_and_clears_statuses() {
    let roots = [PathBuf::from("left-root"), PathBuf::from("right-root")];
    let mut browser = FolderBrowser::new(roots);
    browser
        .overlays
        .insert(PathBuf::from("changed.txt"), DirectoryEntryState::Different);
    browser.overlay_paths.insert(PathBuf::from("changed.txt"));
    let (sender, receiver) = mpsc::channel();
    let cancellation = Arc::new(AtomicBool::new(false));
    browser.overlay_job = Some(OverlayJob {
        receiver,
        cancel: cancellation.clone(),
        signatures: BTreeMap::new(),
    });
    let ctx = egui::Context::default();
    browser.poll_comparisons(&InteractiveView::default(), &ctx, true);
    assert!(browser.overlay_job.is_none());
    assert!(cancellation.load(Ordering::Relaxed));
    assert_eq!(browser.retired_overlay_jobs.len(), 1);
    assert!(browser.overlays.is_empty());
    assert!(browser.overlay_paths.is_empty());
    // A late result from the cancelled generation must never reappear on
    // re-enabling comparison, even when the browser was hidden meanwhile.
    sender
        .send((
            0,
            Ok(
                [(PathBuf::from("changed.txt"), DirectoryEntryState::Different)]
                    .into_iter()
                    .collect(),
            ),
        ))
        .unwrap();
    drop(sender);
    let mut view = InteractiveView::default();
    view.set_editing(true, [false; 3]);
    view.set_run_diff(true);
    browser.poll_comparisons(&view, &ctx, true);
    browser.overlay_not_before = Some(Instant::now());
    browser.poll_comparisons(&view, &ctx, true);
    assert!(browser.overlays.is_empty());
    assert!(browser.overlay_job.is_none());
}
