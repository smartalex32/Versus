use super::MergeView;
use super::*;
use crate::app::interactive_view::projection::editor_byte_edit;
use versus::core::git_merge::{GitMergeModel, MergeChoice};

fn rendered_text(output: &egui::FullOutput) -> Vec<String> {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}

#[test]
fn focused_view_labels_three_fixed_roles_and_selected_scope() {
    let context = egui::Context::default();
    crate::app::visuals::theme::apply_theme(&context);
    let mut model = GitMergeModel::new(
        Some("base\n".into()),
        "incoming\n".into(),
        "current\n".into(),
        "result\n".into(),
    )
    .unwrap();
    let mut view = MergeView::default();
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            ..Default::default()
        },
        |ui| assert!(!view.render(ui, &mut model, "example.rs", false)),
    );
    output.textures_delta.clear();
    let text = rendered_text(&output);
    assert!(
        text.iter()
            .any(|value| value.contains("Incoming (Git Remote)"))
    );
    assert!(text.iter().any(|value| value.contains("Result (editable)")));
    assert!(
        text.iter()
            .any(|value| value.contains("Current (Git Local)"))
    );
    assert!(
        text.iter()
            .any(|value| value.starts_with("Scope — Incoming"))
    );
}

#[test]
fn incoming_arrow_replaces_only_the_selected_result_region() {
    let mut model = GitMergeModel::new(
        Some("base\nkeep\n".into()),
        "incoming\nkeep\n".into(),
        "current\nkeep\n".into(),
        "partial\nkeep\n".into(),
    )
    .unwrap();
    let mut view = MergeView::default();
    view.clamp_selected_region(&model);
    view.choose(&mut model, MergeChoice::Incoming);
    assert_eq!(model.result(), "incoming\nkeep\n");
}

#[test]
fn result_edit_reuses_projection_mapping_to_keep_crlf() {
    let original = "one\r\ntwo\r\n";
    let (range, replacement) = editor_byte_edit(original, "one\nchanged\n", None);
    let mut updated = original.to_owned();
    updated.replace_range(range, &replacement);
    assert_eq!(updated, "one\r\nchanged\r\n");
}

#[test]
fn source_choice_undo_restores_the_conflict_ledger() {
    let mut model = GitMergeModel::new(
        Some("base\n".into()),
        "incoming\n".into(),
        "current\n".into(),
        "<<<<<<< current\ncurrent\n=======\nincoming\n>>>>>>> incoming\n".into(),
    )
    .unwrap();
    let mut view = MergeView::default();
    view.clamp_selected_region(&model);
    view.choose(&mut model, MergeChoice::Incoming);
    assert_eq!(model.unresolved_count(), 0);
    assert!(model.undo());
    assert_eq!(model.unresolved_count(), 1);
    assert!(model.result().starts_with("<<<<<<<"));
}

#[test]
fn busy_render_never_requests_save() {
    let context = egui::Context::default();
    crate::app::visuals::theme::apply_theme(&context);
    let mut model = GitMergeModel::new(
        Some("base\n".into()),
        "incoming\n".into(),
        "current\n".into(),
        "result\n".into(),
    )
    .unwrap();
    let mut view = MergeView::default();
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            ..Default::default()
        },
        |ui| assert!(!view.render(ui, &mut model, "example.rs", true)),
    );
    output.textures_delta.clear();
}

#[test]
fn inline_incoming_arrow_uses_the_shared_gui_control_and_changes_only_result() {
    let context = egui::Context::default();
    crate::app::visuals::theme::apply_theme(&context);
    let mut model = GitMergeModel::new(
        None,
        "incoming\nkeep\n".into(),
        "current\nkeep\n".into(),
        "partial\nkeep\n".into(),
    )
    .unwrap();
    let mut view = MergeView::default();
    let raw = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1400.0, 850.0),
        )),
        ..Default::default()
    };
    context
        .run_ui(raw(), |ui| {
            view.render(ui, &mut model, "example.rs", false);
        })
        .drop_without_applying_deltas();
    let position = context
        .read_response(egui::Id::new(("git-merge-incoming", "take-selected")))
        .unwrap()
        .rect
        .center();
    for pressed in [true, false] {
        let mut input = raw();
        input.events = vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ];
        context
            .run_ui(input, |ui| {
                view.render(ui, &mut model, "example.rs", false);
            })
            .drop_without_applying_deltas();
    }
    assert_eq!(model.result(), "incoming\nkeep\n");
    assert_eq!(model.incoming(), "incoming\nkeep\n");
    assert_eq!(model.current(), "current\nkeep\n");
}

#[test]
fn source_scope_uses_lines_and_handles_mixed_endings_and_unicode() {
    assert_eq!(super::text::line_scope("α\r\nb\rc\n", &(4..6)), "line 2");
    assert_eq!(super::text::display_range("α\r\nb\rc\n", 4..6), 3..5);
}
