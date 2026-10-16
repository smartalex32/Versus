//! File diff regression coverage.

use super::*;
use crate::app::session::update_visible_file_rows;
use crate::app::visuals::status::StatusIcon;
use crate::app::visuals::theme::{apply_theme, set_theme};

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
