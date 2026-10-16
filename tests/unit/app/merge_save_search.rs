//! Merge save search regression coverage.

use super::*;
use crate::app::visuals::theme::{Palette, apply_theme};

#[test]
fn merge_pane_headers_place_back_before_labels_and_save_after_browse() {
    let fixture = SourceFixture::new();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    app.session.set_workspace_merge(true);
    app.session.select_path(0, fixture.path("left"));
    app.session.select_path(1, fixture.path("right"));
    settle_sources(&mut app, &ctx);
    app.session.merge_file_sides = [true; 2];
    app.session.merge_file_open = true;
    app.session.interactive.as_mut().unwrap().open_pair([
        fixture.path("left/model.txt"),
        fixture.path("right/model.txt"),
    ]);
    settle_merge_file(&mut app, &ctx, 0, "common\nold\n");
    let output = render(&mut app, &ctx, vec![]);
    for side in 0..2 {
        let back = ctx
            .read_response(egui::Id::new(format!(
                "Back to {} folder",
                if side == 0 { "left" } else { "right" }
            )))
            .unwrap()
            .rect;
        let label = ctx
            .read_response(egui::Id::new(("merge-side-label", side)))
            .unwrap()
            .rect;
        let browse = ctx
            .read_response(egui::Id::new(format!(
                "Browse {} file",
                if side == 0 { "left" } else { "right" }
            )))
            .unwrap()
            .rect;
        let save = ctx
            .read_response(egui::Id::new(if side == 0 {
                "Save Left"
            } else {
                "Save Right"
            }))
            .unwrap()
            .rect;
        assert!(back.right() < label.left());
        assert!(browse.right() < save.left());
        assert!((back.center().y - label.center().y).abs() < 0.5);
        assert!((browse.center().y - save.center().y).abs() < 0.5);
    }
    assert!(
        ctx.read_response(egui::Id::new("Back to folders"))
            .is_none()
    );
    assert!(text_positions(&output, "Refresh listing").is_empty());
    assert!(text_positions(&output, "Open selected pair").is_empty());
    let before_peer = app
        .session
        .interactive
        .as_ref()
        .unwrap()
        .working_text(1)
        .unwrap()
        .to_owned();
    output.drop_without_applying_deltas();
    click_action(&mut app, &ctx, "Back to left folder");
    assert_eq!(app.session.merge_file_sides, [false, true]);
    assert_eq!(
        app.session.interactive.as_ref().unwrap().working_text(1),
        Some(before_peer.as_str())
    );
}

#[test]
fn merge_save_icons_preview_only_the_chosen_file() {
    let fixture = SourceFixture::new();
    for side in 0..2 {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = VersusApp::default();
        app.session.comparison_mode = ComparisonMode::File;
        app.session.set_workspace_merge(true);
        app.session.select_path(
            side,
            fixture.path(if side == 0 {
                "left/model.txt"
            } else {
                "right/model.txt"
            }),
        );
        let original = if side == 0 {
            "common\nold\n"
        } else {
            "common\nnew\n"
        };
        settle_merge_file(&mut app, &ctx, side, original);
        render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
        let pos = egui::pos2(
            app.pane_rects[side].left() + 65.0,
            app.pane_rects[side].top() + 8.0,
        );
        click(&mut app, &ctx, pos);
        render(&mut app, &ctx, vec![egui::Event::Text("edited ".into())])
            .drop_without_applying_deltas();
        assert!(
            app.session
                .interactive
                .as_ref()
                .unwrap()
                .is_side_dirty(side)
        );
        let output = render(&mut app, &ctx, vec![]);
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text().ends_with("model.txt")
                && app.source_header_rects[side].contains(text.pos)
                && text.override_text_color == Some(Palette::for_context(&ctx).changed))));
        output.drop_without_applying_deltas();
        let save = if side == 0 { "Save Left" } else { "Save Right" };
        let mut enabled = false;
        // Read during the pass: egui swaps this/previous widget maps at
        // end_pass, so an outside read can return the older disabled state.
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                ..Default::default()
            },
            |ui| {
                app.render(ui);
                enabled = ctx.read_response(egui::Id::new(save)).unwrap().enabled();
            },
        );
        output.drop_without_applying_deltas();
        assert!(enabled);
        click_action(&mut app, &ctx, save);
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(text_positions(&output, "Save pending changes?").len(), 1);
        assert_eq!(
            text_positions(
                &output,
                if side == 0 {
                    "Left working copy"
                } else {
                    "Right working copy"
                }
            )
            .len(),
            1
        );
        assert!(
            text_positions(
                &output,
                if side == 0 {
                    "Right working copy"
                } else {
                    "Left working copy"
                }
            )
            .is_empty()
        );
        assert_eq!(
            std::fs::read_to_string(fixture.path(if side == 0 {
                "left/model.txt"
            } else {
                "right/model.txt"
            }))
            .unwrap(),
            original
        );
        output.drop_without_applying_deltas();
    }
}

#[test]
fn file_merge_search_opens_without_creating_a_folder_browser() {
    let fixture = SourceFixture::new();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    app.session.comparison_mode = ComparisonMode::File;
    app.session.set_workspace_merge(true);

    // Search remains available before files are chosen, then tracks an
    // independently loaded side without replacing its editor with a tree.
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    assert!(
        ctx.read_response(egui::Id::new("Find and replace"))
            .is_some()
    );
    click_action(&mut app, &ctx, "Find and replace");
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "Find and Replace").len(), 1);
    assert!(app.session.merge_folders.is_none());
    assert_eq!(
        app.session
            .merge_file_search
            .as_ref()
            .unwrap()
            .source_roots(),
        [None, None]
    );
    output.drop_without_applying_deltas();

    app.session.select_path(0, fixture.path("left/model.txt"));
    settle_merge_file(&mut app, &ctx, 0, "common\nold\n");
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    assert!(app.session.merge_folders.is_none());
    assert!(app.session.merge_file_open);
    assert_eq!(
        app.session
            .merge_file_search
            .as_ref()
            .unwrap()
            .source_roots(),
        [Some(fixture.path("left/model.txt")), None]
    );
}

#[test]
fn editor_save_clears_the_file_search_snapshot_before_leave_guards() {
    let fixture = SourceFixture::new();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    app.session.comparison_mode = ComparisonMode::File;
    app.session.set_workspace_merge(true);
    app.session.select_path(0, fixture.path("left/model.txt"));
    settle_merge_file(&mut app, &ctx, 0, "common\nold\n");

    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    let editor = ctx
        .read_response(app.session.interactive.as_ref().unwrap().editor_id(0))
        .expect("loaded merge editor")
        .rect
        .left_top()
        + egui::vec2(8.0, 8.0);
    click(&mut app, &ctx, editor);
    render(&mut app, &ctx, vec![egui::Event::Text("edited ".into())])
        .drop_without_applying_deltas();
    assert!(app.session.interactive.as_ref().unwrap().any_dirty());

    click_action(&mut app, &ctx, "Find and replace");
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    assert!(app.session.merge_file_search.as_ref().unwrap().any_dirty());
    click_action(&mut app, &ctx, "Close Find and Replace");

    click_action(&mut app, &ctx, "Save Left");
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    let output = render(&mut app, &ctx, vec![]);
    let save = text_centers(&output, "Save")[0];
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, save);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.session.interactive.as_ref().unwrap().is_saving() {
        app.session.interactive.as_mut().unwrap().poll(&ctx);
        assert!(Instant::now() < deadline, "editor save did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
    app.session.sync_file_search_when_clean();
    assert!(!app.session.interactive.as_ref().unwrap().any_dirty());
    assert!(!app.session.merge_file_search.as_ref().unwrap().any_dirty());
    app.session.new_comparison();
    assert!(app.session.pending_merge_exit.is_none());
}

#[test]
fn file_search_owner_is_retained_while_folder_merge_opens() {
    let fixture = SourceFixture::new();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    app.session.comparison_mode = ComparisonMode::File;
    app.session.set_workspace_merge(true);
    app.session.select_path(0, fixture.path("left/model.txt"));
    settle_merge_file(&mut app, &ctx, 0, "common\nold\n");

    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    let editor = ctx
        .read_response(app.session.interactive.as_ref().unwrap().editor_id(0))
        .expect("loaded merge editor")
        .rect
        .left_top()
        + egui::vec2(8.0, 8.0);
    click(&mut app, &ctx, editor);
    render(&mut app, &ctx, vec![egui::Event::Text("edited ".into())])
        .drop_without_applying_deltas();
    click_action(&mut app, &ctx, "Find and replace");
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    assert!(app.session.merge_file_search.as_ref().unwrap().any_dirty());

    app.session.select_path(0, fixture.path("left"));
    settle_sources(&mut app, &ctx);
    assert!(app.session.merge_folders.is_some());
    assert!(
        app.session.merge_file_search.is_some(),
        "a pending file-search save or working snapshot must survive the source transition"
    );
}
