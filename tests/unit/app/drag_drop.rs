//! Drag drop regression coverage.

use super::*;
use crate::app::visuals::panels::split_rect;
use crate::app::visuals::theme::{Palette, apply_theme, set_theme};

#[test]
fn scrollbars_use_pointer_for_tracks_and_dragging_but_not_when_disabled() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for enabled in [true, false] {
            for interactive in [true, false] {
                let ctx = egui::Context::default();
                set_theme(&ctx, theme);
                let render = |events| {
                    let mut scroll_id = egui::Id::NULL;
                    let output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(
                                Pos2::ZERO,
                                egui::vec2(400.0, 300.0),
                            )),
                            events,
                            ..Default::default()
                        },
                        |ui| {
                            ui.add_enabled_ui(enabled, |ui| {
                                scroll_id = egui::ScrollArea::both()
                                    .id_salt("pointer-scrollbar-test")
                                    .scroll_source(if interactive {
                                        egui::scroll_area::ScrollSource::default()
                                    } else {
                                        egui::scroll_area::ScrollSource::NONE
                                    })
                                    .scroll_bar_visibility(
                                        egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                                    )
                                    .max_width(200.0)
                                    .max_height(120.0)
                                    .show(ui, |ui| {
                                        ui.allocate_space(egui::vec2(1000.0, 1000.0));
                                    })
                                    .id;
                            });
                        },
                    );
                    (output, scroll_id)
                };
                for _ in 0..3 {
                    render(vec![]).0.drop_without_applying_deltas();
                }
                let (output, scroll_id) = render(vec![]);
                output.drop_without_applying_deltas();
                for axis in 0..2usize {
                    let rect = ctx.read_response(scroll_id.with(axis)).unwrap().rect;
                    let expected = if enabled && interactive {
                        egui::CursorIcon::PointingHand
                    } else {
                        egui::CursorIcon::Default
                    };
                    for fraction in [0.05, 0.75] {
                        let mut point = rect.center();
                        point[axis] = egui::lerp(rect.min[axis]..=rect.max[axis], fraction);
                        let (output, _) = render(vec![egui::Event::PointerMoved(point)]);
                        let actual = output.platform_output.cursor_icon;
                        output.drop_without_applying_deltas();
                        assert_eq!(
                            actual, expected,
                            "theme={theme:?}, axis={axis}, enabled={enabled}, interactive={interactive}"
                        );
                    }
                    if enabled && interactive {
                        let start = rect.center();
                        render(vec![
                            egui::Event::PointerMoved(start),
                            egui::Event::PointerButton {
                                pos: start,
                                button: egui::PointerButton::Primary,
                                pressed: true,
                                modifiers: Default::default(),
                            },
                        ])
                        .0
                        .drop_without_applying_deltas();
                        let mut outside = start;
                        outside[1 - axis] -= 30.0;
                        outside[axis] += 20.0;
                        let (output, _) = render(vec![egui::Event::PointerMoved(outside)]);
                        let actual = output.platform_output.cursor_icon;
                        output.drop_without_applying_deltas();
                        assert_eq!(
                            actual, expected,
                            "dragging scrollbar must keep pointer cursor"
                        );
                        render(vec![egui::Event::PointerButton {
                            pos: outside,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: Default::default(),
                        }])
                        .0
                        .drop_without_applying_deltas();
                    }
                }
            }
        }
    }
}

#[test]
fn merge_and_diff_selectors_share_panel_and_empty_drop_styles() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for mode in [ComparisonMode::Folder, ComparisonMode::File] {
            let mut selectors = Vec::new();
            for merge in [false, true] {
                let ctx = egui::Context::default();
                set_theme(&ctx, theme);
                let palette = Palette::for_context(&ctx);
                let mut app = VersusApp::default();
                app.session.comparison_mode = mode;
                app.session.set_workspace_merge(merge);
                let output = render(&mut app, &ctx, vec![]);
                let picker = ctx
                    .read_response(egui::Id::new(format!(
                        "Browse left {}",
                        mode.source_label()
                    )))
                    .unwrap()
                    .rect;
                let panel = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect)
                            if rect.fill == palette.panel
                                && rect.stroke == Stroke::new(1.0, palette.border)
                                && rect.rect.contains_rect(picker)
                                && rect.rect.width() > 500.0 =>
                        {
                            Some(rect)
                        }
                        _ => None,
                    })
                    .expect("source picker must be inside the comparison panel");
                assert_eq!(panel.corner_radius.nw, 6);
                assert_eq!(
                    text_positions(&output, "Drop a file or folder here").len(),
                    2
                );
                for side in 0..2 {
                    let half = split_rect(panel.rect)[side];
                    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::LineSegment { points, stroke } if *stroke == Stroke::new(1.0, palette.border) && half.contains(points[0]) && (points[0].distance(points[1])-6.0).abs() < 0.01)));
                }
                selectors.push(picker);
                output.drop_without_applying_deltas();
            }
            assert_eq!(selectors[0].size(), selectors[1].size());
            let shift = selectors[0].left() - selectors[1].left();
            assert!(shift >= selectors[0].width() && shift <= selectors[0].width() + 8.0);
        }
    }
}

#[test]
fn dropped_files_and_folders_open_on_the_chosen_side_and_cancel_old_work() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    drop_sources(&mut app, &ctx, 1, vec![sources.path("right/model.txt")]);
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.comparison_mode, ComparisonMode::File);
    assert_eq!(
        app.session.paths[1],
        sources.path("right/model.txt").display().to_string()
    );
    drop_sources(&mut app, &ctx, 0, vec![sources.path("left/model.txt")]);
    settle_sources(&mut app, &ctx);
    assert!(app.session.file_view.as_ref().unwrap().comparison.is_some());
    drop_sources(&mut app, &ctx, 0, vec![sources.path("left")]);
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.mode(), SelectionMode::Incompatible);
    drop_sources(&mut app, &ctx, 1, vec![sources.path("right")]);
    settle_sources(&mut app, &ctx);
    assert!(app.session.tree.is_some());
    let sender = attach_job(&mut app);
    let flag = app.session.job.as_ref().unwrap().cancellation.clone();
    drop_sources(&mut app, &ctx, 0, vec![sources.path("left/model.txt")]);
    assert!(flag.load(Ordering::Relaxed));
    assert!(sender.send(Ok(fixture())).is_err());
    settle_sources(&mut app, &ctx);
}

#[test]
fn dropping_on_a_file_drilldown_retains_the_other_file_and_rejects_multiple_drops() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.session.open_launch_request(crate::cli::LaunchRequest {
        paths: [sources.path("left"), sources.path("right")],
        mode: None,
    });
    settle_sources(&mut app, &ctx);
    app.session
        .open_file("model.txt".into(), [true, true], false);
    settle_sources(&mut app, &ctx);
    drop_sources(&mut app, &ctx, 0, vec![sources.path("left/model.txt")]);
    settle_sources(&mut app, &ctx);
    let view = app.session.file_view.as_ref().unwrap();
    assert!(!view.from_folders);
    assert_eq!(view.paths[1], sources.path("right/model.txt"));
    let paths = app.session.paths.clone();
    drop_sources(
        &mut app,
        &ctx,
        0,
        vec![sources.path("left"), sources.path("right")],
    );
    assert_eq!(app.session.paths, paths);
    assert!(app.session.drop_message.is_some());
}

#[test]
fn native_drag_positions_survive_pointer_gone_for_hover_and_drop_on_both_sides() {
    let sources = SourceFixture::new();
    for side in 0..2 {
        for relative in ["left", "left/model.txt"] {
            let ctx = egui::Context::default();
            apply_theme(&ctx);
            let mut app = VersusApp::default();
            render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
            let position = app.pane_rects[side].center();
            let input = |dropped: bool| egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerGone,
                ],
                hovered_files: if dropped {
                    vec![]
                } else {
                    vec![egui::HoveredFile {
                        path: Some(sources.path(relative)),
                        ..Default::default()
                    }]
                },
                dropped_files: if dropped {
                    vec![Arc::new(TestDrop(sources.path(relative))) as egui::DroppedFileHandle]
                } else {
                    vec![]
                },
                ..Default::default()
            };
            let mut hover = ctx.run_ui(input(false), |ui| app.render(ui));
            hover.textures_delta.clear();
            assert_eq!(app.drop_hover_side, Some(side));
            assert!(ctx.input(|input| input.pointer.latest_pos()).is_none());
            assert_eq!(
                text_positions(
                    &hover,
                    if side == 0 {
                        "Drop a file or folder on LEFT"
                    } else {
                        "Drop a file or folder on RIGHT"
                    }
                )
                .len(),
                1
            );
            hover.drop_without_applying_deltas();
            let mut dropped = ctx.run_ui(input(true), |ui| app.render(ui));
            dropped.textures_delta.clear();
            assert_eq!(app.session.source_paths[side], Some(sources.path(relative)));
            assert!(app.session.source_paths[1 - side].is_none());
            assert!(app.session.drop_message.is_none());
            dropped.drop_without_applying_deltas();
            settle_sources(&mut app, &ctx);
            assert_eq!(
                app.session.source_kinds[side],
                Some(if relative.ends_with(".txt") {
                    SourceKind::File
                } else {
                    SourceKind::Folder
                })
            );
        }
    }
}

#[test]
fn drop_position_survives_pointer_gone_without_a_hover_frame() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    let position = app.pane_rects[1].center();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            events: vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerGone,
            ],
            dropped_files: vec![Arc::new(TestDrop(sources.path("right")))],
            ..Default::default()
        },
        |ui| app.render(ui),
    );
    output.textures_delta.clear();
    assert_eq!(app.session.source_paths[1], Some(sources.path("right")));
    assert!(app.session.drop_message.is_none());
    output.drop_without_applying_deltas();
    settle_sources(&mut app, &ctx);
}

#[test]
fn drop_hover_marks_the_target_and_outside_drops_preserve_selection() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    let position = app.pane_rects[1].center();
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            events: vec![egui::Event::PointerMoved(position)],
            hovered_files: vec![egui::HoveredFile {
                path: Some(sources.path("right/model.txt")),
                ..Default::default()
            }],
            ..Default::default()
        },
        |ui| app.render(ui),
    );
    assert_eq!(app.drop_hover_side, Some(1));
    assert!(!text_positions(&output, "Drop a file or folder on RIGHT").is_empty());
    output.drop_without_applying_deltas();
    let paths = app.session.paths.clone();
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            events: vec![
                egui::Event::PointerMoved(egui::pos2(600.0, 20.0)),
                egui::Event::PointerGone,
            ],
            dropped_files: vec![Arc::new(TestDrop(sources.path("right/model.txt")))],
            ..Default::default()
        },
        |ui| app.render(ui),
    )
    .drop_without_applying_deltas();
    assert_eq!(app.session.paths, paths);
    assert_eq!(
        app.session.drop_message.as_deref(),
        Some("Drop into the large LEFT or RIGHT view below the path bars.")
    );
    assert!(app.session.source_jobs.iter().all(Option::is_none));
}

#[test]
fn drag_highlight_borders_only_the_hovered_pane_above_empty_or_loaded_content() {
    let sources = SourceFixture::new();
    for dark in [true, false] {
        for loaded in [false, true] {
            let ctx = egui::Context::default();
            set_theme(
                &ctx,
                if dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                },
            );
            let mut app = if loaded {
                loaded_app()
            } else {
                VersusApp::default()
            };
            render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
            for side in [0, 1] {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            egui::vec2(1200.0, 800.0),
                        )),
                        events: vec![egui::Event::PointerMoved(app.pane_rects[side].center())],
                        hovered_files: vec![egui::HoveredFile {
                            path: Some(sources.path("left")),
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                    |ui| app.render(ui),
                );
                output.textures_delta.clear();
                let borders: Vec<_> = output
                    .shapes
                    .iter()
                    .enumerate()
                    .filter_map(|(index, shape)| match &shape.shape {
                        egui::Shape::Rect(rect)
                            if rect.stroke.width == 3.0
                                && rect.stroke.color == Palette::new(dark).accent =>
                        {
                            Some((index, rect.rect))
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(borders.len(), 1);
                assert_eq!(borders[0].1, app.pane_rects[side].shrink(3.0));
                assert!(
                    output.shapes[..borders[0].0]
                        .iter()
                        .any(|shape| matches!(shape.shape, egui::Shape::Text(_)))
                );
                assert_eq!(app.drop_hover_side, Some(side));
                output.drop_without_applying_deltas();
            }
            let mut output = render(&mut app, &ctx, vec![egui::Event::PointerGone]);
            output.textures_delta.clear();
            assert!(app.drop_hover_side.is_none());
            assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(rect) if rect.stroke.width == 3.0 && rect.stroke.color == Palette::new(dark).accent)));
            output.drop_without_applying_deltas();
        }
    }
}

#[test]
fn native_cursor_targets_hover_and_drop_without_pointer_events() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    // Explorer can retain focus and leave egui's pointer at an old location.
    render(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(egui::pos2(600.0, 20.0))],
    )
    .drop_without_applying_deltas();
    for side in [1, 0] {
        app.native_drag_position = Some(app.pane_rects[side].center());
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                hovered_files: vec![egui::HoveredFile {
                    path: Some(sources.path("left")),
                    ..Default::default()
                }],
                ..Default::default()
            },
            |ui| app.render(ui),
        );
        output.textures_delta.clear();
        assert_eq!(app.drop_hover_side, Some(side));
        assert!(app.native_drag_position.is_none());
        assert!(
            !text_positions(
                &output,
                if side == 0 {
                    "Drop a file or folder on LEFT"
                } else {
                    "Drop a file or folder on RIGHT"
                }
            )
            .is_empty()
        );
        output.drop_without_applying_deltas();
    }
    app.native_drag_position = Some(app.pane_rects[0].center());
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            dropped_files: vec![Arc::new(TestDrop(sources.path("left")))],
            ..Default::default()
        },
        |ui| app.render(ui),
    )
    .drop_without_applying_deltas();
    assert_eq!(app.session.source_paths[0], Some(sources.path("left")));
    assert!(app.session.source_paths[1].is_none());
    assert!(app.session.drop_message.is_none());
    settle_sources(&mut app, &ctx);

    // An up-to-date native position outside the panes must reject the drop,
    // even if egui still retains an old, valid pane position.
    let pane = app.pane_rects[1].center();
    render(&mut app, &ctx, vec![egui::Event::PointerMoved(pane)]).drop_without_applying_deltas();
    app.native_drag_position = Some(egui::pos2(600.0, 20.0));
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            dropped_files: vec![Arc::new(TestDrop(sources.path("right")))],
            ..Default::default()
        },
        |ui| app.render(ui),
    )
    .drop_without_applying_deltas();
    assert!(app.session.source_paths[1].is_none());
    assert!(app.session.drop_message.is_some());
}

#[test]
fn drop_event_position_takes_priority_over_a_later_native_cursor_sample() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    let position = app.pane_rects[1].center();
    app.native_drag_position = Some(egui::pos2(600.0, 20.0));
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            events: vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerGone,
            ],
            dropped_files: vec![Arc::new(TestDrop(sources.path("right")))],
            ..Default::default()
        },
        |ui| app.render(ui),
    )
    .drop_without_applying_deltas();
    assert_eq!(app.session.source_paths[1], Some(sources.path("right")));
    assert!(app.session.drop_message.is_none());
    assert!(app.native_drag_position.is_none());
    settle_sources(&mut app, &ctx);
}

#[test]
fn empty_comparisons_keep_drop_hints_and_result_explanations() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    for filtered in [false, true] {
        let mut app = loaded_app();
        app.session.tree = Some(FolderTree::from_diff(&DirectoryDiff {
            cancelled: false,
            entries: vec![],
        }));
        app.session.show_only_differences = filtered;
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(
            text_positions(&output, "Drop a file or folder here").len(),
            2
        );
        assert_eq!(
            text_positions(
                &output,
                if filtered {
                    "No differences"
                } else {
                    "Both folders are empty"
                }
            )
            .len(),
            1
        );
        output.drop_without_applying_deltas();
    }
}

#[test]
fn invalid_sources_keep_drop_targets_available() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    for incompatible in [false, true] {
        let mut app = VersusApp::default();
        app.session.paths = ["left".into(), "right".into()];
        if incompatible {
            app.session.source_kinds = [Some(SourceKind::File), Some(SourceKind::Folder)];
        } else {
            app.session.error = Some("Unavailable source".into());
        }
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(
            text_positions(&output, "Drop a file or folder here").len(),
            2
        );
        assert_eq!(
            text_positions(
                &output,
                if incompatible {
                    "Incompatible source types"
                } else {
                    "Cannot open selected source"
                }
            )
            .len(),
            1
        );
        output.drop_without_applying_deltas();
    }
}

#[test]
fn empty_file_comparisons_keep_drop_hints() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    for empty_files in [false, true] {
        let mut app = loaded_app();
        let mut view = loaded_file_view();
        view.visible_rows.clear();
        if empty_files {
            view.comparison.as_mut().unwrap().rows.clear();
        }
        app.session.file_view = Some(view);
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(
            text_positions(&output, "Drop a file or folder here").len(),
            2
        );
        assert_eq!(
            text_positions(
                &output,
                if empty_files {
                    "Both files are empty"
                } else {
                    "No differences"
                }
            )
            .len(),
            1
        );
        output.drop_without_applying_deltas();
    }
}
