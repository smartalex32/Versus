//! Folder diff regression coverage.

use super::*;
use crate::app::visuals::theme::{apply_theme, set_theme};

#[test]
fn clicking_either_folder_pane_updates_both_rendered_trees() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    let output = render(&mut app, &ctx, vec![]);
    let folders = text_positions(&output, "assembly");
    assert_eq!(folders.len(), 2);
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, folders[0] + egui::vec2(3.0, 5.0));
    assert!(app.session.tree.as_ref().unwrap().is_expanded("assembly"));
    let output = render(&mut app, &ctx, vec![]);
    let children = text_positions(&output, "model.step");
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].y, children[1].y);
    let folders = text_positions(&output, "assembly");
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, folders[1] + egui::vec2(3.0, 5.0));
    assert!(!app.session.tree.as_ref().unwrap().is_expanded("assembly"));
    let output = render(&mut app, &ctx, vec![]);
    assert!(text_positions(&output, "model.step").is_empty());
    output.drop_without_applying_deltas();
}

#[test]
fn one_sided_rows_render_an_aligned_placeholder() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    app.session.tree = Some(FolderTree::from_diff(&DirectoryDiff {
        cancelled: false,
        entries: vec![DirectoryEntry {
            relative_path: "left-only.step".into(),
            left_size: Some(1024),
            right_size: None,
            left_exists: true,
            right_exists: false,
            left_kind: Some(DirectoryEntryKind::File),
            right_kind: None,
            kind: DirectoryEntryKind::File,
            state: DirectoryEntryState::LeftOnly,
        }],
    }));
    let output = render(&mut app, &ctx, vec![]);
    let names = text_positions(&output, "left-only.step");
    let placeholders = text_positions(&output, "—");
    assert_eq!(names.len(), 1);
    assert_eq!(placeholders.len(), 1);
    assert_eq!(names[0].y, placeholders[0].y);
    assert!(placeholders[0].x > 600.0);
    output.drop_without_applying_deltas();
}

#[test]
fn empty_folder_has_no_disclosure_and_click_only_selects_it() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    app.session.tree = Some(FolderTree::from_diff(&DirectoryDiff {
        cancelled: false,
        entries: vec![DirectoryEntry {
            relative_path: "empty-folder".into(),
            left_size: Some(0),
            right_size: Some(0),
            left_exists: true,
            right_exists: true,
            left_kind: Some(DirectoryEntryKind::Directory),
            right_kind: Some(DirectoryEntryKind::Directory),
            kind: DirectoryEntryKind::Directory,
            state: DirectoryEntryState::Same,
        }],
    }));
    let output = render(&mut app, &ctx, vec![]);
    assert!(text_positions(&output, "▸").is_empty());
    let folders = text_positions(&output, "empty-folder");
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, folders[0] + egui::vec2(3.0, 5.0));
    assert_eq!(
        app.session.selected.as_deref(),
        Some(std::path::Path::new("empty-folder"))
    );
    assert!(
        !app.session
            .tree
            .as_ref()
            .unwrap()
            .is_expanded("empty-folder")
    );
}

#[test]
fn clicking_path_or_browse_opens_picker_for_the_correct_side() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    app.session.paths = [String::new(), engineering_path("release/designs")];
    let original_paths = app.session.paths.clone();
    let mut requests = Vec::new();
    {
        let mut render_inputs = |events| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.folder_inputs_with_picker(ui, |side, path, _| {
                        requests.push((side, path.to_owned()));
                        None // Closing the dialog should preserve the current comparison.
                    })
                },
            )
        };
        for (side, path_text) in [(0, "Choose a folder"), (1, original_paths[1].as_str())] {
            for path_area in [true, false] {
                let output = render_inputs(vec![]);
                let target = if path_area {
                    text_centers(&output, path_text)[0]
                } else {
                    ctx.read_response(egui::Id::new(if side == 0 {
                        "Browse left folder"
                    } else {
                        "Browse right folder"
                    }))
                    .unwrap()
                    .rect
                    .center()
                };
                output.drop_without_applying_deltas();
                let output = render_inputs(vec![egui::Event::PointerMoved(target)]);
                assert_eq!(
                    output.platform_output.cursor_icon,
                    egui::CursorIcon::PointingHand
                );
                output.drop_without_applying_deltas();
                for pressed in [true, false] {
                    render_inputs(vec![
                        egui::Event::PointerMoved(target),
                        egui::Event::PointerButton {
                            pos: target,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ])
                    .drop_without_applying_deltas();
                }
            }
        }
    }
    assert_eq!(
        requests,
        vec![
            (0, String::new()),
            (0, String::new()),
            (1, original_paths[1].clone()),
            (1, original_paths[1].clone())
        ]
    );
    assert_eq!(app.session.paths, original_paths);
    assert!(app.session.tree.is_some());
}

#[test]
fn single_path_headers_center_side_labels_and_browse_buttons() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    app.session.paths = [
        engineering_path(&format!("{}release", "long-folder/".repeat(12))),
        engineering_path("release"),
    ];
    let output = render(&mut app, &ctx, vec![]);
    let mut centers = Vec::new();
    for (side, label) in ["LEFT", "RIGHT"].into_iter().enumerate() {
        let paths = text_centers(&output, &app.session.paths[side]);
        assert_eq!(paths.len(), 1, "Selected path must only appear once");
        let side_label = text_centers(&output, label)[0];
        let browse = ctx
            .read_response(egui::Id::new(if side == 0 {
                "Browse left folder"
            } else {
                "Browse right folder"
            }))
            .unwrap();
        assert_eq!(side_label.y, paths[0].y);
        assert_eq!(side_label.y, browse.rect.center().y);
        centers.push(side_label.y);
    }
    assert_eq!(centers[0], centers[1]);
    output.drop_without_applying_deltas();
}

#[test]
fn long_selected_path_wraps_without_truncation() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    app.session.paths[0] = engineering_path(&format!("{}release", "long-folder-name/".repeat(12)));
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 650.0))),
            ..Default::default()
        },
        |ui| app.render(ui),
    );
    output.textures_delta.clear();
    let wrapped = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text)
                if text.galley.text() == app.session.paths[0] && text.galley.rows.len() > 1 =>
            {
                Some(text)
            }
            _ => None,
        })
        .expect("Full selected path should wrap in the pane header");
    assert!(wrapped.pos.x + wrapped.galley.size().x < 450.0);
    assert!(text_positions(&output, "assembly")[0].y > wrapped.pos.y + wrapped.galley.size().y);
    output.drop_without_applying_deltas();
}

#[test]
fn changing_paths_cancels_scan_and_clears_prior_results() {
    let mut app = loaded_app();
    let sender = attach_job(&mut app);
    let flag = app.session.job.as_ref().unwrap().cancellation.clone();
    app.session.invalidate();
    assert!(flag.load(Ordering::Relaxed));
    assert!(app.session.job.is_none() && app.session.tree.is_none() && app.session.roots.is_none());
    assert!(sender.send(Ok(fixture())).is_err());
}

#[test]
fn same_folder_merge_diff_finishes_without_changes_and_keeps_browsing() {
    let fixture = SourceFixture::new();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    app.session.set_workspace_merge(true);
    for side in 0..2 {
        app.session.select_path(side, fixture.path("left"));
    }
    settle_sources(&mut app, &ctx);
    app.session.interactive.as_mut().unwrap().set_run_diff(true);
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    assert!(app.session.job.is_some());
    settle_sources(&mut app, &ctx);
    assert!(app.session.tree.is_some() && app.session.error.is_none());
    assert_eq!(app.session.counts[1..], [0; 5]);
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "model.txt").len(), 0);
    output.drop_without_applying_deltas();
    let filter = ctx
        .read_response(egui::Id::new("Show only differences"))
        .unwrap()
        .rect
        .center();
    click(&mut app, &ctx, filter);
    assert_eq!(
        app.session.interactive.as_ref().unwrap().merge_options(),
        [false, true, true]
    );
    for _ in 0..3 {
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(text_positions(&output, "0 changes").len(), 1);
        assert_eq!(text_positions(&output, "model.txt").len(), 2);
        assert_eq!(
            app.session.interactive.as_ref().unwrap().change_label(),
            "0 changes"
        );
        output.drop_without_applying_deltas();
    }
}

#[test]
fn first_folder_selection_opens_folder_workspace_without_comparing() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.session.select_path(0, sources.path("left"));
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.mode(), SelectionMode::Folder);
    assert!(
        app.session.tree.is_none() && app.session.job.is_none() && app.session.file_view.is_none()
    );
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "Folder Diff").len(), 1);
    assert_eq!(
        text_positions(&output, "Drop a file or folder here").len(),
        2
    );
    output.drop_without_applying_deltas();
}

#[test]
fn header_toggle_shares_the_top_row_and_updates_centered_captions() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for zoom in [1.0, 2.0] {
            let ctx = egui::Context::default();
            set_theme(&ctx, theme);
            ctx.set_zoom_factor(zoom);
            ctx.run_ui(egui::RawInput::default(), |_| {})
                .drop_without_applying_deltas();
            let mut app = VersusApp::default();
            let draw = |app: &mut VersusApp| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            egui::vec2(900.0 / zoom, 650.0 / zoom),
                        )),
                        ..Default::default()
                    },
                    |ui| app.render(ui),
                );
                output.textures_delta.clear();
                output
            };
            for merge in [false, true] {
                app.session.set_workspace_merge(merge);
                let output = draw(&mut app);
                let folder = ctx
                    .read_response(egui::Id::new(ComparisonMode::Folder.label()))
                    .unwrap()
                    .rect;
                let file = ctx
                    .read_response(egui::Id::new(ComparisonMode::File.label()))
                    .unwrap()
                    .rect;
                let toggle = ctx
                    .read_response(egui::Id::new(("workspace-mode", "Merge")))
                    .unwrap()
                    .rect
                    .union(
                        ctx.read_response(egui::Id::new(("workspace-mode", "Diff")))
                            .unwrap()
                            .rect,
                    );
                assert!((folder.union(file).center().x - 450.0 / zoom).abs() < 0.5);
                assert!((toggle.center().y - folder.center().y).abs() < 0.5);
                assert!(
                    toggle.right() + 4.0 < folder.left(),
                    "{toggle:?} overlaps {folder:?}"
                );
                assert_eq!(
                    text_positions(&output, ComparisonMode::Folder.workspace_label(merge)).len(),
                    1
                );
                assert_eq!(
                    text_positions(&output, ComparisonMode::File.workspace_label(merge)).len(),
                    1
                );
                output.drop_without_applying_deltas();
            }
            let toggle = ctx
                .read_response(egui::Id::new(("workspace-mode", "Diff")))
                .unwrap()
                .rect
                .center();
            click(&mut app, &ctx, toggle);
            assert!(!app.session.merge_mode);
        }
    }
}

#[test]
fn errors_use_bottom_right_summary_and_clear_only_on_pane_input() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    app.session.set_workspace_merge(true);
    app.session.error = Some("First error detail".into());
    app.session.source_errors = [
        Some("First error detail".into()),
        Some("Second error detail".into()),
    ];
    let output = render(&mut app, &ctx, vec![]);
    let status = ctx
        .read_response(egui::Id::new("workspace-status"))
        .unwrap()
        .rect;
    let error = ctx
        .read_response(egui::Id::new("workspace-errors"))
        .unwrap()
        .rect;
    assert!(status.contains_rect(error));
    assert!(status.top() >= app.pane_rects[0].bottom());
    assert_eq!(text_positions(&output, "2 errors").len(), 1);
    assert!(text_positions(&output, "First error detail").is_empty());
    assert_eq!(app.error_details(app.session.interactive.as_ref()).len(), 2);
    output.drop_without_applying_deltas();
    ctx.all_styles_mut(|style| {
        style.interaction.tooltip_delay = 0.0;
        style.interaction.show_tooltips_only_when_still = false;
    });
    // Tooltip Areas size themselves invisibly on their first frame.
    render(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(error.center())],
    )
    .drop_without_applying_deltas();
    let output = render(&mut app, &ctx, vec![]);
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("First error detail") && text.galley.text().contains("Second error detail"))));
    output.drop_without_applying_deltas();
    assert!(
        app.session.error.is_some(),
        "hovering errors must keep their details available"
    );
    let pos = app.pane_rects[0].center();
    render(&mut app, &ctx, vec![egui::Event::PointerMoved(pos)]).drop_without_applying_deltas();
    assert!(
        app.error_details(app.session.interactive.as_ref())
            .is_empty()
    );
    app.session.error = Some("New error while pointer is stationary".into());
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    assert!(app.session.error.is_some());
    render(
        &mut app,
        &ctx,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        }],
    )
    .drop_without_applying_deltas();
    assert!(app.session.error.is_none());
}
