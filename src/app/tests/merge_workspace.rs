//! Merge workspace regression coverage.

use super::*;
use crate::app::visuals::theme::{apply_theme, set_theme};

#[test]
fn merge_run_diff_checkbox_uses_pointer_cursor() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    app.session.set_workspace_merge(true);
    let output = render(&mut app, &ctx, vec![]);
    let position = text_centers(&output, "Run Diff")[0];
    output.drop_without_applying_deltas();
    let output = render(&mut app, &ctx, vec![egui::Event::PointerMoved(position)]);
    assert_eq!(
        output.platform_output.cursor_icon,
        egui::CursorIcon::PointingHand
    );
    assert!(!app.session.interactive.as_ref().unwrap().run_diff_enabled());
    output.drop_without_applying_deltas();
}

#[test]
fn merge_file_content_top_padding_matches_native_file_diff() {
    let fixture = SourceFixture::new();
    std::fs::write(fixture.path("left/model.txt"), "left text\n").unwrap();
    std::fs::write(fixture.path("right/model.txt"), "right text\n").unwrap();
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for zoom in [1.0, 2.0] {
            let ctx = egui::Context::default();
            set_theme(&ctx, theme);
            ctx.set_zoom_factor(zoom);
            let mut merge = VersusApp::default();
            merge.session.comparison_mode = ComparisonMode::File;
            merge.session.set_workspace_merge(true);
            merge.session.select_path(0, fixture.path("left/model.txt"));
            merge
                .session
                .select_path(1, fixture.path("right/model.txt"));
            settle_merge_file(&mut merge, &ctx, 0, "left text\n");
            settle_merge_file(&mut merge, &ctx, 1, "right text\n");
            for _ in 0..3 {
                render(&mut merge, &ctx, vec![]).drop_without_applying_deltas();
            }
            let output = render(&mut merge, &ctx, vec![]);
            let merge_offset =
                text_positions(&output, "left text\n")[0].y - merge.source_header_rects[0].bottom();
            output.drop_without_applying_deltas();
            let mut diff = VersusApp::default();
            diff.session.comparison_mode = ComparisonMode::File;
            diff.session.select_path(0, fixture.path("left/model.txt"));
            diff.session.select_path(1, fixture.path("right/model.txt"));
            settle_sources(&mut diff, &ctx);
            let output = render(&mut diff, &ctx, vec![]);
            let diff_offset =
                text_positions(&output, "left text")[0].y - diff.source_header_rects[0].bottom();
            assert!(
                (merge_offset - diff_offset).abs() < 0.6,
                "merge={merge_offset}, diff={diff_offset}, theme={theme:?}, zoom={zoom}"
            );
            output.drop_without_applying_deltas();
        }
    }
}

#[test]
fn merge_file_opens_on_either_side_before_a_peer_is_selected() {
    let fixture = SourceFixture::new();
    for side in 0..2 {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = VersusApp::default();
        app.session.comparison_mode = ComparisonMode::File;
        app.session.set_workspace_merge(true);
        let path = fixture.path(if side == 0 {
            "left/model.txt"
        } else {
            "right/model.txt"
        });
        app.session.select_path(side, path.clone());
        let text = if side == 0 {
            "common\nold\n"
        } else {
            "common\nnew\n"
        };
        settle_merge_file(&mut app, &ctx, side, text);
        let output = render(&mut app, &ctx, vec![]);
        assert!(app.session.merge_file_sides[side] && !app.session.merge_file_sides[1 - side]);
        assert_eq!(
            app.session.interactive.as_ref().unwrap().current_sources()[side],
            Some(path)
        );
        assert_eq!(
            text_positions(&output, "Drop a file or folder here").len(),
            1
        );
        assert!(!text_positions(&output, text).is_empty());
        assert!(app.session.job.is_none() && app.session.tree.is_none());
        assert!(
            !ctx.read_response(egui::Id::new("Accept All Left — replace Right with Left"))
                .unwrap()
                .enabled()
        );
        assert!(text_positions(&output, "Comparison alignment").is_empty());
        assert!(text_positions(&output, "Sync scroll").is_empty());
        assert!(
            text_positions(
                &output,
                "Working copies are ready. Changes stay in memory until saved."
            )
            .is_empty()
        );
        output.drop_without_applying_deltas();
    }
}

#[test]
fn merge_side_classification_does_not_wait_for_a_pending_or_failed_peer() {
    let fixture = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.session.set_workspace_merge(true);
    let (sender, receiver) = mpsc::channel();
    app.session.source_jobs[1] = Some(SourceJob {
        receiver,
        cancellation: Arc::new(AtomicBool::new(false)),
    });
    app.session.select_path(0, fixture.path("left/model.txt"));
    settle_merge_file(&mut app, &ctx, 0, "common\nold\n");
    assert!(app.session.source_jobs[1].is_some());
    sender.send(Err("Unavailable source".into())).unwrap();
    app.session.poll_sources(&ctx);
    assert!(app.session.source_errors[1].is_some());
    assert_eq!(
        app.session.interactive.as_ref().unwrap().working_text(0),
        Some("common\nold\n")
    );
    assert!(app.session.merge_file_sides[0]);
}

#[test]
fn merge_folder_and_file_viewers_can_be_loaded_independently() {
    let fixture = SourceFixture::new();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    app.session.set_workspace_merge(true);
    app.session.select_path(0, fixture.path("left"));
    settle_sources(&mut app, &ctx);
    assert_eq!(
        app.session.merge_folders.as_ref().unwrap().source_roots(),
        [Some(fixture.path("left")), None]
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let output = render(&mut app, &ctx, vec![]);
        let loaded = text_positions(&output, "model.txt").len() == 1;
        if loaded {
            assert_eq!(
                text_positions(&output, "Drop a file or folder here").len(),
                1
            );
        }
        output.drop_without_applying_deltas();
        if loaded {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    app.session.select_path(1, fixture.path("right/model.txt"));
    settle_merge_file(&mut app, &ctx, 1, "common\nnew\n");
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(app.session.merge_file_sides, [false, true]);
    assert_eq!(text_positions(&output, "model.txt").len(), 1);
    assert!(!text_positions(&output, "common\nnew\n").is_empty());
    assert!(app.session.job.is_none() && app.session.tree.is_none());
    output.drop_without_applying_deltas();
}

#[test]
fn merge_folder_expansion_and_scrolling_are_independent() {
    let fixture = SourceFixture::new();
    for side in ["left", "right"] {
        std::fs::create_dir(fixture.path(&format!("{side}/assembly"))).unwrap();
        std::fs::write(fixture.path(&format!("{side}/assembly/child.txt")), "child").unwrap();
        for index in 0..100 {
            std::fs::write(
                fixture.path(&format!("{side}/row-{index:03}.txt")),
                "metadata",
            )
            .unwrap();
        }
    }
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    app.session.set_workspace_merge(true);
    app.session.select_path(0, fixture.path("left"));
    app.session.select_path(1, fixture.path("right"));
    settle_sources(&mut app, &ctx);
    let deadline = Instant::now() + Duration::from_secs(5);
    let folders = loop {
        let output = render(&mut app, &ctx, vec![]);
        let folders = text_positions(&output, "assembly");
        output.drop_without_applying_deltas();
        if folders.len() == 2 {
            break folders;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    };
    click(&mut app, &ctx, folders[0] + egui::vec2(45.0, 3.0));
    let deadline = Instant::now() + Duration::from_secs(5);
    let (left_row, peer_row) = loop {
        let output = render(&mut app, &ctx, vec![]);
        let children = text_positions(&output, "child.txt");
        let peer_row = text_positions(&output, "row-000.txt")
            .into_iter()
            .find(|pos| pos.x > app.pane_rects[1].left());
        if children.len() == 1 {
            assert!(children[0].x < app.pane_rects[0].right());
            let left = text_positions(&output, "row-000.txt")
                .into_iter()
                .find(|pos| pos.x < app.pane_rects[0].right())
                .unwrap();
            output.drop_without_applying_deltas();
            break (left, peer_row.unwrap());
        }
        output.drop_without_applying_deltas();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    };
    let pos = app.pane_rects[0].center();
    render(
        &mut app,
        &ctx,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                phase: egui::TouchPhase::Move,
                delta: egui::vec2(0.0, -160.0),
                modifiers: Default::default(),
            },
        ],
    )
    .drop_without_applying_deltas();
    let output = render(&mut app, &ctx, vec![]);
    let peer_after = text_positions(&output, "row-000.txt")
        .into_iter()
        .find(|pos| pos.x > app.pane_rects[1].left())
        .unwrap();
    assert_eq!(peer_after, peer_row);
    let left_after = text_positions(&output, "row-000.txt")
        .into_iter()
        .find(|pos| pos.x < app.pane_rects[0].right());
    assert!(
        left_after.is_none_or(|pos| pos.y < left_row.y - 1.0),
        "Left tree did not scroll: before={left_row:?}, after={left_after:?}"
    );
    assert!(app.session.job.is_none() && app.session.tree.is_none());
    output.drop_without_applying_deltas();
}

#[test]
fn merge_run_diff_checkbox_is_before_theme_in_the_header() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for zoom in [1.0, 2.0] {
            let ctx = egui::Context::default();
            apply_theme(&ctx);
            set_theme(&ctx, theme);
            ctx.set_zoom_factor(zoom);
            let mut app = VersusApp::default();
            app.session.set_workspace_merge(true);
            let output = render(&mut app, &ctx, vec![]);
            let run = text_positions(&output, "Run Diff");
            assert_eq!(run.len(), 1);
            let theme_button = ctx
                .read_response(egui::Id::new(if theme == egui::Theme::Dark {
                    "Switch to light mode"
                } else {
                    "Switch to dark mode"
                }))
                .unwrap()
                .rect;
            let mode = ctx.read_response(egui::Id::new("File Diff")).unwrap().rect;
            assert!(
                run[0].x > mode.right(),
                "Run Diff overlaps the centered file button"
            );
            assert!(run[0].x < theme_button.left());
            assert!(run[0].y >= mode.top() && run[0].y < mode.bottom());
            output.drop_without_applying_deltas();
            click(&mut app, &ctx, run[0] + egui::vec2(3.0, 3.0));
            assert!(app.session.interactive.as_ref().unwrap().run_diff_enabled());
            assert_eq!(
                app.session.interactive.as_ref().unwrap().merge_options(),
                [true; 3]
            );
            assert!(
                app.session.show_only_differences
                    && app.session.ignore_whitespace
                    && app.session.ignore_line_endings
            );
            render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
            assert!(
                ctx.read_response(egui::Id::new("Auto Merge Non-Conflicts"))
                    .is_none()
            );
        }
    }
}

#[test]
fn merge_diff_buttons_align_with_the_diff_toolbar() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let ctx = egui::Context::default();
        set_theme(&ctx, theme);
        let mut app = VersusApp::default();
        render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
        let labels = [
            "Zoom out",
            "Zoom in",
            "Show only differences",
            "Ignore whitespace",
            "Ignore line endings",
            "Previous difference",
            "Next difference",
        ];
        let before: Vec<_> = labels
            .iter()
            .map(|label| ctx.read_response(egui::Id::new(*label)).unwrap().rect)
            .collect();
        app.session.set_workspace_merge(true);
        app.session.interactive.as_mut().unwrap().set_run_diff(true);
        render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
        render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
        for (label, expected) in labels.into_iter().zip(before) {
            let actual = ctx.read_response(egui::Id::new(label)).unwrap().rect;
            assert!(
                (actual.left() - expected.left()).abs() < 0.5
                    && (actual.top() - expected.top()).abs() < 0.5,
                "{label}: Diff={expected:?}, Merge={actual:?}"
            );
        }
    }
}
