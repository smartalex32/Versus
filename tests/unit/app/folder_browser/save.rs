//! Folder browser save capability regressions.

use super::*;
use crate::app::visuals::theme::set_theme;

#[test]
fn save_completion_keeps_newer_edits_dirty_against_the_saved_baseline() {
    let path = PathBuf::from("changed.txt");
    let mut working = BTreeMap::new();
    working.insert(
        path.clone(),
        WorkingFile {
            identity: None,
            path: Some(path.clone()),
            text: "edited after save started\n".into(),
            encoding: TextEncoding::Utf8 { bom: false },
            fingerprint: FileFingerprint::Bytes(b"original\n".to_vec()),
        },
    );
    let saved = WorkingFile {
        identity: None,
        path: Some(path.clone()),
        text: "snapshot\n".into(),
        encoding: TextEncoding::Utf8 { bom: false },
        fingerprint: FileFingerprint::Bytes(b"snapshot\n".to_vec()),
    };

    retain_saved_snapshot(&mut working, path.clone(), "snapshot\n".into(), saved);

    let current = &working[&path];
    assert_eq!(current.text, "edited after save started\n");
    assert_eq!(
        current.fingerprint,
        FileFingerprint::Bytes(b"snapshot\n".to_vec())
    );
    assert!(current.is_dirty());
}

#[test]
fn pending_save_deduplicates_path_aliases_by_physical_identity() {
    let identity = PathBuf::from("/physical/file.txt");
    let mut working = BTreeMap::new();
    for path in [
        PathBuf::from("left/file.txt"),
        PathBuf::from("right/../left/file.txt"),
    ] {
        working.insert(
            path.clone(),
            WorkingFile {
                identity: Some(identity.clone()),
                path: Some(path),
                text: "edited".into(),
                encoding: TextEncoding::Utf8 { bom: false },
                fingerprint: FileFingerprint::Bytes(b"original".to_vec()),
            },
        );
    }
    let pending = pending_save_files(&working);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].1.identity_key(), identity);
}

#[test]
fn folder_replacements_save_from_workspace_without_opening_the_files() {
    let root = std::env::temp_dir().join(format!(
        "versus-footer-replacement-save-{}",
        std::process::id()
    ));
    let roots = [root.join("left"), root.join("right")];
    for folder in &roots {
        fs::create_dir_all(folder).unwrap();
    }
    let paths = roots.each_ref().map(|folder| folder.join("item.txt"));
    for path in &paths {
        fs::write(path, "match\n").unwrap();
    }
    let ctx = egui::Context::default();
    set_theme(&ctx, egui::Theme::Dark);
    let mut view = InteractiveView::default();
    view.set_editing(true, [false; 3]);
    let mut browser = FolderBrowser::new(roots.clone());
    browser.search_state.query = SearchQuery::plain("match");
    browser.search_state.replacement = "changed".into();
    browser.search_state.searched_query = Some(browser.search_state.query.clone());
    browser.search_state.results = Some(
        versus::search_roots(
            &browser.sources,
            SearchScope::Both,
            &browser.search_state.query,
            None,
            &BTreeMap::new(),
        )
        .unwrap(),
    );
    browser.search_state.selected_hits = browser
        .search_state
        .results
        .as_ref()
        .unwrap()
        .files
        .iter()
        .flat_map(|file| file.hits.iter().map(|hit| hit.id))
        .collect();
    browser.replace_matches(&mut view, false);
    assert!(view.any_dirty());
    assert!(view.working_text(0).is_none() && view.working_text(1).is_none());
    for path in &paths {
        assert_eq!(fs::read_to_string(path).unwrap(), "match\n");
    }
    let mut app = VersusApp::default();
    app.session.merge_mode = true;
    app.session.interactive = Some(view);
    app.session.source_paths = roots.clone().map(Some);
    app.session.source_kinds = [Some(SourceKind::Folder); 2];
    app.session.merge_folders = Some(browser);
    let render = |app: &mut VersusApp, events| {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                events,
                ..Default::default()
            },
            |ui| app.render(ui),
        )
    };
    for _ in 0..3 {
        render(&mut app, vec![]).drop_without_applying_deltas();
    }
    let save = ctx.read_response(egui::Id::new("Save Both")).unwrap();
    assert!(save.enabled());
    for pressed in [true, false] {
        render(
            &mut app,
            vec![
                egui::Event::PointerMoved(save.rect.center()),
                egui::Event::PointerButton {
                    pos: save.rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        )
        .drop_without_applying_deltas();
    }
    assert!(app.session.merge_folders.as_ref().unwrap().is_saving());
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.session.merge_folders.as_ref().unwrap().is_saving() {
        app.session.poll_merge_save_owners(&ctx);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    for path in &paths {
        assert_eq!(fs::read_to_string(path).unwrap(), "changed\n");
    }
    assert!(!app.session.interactive.as_ref().unwrap().any_dirty());
    assert!(
        app.session
            .merge_folders
            .as_ref()
            .unwrap()
            .error_details()
            .is_empty()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_search_save_drains_after_a_synchronous_folder_transition() {
    let root = std::env::temp_dir().join(format!(
        "versus-file-search-save-transition-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("item.txt");
    let peer = root.join("peer.txt");
    fs::write(&path, "before\n").unwrap();
    fs::write(&peer, "peer\n").unwrap();
    let ctx = egui::Context::default();
    let mut view = InteractiveView::default();
    view.set_editing(true, [false; 3]);
    view.open_sources([Some(path.clone()), Some(peer.clone())]);
    let deadline = Instant::now() + Duration::from_secs(5);
    // The save fixture edits the paired workspace; wait for both loads
    // rather than racing the peer loader with the test-only mutation.
    while view.working_text(0) != Some("before\n") || view.working_text(1) != Some("peer\n") {
        view.poll(&ctx);
        assert!(Instant::now() < deadline, "file did not load");
        std::thread::sleep(Duration::from_millis(1));
    }
    view.set_working_text_for_test(0, "saved before\n");
    assert_eq!(view.working_text(0), Some("saved before\n"));

    let mut file_search = FolderBrowser::new_search([Some(path.clone()), None]);
    file_search.begin_save(&view);
    assert!(file_search.is_saving());

    let mut app = VersusApp::default();
    app.session.merge_mode = true;
    app.session.interactive = Some(view);
    app.session.merge_file_search = Some(file_search);
    // This is the source transition that previously dropped the search
    // owner's receiver before it could publish the save result.
    app.session.source_paths[0] = Some(root.clone());
    app.session.source_kinds[0] = Some(SourceKind::Folder);
    app.session.source_paths[1] = Some(peer);
    app.session.source_kinds[1] = Some(SourceKind::File);
    app.session.open_interactive_sources();
    assert!(app.session.merge_folders.is_some());
    assert!(app.session.merge_file_search.as_ref().unwrap().is_saving());

    let deadline = Instant::now() + Duration::from_secs(5);
    while app
        .session
        .merge_file_search
        .as_ref()
        .is_some_and(FolderBrowser::is_saving)
    {
        app.session.poll_merge_save_owners(&ctx);
        assert!(Instant::now() < deadline, "file-search save did not drain");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), "saved before\n");
    assert!(!app.session.interactive.as_ref().unwrap().is_saving());
    assert!(!app.session.interactive.as_ref().unwrap().any_dirty());
    let _ = fs::remove_dir_all(root);
}
