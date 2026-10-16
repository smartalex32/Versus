//! Immediate Find and Replace interactions and result-stream regressions.

use super::*;
use crate::app::visuals::theme::set_theme;

#[test]
fn scope_buttons_immediately_filter_cached_results_and_limit_replacement_targets() {
    let ctx = egui::Context::default();
    set_theme(&ctx, egui::Theme::Dark);
    let mut browser = FolderBrowser::new_search([None, None]);
    let mut view = InteractiveView::default();
    browser.open_search();
    browser.search_state.query = SearchQuery::plain("match");
    browser.search_state.searched_query = Some(browser.search_state.query.clone());
    browser.search_state.replacement = "changed".into();
    browser.search_state.results = Some(SearchResults {
        files: vec![
            streamed_file(0, 1, "left match"),
            streamed_file(1, 2, "right match"),
        ],
        ..Default::default()
    });
    browser.search_state.selected_hits = BTreeSet::from([1, 2]);
    for _ in 0..3 {
        dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
    }
    assert_eq!(
        browser
            .replacement_plan(&view, false)
            .unwrap()
            .changes
            .len(),
        2
    );
    for (label, side, path) in [
        ("LEFT", 0usize, "side-0.txt"),
        ("RIGHT", 1usize, "side-1.txt"),
    ] {
        let button = ctx
            .read_response(egui::Id::new(("folder-search-scope", label)))
            .unwrap()
            .rect;
        click_dialog(&mut browser, &mut view, &ctx, button.center());
        assert_eq!(browser.search_state.results_side, side);
        assert!(browser.search_state.job.is_none() && browser.search_state.due_at.is_none());
        assert!(browser.search_state.results_current());
        assert_eq!(
            browser.search_state.results.as_ref().unwrap().files.len(),
            2
        );
        assert_eq!(browser.search_state.selected_hits, BTreeSet::from([1, 2]));
        dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
        let other = ctx
            .read_response(egui::Id::new(("folder-search-view", 1 - side)))
            .unwrap();
        assert!(!other.enabled());
        click_dialog(&mut browser, &mut view, &ctx, other.rect.center());
        assert_eq!(
            browser.search_state.results_side, side,
            "disabled toggle cannot change the results side"
        );
        let plan = browser.replacement_plan(&view, false).unwrap();
        assert_eq!(plan.changes.len(), 1);
        assert_eq!(plan.changes[0].path, PathBuf::from(path));
    }
    let both = ctx
        .read_response(egui::Id::new(("folder-search-scope", "BOTH")))
        .unwrap()
        .rect;
    click_dialog(&mut browser, &mut view, &ctx, both.center());
    dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
    for side in 0..2usize {
        assert!(
            ctx.read_response(egui::Id::new(("folder-search-view", side)))
                .unwrap()
                .enabled()
        );
    }
    let left = ctx
        .read_response(egui::Id::new(("folder-search-view", 0usize)))
        .unwrap()
        .rect;
    click_dialog(&mut browser, &mut view, &ctx, left.center());
    assert_eq!(browser.search_state.results_side, 0);
    assert!(browser.search_state.results_current() && browser.search_state.job.is_none());
    assert_eq!(
        browser
            .replacement_plan(&view, false)
            .unwrap()
            .changes
            .len(),
        2
    );
    browser.search_state.scope = SearchScope::Left;
    browser.set_search_sources([None, Some(PathBuf::from("right-only.txt"))]);
    assert_eq!(
        browser.search_state.results_side, 0,
        "source changes must not select a disabled results side"
    );
}

#[test]
fn button_actions_bypass_typing_delay_and_scope_changes_keep_the_active_search() {
    let ctx = egui::Context::default();
    set_theme(&ctx, egui::Theme::Dark);
    let mut browser = FolderBrowser::new_search([None, None]);
    let mut view = InteractiveView::default();
    browser.open_search();
    for _ in 0..3 {
        dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
    }
    browser.search_state.query.pattern = "match".into();
    browser.schedule_search(100.0);
    let left = ctx
        .read_response(egui::Id::new(("folder-search-scope", "LEFT")))
        .unwrap()
        .rect;
    click_dialog(&mut browser, &mut view, &ctx, left.center());
    assert!(browser.search_state.due_at.is_none() && browser.search_state.job.is_some());
    assert_eq!(
        browser.search_state.searched_query,
        Some(browser.search_state.query.clone())
    );
    let cancellation = browser.search_state.job.as_ref().unwrap().cancel.clone();
    let right = ctx
        .read_response(egui::Id::new(("folder-search-scope", "RIGHT")))
        .unwrap()
        .rect;
    click_dialog(&mut browser, &mut view, &ctx, right.center());
    assert!(Arc::ptr_eq(
        &browser.search_state.job.as_ref().unwrap().cancel,
        &cancellation
    ));
    assert!(!cancellation.load(Ordering::Relaxed));
    for label in ["Match Case", "Match Whole Word", "Use Regular Expression"] {
        browser.schedule_search(100.0);
        let button = ctx.read_response(egui::Id::new(label)).unwrap().rect;
        click_dialog(&mut browser, &mut view, &ctx, button.center());
        assert!(
            browser.search_state.due_at.is_none() && browser.search_state.job.is_some(),
            "{label} should search immediately"
        );
        assert_eq!(
            browser.search_state.searched_query,
            Some(browser.search_state.query.clone())
        );
    }
    let cancellation = browser.search_state.job.as_ref().unwrap().cancel.clone();
    browser.search_state.query.pattern.clear();
    let button = ctx.read_response(egui::Id::new("Match Case")).unwrap().rect;
    click_dialog(&mut browser, &mut view, &ctx, button.center());
    assert!(cancellation.load(Ordering::Relaxed));
    assert!(
        browser.search_state.job.is_none()
            && browser.search_state.due_at.is_none()
            && browser.search_state.results.is_none()
    );
}

#[test]
fn every_worker_searches_both_sides_regardless_of_replacement_scope() {
    let root =
        std::env::temp_dir().join(format!("versus-always-both-search-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let paths = [root.join("left.txt"), root.join("right.txt")];
    for path in &paths {
        std::fs::write(path, "match\n").unwrap();
    }
    let ctx = egui::Context::default();
    let view = InteractiveView::default();
    let mut browser = FolderBrowser::new_search(paths.map(Some));
    browser.search_state.query = SearchQuery::plain("match");
    for scope in [SearchScope::Left, SearchScope::Right, SearchScope::Both] {
        browser.search_state.scope = scope;
        browser.start_search(&view);
        let deadline = Instant::now() + Duration::from_secs(2);
        while browser.search_state.job.is_some() && Instant::now() < deadline {
            browser.poll(&ctx);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(browser.search_state.results_current());
        let sides: BTreeSet<_> = browser
            .search_state
            .results
            .as_ref()
            .unwrap()
            .files
            .iter()
            .map(|file| file.side)
            .collect();
        assert_eq!(sides, BTreeSet::from([0, 1]));
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn streamed_results_are_visible_before_completion_and_exclusions_survive_view_switches() {
    let ctx = egui::Context::default();
    set_theme(&ctx, egui::Theme::Dark);
    let mut browser = FolderBrowser::new_search([None, None]);
    let mut view = InteractiveView::default();
    browser.open_search();
    browser.search_state.query = SearchQuery::plain("match");
    browser.search_state.scope = SearchScope::Both;
    browser.search_state.searched_query = Some(browser.search_state.query.clone());
    let (sender, receiver) = mpsc::channel();
    browser.search_state.job = Some(SearchJob {
        receiver,
        cancel: Arc::new(AtomicBool::new(false)),
    });
    let left = streamed_file(0, 0, "left match\nlater match");
    let remove_id = egui::Id::new((
        "remove-search-result",
        left.side,
        left.hits[0].id,
        &left.path,
    ));
    sender.send(Ok(SearchEvent::File(left))).unwrap();
    browser.poll(&ctx);
    assert!(browser.search_state.job.is_some());
    assert_eq!(browser.search_match_counts(), [1, 0]);
    assert!(
        !browser.search_state.results_current(),
        "partial output cannot authorize replacements"
    );
    for _ in 0..3 {
        dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
    }
    let remove = ctx
        .read_response(remove_id)
        .expect("streamed match should already be visible")
        .rect;
    click_dialog(&mut browser, &mut view, &ctx, remove.center());
    assert!(!browser.search_state.selected_hits.contains(&0));
    assert_eq!(browser.search_match_counts(), [0, 0]);
    sender
        .send(Ok(SearchEvent::Hits {
            path: PathBuf::from("side-0.txt"),
            side: 0,
            hits: vec![versus::SearchHit {
                id: 2,
                side: 0,
                path: PathBuf::from("side-0.txt"),
                range: 17..22,
                line: 2,
                matched: "match".into(),
            }],
        }))
        .unwrap();
    sender
        .send(Ok(SearchEvent::File(streamed_file(1, 1, "right match"))))
        .unwrap();
    sender
        .send(Ok(SearchEvent::Complete { cancelled: false }))
        .unwrap();
    browser.poll(&ctx);
    assert!(browser.search_state.results_current());
    assert_eq!(browser.search_state.selected_hits, BTreeSet::from([1, 2]));
    assert_eq!(browser.search_match_counts(), [1, 1]);
    assert_eq!(
        browser.search_state.results.as_ref().unwrap().files[0]
            .hits
            .len(),
        2
    );
    let right_toggle = ctx
        .read_response(egui::Id::new(("folder-search-view", 1usize)))
        .unwrap()
        .rect;
    click_dialog(&mut browser, &mut view, &ctx, right_toggle.center());
    assert_eq!(browser.search_state.results_side, 1);
    assert_eq!(browser.search_state.scope, SearchScope::Both);
    assert!(
        browser.search_state.results_current(),
        "display filter must not restart search"
    );
    assert_eq!(
        browser.search_state.results.as_ref().unwrap().files.len(),
        2
    );
    let output = dialog_frame(&mut browser, &mut view, &ctx, vec![]);
    for label in ["LEFT (1)", "RIGHT (1)", "side-1.txt"] {
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text() == label)));
    }
    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text().contains("matches in"))));
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "1: right match")));
    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "1: left match")));
    output.drop_without_applying_deltas();
}
