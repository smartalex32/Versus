use super::*;
use crate::recent::RecentComparisons;
use eframe::Storage;
use versus::ComparisonProgress;
use versus::{DirectoryDiff, DirectoryEntry};

fn engineering_path(relative: &str) -> String {
    // Windows needs a drive-qualified root to avoid resolving fixtures
    // relative to the CI checkout. Build components with native separators.
    let mut path = PathBuf::from(if cfg!(windows) {
        r"C:\engineering"
    } else {
        "/engineering"
    });
    path.extend(relative.split('/'));
    assert!(path.is_absolute());
    path.display().to_string()
}

fn fixture() -> FolderTree {
    FolderTree::from_diff(&DirectoryDiff {
        cancelled: false,
        entries: [
            ("assembly", DirectoryEntryKind::Directory),
            ("assembly/model.step", DirectoryEntryKind::File),
        ]
        .into_iter()
        .map(|(path, kind)| DirectoryEntry {
            relative_path: path.into(),
            left_size: Some(0),
            right_size: Some(0),
            left_exists: true,
            right_exists: true,
            left_kind: Some(kind.clone()),
            right_kind: Some(kind.clone()),
            kind,
            state: DirectoryEntryState::Same,
        })
        .collect(),
    })
}

// Interaction fixtures exercise the full list and whitespace toggle in a
// known starting state; production defaults are covered separately below.
fn unfiltered_app() -> VersusApp {
    let mut app = VersusApp::default();
    app.session.show_only_differences = false;
    app.session.ignore_whitespace = false;
    app
}

fn loaded_app() -> VersusApp {
    let mut app = unfiltered_app();
    app.session.tree = Some(fixture());
    app.session.roots = Some(["left".into(), "right".into()]);
    app
}

fn render(app: &mut VersusApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    // Keep the simulated physical window size fixed when the UI scale changes.
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(1200.0, 800.0) / ctx.zoom_factor(),
            )),
            events,
            ..Default::default()
        },
        |ui| app.render(ui),
    );
    output.textures_delta.clear();
    output
}

fn text_positions(output: &egui::FullOutput, value: &str) -> Vec<Pos2> {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == value => {
                Some(text.pos + text.galley.rect.min.to_vec2())
            }
            _ => None,
        })
        .collect()
}

fn text_centers(output: &egui::FullOutput, value: &str) -> Vec<Pos2> {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == value => {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            _ => None,
        })
        .collect()
}

fn click(app: &mut VersusApp, ctx: &egui::Context, position: Pos2) {
    for pressed in [true, false] {
        render(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
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
fn toolbar_icons_expand_collapse_and_refresh_the_comparison() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    app.session.paths = ["left".into(), "right".into()];
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    let expand = ctx
        .read_response(egui::Id::new("Expand all"))
        .unwrap()
        .rect
        .center();
    click(&mut app, &ctx, expand);
    assert!(app.session.tree.as_ref().unwrap().is_expanded("assembly"));
    let collapse = ctx
        .read_response(egui::Id::new("Collapse all"))
        .unwrap()
        .rect
        .center();
    click(&mut app, &ctx, collapse);
    assert!(!app.session.tree.as_ref().unwrap().is_expanded("assembly"));
    let refresh = ctx
        .read_response(egui::Id::new("Refresh comparison"))
        .unwrap()
        .rect
        .center();
    click(&mut app, &ctx, refresh);
    assert!(app.session.job.is_some());
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
fn merge_source_paths_stay_on_one_line_and_keep_their_names_visible() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for kind in [SourceKind::Folder, SourceKind::File] {
            let ctx = egui::Context::default();
            set_theme(&ctx, theme);
            let mut app = VersusApp::default();
            app.session.comparison_mode = if kind == SourceKind::File {
                ComparisonMode::File
            } else {
                ComparisonMode::Folder
            };
            app.session.set_workspace_merge(true);
            app.session.source_kinds = [Some(kind); 2];
            let name = if kind == SourceKind::File {
                "model.rs"
            } else {
                "release"
            };
            app.session.paths = [
                engineering_path(&format!("{}project/{name}", "é𐐀目录/".repeat(60))),
                engineering_path(&format!("short/{name}")),
            ];
            let output = render(&mut app, &ctx, vec![]);
            let (shape, path) = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text().starts_with('…') => {
                        Some((shape, text))
                    }
                    _ => None,
                })
                .expect("long Merge paths should have a leading ellipsis");
            let suffix = PathBuf::from("project").join(name);
            assert!(path.galley.text().ends_with(suffix.to_str().unwrap()));
            assert_eq!(path.galley.rows.len(), 1);
            let bounds = Rect::from_min_size(path.pos, path.galley.size());
            assert!(app.source_header_rects[0].contains_rect(bounds));
            assert!(shape.clip_rect.contains_rect(bounds));
            let browse = ctx
                .read_response(egui::Id::new(format!(
                    "Browse left {}",
                    if kind == SourceKind::File {
                        "file"
                    } else {
                        "folder"
                    }
                )))
                .unwrap()
                .rect;
            assert!(bounds.right() < browse.left());
            let short = text_centers(&output, &app.session.paths[1])[0];
            assert!((bounds.center().y - short.y).abs() < 0.1);
            let height = app.source_header_rects[0].height();
            output.drop_without_applying_deltas();
            app.session.paths[0] = engineering_path(&format!("short/{name}"));
            render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
            assert_eq!(app.source_header_rects[0].height(), height);
        }
    }
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
fn themed_buttons_use_pointer_cursor_without_overriding_disabled_or_text_fields() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        set_theme(&ctx, theme);
        let mut text = String::from("editable text");
        let mut rects = [Rect::NOTHING; 3];
        let mut frame = |position: Option<Pos2>| {
            ctx.run_ui(
                egui::RawInput {
                    events: position
                        .map(egui::Event::PointerMoved)
                        .into_iter()
                        .collect(),
                    ..Default::default()
                },
                |ui| {
                    rects[0] = ui.button("Save").rect;
                    rects[1] = ui.add_enabled(false, egui::Button::new("Unavailable")).rect;
                    rects[2] = ui.text_edit_singleline(&mut text).rect;
                },
            )
        };
        frame(None).drop_without_applying_deltas();
        // Widget responses describe the previous pass; capture settled rectangles.
        frame(None).drop_without_applying_deltas();
        drop(frame);
        for (index, expected) in [
            egui::CursorIcon::PointingHand,
            egui::CursorIcon::Default,
            egui::CursorIcon::Text,
        ]
        .into_iter()
        .enumerate()
        {
            let output = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::PointerMoved(rects[index].center())],
                    ..Default::default()
                },
                |ui| {
                    let _ = ui.button("Save");
                    ui.add_enabled(false, egui::Button::new("Unavailable"));
                    ui.text_edit_singleline(&mut text);
                },
            );
            assert_eq!(
                output.platform_output.cursor_icon, expected,
                "{theme:?}, {index}"
            );
            output.drop_without_applying_deltas();
        }
    }
}

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
fn hovering_files_and_folders_uses_the_pointer_cursor() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    app.session.tree.as_mut().unwrap().expand_all();
    let output = render(&mut app, &ctx, vec![]);
    let targets = [
        text_positions(&output, "assembly")[0],
        text_positions(&output, "model.step")[1],
    ];
    output.drop_without_applying_deltas();
    for position in targets {
        let output = render(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(position + egui::vec2(3.0, 5.0))],
        );
        assert_eq!(
            output.platform_output.cursor_icon,
            egui::CursorIcon::PointingHand
        );
        output.drop_without_applying_deltas();
    }
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

fn test_file_row(
    left: Option<(usize, String)>,
    right: Option<(usize, String)>,
    state: DirectoryEntryState,
) -> versus::FileComparisonRow {
    versus::FileComparisonRow {
        left_ending: left.as_ref().map(|_| versus::DisplayLineEnding::Lf),
        right_ending: right.as_ref().map(|_| versus::DisplayLineEnding::Lf),
        left_changed: Vec::new(),
        right_changed: Vec::new(),
        left,
        right,
        state,
    }
}

fn loaded_file_view() -> FileView {
    FileView {
        row_height: ROW_HEIGHT,
        from_folders: true,
        paths: ["/left/model.step".into(), "/right/model.step".into()],
        sources: [
            Some("/left/model.step".into()),
            Some("/right/model.step".into()),
        ],
        job: None,
        comparison: Some(FileComparison {
            rows: vec![
                test_file_row(
                    Some((1, "unchanged".into())),
                    Some((1, "unchanged".into())),
                    DirectoryEntryState::Same,
                ),
                test_file_row(
                    Some((2, "old value".into())),
                    Some((2, "new value".into())),
                    DirectoryEntryState::Different,
                ),
                test_file_row(
                    Some((3, "removed".into())),
                    None,
                    DirectoryEntryState::LeftOnly,
                ),
            ],
            message: None,
        }),
        error: None,
        error_icon: StatusIcon::Error,
        scroll_y: 0.0,
        content_widths: [300.0; 2],
        gutter_extra: 0.0,
        counts: [1, 1, 1, 0, 0, 0],
        visible_rows: vec![0, 1, 2],
        difference_rows: vec![1, 2],
        navigation_row: None,
        navigation_scroll_pending: false,
        ignore_line_endings: true,
    }
}

#[test]
fn double_clicking_file_on_either_side_opens_view_and_back_restores_tree() {
    for side in 0..2 {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.session.tree.as_mut().unwrap().expand_all();
        let generation = app.session.scroll_generation;
        let output = render(&mut app, &ctx, vec![]);
        let target = text_positions(&output, "model.step")[side] + egui::vec2(3.0, 5.0);
        output.drop_without_applying_deltas();
        click(&mut app, &ctx, target);
        assert!(
            app.session.file_view.is_none(),
            "Single click should only select a file"
        );
        click(&mut app, &ctx, target);
        let view = app
            .session
            .file_view
            .as_ref()
            .expect("Double click should open file comparison");
        assert_eq!(
            view.paths,
            [
                PathBuf::from("left/assembly/model.step"),
                PathBuf::from("right/assembly/model.step")
            ]
        );
        let cancellation = view.job.as_ref().unwrap().cancellation.clone();
        let output = render(&mut app, &ctx, vec![]);
        let back = ctx
            .read_response(egui::Id::new("Back to folders"))
            .unwrap()
            .rect
            .center();
        assert!(text_positions(&output, "assembly").is_empty());
        output.drop_without_applying_deltas();
        click(&mut app, &ctx, back);
        assert!(app.session.file_view.is_none());
        assert!(cancellation.load(Ordering::Relaxed));
        assert!(app.session.tree.as_ref().unwrap().is_expanded("assembly"));
        assert_eq!(
            app.session.selected.as_deref(),
            Some(std::path::Path::new("assembly/model.step"))
        );
        assert_eq!(app.session.scroll_generation, generation);
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(text_positions(&output, "model.step").len(), 2);
        output.drop_without_applying_deltas();
    }
}

#[test]
fn file_headers_match_folder_styles_and_elide_long_paths_on_the_left() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let ctx = egui::Context::default();
        set_theme(&ctx, theme);
        let mut app = loaded_app();
        let draw = |app: &mut VersusApp| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 650.0))),
                    ..Default::default()
                },
                |ui| app.render(ui),
            );
            output.textures_delta.clear();
            output
        };
        let label_style = |output: &egui::FullOutput, label: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => Some((
                        text.fallback_color,
                        text.galley.job.sections[0].format.font_id.clone(),
                        text.pos.x,
                    )),
                    _ => None,
                })
                .unwrap()
        };
        let output = draw(&mut app);
        let folder_styles = [label_style(&output, "LEFT"), label_style(&output, "RIGHT")];
        output.drop_without_applying_deltas();
        let mut view = loaded_file_view();
        view.paths = [
            format!(
                "C:\\engineering\\{}assembly\\model.step",
                "é𐐀目录\\".repeat(60)
            )
            .into(),
            "/r/model.step".into(),
        ];
        app.session.file_view = Some(view);
        let output = draw(&mut app);
        for (side, label) in ["LEFT", "RIGHT"].into_iter().enumerate() {
            assert_eq!(label_style(&output, label), folder_styles[side]);
        }
        let left_path = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().starts_with('…') => {
                    Some((shape, text))
                }
                _ => None,
            })
            .expect("Long path should show a leading ellipsis");
        let (shape, text) = left_path;
        assert!(text.galley.text().ends_with("assembly\\model.step"));
        assert_eq!(text.galley.rows.len(), 1);
        let bounds = Rect::from_min_size(text.pos, text.galley.size());
        assert!(bounds.right() < 450.0);
        assert!(shape.clip_rect.contains_rect(bounds));
        let path_y = bounds.center().y;
        for label in ["LEFT", "RIGHT", "/r/model.step"] {
            assert!((text_centers(&output, label)[0].y - path_y).abs() < 0.1);
        }
        output.drop_without_applying_deltas();
    }
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
fn scrolling_either_file_pane_keeps_line_rows_aligned() {
    for side in 0..2 {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
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
        app.session.file_view = Some(view);
        render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
        let pointer = egui::pos2(if side == 0 { 300.0 } else { 900.0 }, 300.0);
        let output = render(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0.0, -180.0),
                    modifiers: Default::default(),
                },
            ],
        );
        let assert_aligned = |output: &egui::FullOutput| {
            for number in 1..=200 {
                let positions = text_positions(output, &format!("line {number}"));
                if !positions.is_empty() {
                    assert_eq!(
                        positions.len(),
                        2,
                        "Line {number} should appear in both panes"
                    );
                    assert!(
                        (positions[0].y - positions[1].y).abs() < 0.1,
                        "Line {number} is misaligned: {positions:?}"
                    );
                }
            }
        };
        assert_aligned(&output);
        output.drop_without_applying_deltas();
        for _ in 0..20 {
            let output = render(&mut app, &ctx, vec![]);
            assert_aligned(&output);
            output.drop_without_applying_deltas();
        }
        assert!(app.session.file_view.as_ref().unwrap().scroll_y > 0.0);
    }
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
fn theme_toggle_updates_both_views_without_losing_state() {
    for file_mode in [false, true] {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.session.tree.as_mut().unwrap().expand_all();
        app.session.selected = Some("assembly/model.step".into());
        if file_mode {
            app.session.file_view = Some(loaded_file_view());
        }
        let generation = app.session.scroll_generation;
        for (action, expected_dark) in [
            ("Switch to light mode", false),
            ("Switch to dark mode", true),
        ] {
            let output = render(&mut app, &ctx, vec![]);
            let toggle = ctx.read_response(egui::Id::new(action)).unwrap().rect;
            assert!(toggle.right() <= 1190.0 && toggle.left() > 1100.0 && toggle.bottom() < 45.0);
            assert!(
                text_positions(&output, action).is_empty(),
                "Theme action must use only an icon"
            );
            output.drop_without_applying_deltas();
            click(&mut app, &ctx, toggle.center());
            let output = render(&mut app, &ctx, vec![]);
            assert_eq!(ctx.theme() == egui::Theme::Dark, expected_dark);
            assert_eq!(app.logo_texture.as_ref().unwrap().0, expected_dark);
            assert_eq!(app.session.file_view.is_some(), file_mode);
            assert!(app.session.tree.as_ref().unwrap().is_expanded("assembly"));
            assert_eq!(
                app.session.selected.as_deref(),
                Some(std::path::Path::new("assembly/model.step"))
            );
            assert_eq!(app.session.scroll_generation, generation);
            let neutral = if file_mode { "unchanged" } else { "model.step" };
            let palette = Palette::new(expected_dark);
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape {
                    if text.galley.text() == neutral {
                        assert_eq!(text.fallback_color, palette.muted);
                    }
                }
            }
            output.drop_without_applying_deltas();
        }
    }
}

#[test]
fn logo_texture_respects_transparency_of_stray_and_edge_pixels() {
    let ctx = egui::Context::default();
    let mut app = loaded_app();
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        set_theme(&ctx, theme);
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                ..Default::default()
            },
            |ui| app.render(ui),
        );
        let logo_id = app.logo_texture.as_ref().unwrap().1.id();
        let delta = &output.textures_delta.set.get(&logo_id).unwrap()[0];
        let egui::ImageData::Color(image) = &delta.image;
        let speck = image[(91, 74)];
        assert_eq!(speck.a(), 1);
        assert!(
            speck.r() <= 1 && speck.g() <= 1 && speck.b() <= 1,
            "Nearly transparent pixels must not become bright white"
        );
        for pixel in &image.pixels {
            assert!(
                pixel.r() <= pixel.a() && pixel.g() <= pixel.a() && pixel.b() <= pixel.a(),
                "Logo texture must use premultiplied alpha"
            );
        }
        output.drop_without_applying_deltas();
    }
}

#[test]
fn theme_changes_send_matching_icon_to_native_window() {
    let ctx = egui::Context::default();
    let mut app = loaded_app();
    for theme in [egui::Theme::Dark, egui::Theme::Light, egui::Theme::Dark] {
        set_theme(&ctx, theme);
        let output = render(&mut app, &ctx, vec![]);
        let expected = crate::logo::themed_icon(theme == egui::Theme::Dark);
        assert!(output.viewport_output.values().flat_map(|viewport| &viewport.commands).any(|command| {
            matches!(command,egui::ViewportCommand::Icon(Some(icon)) if icon.rgba == expected.rgba)
        }), "Native window must receive the matching theme icon");
        output.drop_without_applying_deltas();
    }
}

#[test]
fn file_type_mismatch_opens_explanation_without_loading_nonfile_side() {
    let mut app = loaded_app();
    app.session
        .open_file("assembly/model.step".into(), [true, true], true);
    let view = app.session.file_view.as_ref().unwrap();
    assert!(view.job.is_none());
    assert!(
        view.error
            .as_deref()
            .unwrap()
            .contains("Entry types differ")
    );
    assert!(matches!(view.error_icon, StatusIcon::TypeMismatch));
}

#[test]
fn file_worker_errors_remain_in_file_view_with_back_available() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    let (sender, receiver) = mpsc::channel();
    let mut view = loaded_file_view();
    view.comparison = None;
    view.job = Some(FileJob {
        receiver,
        cancellation: Arc::new(AtomicBool::new(false)),
        progress: Arc::new(ComparisonProgress::default()),
        ignore_line_endings: true,
    });
    app.session.file_view = Some(view);
    sender
        .send(Err(CompareError {
            path: None,
            kind: versus::CompareErrorKind::InvalidPath,
            message: "file unavailable".into(),
        }))
        .unwrap();
    app.session.poll_file_comparison(&ctx);
    let output = render(&mut app, &ctx, vec![]);
    assert!(
        ctx.read_response(egui::Id::new("Back to folders"))
            .is_some()
    );
    assert!(
        app.session
            .file_view
            .as_ref()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("file unavailable")
    );
    assert!(app.session.tree.as_ref().is_some());
    output.drop_without_applying_deltas();
}

fn attach_job(app: &mut VersusApp) -> mpsc::Sender<Result<FolderTree, CompareError>> {
    let (sender, receiver) = mpsc::channel();
    app.session.job = Some(ComparisonJob {
        receiver,
        cancellation: Arc::new(AtomicBool::new(false)),
        roots: ["new-left".into(), "new-right".into()],
        started: Instant::now(),
        logged_progress: None,
        last_progress_log: Instant::now(),
        progress: Arc::new(ComparisonProgress::default()),
    });
    sender
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
fn folder_worker_panic_becomes_a_status_error_and_releases_the_job() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    let sender = attach_job(&mut app);
    std::thread::spawn(move || {
        let result = crate::diagnostics::catch_worker(
            "Folder comparison",
            || -> Result<FolderTree, &'static str> {
                panic!("test comparison failure");
            },
        )
        .map_err(|message| CompareError {
            path: None,
            kind: versus::CompareErrorKind::Io,
            message,
        });
        sender.send(result).unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.session.job.is_some() {
        app.session.poll_comparison(&ctx);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        app.session
            .error
            .as_ref()
            .unwrap()
            .contains("worker panicked")
    );
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "1 error").len(), 1);
    assert!(
        app.session.tree.is_some(),
        "failure keeps the previous tree usable"
    );
    output.drop_without_applying_deltas();
}

#[test]
fn cancelled_completed_job_does_not_replace_last_comparison() {
    let mut app = loaded_app();
    let sender = attach_job(&mut app);
    sender.send(Ok(fixture())).unwrap();
    app.session
        .job
        .as_ref()
        .unwrap()
        .cancellation
        .store(true, Ordering::Relaxed);
    app.session.poll_comparison(&egui::Context::default());
    assert!(app.session.job.is_none());
    assert_eq!(
        app.session.roots.as_ref().unwrap(),
        &[PathBuf::from("left"), PathBuf::from("right")]
    );
    assert_eq!(app.session.message, "Comparison cancelled.");
}

#[test]
fn failed_worker_reports_error_and_keeps_last_comparison() {
    let mut app = loaded_app();
    drop(attach_job(&mut app));
    app.session.poll_comparison(&egui::Context::default());
    assert!(
        app.session
            .error
            .as_ref()
            .unwrap()
            .contains("worker stopped")
    );
    assert!(app.session.tree.is_some());
    assert_eq!(
        app.session.roots.as_ref().unwrap()[0],
        PathBuf::from("left")
    );
}

struct SourceFixture(PathBuf);

impl SourceFixture {
    fn new() -> Self {
        static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "versus-source-ui-{unique}-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("left")).unwrap();
        std::fs::create_dir_all(root.join("right")).unwrap();
        std::fs::write(root.join("left/model.txt"), "common\nold\n").unwrap();
        std::fs::write(root.join("right/model.txt"), "common\nnew\n").unwrap();
        Self(root)
    }
    fn path(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }
}

impl Drop for SourceFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn settle_sources(app: &mut VersusApp, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.session.poll_sources(ctx);
        app.session.poll_comparison(ctx);
        app.session.poll_file_comparison(ctx);
        if app.session.source_jobs.iter().all(Option::is_none)
            && app.session.job.is_none()
            && app
                .session
                .file_view
                .as_ref()
                .is_none_or(|view| view.job.is_none())
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Comparison worker did not complete"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn command_line_files_open_direct_comparison_in_supplied_order_without_writes() {
    let sources = SourceFixture::new();
    let paths = [
        sources.path("left/model.txt"),
        sources.path("right/model.txt"),
    ];
    for mode in [None, Some(ComparisonMode::File)] {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = VersusApp::default();
        app.session.open_launch_request(crate::cli::LaunchRequest {
            paths: paths.clone(),
            mode,
        });
        settle_sources(&mut app, &ctx);
        assert_eq!(app.session.comparison_mode, ComparisonMode::File);
        let view = app.session.file_view.as_ref().unwrap();
        assert_eq!(view.paths, paths);
        assert_eq!(view.sources, paths.clone().map(Some));
        assert!(!view.from_folders);
        assert_eq!(view.counts[1], 1);
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(text_positions(&output, "old").len(), 1);
        assert_eq!(text_positions(&output, "new").len(), 1);
        assert!(
            ctx.read_response(egui::Id::new("Back to folders"))
                .is_none()
        );
        output.drop_without_applying_deltas();
    }
    assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "common\nold\n");
    assert_eq!(std::fs::read_to_string(&paths[1]).unwrap(), "common\nnew\n");
}

#[test]
fn command_line_folders_choose_folder_mode_and_reject_forced_file_mode() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let paths = [sources.path("left"), sources.path("right")];
    for mode in [None, Some(ComparisonMode::Folder)] {
        let mut app = VersusApp::default();
        app.session.open_launch_request(crate::cli::LaunchRequest {
            paths: paths.clone(),
            mode,
        });
        settle_sources(&mut app, &ctx);
        assert_eq!(app.session.comparison_mode, ComparisonMode::Folder);
        assert_eq!(app.session.roots, Some(paths.clone()));
        assert!(app.session.tree.is_some() && app.session.file_view.is_none());
    }
    for (mode, paths) in [
        (ComparisonMode::File, paths),
        (
            ComparisonMode::Folder,
            [
                sources.path("left/model.txt"),
                sources.path("right/model.txt"),
            ],
        ),
    ] {
        let mut app = VersusApp::default();
        app.session.open_launch_request(crate::cli::LaunchRequest {
            paths,
            mode: Some(mode),
        });
        settle_sources(&mut app, &ctx);
        assert!(app.session.error.as_ref().unwrap().contains("requires two"));
        assert!(app.session.tree.is_none() && app.session.file_view.is_none());
    }
}

#[test]
fn command_line_invalid_sources_show_errors_and_new_discards_pending_launch() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    for paths in [
        [sources.path("left/model.txt"), sources.path("missing")],
        [sources.path("left/model.txt"), sources.path("right")],
    ] {
        let mut app = VersusApp::default();
        app.session
            .open_launch_request(crate::cli::LaunchRequest { paths, mode: None });
        settle_sources(&mut app, &ctx);
        assert!(app.session.error.is_some() || app.session.mode() == SelectionMode::Incompatible);
        assert!(app.session.tree.is_none() && app.session.file_view.is_none());
    }
    let mut app = VersusApp::default();
    app.session.open_launch_request(crate::cli::LaunchRequest {
        paths: [
            sources.path("left/model.txt"),
            sources.path("right/model.txt"),
        ],
        mode: None,
    });
    app.session.new_comparison();
    settle_sources(&mut app, &ctx);
    assert!(app.session.paths.iter().all(String::is_empty));
    assert!(app.session.source_paths.iter().all(Option::is_none));
    assert!(app.session.launch_mode.is_none());
    assert!(app.session.tree.is_none() && app.session.file_view.is_none());
}

#[test]
fn git_null_device_inputs_show_added_and_deleted_lines_without_opening_devices() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut nulls = vec![PathBuf::from("/dev/null")];
    if cfg!(windows) {
        nulls.extend([PathBuf::from("NUL"), PathBuf::from("nul")]);
    }
    for null in nulls {
        for empty_side in 0..2 {
            let mut paths = [
                sources.path("left/model.txt"),
                sources.path("right/model.txt"),
            ];
            paths[empty_side] = null.clone();
            let mut app = VersusApp::default();
            app.session.open_launch_request(crate::cli::LaunchRequest {
                paths,
                mode: Some(ComparisonMode::File),
            });
            settle_sources(&mut app, &ctx);
            let view = app.session.file_view.as_ref().unwrap();
            assert!(view.error.is_none());
            assert!(view.sources[empty_side].is_none());
            let expected = if empty_side == 0 {
                DirectoryEntryState::RightOnly
            } else {
                DirectoryEntryState::LeftOnly
            };
            let rows = &view.comparison.as_ref().unwrap().rows;
            assert_eq!(rows.len(), 2);
            assert!(rows.iter().all(|row| row.state == expected));
        }
    }
}

#[test]
fn command_line_native_paths_survive_display_conversion() {
    #[cfg(target_os = "linux")]
    use std::os::unix::ffi::OsStringExt;
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    // Linux permits non-UTF-8 filenames; macOS filesystems require Unicode.
    #[cfg(target_os = "linux")]
    let left_name = std::ffi::OsString::from_vec(b"left \xff.txt".to_vec());
    #[cfg(not(target_os = "linux"))]
    let left_name = std::ffi::OsString::from("left space é.txt");
    let left = sources.0.join(left_name);
    let right = sources.path("right space \u{6bd4}\u{8f03}.txt");
    std::fs::write(&left, "before\n").unwrap();
    std::fs::write(&right, "after\n").unwrap();
    let mut app = VersusApp::default();
    app.session.open_launch_request(crate::cli::LaunchRequest {
        paths: [left.clone(), right.clone()],
        mode: Some(ComparisonMode::File),
    });
    settle_sources(&mut app, &ctx);
    let view = app.session.file_view.as_ref().unwrap();
    assert_eq!(view.sources, [Some(left), Some(right)]);
    assert!(view.error.is_none());
    assert_eq!(view.counts[1], 1);
}

#[test]
fn browse_results_replace_folder_state_and_open_the_file_workspace() {
    let sources = SourceFixture::new();
    for (side, path_area) in [(0, false), (1, true)] {
        let ctx = egui::Context::default();
        let mut app = loaded_app();
        app.session.comparison_mode = ComparisonMode::File;
        let picked = sources.path(if side == 0 {
            "left/model.txt"
        } else {
            "right/model.txt"
        });
        let mut requests = Vec::new();
        {
            let mut draw = |events| {
                ctx.run_ui(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        app.folder_inputs_with_picker(ui, |requested_side, _, _| {
                            requests.push(requested_side);
                            Some(picked.clone())
                        })
                    },
                )
            };
            let output = draw(vec![]);
            let target = if path_area {
                text_centers(&output, "Choose a file")[side]
            } else {
                ctx.read_response(egui::Id::new("Browse left file"))
                    .unwrap()
                    .rect
                    .center()
            };
            output.drop_without_applying_deltas();
            for pressed in [true, false] {
                draw(vec![
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
        assert_eq!(requests, vec![side]);
        assert_eq!(app.session.paths[side], picked.display().to_string());
        assert!(app.session.tree.is_none() && app.session.roots.is_none());
        settle_sources(&mut app, &ctx);
        assert_eq!(app.session.mode(), SelectionMode::File);
        assert!(!app.session.file_view.as_ref().unwrap().from_folders);
    }
}

#[test]
fn selected_sources_switch_views_and_compare_only_matching_types() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    app.session.switch_comparison_mode(ComparisonMode::File);
    // Starting on the right must work just as starting on the left does.
    app.session.select_path(1, sources.path("right/model.txt"));
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.mode(), SelectionMode::File);
    let view = app.session.file_view.as_ref().unwrap();
    assert!(!view.from_folders && view.comparison.is_none() && view.job.is_none());
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "File Diff").len(), 1);
    assert!(
        ctx.read_response(egui::Id::new("Back to folders"))
            .is_none()
    );
    assert!(
        ctx.read_response(egui::Id::new("Browse left file"))
            .is_some()
    );
    assert!(
        ctx.read_response(egui::Id::new("Browse right file"))
            .is_some()
    );
    output.drop_without_applying_deltas();
    app.session.select_path(0, sources.path("left/model.txt"));
    settle_sources(&mut app, &ctx);
    assert!(app.session.file_view.as_ref().unwrap().comparison.is_some());
    assert_eq!(app.session.file_view.as_ref().unwrap().counts[1], 1);
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "old").len(), 1);
    assert_eq!(text_positions(&output, "new").len(), 1);
    assert!(
        ctx.read_response(egui::Id::new("Back to folders"))
            .is_none()
    );
    output.drop_without_applying_deltas();
    // Replacing the right file with a folder rejects the mixed pair.
    app.session.select_path(1, sources.path("right"));
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.mode(), SelectionMode::Incompatible);
    assert!(
        app.session.file_view.is_none() && app.session.tree.is_none() && app.session.job.is_none()
    );
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(
        text_positions(&output, "Incompatible source types").len(),
        1
    );
    assert!(text_positions(&output, &app.session.message).len() >= 1);
    assert!(
        app.session
            .message
            .contains("No comparison can be made between a file and a folder")
    );
    output.drop_without_applying_deltas();
    // Replacing the left side with a folder restores automatic comparison.
    app.session.select_path(0, sources.path("left"));
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.mode(), SelectionMode::Folder);
    assert!(app.session.file_view.is_none() && app.session.tree.is_some());
    assert_eq!(app.session.counts[1], 1);
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

fn settle_merge_file(app: &mut VersusApp, ctx: &egui::Context, side: usize, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.session.poll_sources(ctx);
        app.session.interactive.as_mut().unwrap().poll(ctx);
        if app.session.interactive.as_ref().unwrap().working_text(side) == Some(expected) {
            break;
        }
        assert!(Instant::now() < deadline, "Merge side did not load");
        std::thread::sleep(Duration::from_millis(1));
    }
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
fn filtered_merge_typing_after_hidden_lines_preserves_source_text_and_undo() {
    let fixture = SourceFixture::new();
    let left = "prefix\noldfirst\nmiddle\noldsecond\nsuffix\n";
    let right = "prefix\nnewfirst\nmiddle\nnewsecond\nsuffix\n";
    std::fs::write(fixture.path("left/model.txt"), left).unwrap();
    std::fs::write(fixture.path("right/model.txt"), right).unwrap();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    app.session.comparison_mode = ComparisonMode::File;
    app.session.set_workspace_merge(true);
    app.session.select_path(0, fixture.path("left/model.txt"));
    app.session.select_path(1, fixture.path("right/model.txt"));
    settle_merge_file(&mut app, &ctx, 0, left);
    settle_merge_file(&mut app, &ctx, 1, right);
    app.session.interactive.as_mut().unwrap().set_run_diff(true);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app
        .session
        .interactive
        .as_ref()
        .unwrap()
        .loaded_pair_comparison()
        .is_none()
    {
        app.session.interactive.as_mut().unwrap().poll(&ctx);
        assert!(Instant::now() < deadline, "Merge comparison did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
    for _ in 0..3 {
        render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    }
    let output = render(&mut app, &ctx, vec![]);
    let position = text_positions(&output, "oldfirst\noldsecond\n")[0];
    let font = comparison_font(&ctx);
    let row_height = ROW_HEIGHT.max(ctx.fonts_mut(|fonts| fonts.row_height(&font)).ceil() + 8.0);
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, position + egui::vec2(1.0, row_height + 6.0));
    render(&mut app, &ctx, vec![egui::Event::Text("X".into())]).drop_without_applying_deltas();
    assert_eq!(
        app.session.interactive.as_ref().unwrap().working_text(0),
        Some("prefix\noldfirst\nmiddle\nXoldsecond\nsuffix\n")
    );
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    )
    .drop_without_applying_deltas();
    render(
        &mut app,
        &ctx,
        vec![
            egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::Text("Y".into()),
        ],
    )
    .drop_without_applying_deltas();
    assert_eq!(
        app.session.interactive.as_ref().unwrap().working_text(0),
        Some("prefix\noldfirst\nmiddle\nX\nYoldsecond\nsuffix\n")
    );
    for _ in 0..3 {
        click_action(&mut app, &ctx, "Undo");
    }
    assert_eq!(
        app.session.interactive.as_ref().unwrap().working_text(0),
        Some(left)
    );
    assert_eq!(
        app.session.interactive.as_ref().unwrap().working_text(1),
        Some(right)
    );
    assert_eq!(
        std::fs::read_to_string(fixture.path("left/model.txt")).unwrap(),
        left
    );
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
fn comparison_mode_buttons_stay_centered_with_icons_in_both_themes() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for (width, height) in [(900.0, 650.0), (1200.0, 800.0)] {
            for state in 0..4 {
                let ctx = egui::Context::default();
                set_theme(&ctx, theme);
                let mut app = if state == 0 {
                    VersusApp::default()
                } else {
                    loaded_app()
                };
                let _sender = (state == 2).then(|| attach_job(&mut app));
                if state == 3 {
                    app.session.comparison_mode = ComparisonMode::File;
                    let mut view = loaded_file_view();
                    view.from_folders = false;
                    app.session.file_view = Some(view);
                }
                let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(width, height));
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        ..Default::default()
                    },
                    |ui| app.render(ui),
                );
                let texture = app.logo_texture.as_ref().unwrap().1.id();
                let logo = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect) if rect.fill_texture_id() == texture => {
                            Some(rect.rect)
                        }
                        _ => None,
                    })
                    .unwrap();
                let folder = ctx
                    .read_response(egui::Id::new("Folder Diff"))
                    .unwrap()
                    .rect;
                let file = ctx.read_response(egui::Id::new("File Diff")).unwrap().rect;
                assert!(ctx.read_response(egui::Id::new("File Merge")).is_none());
                assert!((folder.union(file).center().x - screen.center().x).abs() < 0.5);
                assert!(folder.right() < file.left());
                assert!(file.right() < screen.right());
                assert!(logo.right() < folder.left());
                let theme_button = ctx
                    .read_response(egui::Id::new(if theme == egui::Theme::Dark {
                        "Switch to light mode"
                    } else {
                        "Switch to dark mode"
                    }))
                    .unwrap()
                    .rect;
                assert!(file.right() < theme_button.left());
                for (label, rect) in [("Folder Diff", folder), ("File Diff", file)] {
                    assert!(screen.contains_rect(rect));
                    let text = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) if text.galley.text() == label => Some(text),
                            _ => None,
                        })
                        .unwrap();
                    let bounds = Rect::from_min_size(text.pos, text.galley.size());
                    assert!((bounds.center().y - logo.center().y).abs() < 1.0);
                    assert!(rect.contains_rect(bounds));
                    let icon_center = egui::pos2(
                        rect.left() + ctx.style_of(theme).spacing.button_padding.x + 8.0,
                        rect.center().y,
                    );
                    let icon = Rect::from_center_size(icon_center, egui::vec2(16.0, 16.0));
                    assert!(icon.right() < bounds.left());
                    assert!(
                        output.shapes.iter().any(|shape| match &shape.shape {
                            egui::Shape::Rect(shape) if label == "Folder Diff" =>
                                icon.contains_rect(shape.rect)
                                    && shape.rect.width() == 13.0
                                    && shape.stroke.width > 0.0,
                            egui::Shape::Path(shape) if label == "File Diff" =>
                                shape.closed
                                    && shape.points.len() == 5
                                    && shape.points.iter().all(|point| icon.contains(*point)),
                            _ => false,
                        }),
                        "{label} must have a visible icon"
                    );
                }
                output.drop_without_applying_deltas();
            }
        }
    }
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

#[test]
fn diff_after_merge_keeps_source_errors_in_bottom_status() {
    let fixture = SourceFixture::new();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    app.session.set_workspace_merge(true);
    app.session.select_path(0, fixture.path("left/model.txt"));
    settle_merge_file(&mut app, &ctx, 0, "common\nold\n");
    app.session.set_workspace_merge(false);
    assert!(app.session.merge_file_open);
    app.session.source_errors[1] = Some("Cannot inspect peer source".into());
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "1 error").len(), 1);
    assert!(
        ctx.read_response(egui::Id::new("workspace-status"))
            .unwrap()
            .rect
            .top()
            >= app.pane_rects[0].bottom()
    );
    output.drop_without_applying_deltas();
}

#[test]
fn merge_folder_search_and_refresh_are_icon_actions_in_the_expected_rows() {
    let fixture = SourceFixture::new();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    app.session.set_workspace_merge(true);
    app.session.select_path(0, fixture.path("left"));
    settle_sources(&mut app, &ctx);
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    let new = ctx
        .read_response(egui::Id::new("New comparison"))
        .unwrap()
        .rect;
    let search = ctx
        .read_response(egui::Id::new("Find and replace"))
        .unwrap()
        .rect;
    assert!(search.left() > new.right());
    assert!((search.center().y - new.center().y).abs() < 0.5);
    assert!(
        ctx.read_response(egui::Id::new("Refresh comparison"))
            .is_none()
    );
    click_action(&mut app, &ctx, "Find and replace");
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "Find and Replace").len(), 1);
    output.drop_without_applying_deltas();
    app.session.interactive.as_mut().unwrap().set_run_diff(true);
    let output = render(&mut app, &ctx, vec![]);
    let refresh = ctx
        .read_response(egui::Id::new("Refresh comparison"))
        .unwrap()
        .rect;
    let run = text_positions(&output, "Run Diff")[0];
    assert!(refresh.left() > run.x);
    assert!(refresh.top() < app.pane_rects[0].top());
    output.drop_without_applying_deltas();
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

#[test]
fn compare_icons_are_clickable_and_mode_buttons_support_keyboard_activation() {
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    let file = ctx.read_response(egui::Id::new("File Diff")).unwrap().rect;
    let icon = egui::pos2(file.left() + 12.0, file.center().y);
    let output = render(&mut app, &ctx, vec![egui::Event::PointerMoved(icon)]);
    assert_eq!(
        output.platform_output.cursor_icon,
        egui::CursorIcon::PointingHand
    );
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, icon);
    assert_eq!(app.session.comparison_mode, ComparisonMode::File);
    for (key, mode) in [
        (egui::Key::Enter, ComparisonMode::Folder),
        (egui::Key::Space, ComparisonMode::File),
    ] {
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new(mode.label())));
        render(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        )
        .drop_without_applying_deltas();
        assert_eq!(app.session.comparison_mode, mode);
    }
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
fn new_comparison_button_clears_both_sides_and_preserves_theme() {
    let ctx = egui::Context::default();
    set_theme(&ctx, egui::Theme::Light);
    let mut app = VersusApp::default();
    app.session.paths = ["/left/model.step".into(), "/right/model.step".into()];
    app.session.source_kinds = [Some(SourceKind::File); 2];
    let mut view = loaded_file_view();
    view.from_folders = false;
    app.session.file_view = Some(view);
    let output = render(&mut app, &ctx, vec![]);
    let new = ctx
        .read_response(egui::Id::new("New comparison"))
        .unwrap()
        .rect;
    let legend = text_centers(&output, "Different  1")[0];
    assert!(new.center().y < app.pane_rects[0].top());
    assert!(legend.y > app.pane_rects[0].bottom());
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, new.center());
    assert_eq!(app.session.paths, [String::new(), String::new()]);
    assert_eq!(app.session.mode(), SelectionMode::Empty);
    assert!(
        app.session.file_view.is_none()
            && app.session.tree.is_none()
            && app.session.roots.is_none()
    );
    assert_eq!(ctx.theme(), egui::Theme::Light);
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "File Diff").len(), 1);
    assert_eq!(text_positions(&output, "Choose a folder").len(), 2);
    output.drop_without_applying_deltas();
}

#[test]
fn new_comparison_cancels_all_jobs_and_discards_late_results() {
    let mut app = loaded_app();
    let folder_sender = attach_job(&mut app);
    let folder_flag = app.session.job.as_ref().unwrap().cancellation.clone();
    let (file_sender, file_receiver) = mpsc::channel();
    let file_flag = Arc::new(AtomicBool::new(false));
    let mut view = loaded_file_view();
    view.job = Some(FileJob {
        receiver: file_receiver,
        cancellation: file_flag.clone(),
        progress: Arc::new(ComparisonProgress::default()),
        ignore_line_endings: true,
    });
    app.session.file_view = Some(view);
    let (source_sender, source_receiver) = mpsc::channel();
    let source_flag = Arc::new(AtomicBool::new(false));
    app.session.source_jobs[0] = Some(SourceJob {
        receiver: source_receiver,
        cancellation: source_flag.clone(),
    });
    app.session.source_errors[1] = Some("old error".into());
    app.session.paths = ["/old/left".into(), "/old/right".into()];
    app.session.source_kinds = [Some(SourceKind::Folder); 2];
    app.session.new_comparison();
    for flag in [folder_flag, file_flag, source_flag] {
        assert!(flag.load(Ordering::Relaxed));
    }
    assert!(folder_sender.send(Ok(fixture())).is_err());
    assert!(file_sender.send(Ok(None)).is_err());
    assert!(source_sender.send(Ok(SourceKind::File)).is_err());
    settle_sources(&mut app, &egui::Context::default());
    assert_eq!(app.session.mode(), SelectionMode::Empty);
    assert_eq!(app.session.source_kinds, [None; 2]);
    assert!(app.session.source_errors.iter().all(Option::is_none));
    assert!(app.session.error.is_none());
}

#[test]
fn replacing_a_source_discards_pending_classification_and_prior_results() {
    let sources = SourceFixture::new();
    let mut app = loaded_app();
    let (sender, receiver) = mpsc::channel();
    let cancellation = Arc::new(AtomicBool::new(false));
    app.session.source_jobs[0] = Some(SourceJob {
        receiver,
        cancellation: cancellation.clone(),
    });
    app.session.select_path(0, sources.path("left/model.txt"));
    assert!(cancellation.load(Ordering::Relaxed));
    assert!(sender.send(Ok(SourceKind::Folder)).is_err());
    assert!(app.session.tree.is_none() && app.session.roots.is_none());
    settle_sources(&mut app, &egui::Context::default());
    assert_eq!(app.session.source_kinds[0], Some(SourceKind::File));
    assert_eq!(app.session.mode(), SelectionMode::File);
}

#[test]
fn unavailable_selected_source_shows_error_and_keeps_browse_available() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = loaded_app();
    app.session.select_path(0, sources.path("missing"));
    settle_sources(&mut app, &ctx);
    assert!(
        app.session
            .error
            .as_deref()
            .unwrap()
            .contains("Cannot inspect")
    );
    assert!(app.session.tree.is_none() && app.session.file_view.is_none());
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(
        text_positions(&output, "Cannot open selected source").len(),
        1
    );
    assert!(
        ctx.read_response(egui::Id::new("Browse left folder"))
            .is_some()
    );
    output.drop_without_applying_deltas();
    app.session.select_path(0, sources.path("left"));
    settle_sources(&mut app, &ctx);
    assert!(app.session.error.is_none());
    assert_eq!(app.session.mode(), SelectionMode::Folder);
}
#[test]
fn mode_buttons_switch_the_workspace_and_route_browse_directly() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    app.session.paths = ["/left/folder".into(), "/right/folder".into()];
    let sender = attach_job(&mut app);
    let cancellation = app.session.job.as_ref().unwrap().cancellation.clone();
    let output = render(&mut app, &ctx, vec![]);
    let file_button = text_centers(&output, "File Diff")[0];
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, file_button);
    assert_eq!(app.session.comparison_mode, ComparisonMode::File);
    assert!(app.session.paths.iter().all(String::is_empty));
    assert!(
        app.session.tree.is_none() && app.session.job.is_none() && app.session.file_view.is_none()
    );
    assert!(cancellation.load(Ordering::Relaxed));
    assert!(sender.send(Ok(fixture())).is_err());
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "Choose a file").len(), 2);
    assert_eq!(
        text_positions(&output, "Drop a file or folder here").len(),
        2
    );
    output.drop_without_applying_deltas();

    // Exercise both the path section and browse icon for both workflows.
    for mode in [ComparisonMode::File, ComparisonMode::Folder] {
        let output = render(&mut app, &ctx, vec![]);
        let button = text_centers(&output, mode.label())[0];
        output.drop_without_applying_deltas();
        click(&mut app, &ctx, button);
        assert_eq!(app.session.comparison_mode, mode);
        let mut requests = Vec::new();
        {
            let mut draw = |events| {
                ctx.run_ui(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        app.folder_inputs_with_picker(ui, |side, _, picker_mode| {
                            requests.push((side, picker_mode));
                            None
                        })
                    },
                )
            };
            for (side, path_area) in [(0, true), (1, false)] {
                let output = draw(vec![]);
                let target = if path_area {
                    text_centers(&output, &format!("Choose a {}", mode.source_label()))[side]
                } else {
                    ctx.read_response(egui::Id::new(format!(
                        "Browse right {}",
                        mode.source_label()
                    )))
                    .unwrap()
                    .rect
                    .center()
                };
                output.drop_without_applying_deltas();
                for pressed in [true, false] {
                    draw(vec![
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
        assert_eq!(requests, vec![(0, mode), (1, mode)]);
    }
}

#[test]
fn active_mode_button_preserves_comparison_and_new_preserves_file_mode() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    set_theme(&ctx, egui::Theme::Light);
    let mut app = VersusApp::default();
    app.session.switch_comparison_mode(ComparisonMode::File);
    app.session.select_path(0, sources.path("left/model.txt"));
    app.session.select_path(1, sources.path("right/model.txt"));
    settle_sources(&mut app, &ctx);
    let paths = app.session.paths.clone();
    let output = render(&mut app, &ctx, vec![]);
    let file_button = text_centers(&output, "File Diff")[0];
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, file_button);
    assert_eq!(app.session.paths, paths);
    assert!(app.session.file_view.as_ref().unwrap().comparison.is_some());
    let output = render(&mut app, &ctx, vec![]);
    let new = ctx
        .read_response(egui::Id::new("New comparison"))
        .unwrap()
        .rect
        .center();
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, new);
    assert_eq!(app.session.comparison_mode, ComparisonMode::File);
    assert!(app.session.paths.iter().all(String::is_empty));
    assert_eq!(ctx.theme(), egui::Theme::Light);
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "Choose a file").len(), 2);
    output.drop_without_applying_deltas();
}

#[test]
fn folder_mode_button_returns_from_file_drilldown_without_clearing_tree() {
    let ctx = egui::Context::default();
    let mut app = loaded_app();
    app.session.tree.as_mut().unwrap().expand_all();
    app.session.selected = Some("assembly/model.step".into());
    app.session.file_view = Some(loaded_file_view());
    let generation = app.session.scroll_generation;
    let output = render(&mut app, &ctx, vec![]);
    let folder_button = text_centers(&output, "Folder Diff")[0];
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, folder_button);
    assert!(app.session.file_view.is_none());
    assert!(app.session.tree.as_ref().unwrap().is_expanded("assembly"));
    assert_eq!(app.session.selected, Some("assembly/model.step".into()));
    assert_eq!(app.session.scroll_generation, generation);
}

fn click_action(app: &mut VersusApp, ctx: &egui::Context, label: &str) {
    click_action_frame(app, ctx, label).drop_without_applying_deltas();
}

fn click_action_frame(app: &mut VersusApp, ctx: &egui::Context, label: &str) -> egui::FullOutput {
    render(app, ctx, vec![]).drop_without_applying_deltas();
    let response = ctx.read_response(egui::Id::new(label)).unwrap();
    assert!(response.enabled(), "{label} should be enabled");
    let position = response.rect.center();
    let events = |pressed| {
        vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ]
    };
    render(app, ctx, events(true)).drop_without_applying_deltas();
    render(app, ctx, events(false))
}

#[test]
fn ignore_refresh_keeps_file_rows_and_viewport_until_results_are_ready() {
    let sources = SourceFixture::new();
    let left: String = (1..=200)
        .map(|number| format!("line {number}: value\n"))
        .collect();
    let right: String = (1..=200)
        .map(|number| match number {
            81 => "line 81: changed\n".into(),
            100 => "line 100:  value\n".into(),
            _ => format!("line {number}: value\n"),
        })
        .collect();
    std::fs::write(sources.path("left/model.txt"), left).unwrap();
    std::fs::write(sources.path("right/model.txt"), right).unwrap();
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        set_theme(&ctx, theme);
        let mut app = unfiltered_app();
        app.session.open_launch_request(crate::cli::LaunchRequest {
            paths: [
                sources.path("left/model.txt"),
                sources.path("right/model.txt"),
            ],
            mode: None,
        });
        settle_sources(&mut app, &ctx);
        let view = app.session.file_view.as_mut().unwrap();
        view.scroll_y = 80.0 * ROW_HEIGHT + 7.0;
        view.navigation_row = Some(80);
        let before = render(&mut app, &ctx, vec![]);
        let positions = text_positions(&before, "line 81: value");
        assert_eq!(positions.len(), 1);
        let panes = app.pane_rects;
        before.drop_without_applying_deltas();
        let updating = click_action_frame(&mut app, &ctx, "Ignore whitespace");
        assert_eq!(positions, text_positions(&updating, "line 81: value"));
        assert_eq!(panes, app.pane_rects);
        assert!(text_positions(&updating, "Loading file comparison…").is_empty());
        updating.drop_without_applying_deltas();
        let view = app.session.file_view.as_ref().unwrap();
        assert!(view.job.is_some() && view.comparison.is_some());
        assert_eq!(view.navigation_row, Some(80));
        assert_eq!(view.counts[1], 2);
        settle_sources(&mut app, &ctx);
        let after = render(&mut app, &ctx, vec![]);
        assert_eq!(positions, text_positions(&after, "line 81: value"));
        assert_eq!(panes, app.pane_rects);
        let view = app.session.file_view.as_ref().unwrap();
        assert_eq!(view.counts.iter().sum::<usize>(), 200);
        assert_eq!(view.counts[1], 1);
        assert_eq!(view.navigation_row, Some(80));
        assert_eq!(view.scroll_y, 80.0 * ROW_HEIGHT + 7.0);
        after.drop_without_applying_deltas();
    }
}

#[test]
fn ignore_refresh_keeps_folder_rows_expansion_selection_and_scroll() {
    let sources = SourceFixture::new();
    for side in ["left", "right"] {
        std::fs::create_dir_all(sources.path(&format!("{side}/z_group"))).unwrap();
        std::fs::write(sources.path(&format!("{side}/z_group/detail.txt")), side).unwrap();
        for number in 0..80 {
            let text = if side == "right" && number == 20 {
                "same  value\n"
            } else {
                "same value\n"
            };
            std::fs::write(sources.path(&format!("{side}/a_{number:03}.txt")), text).unwrap();
        }
    }
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = unfiltered_app();
    app.session.open_launch_request(crate::cli::LaunchRequest {
        paths: [sources.path("left"), sources.path("right")],
        mode: None,
    });
    settle_sources(&mut app, &ctx);
    app.session.tree.as_mut().unwrap().expand_all();
    app.session.selected = Some("z_group/detail.txt".into());
    app.session.tree_scroll_y = 20.0 * ROW_HEIGHT + 7.0;
    let before = render(&mut app, &ctx, vec![]);
    let positions = text_positions(&before, "a_020.txt");
    assert_eq!(positions.len(), 2);
    let panes = app.pane_rects;
    let generation = app.session.scroll_generation;
    before.drop_without_applying_deltas();
    let updating = click_action_frame(&mut app, &ctx, "Ignore whitespace");
    assert_eq!(positions, text_positions(&updating, "a_020.txt"));
    assert_eq!(panes, app.pane_rects);
    assert!(app.session.tree.is_some() && app.session.job.is_some());
    updating.drop_without_applying_deltas();
    settle_sources(&mut app, &ctx);
    let after = render(&mut app, &ctx, vec![]);
    assert_eq!(positions, text_positions(&after, "a_020.txt"));
    assert_eq!(panes, app.pane_rects);
    assert_eq!(app.session.scroll_generation, generation);
    assert_eq!(
        app.session.selected.as_deref(),
        Some(std::path::Path::new("z_group/detail.txt"))
    );
    assert!(app.session.tree.as_ref().unwrap().is_expanded("z_group"));
    assert_eq!(app.session.tree_scroll_y, 20.0 * ROW_HEIGHT + 7.0);
    assert_eq!(app.session.counts[1], 3); // model.txt, z_group and its detail.txt
    after.drop_without_applying_deltas();
}

#[test]
fn file_refresh_preserves_source_line_anchors_when_alignment_changes() {
    let ctx = egui::Context::default();
    let mut app = loaded_app();
    let mut view = loaded_file_view();
    view.scroll_y = ROW_HEIGHT + 5.0;
    view.navigation_row = Some(1);
    let mut replacement = view.comparison.as_ref().unwrap().clone();
    replacement.rows.insert(
        1,
        test_file_row(
            None,
            Some((99, "inserted".into())),
            DirectoryEntryState::RightOnly,
        ),
    );
    let (sender, receiver) = mpsc::channel();
    view.job = Some(FileJob {
        receiver,
        cancellation: Arc::new(AtomicBool::new(false)),
        progress: Arc::new(ComparisonProgress::default()),
        ignore_line_endings: false,
    });
    let sources = view.sources.clone();
    app.session.file_view = Some(view);
    sender
        .send(Ok(Some(LoadedFile {
            comparison: replacement,
            sources,
        })))
        .unwrap();
    app.session.poll_file_comparison(&ctx);
    let view = app.session.file_view.as_ref().unwrap();
    assert_eq!(view.scroll_y, 2.0 * ROW_HEIGHT + 5.0);
    assert_eq!(view.navigation_row, Some(2));
    assert_eq!(view.counts, [1, 1, 1, 1, 0, 0]);
    assert!(!view.ignore_line_endings);
}

#[test]
fn failed_or_cancelled_file_refresh_keeps_the_last_result_visible() {
    for cancelled in [false, true] {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        let mut view = loaded_file_view();
        let before = view.comparison.clone();
        let (sender, receiver) = mpsc::channel();
        view.job = Some(FileJob {
            receiver,
            cancellation: Arc::new(AtomicBool::new(cancelled)),
            progress: Arc::new(ComparisonProgress::default()),
            ignore_line_endings: false,
        });
        let sources = view.sources.clone();
        app.session.file_view = Some(view);
        if cancelled {
            sender
                .send(Ok(Some(LoadedFile {
                    comparison: FileComparison {
                        rows: vec![],
                        message: None,
                    },
                    sources,
                })))
                .unwrap();
        } else {
            sender
                .send(Err(CompareError {
                    path: None,
                    kind: versus::CompareErrorKind::InvalidPath,
                    message: "file unavailable".into(),
                }))
                .unwrap();
        }
        app.session.poll_file_comparison(&ctx);
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(text_positions(&output, "old value").len(), 1);
        let view = app.session.file_view.as_ref().unwrap();
        assert_eq!(view.comparison, before);
        assert!(view.error.is_some() && view.job.is_none());
        assert!(view.ignore_line_endings);
        output.drop_without_applying_deltas();
    }
}

#[test]
fn differences_filter_hides_equal_lines_and_preserves_original_numbers() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    app.session.file_view = Some(loaded_file_view());
    click_action(&mut app, &ctx, "Show only differences");
    let output = render(&mut app, &ctx, vec![]);
    assert!(text_positions(&output, "unchanged").is_empty());
    assert!(text_positions(&output, "1").is_empty());
    assert_eq!(text_positions(&output, "2").len(), 2);
    assert_eq!(text_positions(&output, "3").len(), 1);
    let left = text_positions(&output, "old value")[0];
    let right = text_positions(&output, "new value")[0];
    assert!((left.y - right.y).abs() < 0.1);
    output.drop_without_applying_deltas();
    click_action(&mut app, &ctx, "Show only differences");
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "unchanged").len(), 2);
    output.drop_without_applying_deltas();
}

#[test]
fn navigation_scrolls_both_file_panes_and_advances_when_rows_fit_the_window() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    let mut view = loaded_file_view();
    view.comparison.as_mut().unwrap().rows = (1..=200)
        .map(|number| {
            let changed = [1, 20, 80, 150, 200].contains(&number);
            test_file_row(
                Some((number, format!("left line {number}"))),
                Some((number, format!("right line {number}"))),
                if changed {
                    DirectoryEntryState::Different
                } else {
                    DirectoryEntryState::Same
                },
            )
        })
        .collect();
    update_visible_file_rows(&mut view, false);
    app.session.file_view = Some(view);
    for (action, number) in [
        ("Next difference", 1),
        ("Next difference", 20),
        ("Next difference", 80),
        ("Next difference", 150),
        ("Next difference", 200),
        ("Previous difference", 150),
        ("Previous difference", 80),
        ("Previous difference", 20),
    ] {
        let first = click_action_frame(&mut app, &ctx, action);
        let first_positions = text_positions(&first, &format!("left line {number}"));
        assert_eq!(first_positions.len(), 1);
        first.drop_without_applying_deltas();
        let output = render(&mut app, &ctx, vec![]);
        let left = text_positions(&output, &format!("left line {number}"))[0];
        let right = text_positions(&output, &format!("right line {number}"))[0];
        assert!((left.y - right.y).abs() < 0.1);
        // Keep the preceding line visible, or clamp to the file start.
        if number > 1 {
            assert_eq!(
                text_positions(&output, &format!("left line {}", number - 1)).len(),
                1
            );
        } else {
            assert_eq!(app.session.file_view.as_ref().unwrap().scroll_y, 0.0);
        }
        assert_eq!(
            first_positions,
            vec![left],
            "Jump should settle on its first frame"
        );
        assert_eq!(
            app.session.file_view.as_ref().unwrap().navigation_row,
            Some(number - 1)
        );
        output.drop_without_applying_deltas();
    }
    app.session.file_view = Some(loaded_file_view());
    for (action, index) in [
        ("Next difference", 1),
        ("Next difference", 2),
        ("Previous difference", 1),
    ] {
        click_action(&mut app, &ctx, action);
        assert_eq!(
            app.session.file_view.as_ref().unwrap().navigation_row,
            Some(index)
        );
    }
}

#[test]
fn folder_navigation_reveals_collapsed_parents_in_both_panes() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut entries = Vec::new();
    for index in 0..100 {
        entries.push(DirectoryEntry {
            relative_path: format!("a_common_{index:03}").into(),
            left_exists: true,
            right_exists: true,
            left_kind: Some(DirectoryEntryKind::Directory),
            right_kind: Some(DirectoryEntryKind::Directory),
            left_size: Some(0),
            right_size: Some(0),
            kind: DirectoryEntryKind::Directory,
            state: DirectoryEntryState::Same,
        });
    }
    for (path, state) in [
        ("z_changed/deep/changed.txt", DirectoryEntryState::Different),
        ("z_changed/deep/removed.txt", DirectoryEntryState::LeftOnly),
        ("z_changed/deep/same.txt", DirectoryEntryState::Same),
        ("z_other/added.txt", DirectoryEntryState::RightOnly),
    ] {
        let left_exists = state != DirectoryEntryState::RightOnly;
        let right_exists = state != DirectoryEntryState::LeftOnly;
        entries.push(DirectoryEntry {
            relative_path: path.into(),
            left_exists,
            right_exists,
            left_kind: left_exists.then_some(DirectoryEntryKind::File),
            right_kind: right_exists.then_some(DirectoryEntryKind::File),
            left_size: left_exists.then_some(1),
            right_size: right_exists.then_some(1),
            kind: DirectoryEntryKind::File,
            state,
        });
    }
    let tree = FolderTree::from_diff(&DirectoryDiff {
        entries,
        cancelled: false,
    });
    let mut app = unfiltered_app();
    let sender = attach_job(&mut app);
    sender.send(Ok(tree)).unwrap();
    app.session.poll_comparison(&ctx);
    let first = click_action_frame(&mut app, &ctx, "Next difference");
    let first_positions = text_positions(&first, "changed.txt");
    assert_eq!(first_positions.len(), 2);
    first.drop_without_applying_deltas();
    assert_eq!(
        app.session.selected.as_deref(),
        Some(std::path::Path::new("z_changed/deep/changed.txt"))
    );
    assert!(app.session.tree.as_ref().unwrap().is_expanded("z_changed"));
    assert!(
        app.session
            .tree
            .as_ref()
            .unwrap()
            .is_expanded("z_changed/deep")
    );
    let output = render(&mut app, &ctx, vec![]);
    let rows = text_positions(&output, "changed.txt");
    assert_eq!(rows.len(), 2);
    assert_eq!(
        first_positions, rows,
        "Jump should settle on its first frame"
    );
    assert!((rows[0].y - rows[1].y).abs() < 0.1);
    output.drop_without_applying_deltas();
    click_action(&mut app, &ctx, "Show only differences");
    let output = render(&mut app, &ctx, vec![]);
    assert!(text_positions(&output, "a_common_000").is_empty());
    assert_eq!(text_positions(&output, "changed.txt").len(), 2);
    output.drop_without_applying_deltas();
    for path in ["z_changed/deep/removed.txt", "z_other/added.txt"] {
        click_action(&mut app, &ctx, "Next difference");
        assert_eq!(
            app.session.selected.as_deref(),
            Some(std::path::Path::new(path))
        );
    }
    assert!(app.session.tree.as_ref().unwrap().is_expanded("z_other"));
    click_action(&mut app, &ctx, "Next difference");
    assert_eq!(
        app.session.selected.as_deref(),
        Some(std::path::Path::new("z_other/added.txt"))
    );
    click_action(&mut app, &ctx, "Previous difference");
    assert_eq!(
        app.session.selected.as_deref(),
        Some(std::path::Path::new("z_changed/deep/removed.txt"))
    );
    click_action(&mut app, &ctx, "Previous difference");
    assert_eq!(
        app.session.selected.as_deref(),
        Some(std::path::Path::new("z_changed/deep/changed.txt"))
    );
}

#[test]
fn folder_navigation_is_disabled_when_only_folders_differ() {
    let ctx = egui::Context::default();
    let tree = FolderTree::from_diff(&DirectoryDiff {
        entries: vec![DirectoryEntry {
            relative_path: "empty-left-only".into(),
            left_exists: true,
            right_exists: false,
            left_kind: Some(DirectoryEntryKind::Directory),
            right_kind: None,
            left_size: Some(0),
            right_size: None,
            kind: DirectoryEntryKind::Directory,
            state: DirectoryEntryState::LeftOnly,
        }],
        cancelled: false,
    });
    let mut app = VersusApp::default();
    let sender = attach_job(&mut app);
    sender.send(Ok(tree)).unwrap();
    app.session.poll_comparison(&ctx);
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    for label in ["Previous difference", "Next difference"] {
        assert!(!ctx.read_response(egui::Id::new(label)).unwrap().enabled());
    }
    assert!(app.session.folder_differences.is_empty());
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
fn default_options_filter_equal_lines_and_ignore_whitespace_and_endings() {
    let sources = SourceFixture::new();
    std::fs::write(
        sources.path("left/model.txt"),
        "common\nspacing value\nold\n",
    )
    .unwrap();
    std::fs::write(
        sources.path("right/model.txt"),
        "common\r\nspacing  value\r\nnew\r\n",
    )
    .unwrap();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    assert!(
        app.session.show_only_differences
            && app.session.ignore_whitespace
            && app.session.ignore_line_endings
    );
    app.session.open_launch_request(crate::cli::LaunchRequest {
        paths: [
            sources.path("left/model.txt"),
            sources.path("right/model.txt"),
        ],
        mode: None,
    });
    settle_sources(&mut app, &ctx);
    let view = app.session.file_view.as_ref().unwrap();
    assert_eq!(view.counts[1], 1);
    assert_eq!(view.visible_rows, vec![2]);
    let output = render(&mut app, &ctx, vec![]);
    assert!(text_positions(&output, "common").is_empty());
    assert!(text_positions(&output, "spacing value").is_empty());
    assert_eq!(text_positions(&output, "old").len(), 1);
    assert_eq!(text_positions(&output, "new").len(), 1);
    output.drop_without_applying_deltas();
}

#[test]
fn line_ending_ignore_defaults_on_in_both_comparison_views() {
    let sources = SourceFixture::new();
    std::fs::write(sources.path("left/model.txt"), "same\r\nnext\r\n").unwrap();
    std::fs::write(sources.path("right/model.txt"), "same\nnext\n").unwrap();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    assert!(app.session.ignore_line_endings);
    app.session.open_launch_request(crate::cli::LaunchRequest {
        paths: [sources.path("left"), sources.path("right")],
        mode: None,
    });
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.counts[1], 0);
    app.session
        .open_file("model.txt".into(), [true, true], false);
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.file_view.as_ref().unwrap().counts[1], 0);
    click_action(&mut app, &ctx, "Ignore line endings");
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.counts[1], 1);
    assert_eq!(app.session.file_view.as_ref().unwrap().counts[1], 2);
    click_action(&mut app, &ctx, "Ignore line endings");
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.counts[1], 0);
    assert_eq!(app.session.file_view.as_ref().unwrap().counts[1], 0);
}

#[test]
fn inserted_line_and_commented_peers_align_with_original_numbers_in_both_panes() {
    let sources = SourceFixture::new();
    let prefix: String = (1..50)
        .map(|line| format!("unchanged line {line}\n"))
        .collect();
    std::fs::write(
        sources.path("left/model.txt"),
        format!("{prefix}extra_call();\n    first_action();\n    second_action();\ntail\n"),
    )
    .unwrap();
    std::fs::write(
        sources.path("right/model.txt"),
        format!("{prefix}    // first_action();\n    // second_action();\ntail\n"),
    )
    .unwrap();
    let ctx = egui::Context::default();
    let mut app = unfiltered_app();
    app.session.open_launch_request(crate::cli::LaunchRequest {
        paths: [
            sources.path("left/model.txt"),
            sources.path("right/model.txt"),
        ],
        mode: Some(ComparisonMode::File),
    });
    settle_sources(&mut app, &ctx);
    let comparison = app
        .session
        .file_view
        .as_ref()
        .unwrap()
        .comparison
        .as_ref()
        .unwrap();
    assert_eq!(comparison.rows[49].left.as_ref().unwrap().0, 50);
    assert!(comparison.rows[49].right.is_none());
    for (index, left_number, right_number) in [(50, 51, 50), (51, 52, 51)] {
        let row = &comparison.rows[index];
        assert_eq!(row.left.as_ref().unwrap().0, left_number);
        assert_eq!(row.right.as_ref().unwrap().0, right_number);
        assert_eq!(row.state, DirectoryEntryState::Different);
    }
    click_action(&mut app, &ctx, "Next difference");
    for differences_only in [false, true] {
        if differences_only {
            click_action(&mut app, &ctx, "Show only differences");
        }
        let mut output = render(&mut app, &ctx, vec![]);
        output.textures_delta.clear();
        for (left, right) in [
            ("    first_action();", "    // first_action();"),
            ("    second_action();", "    // second_action();"),
        ] {
            let left_position = text_positions(&output, left)[0];
            let right_position = text_positions(&output, right)[0];
            assert!((left_position.y - right_position.y).abs() < 0.1);
            let highlighted: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == right => Some(&text.galley),
                    _ => None,
                })
                .flat_map(|galley| {
                    galley.job.sections.iter().filter_map(|section| {
                        (section.format.background != Color32::TRANSPARENT).then(|| {
                            galley.job.text[section.byte_range.start.0..section.byte_range.end.0]
                                .to_owned()
                        })
                    })
                })
                .collect();
            assert_eq!(highlighted.concat(), "// ");
        }
        output.drop_without_applying_deltas();
    }
}

#[test]
fn ignore_buttons_recompare_file_and_folder_results_and_show_ending_changes() {
    let sources = SourceFixture::new();
    std::fs::write(sources.path("left/model.txt"), "let value = 1;\r\n").unwrap();
    std::fs::write(sources.path("right/model.txt"), "letvalue=1;\n").unwrap();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = unfiltered_app();
    app.session.open_launch_request(crate::cli::LaunchRequest {
        paths: [sources.path("left"), sources.path("right")],
        mode: None,
    });
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.counts[1], 1);
    app.session
        .open_file("model.txt".into(), [true, true], false);
    settle_sources(&mut app, &ctx);
    click_action(&mut app, &ctx, "Ignore whitespace");
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.counts[1], 0);
    assert_eq!(app.session.file_view.as_ref().unwrap().counts[1], 0);
    click_action(&mut app, &ctx, "Show only differences");
    let output = render(&mut app, &ctx, vec![]);
    assert!(!text_positions(&output, "No differences with the current options.").is_empty());
    output.drop_without_applying_deltas();
    click_action(&mut app, &ctx, "Ignore line endings");
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.counts[1], 1);
    assert_eq!(app.session.file_view.as_ref().unwrap().counts[1], 1);
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "CRLF").len(), 1);
    assert_eq!(text_positions(&output, "LF").len(), 1);
    output.drop_without_applying_deltas();
    click_action(&mut app, &ctx, "Back to folders");
    assert!(app.session.file_view.is_none());
    assert_eq!(app.session.counts[1], 1);
}

#[test]
fn changed_text_is_highlighted_while_common_text_stays_unhighlighted() {
    let sources = SourceFixture::new();
    std::fs::write(sources.path("left/model.txt"), "old stable old\n").unwrap();
    std::fs::write(sources.path("right/model.txt"), "new stable new\n").unwrap();
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        set_theme(&ctx, theme);
        let mut app = VersusApp::default();
        app.session.open_launch_request(crate::cli::LaunchRequest {
            paths: [
                sources.path("left/model.txt"),
                sources.path("right/model.txt"),
            ],
            mode: None,
        });
        settle_sources(&mut app, &ctx);
        let output = render(&mut app, &ctx, vec![]);
        for expected in ["old stable old", "new stable new"] {
            let galley = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == expected => Some(&text.galley),
                    _ => None,
                })
                .unwrap();
            let highlighted: Vec<_> = galley
                .job
                .sections
                .iter()
                .filter(|section| section.format.background != Color32::TRANSPARENT)
                .map(|section| {
                    &galley.job.text[section.byte_range.start.0..section.byte_range.end.0]
                })
                .collect();
            assert_eq!(
                highlighted,
                if expected.starts_with("old") {
                    vec!["old", "old"]
                } else {
                    vec!["new", "new"]
                }
            );
        }
        output.drop_without_applying_deltas();
    }
}

#[derive(Debug)]
struct TestDrop(PathBuf);
impl egui::DroppedFile for TestDrop {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        panic!("Drop handlers must not read file contents on the UI thread")
    }
}

fn drop_sources(app: &mut VersusApp, ctx: &egui::Context, side: usize, paths: Vec<PathBuf>) {
    render(app, ctx, vec![]).drop_without_applying_deltas();
    let position = app.pane_rects[side].center();
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            events: vec![egui::Event::PointerMoved(position)],
            dropped_files: paths
                .into_iter()
                .map(|path| Arc::new(TestDrop(path)) as egui::DroppedFileHandle)
                .collect(),
            ..Default::default()
        },
        |ui| app.render(ui),
    )
    .drop_without_applying_deltas();
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
fn arrow_keys_move_tree_selection_through_visible_rows() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    let output = render(&mut app, &ctx, vec![]);
    let assembly = text_positions(&output, "assembly")[0];
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, assembly + egui::vec2(3.0, 4.0));
    assert_eq!(
        app.session.selected.as_deref(),
        Some(std::path::Path::new("assembly"))
    );
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::ArrowDown,
            physical_key: Some(egui::Key::ArrowDown),
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    )
    .drop_without_applying_deltas();
    assert_eq!(
        app.session.selected.as_deref(),
        Some(std::path::Path::new("assembly/model.step"))
    );
    assert_eq!(
        app.session.tree_keyboard_focus.as_deref(),
        Some(std::path::Path::new("assembly/model.step"))
    );
}

#[test]
fn tree_tab_navigation_escapes_both_panes_in_either_direction() {
    for side in 0..2 {
        for backwards in [false, true] {
            let ctx = egui::Context::default();
            apply_theme(&ctx);
            let mut app = loaded_app();
            let output = render(&mut app, &ctx, vec![]);
            let assembly = text_positions(&output, "assembly")[side];
            output.drop_without_applying_deltas();
            click(&mut app, &ctx, assembly + egui::vec2(3.0, 4.0));
            let focused_before_tab = ctx.memory(|memory| memory.focused()).unwrap();
            render(
                &mut app,
                &ctx,
                vec![egui::Event::Key {
                    key: egui::Key::Tab,
                    physical_key: Some(egui::Key::Tab),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers {
                        shift: backwards,
                        ..Default::default()
                    },
                }],
            )
            .drop_without_applying_deltas();
            render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
            assert_ne!(
                ctx.memory(|memory| memory.focused()),
                Some(focused_before_tab)
            );
            assert_eq!(
                ctx.memory(|memory| memory.focused()),
                Some(egui::Id::new(if backwards {
                    "Browse right folder"
                } else {
                    "Folder Diff"
                }))
            );
            assert!(app.session.tree_keyboard_focus.is_none());
        }
    }
}

#[test]
fn keyboard_tree_navigation_keeps_a_long_list_visible_without_resetting_scroll() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = unfiltered_app();
    app.session.tree = Some(FolderTree::from_diff(&DirectoryDiff {
        cancelled: false,
        entries: (0..80)
            .map(|index| DirectoryEntry {
                relative_path: format!("item-{index:03}.txt").into(),
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
    let output = render(&mut app, &ctx, vec![]);
    let first = text_positions(&output, "item-000.txt")[0];
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, first + egui::vec2(3.0, 4.0));
    for _ in 0..50 {
        render(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::ArrowDown,
                physical_key: Some(egui::Key::ArrowDown),
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        )
        .drop_without_applying_deltas();
        render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    }
    assert_eq!(
        app.session.selected.as_deref(),
        Some(std::path::Path::new("item-050.txt"))
    );
    assert!(app.session.tree_scroll_y > 0.0);
}

#[test]
fn focused_file_pane_scrolls_with_arrow_keys() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = unfiltered_app();
    let mut view = loaded_file_view();
    let rows: Vec<_> = (1..=80)
        .map(|number| {
            test_file_row(
                Some((number, format!("left line {number}"))),
                Some((number, format!("right line {number}"))),
                DirectoryEntryState::Same,
            )
        })
        .collect();
    view.comparison.as_mut().unwrap().rows = rows;
    view.visible_rows = (0..80).collect();
    app.session.file_view = Some(view);
    let output = render(&mut app, &ctx, vec![]);
    let line = text_positions(&output, "left line 1")[0];
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, line + egui::vec2(8.0, 3.0));
    assert_eq!(app.session.file_keyboard_focus_side, Some(0));
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::ArrowDown,
            physical_key: Some(egui::Key::ArrowDown),
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    )
    .drop_without_applying_deltas();
    assert_eq!(
        app.session.file_view.as_ref().unwrap().scroll_y,
        app.session.row_height
    );
    let focused = ctx.memory(|memory| memory.focused());
    assert!(focused.is_some());
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    assert_eq!(ctx.memory(|memory| memory.focused()), focused);
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::ArrowDown,
            physical_key: Some(egui::Key::ArrowDown),
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    )
    .drop_without_applying_deltas();
    assert_eq!(
        app.session.file_view.as_ref().unwrap().scroll_y,
        2.0 * app.session.row_height
    );
}

#[test]
fn filter_preserves_folder_anchor_and_nearest_survivor() {
    let mut app = unfiltered_app();
    app.session.row_height = 34.0;
    app.session.tree = Some(FolderTree::from_diff(&DirectoryDiff {
        cancelled: false,
        entries: (0..100)
            .map(|index| DirectoryEntry {
                relative_path: format!("entry-{index:03}.txt").into(),
                left_size: Some(1),
                right_size: Some(1),
                left_exists: true,
                right_exists: true,
                left_kind: Some(DirectoryEntryKind::File),
                right_kind: Some(DirectoryEntryKind::File),
                kind: DirectoryEntryKind::File,
                state: if index % 2 == 0 {
                    DirectoryEntryState::Different
                } else {
                    DirectoryEntryState::Same
                },
            })
            .collect(),
    }));
    app.session.selected = Some("entry-040.txt".into());
    app.session.tree_scroll_y = 40.0 * app.session.row_height + 7.0;
    app.session.set_differences_only(true);
    assert_eq!(
        app.session.tree_scroll_y,
        20.0 * app.session.row_height + 7.0
    );
    assert_eq!(
        app.session.selected.as_deref(),
        Some(std::path::Path::new("entry-040.txt"))
    );
    app.session.set_differences_only(false);
    assert_eq!(
        app.session.tree_scroll_y,
        40.0 * app.session.row_height + 7.0
    );
    app.session.tree_scroll_y = 41.0 * app.session.row_height + 7.0;
    app.session.set_differences_only(true);
    assert_eq!(
        app.session.tree_scroll_y,
        21.0 * app.session.row_height + 7.0
    );
    app.session.set_differences_only(false);
    app.session.tree_scroll_y = 99.0 * app.session.row_height;
    app.session.set_differences_only(true);
    assert_eq!(app.session.tree_scroll_y, 49.0 * app.session.row_height);
}

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

#[derive(Default)]
struct ZoomStorage(std::collections::HashMap<String, String>);
impl eframe::Storage for ZoomStorage {
    fn get_string(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
    fn set_string(&mut self, key: &str, value: String) {
        self.0.insert(key.into(), value);
    }
    fn remove_string(&mut self, key: &str) {
        self.0.remove(key);
    }
    fn flush(&mut self) {}
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

#[test]
fn accesskit_exposes_empty_targets_and_file_lines_with_side_number_and_status() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    ctx.enable_accesskit();
    let mut app = VersusApp::default();
    let output = render(&mut app, &ctx, vec![]);
    let labels: Vec<_> = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .filter_map(|(_, node)| node.label().or_else(|| node.value()))
        .collect();
    for side in ["Left", "Right"] {
        assert!(
            labels
                .iter()
                .any(|label| label.starts_with(side) && label.contains("Drop a file or folder"))
        );
    }
    output.drop_without_applying_deltas();
    app.session.file_view = Some(loaded_file_view());
    let output = render(&mut app, &ctx, vec![]);
    let labels: Vec<_> = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .filter_map(|(_, node)| node.label().or_else(|| node.value()))
        .collect();
    assert!(
        labels
            .iter()
            .any(|label| label.contains("Left line 2") && label.contains("Different")),
        "{labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|label| label.contains("Right file comparison pane") && label.contains("right"))
    );
    output.drop_without_applying_deltas();
}

#[test]
fn long_unicode_file_lines_have_bounded_accessibility_updates() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    ctx.enable_accesskit();
    let mut app = unfiltered_app();
    let mut view = loaded_file_view();
    let text = "工程🦀".repeat(20_000);
    view.comparison.as_mut().unwrap().rows = vec![test_file_row(
        Some((1, text.clone())),
        Some((1, text)),
        DirectoryEntryState::Different,
    )];
    view.visible_rows = vec![0];
    app.session.file_view = Some(view);
    let output = render(&mut app, &ctx, vec![]);
    let labels: Vec<_> = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .filter_map(|(_, node)| node.value())
        .filter(|label| label.contains("line 1:"))
        .collect();
    assert_eq!(labels.len(), 2);
    for label in labels {
        assert!(label.len() < 4200);
        assert!(label.contains("bytes omitted"));
        assert!(label.contains("工程🦀"));
    }
    output.drop_without_applying_deltas();
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

#[test]
fn navigation_status_reports_current_change_in_both_views() {
    let mut app = loaded_app();
    app.session.folder_differences = vec!["a.txt".into(), "b.txt".into(), "c.txt".into()];
    app.session.selected = Some("b.txt".into());
    assert_eq!(app.session.navigation_status(), "Change 2 of 3");
    app.session.file_view = Some(loaded_file_view());
    let view = app.session.file_view.as_mut().unwrap();
    view.navigation_row = Some(view.difference_rows[0]);
    let count = view.difference_rows.len();
    assert_eq!(
        app.session.navigation_status(),
        format!("Change 1 of {count}")
    );
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

#[test]
fn recent_pairs_reopen_and_are_kept_when_starting_over() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let paths = [
        sources.path("left/model.txt"),
        sources.path("right/model.txt"),
    ];
    let mut app = VersusApp::default();
    app.session.open_recent(RecentComparison {
        mode: ComparisonMode::File,
        paths: paths.clone(),
    });
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.comparison_paths(), paths);
    assert_eq!(app.session.recent.entries().len(), 1);
    app.session.new_comparison();
    assert_eq!(app.session.recent.entries().len(), 1);
}

#[test]
fn refreshing_a_folder_drilldown_rechecks_newly_created_sides() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.session.select_path(0, sources.path("left"));
    app.session.select_path(1, sources.path("right"));
    settle_sources(&mut app, &ctx);
    std::fs::remove_file(sources.path("right/model.txt")).unwrap();
    app.session
        .open_file("model.txt".into(), [true, false], false);
    settle_sources(&mut app, &ctx);
    std::fs::write(sources.path("right/model.txt"), "common\nold\n").unwrap();
    app.session.refresh_file();
    settle_sources(&mut app, &ctx);
    assert!(
        app.session
            .file_view
            .as_ref()
            .unwrap()
            .sources
            .iter()
            .all(Option::is_some)
    );
    assert_eq!(app.session.file_view.as_ref().unwrap().counts[0], 2);
}

#[cfg(unix)]
#[test]
fn refreshing_a_folder_drilldown_does_not_follow_a_symlink() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.session.select_path(0, sources.path("left"));
    app.session.select_path(1, sources.path("right"));
    settle_sources(&mut app, &ctx);
    app.session
        .open_file("model.txt".into(), [true, true], false);
    settle_sources(&mut app, &ctx);
    std::fs::remove_file(sources.path("left/model.txt")).unwrap();
    std::os::unix::fs::symlink(
        sources.path("right/model.txt"),
        sources.path("left/model.txt"),
    )
    .unwrap();
    app.session.refresh_file();
    settle_sources(&mut app, &ctx);
    assert!(
        app.session
            .file_view
            .as_ref()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("non-symlink")
    );
}

#[test]
fn folder_file_refresh_reports_inspection_errors_instead_of_an_empty_side() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.session.select_path(0, sources.path("left"));
    app.session.select_path(1, sources.path("right"));
    settle_sources(&mut app, &ctx);
    app.session
        .open_file("model.txt".into(), [true, true], false);
    settle_sources(&mut app, &ctx);
    // A child below a regular file is NotADirectory on Unix but NotFound on
    // Windows. An embedded NUL produces a genuine inspection error on both.
    let invalid_path = sources.path("right/invalid\0path.txt");
    let inspection_error = std::fs::symlink_metadata(&invalid_path).unwrap_err();
    assert_eq!(inspection_error.kind(), std::io::ErrorKind::InvalidInput);
    let previous = app.session.file_view.as_ref().unwrap().comparison.clone();
    app.session.file_view.as_mut().unwrap().paths[1] = invalid_path;
    app.session.refresh_file();
    settle_sources(&mut app, &ctx);
    assert!(app.session.file_view.as_ref().unwrap().error.is_some());
    assert_eq!(app.session.file_view.as_ref().unwrap().comparison, previous);
}

#[test]
fn recent_pairs_reopen_from_startup_and_survive_new_comparison() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    for mode in [ComparisonMode::File, ComparisonMode::Folder] {
        let mut app = VersusApp::default();
        let paths = match mode {
            ComparisonMode::File => [
                sources.path("left/model.txt"),
                sources.path("right/model.txt"),
            ],
            ComparisonMode::Folder => [sources.path("left"), sources.path("right")],
        };
        app.session.open_recent(RecentComparison {
            mode,
            paths: paths.clone(),
        });
        settle_sources(&mut app, &ctx);
        assert!(app.session.error.is_none());
        assert_eq!(app.session.comparison_mode, mode);
        assert_eq!(
            app.session.recent.entries(),
            &[RecentComparison {
                mode,
                paths: paths.clone()
            }]
        );
        assert_eq!(app.session.tree.is_some(), mode == ComparisonMode::Folder);
        assert_eq!(
            app.session.file_view.is_some(),
            mode == ComparisonMode::File
        );
        app.session.new_comparison();
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(text_positions(&output, "Recently compared").len(), 1);
        let target = ctx
            .read_response(egui::Id::new(("recent-comparison", mode.source_label(), 0)))
            .unwrap()
            .rect
            .center();
        output.drop_without_applying_deltas();
        click(&mut app, &ctx, target);
        settle_sources(&mut app, &ctx);
        assert!(app.session.error.is_none());
        assert_eq!(app.session.comparison_paths(), paths);
        assert_eq!(app.session.recent.entries().len(), 1);
    }
}

#[test]
fn recent_list_filters_by_mode_and_uses_plain_alternating_rows() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        set_theme(&ctx, theme);
        let mut app = VersusApp::default();
        for (mode, name) in [
            (ComparisonMode::Folder, "folder-old"),
            (ComparisonMode::File, "file-old"),
            (ComparisonMode::Folder, "folder-new"),
            (ComparisonMode::File, "file-new"),
        ] {
            app.session.recent.record(
                mode,
                [
                    format!("/{name}/left").into(),
                    format!("/{name}/right").into(),
                ],
            );
        }
        for (mode, visible, hidden) in [
            (ComparisonMode::Folder, "folder", "file"),
            (ComparisonMode::File, "file", "folder"),
        ] {
            app.session.switch_comparison_mode(mode);
            let output = render(&mut app, &ctx, vec![]);
            let palette = Palette::for_context(&ctx);
            let mut positions = Vec::new();
            for (index, age, fill) in [(0, "new", palette.panel), (1, "old", palette.alternate)] {
                let left = text_positions(&output, &format!("/{visible}-{age}/left"));
                let right = text_positions(&output, &format!("/{visible}-{age}/right"));
                assert_eq!(left.len(), 1);
                assert_eq!(right.len(), 1);
                assert_eq!(left[0].y, right[0].y);
                let row = ctx
                    .read_response(egui::Id::new((
                        "recent-comparison",
                        mode.source_label(),
                        index,
                    )))
                    .unwrap()
                    .rect;
                assert!(left[0].x < row.center().x && right[0].x > left[0].x);
                assert!(output.shapes.iter().any(|shape| {
                    matches!(&shape.shape, egui::Shape::Rect(shape) if shape.rect == row && shape.fill == fill && shape.stroke == Stroke::NONE)
                }));
                assert!(text_positions(&output, &format!("/{hidden}-{age}/left")).is_empty());
                assert!(text_positions(&output, &format!("/{hidden}-{age}/right")).is_empty());
                positions.push(left[0]);
            }
            assert_eq!(positions[0].x, positions[1].x);
            assert!(positions[0].y < positions[1].y);
            output.drop_without_applying_deltas();
        }
    }
}

#[test]
fn recent_list_shows_an_empty_message_for_the_active_mode() {
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.session.recent.record(
        ComparisonMode::Folder,
        ["/left/folder".into(), "/right/folder".into()],
    );
    app.session.switch_comparison_mode(ComparisonMode::File);
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(
        text_positions(&output, "No recent file comparisons.").len(),
        1
    );
    assert!(
        text_positions(&output, "No recent file comparisons.")[0].x
            > text_positions(&output, "Recently compared")[0].x + 6.0
    );
    assert!(text_positions(&output, "/left/folder").is_empty());
    assert!(text_positions(&output, "/right/folder").is_empty());
    output.drop_without_applying_deltas();
}

#[test]
fn recent_missing_or_changed_sources_report_errors_and_keep_history() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    for (paths, expected) in [
        (
            [sources.path("missing.txt"), sources.path("right/model.txt")],
            "Cannot inspect",
        ),
        (
            [sources.path("left"), sources.path("right")],
            "requires two regular",
        ),
    ] {
        let mut app = VersusApp::default();
        let entry = RecentComparison {
            mode: ComparisonMode::File,
            paths,
        };
        app.session.recent.record(entry.mode, entry.paths.clone());
        app.session.open_recent(entry.clone());
        settle_sources(&mut app, &ctx);
        assert!(app.session.error.as_deref().unwrap().contains(expected));
        assert!(app.session.job.is_none() && app.session.file_view.is_none());
        assert_eq!(app.session.recent.entries(), &[entry]);
        app.session.new_comparison();
        assert_eq!(app.session.recent.entries().len(), 1);
    }
}

#[test]
fn history_worker_finishes_writes_on_close_and_merges_startup_history() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let history_path = sources.path("settings/recent-comparisons.v1");
    let first = [sources.path("left"), sources.path("right")];
    let second = [
        sources.path("left/model.txt"),
        sources.path("right/model.txt"),
    ];
    let mut saved = RecentComparisons::default();
    saved.record(ComparisonMode::Folder, first.clone());
    saved.save_to(&history_path).unwrap();
    let mut app = VersusApp::default();
    app.session.recent_store = Some(RecentStore::start(&ctx, Some(history_path.clone())));
    app.session
        .record_recent(ComparisonMode::File, second.clone());
    // Closing must drain the queued save, including when load was still pending.
    drop(app);
    let reopened = RecentComparisons::load_from(&history_path).unwrap();
    assert_eq!(
        reopened.entries(),
        &[
            RecentComparison {
                mode: ComparisonMode::File,
                paths: second.clone()
            },
            RecentComparison {
                mode: ComparisonMode::Folder,
                paths: first.clone()
            },
        ]
    );
    let mut app = VersusApp::default();
    app.session.recent_store = Some(RecentStore::start(&ctx, Some(history_path)));
    app.session
        .record_recent(ComparisonMode::Folder, first.clone());
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.session.recent.entries().len() != 2 {
        app.session.poll_recent(&ctx);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        app.session.recent.entries(),
        &[
            RecentComparison {
                mode: ComparisonMode::Folder,
                paths: first
            },
            RecentComparison {
                mode: ComparisonMode::File,
                paths: second
            },
        ]
    );
    assert!(app.session.recent_message.is_none());
}

#[test]
fn refresh_button_reloads_direct_files_and_folder_drilldown() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    for from_folders in [false, true] {
        std::fs::write(sources.path("right/model.txt"), "common\nnew\n").unwrap();
        let mut app = VersusApp::default();
        if from_folders {
            app.session.select_path(0, sources.path("left"));
            app.session.select_path(1, sources.path("right"));
            settle_sources(&mut app, &ctx);
            app.session.selected = Some("model.txt".into());
            app.session
                .open_file("model.txt".into(), [true, true], false);
        } else {
            app.session.comparison_mode = ComparisonMode::File;
            app.session.select_path(0, sources.path("left/model.txt"));
            app.session.select_path(1, sources.path("right/model.txt"));
        }
        settle_sources(&mut app, &ctx);
        let paths = app.session.file_view.as_ref().unwrap().paths.clone();
        assert_eq!(app.session.file_view.as_ref().unwrap().counts[1], 1);
        std::fs::write(sources.path("right/model.txt"), "common\nold\n").unwrap();
        render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
        let refresh = ctx
            .read_response(egui::Id::new("Refresh comparison"))
            .unwrap()
            .rect
            .center();
        click(&mut app, &ctx, refresh);
        assert!(app.session.file_view.as_ref().unwrap().job.is_some());
        assert!(app.session.job.is_none());
        settle_sources(&mut app, &ctx);
        let view = app.session.file_view.as_ref().unwrap();
        assert_eq!(view.paths, paths);
        assert_eq!(view.from_folders, from_folders);
        assert_eq!(view.counts[1], 0);
        assert_eq!(view.counts[0], 2);
        if from_folders {
            app.session.file_view = None;
            assert_eq!(
                app.session.selected.as_deref(),
                Some(std::path::Path::new("model.txt"))
            );
            assert!(app.session.tree.is_some());
        }
    }
}

#[test]
fn folder_file_refresh_discovers_created_and_removed_sides_and_rejects_symlinks() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.session.select_path(0, sources.path("left"));
    app.session.select_path(1, sources.path("right"));
    settle_sources(&mut app, &ctx);
    std::fs::remove_file(sources.path("right/model.txt")).unwrap();
    app.session
        .open_file("model.txt".into(), [true, false], false);
    settle_sources(&mut app, &ctx);
    std::fs::write(sources.path("right/model.txt"), "common\nold\n").unwrap();
    app.session.refresh_file();
    settle_sources(&mut app, &ctx);
    assert!(
        app.session
            .file_view
            .as_ref()
            .unwrap()
            .sources
            .iter()
            .all(Option::is_some)
    );
    assert_eq!(app.session.file_view.as_ref().unwrap().counts[0], 2);
    std::fs::remove_file(sources.path("left/model.txt")).unwrap();
    app.session.refresh_file();
    settle_sources(&mut app, &ctx);
    assert!(app.session.file_view.as_ref().unwrap().sources[0].is_none());
    assert_eq!(app.session.file_view.as_ref().unwrap().counts[3], 2);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            sources.path("right/model.txt"),
            sources.path("left/model.txt"),
        )
        .unwrap();
        app.session.refresh_file();
        settle_sources(&mut app, &ctx);
        assert!(
            app.session
                .file_view
                .as_ref()
                .unwrap()
                .error
                .as_deref()
                .unwrap()
                .contains("non-symlink")
        );
    }
}
