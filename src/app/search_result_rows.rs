//! Compact, stable rows for the Find and Replace result panel.
//!
//! The row deliberately only receives one already-loaded source snapshot.  It
//! derives a small window around the hit offset instead of walking the whole
//! file, which keeps a large result list inexpensive to paint.

use super::*;
use crate::app::visuals::theme::{Palette, control_extent};

const CONTEXT_BYTES: usize = 72;
const MATCH_BYTES: usize = 96;

#[derive(Debug, Eq, PartialEq)]
struct PreviewParts {
    before: String,
    matched: String,
    after: String,
    leading_ellipsis: bool,
    trailing_ellipsis: bool,
}

/// Paint one result row and return whether its hover-only remove control was
/// activated. The caller owns the selected-result set and performs removal.
pub(super) fn result_row(ui: &mut egui::Ui, hit: &versus::SearchHit, source_text: &str) -> bool {
    let palette = Palette::for_context(ui.ctx());
    let height = control_extent(ui);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let row_id = egui::Id::new(("search-result-row", hit.side, &hit.path, hit.id));
    let row = ui.interact(rect, row_id, Sense::hover());
    let remove_size = height;
    let remove_rect = Rect::from_min_size(
        egui::pos2(rect.right() - remove_size, rect.top()),
        egui::vec2(remove_size, remove_size),
    );
    let remove_id = egui::Id::new(("remove-search-result", hit.side, hit.id, &hit.path));
    let remove = ui.interact(remove_rect, remove_id, Sense::click());
    remove
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Remove result"));

    if row.hovered() || row.has_focus() || remove.hovered() || remove.has_focus() {
        ui.painter().rect_filled(rect, 3.0, palette.hover);
    }

    let text_rect = Rect::from_min_max(
        rect.min,
        egui::pos2(remove_rect.left() - 2.0, rect.bottom()),
    );
    let label = format!("{}: ", hit.line);
    let preview = fit_preview(
        ui,
        &label,
        preview_parts(source_text, hit),
        (text_rect.width() - 8.0).max(1.0),
    );
    let mut job = egui::text::LayoutJob::default();
    append(&mut job, &label, palette.muted, Color32::TRANSPARENT);
    if preview.leading_ellipsis {
        append(&mut job, "…", palette.muted, Color32::TRANSPARENT);
    }
    append(
        &mut job,
        &preview.before,
        palette.text,
        Color32::TRANSPARENT,
    );
    append(
        &mut job,
        &preview.matched,
        palette.text,
        palette.changed.gamma_multiply(0.35),
    );
    append(&mut job, &preview.after, palette.text, Color32::TRANSPARENT);
    if preview.trailing_ellipsis {
        append(&mut job, "…", palette.muted, Color32::TRANSPARENT);
    }
    // This remains a last line of defence for unusually narrow windows and
    // font substitutions.  `fit_preview` keeps the match in the supplied
    // text; the layout engine provides a proper final ellipsis if a glyph's
    // actual width exceeds our measurement.
    job.wrap.max_width = (text_rect.width() - 8.0).max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = ui.painter().layout_job(job);
    ui.painter().with_clip_rect(text_rect).galley(
        egui::pos2(
            text_rect.left() + 4.0,
            text_rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        palette.text,
    );

    let show_remove = row.hovered() || row.has_focus() || remove.hovered() || remove.has_focus();
    if show_remove {
        if remove.hovered() || remove.has_focus() {
            ui.painter()
                .rect_filled(remove_rect.shrink(2.0), 3.0, palette.border);
        }
        paint_cancel(ui.painter(), remove_rect.center(), palette.error);
    }
    remove
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Remove result")
        .clicked()
}

fn append(job: &mut egui::text::LayoutJob, text: &str, color: Color32, background: Color32) {
    if text.is_empty() {
        return;
    }
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: FontId::monospace(11.0),
            color,
            background,
            ..Default::default()
        },
    );
}

fn paint_cancel(painter: &egui::Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(1.4, color);
    painter.line_segment(
        [center - egui::vec2(4.0, 4.0), center + egui::vec2(4.0, 4.0)],
        stroke,
    );
    painter.line_segment(
        [
            center + egui::vec2(-4.0, 4.0),
            center + egui::vec2(4.0, -4.0),
        ],
        stroke,
    );
}

fn preview_parts(source: &str, hit: &versus::SearchHit) -> PreviewParts {
    let start = floor_boundary(source, hit.range.start.min(source.len()));
    let end = ceil_boundary(source, hit.range.end.clamp(start, source.len()));

    let before_window_start = floor_boundary(source, start.saturating_sub(CONTEXT_BYTES));
    let before_window = &source[before_window_start..start];
    let (before, leading_ellipsis) = match before_window.rfind('\n') {
        Some(newline) => (&before_window[newline + 1..], false),
        None => (before_window, before_window_start > 0),
    };

    let after_window_end =
        ceil_boundary(source, end.saturating_add(CONTEXT_BYTES).min(source.len()));
    let after_window = &source[end..after_window_end];
    let (after, trailing_ellipsis) = match after_window.find('\n') {
        Some(newline) => (&after_window[..newline], false),
        None => (after_window, after_window_end < source.len()),
    };

    let matched = &source[start..end];
    let matched = if matched.is_empty() { "∅" } else { matched };
    let (matched, match_clipped) = middle_clip(matched, MATCH_BYTES);
    PreviewParts {
        before: display_text(before),
        matched: display_text(matched),
        after: display_text(after),
        leading_ellipsis,
        trailing_ellipsis: trailing_ellipsis || match_clipped,
    }
}

/// Fit the local context around the match to the actual row width.  The left
/// context is cut from its left edge and the right context from its right edge,
/// so the nearest source characters and the highlighted match survive.
fn fit_preview(
    ui: &egui::Ui,
    label: &str,
    mut preview: PreviewParts,
    max_width: f32,
) -> PreviewParts {
    let ellipsis_width = text_width(ui, "…");
    let mut fixed_width = text_width(ui, label);
    // Reserve a truncation marker for each available context even before we
    // know whether it will need clipping. This prevents a newly-added marker
    // from displacing the match in a narrow row.
    if preview.leading_ellipsis || !preview.before.is_empty() {
        fixed_width += ellipsis_width;
    }
    if preview.trailing_ellipsis || !preview.after.is_empty() {
        fixed_width += ellipsis_width;
    }

    let match_room = (max_width - fixed_width).max(0.0);
    let (matched, match_trimmed) = prefix_to_width(ui, &preview.matched, match_room);
    if match_trimmed {
        preview.matched = matched;
        preview.after.clear();
        preview.trailing_ellipsis = true;
    }

    let match_width = text_width(ui, &preview.matched);
    let context_room = (max_width - fixed_width - match_width).max(0.0);
    let before_room = if preview.before.is_empty() {
        0.0
    } else if preview.after.is_empty() {
        context_room
    } else {
        context_room * 0.45
    };
    let (before, before_trimmed) = suffix_to_width(ui, &preview.before, before_room);
    preview.before = before;
    if before_trimmed {
        preview.leading_ellipsis = true;
    }

    let used = text_width(ui, &preview.before);
    let after_room = (context_room - used).max(0.0);
    let (after, after_trimmed) = prefix_to_width(ui, &preview.after, after_room);
    preview.after = after;
    if after_trimmed {
        preview.trailing_ellipsis = true;
    }
    preview
}

fn text_width(ui: &egui::Ui, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), FontId::monospace(11.0), Color32::WHITE)
        .size()
        .x
}

fn prefix_to_width(ui: &egui::Ui, text: &str, max_width: f32) -> (String, bool) {
    if text_width(ui, text) <= max_width {
        return (text.to_owned(), false);
    }
    let mut end = 0;
    for (index, character) in text.char_indices() {
        let next = index + character.len_utf8();
        if text_width(ui, &text[..next]) > max_width {
            break;
        }
        end = next;
    }
    (text[..end].to_owned(), end < text.len())
}

fn suffix_to_width(ui: &egui::Ui, text: &str, max_width: f32) -> (String, bool) {
    if text_width(ui, text) <= max_width {
        return (text.to_owned(), false);
    }
    let mut start = text.len();
    for (index, _) in text.char_indices().rev() {
        if text_width(ui, &text[index..]) > max_width {
            break;
        }
        start = index;
    }
    (text[start..].to_owned(), start > 0)
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

fn middle_clip(text: &str, max_bytes: usize) -> (&str, bool) {
    if text.len() <= max_bytes {
        return (text, false);
    }
    let prefix_end = floor_boundary(text, max_bytes / 2);
    // Keeping the beginning of a match visible is more useful than an invalid
    // byte slice. The result remains bounded by the caller's local window.
    (&text[..prefix_end], true)
}

fn display_text(text: &str) -> String {
    text.replace('\r', "")
        .replace('\n', " ↵ ")
        .replace('\t', "    ")
}

#[cfg(test)]
mod tests;
