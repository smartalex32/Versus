//! Inline transfer affordances and code-line layout shared by editable and read-only panes.

use super::*;
use crate::app::visuals::theme::Palette;

pub(super) fn inline_action(
    ui: &egui::Ui,
    rect: Rect,
    id: egui::Id,
    glyph: &str,
    label: &str,
) -> egui::Response {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let palette = Palette::for_context(ui.ctx());
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect.shrink(1.0), 2.0, palette.hover);
    }
    let color = if response.hovered() {
        palette.accent
    } else {
        palette.text
    };
    let stroke = Stroke::new(2.0, color);
    let center = rect.center();
    ui.painter().line_segment(
        [center - egui::vec2(6.0, 0.0), center + egui::vec2(6.0, 0.0)],
        stroke,
    );
    if glyph == "+" {
        ui.painter().line_segment(
            [center - egui::vec2(0.0, 6.0), center + egui::vec2(0.0, 6.0)],
            stroke,
        );
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
}

/// Larger, side-colored transfer arrows use the same palette slots as File
/// Diff's left-only and right-only status icons.  This stays separate from the
/// smaller expand/collapse controls kept inside the file content.
pub(super) fn transfer_action(
    ui: &egui::Ui,
    rect: Rect,
    id: egui::Id,
    side: usize,
    label: &str,
) -> egui::Response {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let palette = Palette::for_context(ui.ctx());
    let color = if side == 0 {
        palette.left_only
    } else {
        palette.right_only
    };
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect.shrink(1.0), 2.0, palette.hover);
    }
    // Vector geometry avoids missing-glyph boxes in platform font fallbacks.
    let direction = if side == 0 { 1.0 } else { -1.0 };
    let point = |x: f32, y: f32| rect.center() + egui::vec2(x * direction, y);
    let stroke = Stroke::new(2.0, color);
    ui.painter()
        .line_segment([point(-5.0, 0.0), point(5.0, 0.0)], stroke);
    ui.painter().line(
        vec![point(0.0, -5.0), point(5.0, 0.0), point(0.0, 5.0)],
        stroke,
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
}

pub(super) fn append_code_line(
    job: &mut egui::text::LayoutJob,
    line: &str,
    format: egui::TextFormat,
    syntax: bool,
    palette: Palette,
) {
    if !syntax {
        job.append(line, 0.0, format);
        return;
    }
    let keywords = [
        "fn", "pub", "let", "mut", "impl", "struct", "enum", "use", "if", "else", "return",
        "match", "async", "await", "class", "def", "import", "from", "const", "function", "var",
        "void", "int", "for", "while", "true", "false", "None", "null",
    ];
    let mut offset = 0;
    for token in line.split_inclusive(|c: char| !c.is_alphanumeric() && c != '_') {
        let word = token.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_');
        if keywords.contains(&word) {
            let mut colored = format.clone();
            colored.color = palette.accent;
            job.append(word, 0.0, colored);
            job.append(&token[word.len()..], 0.0, format.clone());
        } else {
            job.append(token, 0.0, format.clone());
        }
        offset += token.len();
    }
    if offset < line.len() {
        job.append(&line[offset..], 0.0, format);
    }
}
