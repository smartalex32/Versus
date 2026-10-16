//! Debounce, refresh, reopen, cancellation, and replacement lifecycle regressions.

use super::*;
use crate::app::visuals::theme::set_theme;

#[test]
fn refresh_repopulates_exclusions_and_rereads_changed_files() {
    let path =
        std::env::temp_dir().join(format!("versus-refresh-search-{}.txt", std::process::id()));
    std::fs::write(&path, "match old\n").unwrap();
    let ctx = egui::Context::default();
    set_theme(&ctx, egui::Theme::Dark);
    let mut browser = FolderBrowser::new_search([Some(path.clone()), None]);
    let mut view = InteractiveView::default();
    view.ingest_working_files(&BTreeMap::from([(
        path.clone(),
        load_working_file(Some(&path)).unwrap(),
    )]));
    view.open_sources([Some(path.clone()), None]);
    browser.open_search();
    browser.search_state.query = SearchQuery::plain("match");
    browser.start_search(&view);
    let wait = |browser: &mut FolderBrowser| {
        let deadline = Instant::now() + Duration::from_secs(2);
        while browser.search_state.job.is_some() && Instant::now() < deadline {
            browser.poll(&ctx);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(browser.search_state.results_current());
    };
    wait(&mut browser);
    browser.search_state.selected_hits.clear();
    std::fs::write(&path, "match new\nmatch added\n").unwrap();
    for _ in 0..3 {
        dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
    }
    let refresh = ctx
        .read_response(egui::Id::new("Refresh search"))
        .unwrap()
        .rect;
    click_dialog(&mut browser, &mut view, &ctx, refresh.center());
    wait(&mut browser);
    let file = &browser.search_state.results.as_ref().unwrap().files[0];
    assert_eq!(file.working_file.text, "match new\nmatch added\n");
    assert_eq!(browser.search_state.selected_hits.len(), 2);
    browser.search_state.replacement = "found".into();
    browser.replace_matches(&mut view, false);
    assert_eq!(view.working_text(0), Some("found new\nfound added\n"));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "match new\nmatch added\n"
    );
    view.undo_batch(false).unwrap();
    assert_eq!(view.working_text(0), Some("match new\nmatch added\n"));
    view.undo_batch(true).unwrap();
    std::fs::write(&path, "changed externally again\n").unwrap();
    let mut pending = view.export_working_files()[&path].clone();
    assert!(versus::save_working_file(&mut pending).is_err());
    assert!(!view.run_diff_enabled());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn empty_state_is_clipped_to_results_and_cancelled_partial_search_restarts_on_reopen() {
    let ctx = egui::Context::default();
    set_theme(&ctx, egui::Theme::Dark);
    let mut browser = FolderBrowser::new_search([None, None]);
    let mut view = InteractiveView::default();
    browser.open_search();
    for _ in 0..3 {
        dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
    }
    let panel = ctx
        .read_response(egui::Id::new("folder-search-results"))
        .unwrap()
        .rect;
    let output = dialog_frame(&mut browser, &mut view, &ctx, vec![]);
    let hints: Vec<_> = output
        .shapes
        .iter()
        .filter(|shape| {
            matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "Search results will appear here")
        })
        .collect();
    assert_eq!(hints.len(), 1);
    assert!(panel.contains_rect(hints[0].clip_rect));
    output.drop_without_applying_deltas();
    browser.search_state.query = SearchQuery::plain("match");
    browser.search_state.searched_query = Some(browser.search_state.query.clone());
    let (sender, receiver) = mpsc::channel();
    browser.search_state.job = Some(SearchJob {
        receiver,
        cancel: Arc::new(AtomicBool::new(false)),
    });
    sender
        .send(Ok(SearchEvent::File(streamed_file(0, 0, "match"))))
        .unwrap();
    browser.poll(&ctx);
    dialog_frame(
        &mut browser,
        &mut view,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    )
    .drop_without_applying_deltas();
    assert!(!browser.search_state.open);
    assert!(!browser.search_state.results_current());
    browser.open_search();
    dialog_frame_at(&mut browser, &mut view, &ctx, vec![], Some(10.0))
        .drop_without_applying_deltas();
    assert!(browser.search_state.due_at.is_none());
    assert_eq!(
        browser.search_state.searched_query,
        Some(browser.search_state.query.clone())
    );
    assert!(browser.search_state.job.is_some());
}

#[test]
fn typing_search_waits_a_quarter_second_after_the_last_edit_and_lists_results_on_the_right() {
    let path = std::env::temp_dir().join(format!("versus-live-search-{}.txt", std::process::id()));
    std::fs::write(&path, "alpha beta\nother content\n").unwrap();
    let ctx = egui::Context::default();
    set_theme(&ctx, egui::Theme::Dark);
    let mut browser = FolderBrowser::new_search([Some(path.clone()), None]);
    let mut view = InteractiveView::default();
    browser.open_search();
    for time in [0.0, 0.1, 0.2] {
        dialog_frame_at(&mut browser, &mut view, &ctx, vec![], Some(time))
            .drop_without_applying_deltas();
    }
    dialog_frame_at(
        &mut browser,
        &mut view,
        &ctx,
        vec![egui::Event::Text("alpha".into())],
        Some(1.0),
    )
    .drop_without_applying_deltas();
    assert_eq!(browser.search_state.query.pattern, "alpha");
    assert_eq!(browser.search_state.due_at, Some(1.25));
    assert!(browser.search_state.job.is_none());
    dialog_frame_at(
        &mut browser,
        &mut view,
        &ctx,
        vec![egui::Event::Text(" beta".into())],
        Some(1.2),
    )
    .drop_without_applying_deltas();
    assert_eq!(browser.search_state.query.pattern, "alpha beta");
    dialog_frame_at(&mut browser, &mut view, &ctx, vec![], Some(1.44))
        .drop_without_applying_deltas();
    assert!(
        browser.search_state.job.is_none(),
        "typing must reset the delay"
    );
    dialog_frame_at(&mut browser, &mut view, &ctx, vec![], Some(1.45))
        .drop_without_applying_deltas();
    assert!(browser.search_state.job.is_some());
    assert_eq!(
        browser
            .search_state
            .searched_query
            .as_ref()
            .unwrap()
            .pattern,
        "alpha beta"
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while browser.search_state.job.is_some() && Instant::now() < deadline {
        browser.poll(&ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    let results = browser
        .search_state
        .results
        .as_ref()
        .expect("live search must finish");
    assert_eq!(results.files.len(), 1);
    assert_eq!(results.files[0].hits.len(), 1);
    assert!(browser.search_state.results_current());
    dialog_frame_at(&mut browser, &mut view, &ctx, vec![], Some(2.0))
        .drop_without_applying_deltas();
    let output = dialog_frame_at(&mut browser, &mut view, &ctx, vec![], Some(2.1));
    let panel = ctx
        .read_response(egui::Id::new("folder-search-results"))
        .unwrap()
        .rect;
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "1: alpha beta"
                && panel.contains_rect(Rect::from_min_size(text.pos, text.galley.size())))));
    assert!(!view.run_diff_enabled());
    assert!(browser.jobs.iter().all(BTreeMap::is_empty));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "alpha beta\nother content\n"
    );
    output.drop_without_applying_deltas();
    dialog_frame_at(
        &mut browser,
        &mut view,
        &ctx,
        vec![egui::Event::Text(" more".into())],
        Some(2.2),
    )
    .drop_without_applying_deltas();
    assert!(browser.search_state.due_at.is_some());
    dialog_frame_at(
        &mut browser,
        &mut view,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        Some(2.3),
    )
    .drop_without_applying_deltas();
    assert!(!browser.search_state.open);
    assert!(browser.search_state.due_at.is_none() && browser.search_state.job.is_none());
    browser.open_search();
    dialog_frame_at(&mut browser, &mut view, &ctx, vec![], Some(2.5))
        .drop_without_applying_deltas();
    assert!(browser.search_state.due_at.is_none());
    assert_eq!(
        browser.search_state.searched_query,
        Some(browser.search_state.query.clone())
    );
    assert!(
        browser.search_state.job.is_some(),
        "reopening searches without a typing delay"
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn reopening_completed_search_rereads_files_and_repopulates_exclusions() {
    let root = std::env::temp_dir().join(format!("versus-reopen-search-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("item.txt");
    let ctx = egui::Context::default();
    set_theme(&ctx, egui::Theme::Dark);
    for folders in [false, true] {
        fs::write(&path, "match old\n").unwrap();
        let mut view = InteractiveView::default();
        view.ingest_working_files(&BTreeMap::from([(
            path.clone(),
            load_working_file(Some(&path)).unwrap(),
        )]));
        let mut browser = FolderBrowser::new_search([
            Some(if folders { root.clone() } else { path.clone() }),
            None,
        ]);
        browser.search_state.query = SearchQuery::plain("match");
        browser.search_state.replacement = "replacement".into();
        browser.search_state.scope = SearchScope::Left;
        let wait = |browser: &mut FolderBrowser| {
            let deadline = Instant::now() + Duration::from_secs(3);
            while browser.search_state.job.is_some() {
                browser.poll(&ctx);
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(browser.search_state.results_current());
        };
        let close = |browser: &mut FolderBrowser, view: &mut InteractiveView| {
            dialog_frame(
                browser,
                view,
                &ctx,
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
            )
            .drop_without_applying_deltas();
            assert!(!browser.search_state.open);
        };
        browser.open_search();
        dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
        wait(&mut browser);
        assert_eq!(browser.search_match_counts(), [1, 0]);
        browser.search_state.selected_hits.clear();
        close(&mut browser, &mut view);
        fs::write(&path, "match new\nmatch added\n").unwrap();
        browser.open_search();
        assert!(!browser.search_state.results_current());
        dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
        assert!(browser.search_state.job.is_some() && browser.search_state.due_at.is_none());
        wait(&mut browser);
        assert_eq!(browser.search_match_counts(), [2, 0]);
        assert_eq!(
            browser.search_state.results.as_ref().unwrap().files[0]
                .working_file
                .text,
            "match new\nmatch added\n"
        );
        assert_eq!(browser.search_state.replacement, "replacement");
        assert_eq!(browser.search_state.scope, SearchScope::Left);
        close(&mut browser, &mut view);
        let mut dirty = load_working_file(Some(&path)).unwrap();
        dirty.text = "match in memory\n".into();
        view.ingest_working_files(&BTreeMap::from([(path.clone(), dirty)]));
        browser.open_search();
        dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
        wait(&mut browser);
        assert_eq!(browser.search_match_counts(), [1, 0]);
        assert_eq!(
            browser.search_state.results.as_ref().unwrap().files[0]
                .working_file
                .text,
            "match in memory\n"
        );
        assert!(view.any_dirty());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "match new\nmatch added\n"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn changing_search_cancels_stale_results_and_clearing_it_does_not_start_a_worker() {
    let ctx = egui::Context::default();
    let mut browser = FolderBrowser::new_search([None, None]);
    let view = InteractiveView::default();
    let (sender, receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    browser.search_state.job = Some(SearchJob {
        receiver,
        cancel: cancel.clone(),
    });
    browser.search_state.results = Some(SearchResults::default());
    browser.search_state.query.pattern = "changed".into();
    browser.schedule_search(1.0);
    assert!(cancel.load(Ordering::Relaxed));
    assert!(
        sender
            .send(Ok(SearchEvent::Complete { cancelled: false }))
            .is_err(),
        "old worker cannot publish results"
    );
    assert!(browser.search_state.results.is_none() && !browser.search_state.results_current());
    browser.run_scheduled_search(&ctx, &view, 1.24);
    assert!(browser.search_state.job.is_none());
    browser.search_state.query.pattern.clear();
    browser.schedule_search(1.5);
    browser.run_scheduled_search(&ctx, &view, 2.0);
    assert!(browser.search_state.due_at.is_none() && browser.search_state.job.is_none());
}

#[test]
fn replace_next_and_all_immediately_advance_working_copies_without_saving() {
    let root = std::env::temp_dir().join(format!("versus-next-replace-{}", std::process::id()));
    std::fs::create_dir_all(root.join("left")).unwrap();
    std::fs::create_dir_all(root.join("right")).unwrap();
    let paths = [root.join("left/a.txt"), root.join("right/a.txt")];
    std::fs::write(&paths[0], "foo foo foo\n").unwrap();
    std::fs::write(&paths[1], "foo\n").unwrap();
    let working: BTreeMap<_, _> = paths
        .iter()
        .map(|path| (path.clone(), load_working_file(Some(path)).unwrap()))
        .collect();
    let mut view = InteractiveView::default();
    view.ingest_working_files(&working);
    view.open_sources(paths.clone().map(Some));
    view.set_editing(true, [true; 3]);
    let mut browser =
        FolderBrowser::new_with_sources([Some(root.join("left")), Some(root.join("right"))]);
    browser.open_search();
    browser.search_state.query = SearchQuery::plain("foo");
    browser.search_state.replacement = "long value".into();
    browser.search_state.searched_query = Some(browser.search_state.query.clone());
    browser.search_state.results = Some(
        versus::search_roots(
            &browser.sources,
            SearchScope::Both,
            &browser.search_state.query,
            None,
            &working,
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
    let ctx = egui::Context::default();
    set_theme(&ctx, egui::Theme::Dark);
    for _ in 0..3 {
        dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
    }
    // Replacing text with itself is a no-op, including for undo history.
    browser.search_state.replacement = "foo".into();
    let next = ctx.read_response(egui::Id::new("Replace Next")).unwrap();
    click_dialog(&mut browser, &mut view, &ctx, next.rect.center());
    assert_eq!(view.working_text(0), Some("foo foo foo\n"));
    assert!(!view.can_undo_batch());
    browser.search_state.replacement = "long value".into();
    for (action, expected_left, expected_right, remaining) in [
        ("Replace Next", "long value foo foo\n", "foo\n", 3),
        ("Replace Next", "long value long value foo\n", "foo\n", 2),
        (
            "Replace All",
            "long value long value long value\n",
            "long value\n",
            0,
        ),
    ] {
        let button = ctx.read_response(egui::Id::new(action)).unwrap();
        assert!(button.enabled());
        click_dialog(&mut browser, &mut view, &ctx, button.rect.center());
        assert_eq!(view.working_text(0), Some(expected_left));
        assert_eq!(view.working_text(1), Some(expected_right));
        assert_eq!(browser.search_state.selected_hits.len(), remaining);
        assert_eq!(
            browser.search_match_counts(),
            if remaining == 0 {
                [0, 0]
            } else {
                [remaining - 1, 1]
            }
        );
        let output = dialog_frame(&mut browser, &mut view, &ctx, vec![]);
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text().contains("Apply preview")
                    || text.galley.text().starts_with("Preview —")
                    || text.galley.text().contains("unsaved replacements")
                    || text.galley.text().contains("working copies")
                    || text.galley.text().contains("Save pending")
                    || text.galley.text().contains("Cancel save")
                    || text.galley.text().contains("matches in"))));
        output.drop_without_applying_deltas();
    }
    assert_eq!(view.working_text(1), Some("long value\n"));
    assert!(
        browser
            .search_state
            .results
            .as_ref()
            .unwrap()
            .files
            .is_empty()
    );
    assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "foo foo foo\n");
    assert_eq!(std::fs::read_to_string(&paths[1]).unwrap(), "foo\n");
    browser.undo_batch(&mut view, false);
    assert_eq!(view.working_text(1), Some("foo\n"));
    assert_eq!(view.working_text(0), Some("long value long value foo\n"));
    browser.undo_batch(&mut view, true);
    assert_eq!(
        view.working_text(0),
        Some("long value long value long value\n")
    );
    assert_eq!(view.working_text(1), Some("long value\n"));
    assert!(
        browser.search_state.job.is_some() && browser.search_state.due_at.is_none(),
        "Undo must refresh immediately"
    );
    assert!(
        browser.search_state.results.is_none(),
        "undo invalidates old result snapshots"
    );
    assert!(!view.run_diff_enabled());
    std::fs::remove_dir_all(root).unwrap();
}
