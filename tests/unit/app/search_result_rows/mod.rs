use super::*;
use std::{ops::Range, path::PathBuf};

fn hit(range: Range<usize>) -> versus::SearchHit {
    versus::SearchHit {
        id: 7,
        path: PathBuf::from("sample.rs"),
        side: 0,
        line: 4,
        matched: String::new(),
        range,
    }
}

#[test]
fn preview_keeps_match_and_nearby_context() {
    let source = "before text needle after text";
    let preview = preview_parts(source, &hit(12..18));
    assert_eq!(preview.before, "before text ");
    assert_eq!(preview.matched, "needle");
    assert_eq!(preview.after, " after text");
}

#[test]
fn preview_respects_utf8_boundaries() {
    let source = format!("{} needle {}", "é".repeat(60), "界".repeat(60));
    let start = source.find("needle").unwrap();
    let preview = preview_parts(&source, &hit(start..start + 6));
    assert_eq!(preview.matched, "needle");
    assert!(preview.before.is_char_boundary(preview.before.len()));
    assert!(preview.leading_ellipsis);
    assert!(preview.trailing_ellipsis);
}

#[test]
fn long_line_shows_far_right_match_with_ellipsis() {
    let source = format!("{}needle tail", "a".repeat(2_000));
    let start = source.find("needle").unwrap();
    let preview = preview_parts(&source, &hit(start..start + 6));
    assert!(preview.leading_ellipsis);
    assert_eq!(preview.matched, "needle");
    assert!(preview.after.contains("tail"));
}

#[test]
fn clicking_remove_reports_action_without_mutating_hit() {
    let context = egui::Context::default();
    let original = hit(0..4);
    let draw = |events: Vec<egui::Event>| {
        let mut removed = false;
        context
            .run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(320.0, 120.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    removed = result_row(ui, &original, "find me");
                },
            )
            .drop_without_applying_deltas();
        removed
    };
    assert!(!draw(Vec::new()));
    let position = context
        .read_response(egui::Id::new((
            "remove-search-result",
            original.side,
            original.id,
            &original.path,
        )))
        .unwrap()
        .rect
        .center();
    assert!(!draw(vec![
        egui::Event::PointerMoved(position),
        egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]));
    assert!(draw(vec![
        egui::Event::PointerMoved(position),
        egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]));
    assert_eq!(original.id, 7);
    assert_eq!(original.range, 0..4);
}

#[test]
fn narrow_row_keeps_highlighted_match_and_ellipsis_in_bounds() {
    let context = egui::Context::default();
    let source = format!("{}needle{}", "prefix ".repeat(32), " suffix".repeat(32));
    let start = source.find("needle").unwrap();
    let result = context.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(160.0, 80.0))),
            ..Default::default()
        },
        |ui| {
            assert!(!result_row(
                ui,
                &hit(start..start + "needle".len()),
                &source
            ));
        },
    );
    let text_shape = result
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text)
                if text.galley.text().contains("needle")
                    && text
                        .galley
                        .job
                        .sections
                        .iter()
                        .any(|section| section.format.background != Color32::TRANSPARENT) =>
            {
                Some((shape.clip_rect, text.galley.clone()))
            }
            _ => None,
        })
        .expect("the visible row should retain the highlighted match");
    assert!(text_shape.1.text().contains('…'));
    assert!(text_shape.1.size().x <= text_shape.0.width());
    result.drop_without_applying_deltas();
}
