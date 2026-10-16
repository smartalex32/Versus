//! Session sources regression coverage.

use super::*;
use crate::app::visuals::theme::{apply_theme, set_theme};

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
