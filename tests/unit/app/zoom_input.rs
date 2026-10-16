//! Zoom input regression coverage.

use super::*;
use crate::app::session::update_visible_file_rows;
use crate::app::visuals::theme::{
    MAX_ZOOM, MIN_ZOOM, Palette, ROW_HEIGHT, apply_theme, restore_zoom, set_theme,
};

#[test]
fn enlarged_fonts_expand_row_and_control_geometry() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    ctx.all_styles_mut(|style| {
        style
            .text_styles
            .insert(egui::TextStyle::Monospace, FontId::monospace(24.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(24.0));
    });
    let mut app = loaded_app();
    let output = render(&mut app, &ctx, vec![]);
    assert!(app.session.row_height > ROW_HEIGHT);
    let response = ctx
        .read_response(egui::Id::new("Show only differences"))
        .unwrap();
    assert!(response.rect.height() > 24.0);
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "assembly" && text.galley.size().y > 22.0)));
    output.drop_without_applying_deltas();
}

#[test]
fn magnifier_buttons_zoom_and_percentage_resets_in_both_views() {
    for file_view in [false, true] {
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let ctx = egui::Context::default();
            apply_theme(&ctx);
            set_theme(&ctx, theme);
            let mut app = loaded_app();
            app.session.selected = Some("assembly".into());
            if file_view {
                app.session.file_view = Some(loaded_file_view());
            }
            let paths = app.session.paths.clone();
            let output = render(&mut app, &ctx, vec![]);
            let out = ctx.read_response(egui::Id::new("Zoom out")).unwrap().rect;
            let zoom = text_centers(&output, "Zoom 100%")[0];
            let inside = ctx.read_response(egui::Id::new("Zoom in")).unwrap().rect;
            assert!(zoom.x < out.left() && out.right() < inside.left());
            for rect in [out, inside] {
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Circle(circle) if circle.radius == 5.5 && rect.contains(circle.center))));
            }
            output.drop_without_applying_deltas();
            click_action(&mut app, &ctx, "Zoom in");
            render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
            assert!((ctx.zoom_factor() - 1.1).abs() < 0.001);
            click_action(&mut app, &ctx, "Zoom out");
            render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
            assert_eq!(ctx.zoom_factor(), 1.0);
            click_action(&mut app, &ctx, "Zoom in");
            let output = render(&mut app, &ctx, vec![]);
            let reset = text_centers(&output, "Zoom 110%")[0];
            output.drop_without_applying_deltas();
            click(&mut app, &ctx, reset);
            render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
            assert_eq!(ctx.zoom_factor(), 1.0);
            assert_eq!(app.session.paths, paths);
            assert_eq!(
                app.session.selected.as_deref(),
                Some(std::path::Path::new("assembly"))
            );
            assert_eq!(app.session.file_view.is_some(), file_view);
            assert_eq!(ctx.theme(), theme);
            for (limit, label) in [(MIN_ZOOM, "Zoom out"), (MAX_ZOOM, "Zoom in")] {
                ctx.set_zoom_factor(limit);
                render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
                // egui derives interaction/disabled state from the preceding pass.
                render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
                assert!(
                    !ctx.read_response(egui::Id::new(label)).unwrap().enabled(),
                    "limit {limit}, actual {}: {label}",
                    ctx.zoom_factor()
                );
            }
        }
    }
}

#[test]
fn ctrl_wheel_zooms_without_scrolling_and_plain_wheel_still_scrolls() {
    for file_view in [false, true] {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = unfiltered_app();
        if file_view {
            let mut view = loaded_file_view();
            view.comparison.as_mut().unwrap().rows = (1..=200)
                .map(|number| {
                    test_file_row(
                        Some((number, format!("line {number}"))),
                        Some((number, format!("line {number}"))),
                        DirectoryEntryState::Same,
                    )
                })
                .collect();
            update_visible_file_rows(&mut view, false);
            view.scroll_y = 200.0;
            app.session.file_view = Some(view);
        } else {
            app.session.tree = Some(FolderTree::from_diff(&DirectoryDiff {
                cancelled: false,
                entries: (0..200)
                    .map(|index| DirectoryEntry {
                        relative_path: format!("entry-{index:03}.txt").into(),
                        left_size: Some(1),
                        right_size: Some(1),
                        left_exists: true,
                        right_exists: true,
                        left_kind: Some(DirectoryEntryKind::File),
                        right_kind: Some(DirectoryEntryKind::File),
                        kind: DirectoryEntryKind::File,
                        state: DirectoryEntryState::Same,
                    })
                    .collect(),
            }));
            app.session.tree_scroll_y = 200.0;
        }
        render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
        let scroll = |app: &VersusApp| {
            app.session
                .file_view
                .as_ref()
                .map_or(app.session.tree_scroll_y, |view| view.scroll_y)
        };
        let pointer = app.pane_rects[0].center();
        let wheel = |delta, ctrl, unit| {
            vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::MouseWheel {
                    unit,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0.0, delta),
                    modifiers: egui::Modifiers {
                        ctrl,
                        ..Default::default()
                    },
                },
            ]
        };
        // Line-based wheel ticks keep zooming during egui's smoothing tail.
        render(&mut app, &ctx, wheel(1.0, true, egui::MouseWheelUnit::Line))
            .drop_without_applying_deltas();
        render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
        let first_zoom = ctx.zoom_factor();
        assert!(first_zoom > 1.0);
        for _ in 0..30 {
            render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
        }
        assert!(ctx.zoom_factor() > first_zoom);
        assert_eq!(scroll(&app), 200.0);
        render(
            &mut app,
            &ctx,
            wheel(-1.0, true, egui::MouseWheelUnit::Line),
        )
        .drop_without_applying_deltas();
        for _ in 0..30 {
            render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
        }
        assert!((ctx.zoom_factor() - 1.0).abs() < 0.01);
        assert_eq!(scroll(&app), 200.0);
        let zoom = ctx.zoom_factor();
        render(
            &mut app,
            &ctx,
            wheel(-100.0, false, egui::MouseWheelUnit::Point),
        )
        .drop_without_applying_deltas();
        for _ in 0..30 {
            render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
        }
        assert!(scroll(&app) > 200.0);
        assert_eq!(ctx.zoom_factor(), zoom);
        for (start, delta, expected) in [(4.99, 100.0, MAX_ZOOM), (0.21, -100.0, MIN_ZOOM)] {
            ctx.set_zoom_factor(start);
            render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
            render(
                &mut app,
                &ctx,
                wheel(delta, true, egui::MouseWheelUnit::Point),
            )
            .drop_without_applying_deltas();
            for _ in 0..30 {
                render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
            }
            assert_eq!(ctx.zoom_factor(), expected);
        }
        let mut storage = ZoomStorage::default();
        eframe::App::save(&mut app, &mut storage);
        assert_eq!(storage.get_string("versus.zoom").as_deref(), Some("0.2"));
    }
}

#[test]
fn wheel_zoom_is_not_applied_twice_during_a_layout_retry() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            events: vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, 4.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::CTRL,
            }],
            ..Default::default()
        },
        |ui| {
            app.render(ui);
            if ui.ctx().current_pass_index() == 0 {
                ui.ctx().request_discard("test layout retry");
            }
        },
    );
    assert_eq!(output.platform_output.num_completed_passes, 2);
    assert!((ctx.zoom_factor() - (4.0_f32 / 200.0).exp()).abs() < 0.001);
    output.drop_without_applying_deltas();
}

#[test]
fn zoom_is_restored_and_only_scale_is_saved() {
    let ctx = egui::Context::default();
    let mut storage = ZoomStorage::default();
    storage.set_string("versus.zoom", "1.4".into());
    restore_zoom(&ctx, &storage);
    let mut app = loaded_app();
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    assert_eq!(ctx.zoom_factor(), 1.4);
    storage.0.clear();
    eframe::App::save(&mut app, &mut storage);
    assert_eq!(storage.0.len(), 1);
    assert_eq!(storage.get_string("versus.zoom").as_deref(), Some("1.4"));
    assert!(!eframe::App::persist_egui_memory(&app));
    for value in ["NaN", "inf", "-1", "0", "6", "broken"] {
        let ctx = egui::Context::default();
        storage.set_string("versus.zoom", value.into());
        restore_zoom(&ctx, &storage);
        render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
        assert_eq!(ctx.zoom_factor(), 1.0, "invalid saved zoom {value}");
    }
}

#[test]
fn toolbar_press_feedback_precedes_activation_and_release_outside_cancels() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    let position = ctx
        .read_response(egui::Id::new("Show only differences"))
        .unwrap()
        .rect
        .center();
    let event = |position, pressed| egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    let output = render(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(position), event(position, true)],
    );
    assert!(!app.session.show_only_differences);
    let fill = Palette::for_context(&ctx).accent.gamma_multiply(0.35);
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == fill && rect.rect.contains(position))));
    output.drop_without_applying_deltas();
    let outside = egui::pos2(5.0, 700.0);
    render(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(outside), event(outside, false)],
    )
    .drop_without_applying_deltas();
    assert!(!app.session.show_only_differences);
    click_action(&mut app, &ctx, "Show only differences");
    assert!(app.session.show_only_differences);
}
