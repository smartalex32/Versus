use super::*;
use versus::{FileFingerprint, SearchEvent, SearchQuery, SearchResults, TextEncoding};

fn tree_entry(
    path: &str,
    kind: DirectoryEntryKind,
    state: DirectoryEntryState,
) -> versus::DirectoryEntry {
    let left_exists = !matches!(state, DirectoryEntryState::RightOnly);
    let right_exists = !matches!(state, DirectoryEntryState::LeftOnly);
    versus::DirectoryEntry {
        relative_path: PathBuf::from(path),
        left_exists,
        right_exists,
        left_kind: left_exists.then(|| kind.clone()),
        right_kind: right_exists.then(|| kind.clone()),
        left_size: None,
        right_size: None,
        kind,
        state,
    }
}

fn folder_tree(entries: Vec<versus::DirectoryEntry>) -> FolderTree {
    FolderTree::from_diff(&versus::DirectoryDiff {
        entries,
        cancelled: false,
    })
}

fn dialog_frame(
    browser: &mut FolderBrowser,
    view: &mut InteractiveView,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    dialog_frame_at(browser, view, ctx, events, None)
}
fn dialog_frame_at(
    browser: &mut FolderBrowser,
    view: &mut InteractiveView,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    time: Option<f64>,
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            events,
            time,
            ..Default::default()
        },
        |_| browser.search_window(ctx, view),
    )
}

#[test]
fn merged_states_reaggregate_nested_parents_and_preserve_intrinsic_states() {
    let mut mixed_type = tree_entry(
        "file-folder",
        DirectoryEntryKind::File,
        DirectoryEntryState::TypeMismatch,
    );
    mixed_type.left_kind = Some(DirectoryEntryKind::Directory);
    mixed_type.left_size = None;
    let tree = folder_tree(vec![
        mixed_type,
        tree_entry(
            "src",
            DirectoryEntryKind::Directory,
            DirectoryEntryState::Same,
        ),
        tree_entry(
            "src/changed.rs",
            DirectoryEntryKind::File,
            DirectoryEntryState::Different,
        ),
        tree_entry(
            "src/clean.rs",
            DirectoryEntryKind::File,
            DirectoryEntryState::Same,
        ),
        tree_entry(
            "only-left",
            DirectoryEntryKind::Directory,
            DirectoryEntryState::LeftOnly,
        ),
        tree_entry(
            "mismatch",
            DirectoryEntryKind::File,
            DirectoryEntryState::TypeMismatch,
        ),
        tree_entry(
            "only-right.txt",
            DirectoryEntryKind::File,
            DirectoryEntryState::RightOnly,
        ),
        tree_entry(
            "unreadable",
            DirectoryEntryKind::Directory,
            DirectoryEntryState::Error(CompareError::io(
                "unreadable",
                std::io::Error::other("denied"),
            )),
        ),
    ]);
    let mut overlays = BTreeMap::new();
    overlays.insert(PathBuf::from("src/changed.rs"), DirectoryEntryState::Same);
    let cache = build_merged_folder_cache(Some(&tree), &overlays, Some(1));
    assert_eq!(
        cache.states[std::path::Path::new("src")],
        DirectoryEntryState::Same,
        "a clean overlay must restore all nested parent badges"
    );
    assert_eq!(
        cache.states[std::path::Path::new("only-left")],
        DirectoryEntryState::LeftOnly
    );
    assert_eq!(
        cache.states[std::path::Path::new("mismatch")],
        DirectoryEntryState::TypeMismatch
    );
    assert_eq!(
        cache.states[std::path::Path::new("only-right.txt")],
        DirectoryEntryState::RightOnly
    );
    assert!(matches!(
        cache.states[std::path::Path::new("unreadable")],
        DirectoryEntryState::Error(_)
    ));
    assert_eq!(
        cache.difference_paths.as_slice(),
        [
            PathBuf::from("file-folder"),
            PathBuf::from("mismatch"),
            PathBuf::from("only-right.txt")
        ]
    );
    for terminal in [
        DirectoryEntryState::TypeMismatch,
        DirectoryEntryState::LeftOnly,
        DirectoryEntryState::RightOnly,
        DirectoryEntryState::Error(CompareError::io(
            "src",
            std::io::Error::other("overlay failed"),
        )),
    ] {
        overlays.insert(PathBuf::from("src"), terminal.clone());
        let cache = build_merged_folder_cache(Some(&tree), &overlays, Some(1));
        assert_eq!(
            cache.states[std::path::Path::new("src")],
            terminal,
            "a direct terminal overlay must survive parent aggregation"
        );
    }
}

#[test]
fn merged_state_cache_is_reused_for_large_identical_trees_until_invalidated() {
    let mut entries = Vec::new();
    for index in 0..8_000 {
        entries.push(tree_entry(
            &format!("dir-{index}"),
            DirectoryEntryKind::Directory,
            DirectoryEntryState::Same,
        ));
        entries.push(tree_entry(
            &format!("dir-{index}/file.txt"),
            DirectoryEntryKind::File,
            DirectoryEntryState::Same,
        ));
    }
    let tree = folder_tree(entries);
    let mut browser = FolderBrowser::new_search([None, None]);
    let first = browser.merged_folder_cache(Some(&tree)).states.clone();
    let first_paths = browser
        .merged_folder_cache(Some(&tree))
        .difference_paths
        .clone();
    let second = browser.merged_folder_cache(Some(&tree));
    assert!(Arc::ptr_eq(&first, &second.states));
    assert!(Arc::ptr_eq(&first_paths, &second.difference_paths));
    assert!(second.difference_paths.is_empty());
    browser.invalidate_comparison_cache();
    let rebuilt = browser.merged_folder_cache(Some(&tree));
    assert!(!Arc::ptr_eq(&first, &rebuilt.states));
}

#[test]
fn find_modal_has_centered_half_width_layout_and_a_styled_results_placeholder() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let ctx = egui::Context::default();
        set_theme(&ctx, theme);
        // Inspect the final colors rather than the Area's fade-in opacity.
        ctx.all_styles_mut(|style| style.animation_time = 0.0);
        let mut browser = FolderBrowser::new_with_sources([None, None]);
        let mut view = InteractiveView::default();
        browser.open_search();
        // Area sizing is invisible on its first pass; responses retain the
        // previous pass, so settle the anchored layout before inspecting it.
        dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
        dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
        let output = dialog_frame(&mut browser, &mut view, &ctx, vec![]);
        let modal = ctx
            .memory(|memory| memory.area_rect(egui::Id::new("folder-find-replace-modal")))
            .unwrap();
        assert!(modal.center().distance(ctx.content_rect().center()) < 1.0);
        assert!(
            (modal.width() - 600.0).abs() < 10.0 && modal.height() < 450.0,
            "{modal:?}"
        );
        let find = ctx
            .read_response(egui::Id::new(("folder-find-pattern", "frame")))
            .unwrap()
            .rect;
        let replacement = ctx
            .read_response(egui::Id::new(("folder-replacement-text", "frame")))
            .unwrap()
            .rect;
        assert!(find.width() > modal.width() / 2.0 - 30.0 && find.height() > 32.0);
        let results = ctx
            .read_response(egui::Id::new("folder-search-results"))
            .unwrap()
            .rect;
        assert!(results.left() >= modal.center().x && find.right() < results.left());
        assert!(modal.contains_rect(results));
        assert!(
            ctx.read_response(egui::Id::new("Find")).is_none(),
            "the Find action was removed"
        );
        assert!(replacement.height() > 32.0);
        for label in ["Match Case", "Match Whole Word", "Use Regular Expression"] {
            let control = ctx.read_response(egui::Id::new(label)).unwrap().rect;
            assert!(find.contains_rect(control));
            assert!(
                control.left()
                    > ctx
                        .read_response(egui::Id::new("folder-find-pattern"))
                        .unwrap()
                        .rect
                        .right()
            );
        }
        let next = ctx
            .read_response(egui::Id::new("Replace Next"))
            .unwrap()
            .rect;
        let all = ctx
            .read_response(egui::Id::new("Replace All"))
            .unwrap()
            .rect;
        assert!(next.left() > replacement.right() && all.left() > next.right());
        assert!(
            (next.center().y - replacement.center().y).abs() < 1.0,
            "next={next:?}, replacement={replacement:?}"
        );
        let palette = Palette::for_context(&ctx);
        assert!(results.width() > modal.width() / 2.0 - 40.0);
        assert!(results.height() > 300.0);
        assert!(
            output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(frame) if frame.fill == palette.panel
                    && frame.rect.contains_rect(results)
                    && (frame.rect.width() - results.width() - 18.0).abs() < 1.0)),
            "the painted panel must fill the right column even without results"
        );
        let left_view = ctx
            .read_response(egui::Id::new(("folder-search-view", 0usize)))
            .unwrap()
            .rect;
        let right_view = ctx
            .read_response(egui::Id::new(("folder-search-view", 1usize)))
            .unwrap()
            .rect;
        assert!((left_view.width() - right_view.width()).abs() < 1.0);
        assert!((left_view.left() - (results.left() - 9.0)).abs() < 1.0);
        assert!((right_view.right() - (results.right() + 9.0)).abs() < 1.0);
        for label in ["LEFT (0)", "RIGHT (0)"] {
            assert!(
                output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == label
                        && text.fallback_color == palette.text
                        && text.galley.rect.translate(text.pos.to_vec2()).center().y < results.top()
                        && text.pos.x > modal.center().x)),
                "result view toggle must be neutral"
            );
        }
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text() == "Search results will appear here"
                    && text.galley.job.sections[0].format.font_id == FontId::proportional(14.0)
                    && text.fallback_color == palette.muted
                    && results.contains_rect(text.galley.rect.translate(text.pos.to_vec2()))
                    && text.galley.rect.translate(text.pos.to_vec2()).center().distance(results.center()) < 1.0)));
        for (label, color) in [("LEFT", palette.left_only), ("RIGHT", palette.right_only)] {
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == label && text.fallback_color == color)),
                    "{label} expected={color:?}, actual={:?}", output.shapes.iter().filter_map(|shape|
                        if let egui::Shape::Text(text) = &shape.shape { (text.galley.text() == label).then_some(text.fallback_color) } else { None }).collect::<Vec<_>>());
        }
        for placeholder in ["Find", "Replace"] {
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == placeholder
                        && text.galley.job.sections[0].format.font_id.size == 14.0
                        && text.galley.job.sections[0].format.italics
                        && text.galley.job.sections[0].format.color == palette.muted.gamma_multiply(0.8))));
        }
        output.drop_without_applying_deltas();
        for label in ["Match Case", "Match Whole Word", "Use Regular Expression"] {
            let pos = ctx
                .read_response(egui::Id::new(label))
                .unwrap()
                .rect
                .center();
            for pressed in [true, false] {
                dialog_frame(
                    &mut browser,
                    &mut view,
                    &ctx,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                )
                .drop_without_applying_deltas();
            }
        }
        assert!(
            browser.search_state.query.case_sensitive
                && browser.search_state.query.whole_word
                && browser.search_state.query.regex
        );
        assert!(
            browser.search_state.open,
            "clicking controls inside the field must keep the modal open"
        );
        let outside = egui::pos2(modal.left() - 10.0, modal.center().y);
        for pressed in [true, false] {
            dialog_frame(
                &mut browser,
                &mut view,
                &ctx,
                vec![
                    egui::Event::PointerMoved(outside),
                    egui::Event::PointerButton {
                        pos: outside,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            )
            .drop_without_applying_deltas();
        }
        assert!(!browser.search_state.open);
    }
}

#[test]
fn find_modal_columns_and_controls_fit_at_normal_and_high_zoom() {
    for zoom in [1.0, 2.0] {
        let ctx = egui::Context::default();
        set_theme(&ctx, egui::Theme::Light);
        ctx.set_zoom_factor(zoom);
        let mut browser = FolderBrowser::new_search([None, None]);
        let mut view = InteractiveView::default();
        browser.open_search();
        for _ in 0..6 {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        Pos2::ZERO,
                        egui::vec2(900.0, 800.0) / zoom,
                    )),
                    ..Default::default()
                },
                |_| browser.search_window(&ctx, &mut view),
            )
            .drop_without_applying_deltas();
        }
        let modal = ctx
            .memory(|memory| memory.area_rect(egui::Id::new("folder-find-replace-modal")))
            .unwrap();
        assert!(
            ctx.content_rect().contains_rect(modal),
            "zoom={zoom}: {modal:?}"
        );
        let results = ctx
            .read_response(egui::Id::new("folder-search-results"))
            .unwrap()
            .rect;
        for label in [
            "Match Case",
            "Match Whole Word",
            "Use Regular Expression",
            "Replace Next",
            "Replace All",
        ] {
            let control = ctx.read_response(egui::Id::new(label)).unwrap().rect;
            assert!(
                modal.contains_rect(control) && control.right() < results.left(),
                "zoom={zoom}: {label}"
            );
        }
        let replace = ctx
            .read_response(egui::Id::new(("folder-replacement-text", "frame")))
            .unwrap()
            .rect;
        let both = ctx
            .read_response(egui::Id::new(("folder-search-scope", "BOTH")))
            .unwrap()
            .rect;
        assert!(
            both.top() > replace.bottom(),
            "scope must not overlap Replace"
        );
        let actions: Vec<_> = [
            "Undo replacement batch",
            "Redo replacement batch",
            "Refresh search",
        ]
        .into_iter()
        .map(|label| ctx.read_response(egui::Id::new(label)).unwrap().rect)
        .collect();
        assert!(
            actions.iter().all(|rect| modal.contains_rect(*rect)),
            "zoom={zoom} modal={modal:?} actions={actions:?}"
        );
        assert!(
            actions
                .windows(2)
                .all(|pair| pair[1].left() > pair[0].right())
        );
        if zoom == 1.0 {
            assert!(actions[0].left() > both.right());
            assert!((actions[0].center().y - both.center().y).abs() < 1.0);
        }
    }
}

fn streamed_file(side: usize, id: u64, text: &str) -> versus::SearchFileResults {
    let path = PathBuf::from(format!("side-{side}.txt"));
    let range = text.find("match").unwrap()..text.find("match").unwrap() + 5;
    versus::SearchFileResults {
        side,
        path: path.clone(),
        working_file: WorkingFile {
            identity: None,
            path: Some(path.clone()),
            text: text.into(),
            encoding: TextEncoding::Utf8 { bom: false },
            fingerprint: FileFingerprint::Bytes(text.as_bytes().to_vec()),
        },
        hits: vec![versus::SearchHit {
            id,
            side,
            path,
            range,
            line: 1,
            matched: "match".into(),
        }],
    }
}

fn click_dialog(
    browser: &mut FolderBrowser,
    view: &mut InteractiveView,
    ctx: &egui::Context,
    pos: Pos2,
) {
    for pressed in [true, false] {
        dialog_frame(
            browser,
            view,
            ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        )
        .drop_without_applying_deltas();
    }
}

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
