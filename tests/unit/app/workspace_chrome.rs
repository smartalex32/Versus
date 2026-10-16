//! Workspace chrome regression coverage.

use super::*;
use crate::app::visuals::footer::{DIFF_LEGEND_ICONS, progress_text};
use crate::app::visuals::status::format_size;
use crate::app::visuals::status::{StatusIcon, state_color, status_icon};
use crate::app::visuals::theme::{Palette, apply_theme, set_theme};

#[test]
fn minimum_window_keeps_controls_and_footer_inside_the_viewport() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 650.0))),
            ..Default::default()
        },
        |ui| app.render(ui),
    );
    output.textures_delta.clear();
    let controls: Vec<_> = ["Collapse all", "Expand all", "Refresh comparison"]
        .into_iter()
        .map(|label| ctx.read_response(egui::Id::new(label)).unwrap().rect)
        .collect();
    assert!(
        controls
            .iter()
            .all(|rect| rect.right() <= 890.0 && rect.left() > 700.0 && rect.bottom() <= 45.0)
    );
    for label in [
        "Show only differences",
        "Ignore whitespace",
        "Ignore line endings",
        "Previous difference",
        "Next difference",
        "New comparison",
    ] {
        let rect = ctx.read_response(egui::Id::new(label)).unwrap().rect;
        assert!(
            rect.left() >= 0.0 && rect.right() <= 890.0,
            "{label}: {rect:?}"
        );
        assert!(rect.top() >= 45.0 && rect.bottom() < app.pane_rects[0].top());
    }
    for text in ["Choose a folder on each side to begin."] {
        let positions = text_positions(&output, text);
        assert!(!positions.is_empty(), "Missing {text}");
        assert!(
            positions.iter().all(|position| position.x >= 0.0
                && position.x < 900.0
                && position.y >= 0.0
                && position.y < 630.0),
            "{text} outside viewport: {positions:?}"
        );
    }
    output.drop_without_applying_deltas();
}

#[test]
fn selected_item_guidance_is_visible_at_bottom_left() {
    for (width, height) in [(900.0, 650.0), (1200.0, 800.0)] {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.session.tree.as_mut().unwrap().expand_all();
        let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(width, height));
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ui| app.render(ui),
        );
        output.textures_delta.clear();
        let file = text_positions(&output, "model.step")[1];
        output.drop_without_applying_deltas();
        for pressed in [true, false] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events: vec![
                        egui::Event::PointerMoved(file + egui::vec2(3.0, 5.0)),
                        egui::Event::PointerButton {
                            pos: file + egui::vec2(3.0, 5.0),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                    ..Default::default()
                },
                |ui| app.render(ui),
            );
            output.textures_delta.clear();
            output.drop_without_applying_deltas();
        }
        assert_eq!(
            app.session.selected.as_deref(),
            Some(std::path::Path::new("assembly/model.step"))
        );
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ui| app.render(ui),
        );
        output.textures_delta.clear();
        let selected_path = PathBuf::from("assembly").join("model.step");
        let selected_text = selected_path.display().to_string();
        let (shape, text) = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == selected_text => {
                    Some((shape, text))
                }
                _ => None,
            })
            .expect("Clicked item should appear in bottom guidance");
        let bounds = Rect::from_min_size(text.pos, text.galley.size());
        assert!(bounds.top() >= app.pane_rects[0].bottom());
        assert!(
            ctx.read_response(egui::Id::new("workspace-guidance"))
                .unwrap()
                .rect
                .contains_rect(bounds)
        );
        let guidance = ctx
            .read_response(egui::Id::new("workspace-guidance"))
            .unwrap()
            .rect;
        assert!((bounds.left() - guidance.left()).abs() < 1.0);
        assert!(
            screen.shrink(10.0).contains_rect(bounds),
            "Status outside window: {bounds:?}"
        );
        assert!(
            shape.clip_rect.contains_rect(bounds),
            "Status clipped: {bounds:?}"
        );
        output.drop_without_applying_deltas();
    }
}

#[test]
fn paths_precede_tree_rows_and_legend_follows_the_panes() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    app.session.paths = [
        engineering_path("current/designs"),
        engineering_path("release/designs"),
    ];
    let output = render(&mut app, &ctx, vec![]);
    let legend = text_positions(&output, "Different  0")[0];
    let rows = text_positions(&output, "assembly");
    for path in &app.session.paths {
        let positions = text_positions(&output, path);
        assert!(!positions.is_empty());
        assert!(
            positions
                .iter()
                .all(|position| position.y < rows[0].y && rows[0].y < legend.y)
        );
    }
    assert!(rows[0].y < 160.0, "Compact tree should begin near the top");
    output.drop_without_applying_deltas();
}

#[test]
fn row_shows_size_beside_status_icon_without_a_status_word() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    let output = render(&mut app, &ctx, vec![]);
    let names = text_centers(&output, "assembly");
    let sizes = text_centers(&output, "0 B");
    assert_eq!(sizes.len(), 2);
    assert!(
        sizes
            .iter()
            .zip(names)
            .all(|(size, name)| size.y == name.y && size.x > name.x)
    );
    assert!(text_positions(&output, "Identical").is_empty());
    output.drop_without_applying_deltas();
    assert_eq!(format_size(Some(1024)), "1.0 KiB");
    assert_eq!(format_size(Some(1024 * 1024)), "1.0 MiB");
    assert_eq!(format_size(None), "—");
}

#[test]
fn file_lines_are_aligned_and_share_difference_legend() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    app.session.file_view = Some(loaded_file_view());
    let output = render(&mut app, &ctx, vec![]);
    let identical = text_centers(&output, "unchanged");
    assert_eq!(identical.len(), 2);
    assert_eq!(identical[0].y, identical[1].y);
    assert_eq!(
        text_centers(&output, "old value")[0].y,
        text_centers(&output, "new value")[0].y
    );
    assert_eq!(
        text_centers(&output, "removed")[0].y,
        text_centers(&output, "—")[0].y
    );
    for label in [
        "Different  1",
        "Left only  1",
        "Right only  0",
        "Type mismatch  0",
        "Read error  0",
    ] {
        assert_eq!(text_positions(&output, label).len(), 1);
    }
    assert!(text_positions(&output, "Identical  1").is_empty());
    for shape in &output.shapes {
        if let egui::Shape::Text(text) = &shape.shape {
            if text.galley.text() == "unchanged" {
                assert_eq!(text.fallback_color, Palette::new(true).muted);
            }
        }
    }
    assert!(status_icon(&DirectoryEntryState::Same).is_none());
    assert_eq!(
        state_color(&DirectoryEntryState::Same, Palette::new(true)),
        Palette::new(true).muted
    );
    output.drop_without_applying_deltas();
}

#[test]
fn back_button_stays_left_of_new_above_the_panes() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    app.session.file_view = Some(loaded_file_view());
    let output = render(&mut app, &ctx, vec![]);
    let back = ctx
        .read_response(egui::Id::new("Back to folders"))
        .unwrap()
        .rect
        .center();
    let legend = text_centers(&output, "Different  1")[0];
    let title = text_centers(&output, "Versus")[0];
    assert!(back.x < legend.x);
    let new = ctx
        .read_response(egui::Id::new("New comparison"))
        .unwrap()
        .rect;
    assert!(back.x < new.left());
    assert!((back.y - new.center().y).abs() < 1.0);
    assert!(legend.y > app.pane_rects[0].bottom());
    assert!(back.y > title.y);
    output.drop_without_applying_deltas();
}

#[test]
fn legends_are_centered_below_both_viewers_with_icons_and_full_labels() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for zoom in [1.0, 2.0] {
            for (merge, run_diff) in [(false, false), (true, false), (true, true)] {
                let ctx = egui::Context::default();
                set_theme(&ctx, theme);
                ctx.set_zoom_factor(zoom);
                // Apply the pending scale before rendering fixed physical bounds.
                ctx.run_ui(egui::RawInput::default(), |_| {})
                    .drop_without_applying_deltas();
                let mut app = VersusApp::default();
                app.session.set_workspace_merge(merge);
                if merge {
                    app.session
                        .interactive
                        .as_mut()
                        .unwrap()
                        .set_run_diff(run_diff);
                }
                render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
                let output = render(&mut app, &ctx, vec![]);
                let mut icons = DIFF_LEGEND_ICONS.to_vec();
                if merge {
                    icons.push(StatusIcon::Conflict);
                }
                let mut bounds = Rect::NOTHING;
                for icon in icons {
                    let rect = ctx
                        .read_response(egui::Id::new(("legend", icon.label())))
                        .unwrap()
                        .rect;
                    assert!(
                        rect.top() >= app.pane_rects[0].bottom(),
                        "legend above viewer: {rect:?}"
                    );
                    assert!(
                        !text_positions(&output, icon.label()).is_empty(),
                        "missing legend text {}",
                        icon.label()
                    );
                    assert!(
                        output
                            .shapes
                            .iter()
                            .any(|shape| rect.intersects(shape.shape.visual_bounding_rect())
                                && !matches!(shape.shape, egui::Shape::Text(_))),
                        "icon={} merge={merge} zoom={zoom} rect={rect:?}",
                        icon.label()
                    );
                    bounds = bounds.union(rect);
                }
                let legend = ctx
                    .read_response(egui::Id::new("workspace-legend"))
                    .unwrap()
                    .rect;
                assert!((legend.center().x - ctx.viewport_rect().center().x).abs() < 1.0);
                assert!(legend.contains_rect(bounds));
                assert!((bounds.center().x - legend.center().x).abs() < 1.0);
                assert!(bounds.bottom() <= ctx.viewport_rect().bottom());
                if merge {
                    let save = text_centers(&output, "Save Both")[0];
                    assert!((bounds.center().y - save.y).abs() < 1.0);
                }
                if merge && !run_diff {
                    assert!(
                        ctx.read_response(egui::Id::new(("legend", "Different")))
                            .is_some()
                    );
                    assert!(text_positions(&output, "0 changes").is_empty());
                }
                output.drop_without_applying_deltas();
            }
        }
    }
}

#[test]
fn footer_guidance_and_activity_share_the_legend_row_in_both_modes() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for zoom in [1.0, 2.0] {
            for merge in [false, true] {
                let ctx = egui::Context::default();
                set_theme(&ctx, theme);
                ctx.set_zoom_factor(zoom);
                // Apply the pending scale before rendering fixed physical bounds.
                ctx.run_ui(egui::RawInput::default(), |_| {})
                    .drop_without_applying_deltas();
                let mut app = VersusApp::default();
                app.session.switch_comparison_mode(ComparisonMode::File);
                app.session.set_workspace_merge(merge);
                render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
                let output = render(&mut app, &ctx, vec![]);
                let info = ctx
                    .read_response(egui::Id::new("workspace-guidance"))
                    .unwrap()
                    .rect;
                let legend = ctx
                    .read_response(egui::Id::new("workspace-legend"))
                    .unwrap()
                    .rect;
                let status = ctx
                    .read_response(egui::Id::new("workspace-status"))
                    .unwrap()
                    .rect;
                let guidance = text_centers(&output, "Choose a file on each side to begin.")[0];
                assert!(
                    info.contains(guidance),
                    "merge={merge} zoom={zoom} info={info:?} label={guidance:?}"
                );
                assert!(info.right() <= legend.left());
                assert!(legend.right() <= status.left());
                assert!((info.center().y - legend.center().y).abs() < 1.0);
                assert!((status.center().y - legend.center().y).abs() < 1.0);
                if merge {
                    let save = text_centers(&output, "Save Both")[0];
                    assert!(status.right() < save.x);
                    assert!((save.y - status.center().y).abs() < 1.0);
                    assert!(!app.session.interactive.as_ref().unwrap().run_diff_enabled());
                }
                assert!(
                    output.shapes.iter().all(|shape| match &shape.shape {
                        egui::Shape::Text(text) => !status.contains(text.pos),
                        _ => true,
                    }),
                    "idle status area should be empty"
                );
                output.drop_without_applying_deltas();
                let panes = app.pane_rects;
                let (_sender, receiver) = mpsc::channel();
                let progress = Arc::new(ComparisonProgress::default());
                progress.begin(ProgressStage::ComparingFiles, Some(100));
                progress.advance(25);
                let cancellation = Arc::new(AtomicBool::new(false));
                app.session.job = Some(ComparisonJob {
                    receiver,
                    cancellation,
                    progress,
                    started: Instant::now(),
                    logged_progress: None,
                    last_progress_log: Instant::now(),
                    roots: ["left".into(), "right".into()],
                });
                let output = render(&mut app, &ctx, vec![]);
                assert_eq!(app.pane_rects, panes);
                let cancel = ctx
                    .read_response(egui::Id::new("Cancel folder comparison"))
                    .unwrap()
                    .rect;
                assert!(
                    status.contains_rect(cancel),
                    "cancel={cancel:?}, status={status:?}"
                );
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if status.contains(text.pos)
                        && text.galley.text().contains("25% complete"))));
                assert_eq!(
                    text_positions(&output, "Choose a file on each side to begin.").len(),
                    1
                );
                output.drop_without_applying_deltas();
            }
        }
    }
}

#[test]
fn new_button_and_change_count_stay_left_and_options_align_right() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let ctx = egui::Context::default();
        set_theme(&ctx, theme);
        let mut app = loaded_app();
        let output = render(&mut app, &ctx, vec![]);
        let new = ctx
            .read_response(egui::Id::new("New comparison"))
            .unwrap()
            .rect;
        let count = text_centers(&output, &app.session.navigation_status())[0];
        let filter = ctx
            .read_response(egui::Id::new("Show only differences"))
            .unwrap()
            .rect;
        assert!(new.right() < count.x && count.x < filter.left());
        assert!((new.center().y - count.y).abs() < 1.0);
        assert!((new.center().y - filter.center().y).abs() < 1.0);
        let legend = ctx
            .read_response(egui::Id::new(("legend", "Different")))
            .unwrap()
            .rect;
        assert!(legend.top() >= app.pane_rects[0].bottom());
        assert!(
            ctx.read_response(egui::Id::new("Next difference"))
                .unwrap()
                .rect
                .right()
                > 1170.0
        );
        output.drop_without_applying_deltas();
    }
}

#[test]
fn bottom_right_status_and_progress_do_not_move_the_panes() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for size in [egui::vec2(900.0, 650.0), egui::vec2(1200.0, 800.0)] {
            for file in [false, true] {
                let ctx = egui::Context::default();
                set_theme(&ctx, theme);
                let mut app = loaded_app();
                if file {
                    app.session.file_view = Some(loaded_file_view());
                }
                let draw = |app: &mut VersusApp| {
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |ui| app.render(ui),
                    );
                    output.textures_delta.clear();
                    output
                };
                let output = draw(&mut app);
                let panes = app.pane_rects;
                let status = ctx
                    .read_response(egui::Id::new("workspace-status"))
                    .unwrap()
                    .rect;
                let info = ctx
                    .read_response(egui::Id::new("workspace-guidance"))
                    .unwrap()
                    .rect;
                let status_text = text_centers(&output, &app.session.message)[0];
                assert!(status.top() >= panes[0].bottom());
                assert!((status.right() - (size.x - 10.0)).abs() < 1.0);
                assert!(info.contains(status_text));
                assert!(info.right() < status.left());
                assert!((status_text.y - status.center().y).abs() < 1.0);
                output.drop_without_applying_deltas();
                let progress = Arc::new(ComparisonProgress::default());
                progress.begin(ProgressStage::ComparingFiles, Some(100));
                progress.advance(25);
                let cancellation = Arc::new(AtomicBool::new(false));
                let (_folder_sender, _file_sender) = if file {
                    let (sender, receiver) = mpsc::channel();
                    app.session.file_view.as_mut().unwrap().job = Some(FileJob {
                        receiver,
                        cancellation: cancellation.clone(),
                        progress,
                        ignore_line_endings: true,
                    });
                    (None, Some(sender))
                } else {
                    let (sender, receiver) = mpsc::channel();
                    app.session.job = Some(ComparisonJob {
                        receiver,
                        cancellation: cancellation.clone(),
                        progress,
                        started: Instant::now(),
                        logged_progress: None,
                        last_progress_log: Instant::now(),
                        roots: app.session.roots.clone().unwrap(),
                    });
                    (Some(sender), None)
                };
                let output = draw(&mut app);
                assert_eq!(app.pane_rects, panes);
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("25% complete"))));
                let cancel = ctx
                    .read_response(egui::Id::new(if file {
                        "Cancel file comparison"
                    } else {
                        "Cancel folder comparison"
                    }))
                    .unwrap()
                    .rect;
                assert!(status.contains_rect(cancel));
                output.drop_without_applying_deltas();
                app.session.job = None;
                if let Some(view) = &mut app.session.file_view {
                    view.job = None;
                }
                app.session.drop_message = Some("Drop status: invalid source".into());
                let output = draw(&mut app);
                assert_eq!(app.pane_rects, panes);
                assert!(status.contains(text_centers(&output, "1 error")[0]));
                output.drop_without_applying_deltas();
            }
        }
    }
}

#[test]
fn legend_status_cancel_button_still_cancels_file_comparison() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    let mut view = loaded_file_view();
    let (_sender, receiver) = mpsc::channel();
    let cancellation = Arc::new(AtomicBool::new(false));
    view.job = Some(FileJob {
        receiver,
        cancellation: cancellation.clone(),
        progress: Arc::new(ComparisonProgress::default()),
        ignore_line_endings: true,
    });
    app.session.file_view = Some(view);
    click_action(&mut app, &ctx, "Cancel file comparison");
    assert!(cancellation.load(Ordering::Relaxed));
}

#[test]
fn progress_displays_completion_without_time_estimates() {
    let ctx = egui::Context::default();
    let mut app = loaded_app();
    let _sender = attach_job(&mut app);
    let job = app.session.job.as_ref().unwrap();
    job.progress.begin(ProgressStage::ComparingFiles, Some(100));
    job.progress.advance(25);
    let text = progress_text(job.progress.snapshot());
    assert_eq!(text, "Comparing files… 25% complete");
    let output = render(&mut app, &ctx, vec![]);
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("25% complete"))));
    output.drop_without_applying_deltas();
    let job = app.session.job.as_ref().unwrap();
    job.progress.begin(ProgressStage::Scanning, None);
    job.progress.advance(50);
    assert_eq!(
        progress_text(job.progress.snapshot()),
        "Scanning folders… 50 entries found"
    );
}
