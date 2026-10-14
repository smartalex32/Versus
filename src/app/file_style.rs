//! Shared file-row presentation used by Diff and the diff-assisted Merge view.
//!
//! Keeping the primitive drawing here prevents the editable Merge surface from
//! slowly acquiring a nearly-the-same version of the File Diff presentation.

use super::*;
use std::ops::Range;

/// Paint the fill, alternating band, selection overlay, and state stripe for a
/// comparison row.
pub(super) fn paint_background(
    painter: &egui::Painter,
    rect: Rect,
    state: &DirectoryEntryState,
    alternate: bool,
    selected: bool,
) {
    let palette = Palette::for_context(painter.ctx());
    let color = state_color(state, palette);
    painter.rect_filled(
        rect,
        0,
        if alternate {
            palette.alternate
        } else {
            palette.panel
        },
    );
    if selected {
        painter.rect_filled(rect, 0, palette.accent.gamma_multiply(0.13));
    }
    if *state != DirectoryEntryState::Same {
        painter.rect_filled(
            Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
            0,
            color,
        );
    }
}

/// Paint the line number and status icon at the fixed File Diff gutter slots.
pub(super) fn paint_gutter(
    painter: &egui::Painter,
    rect: Rect,
    number: usize,
    state: &DirectoryEntryState,
) {
    let palette = Palette::for_context(painter.ctx());
    painter.text(
        rect.left_center() + egui::vec2(42.0, 0.0),
        Align2::RIGHT_CENTER,
        number.to_string(),
        FontId::monospace(10.0),
        palette.muted,
    );
    if let Some(icon) = status_icon(state) {
        paint_status_icon(painter, rect.left_center() + egui::vec2(54.0, 0.0), icon);
    }
}

/// Append text using the File Diff colors and inline changed-character blocks.
///
/// Invalid byte ranges are ignored deliberately: the diff model is allowed to
/// fall back to a plain line if an old comparison races a newer working copy.
pub(super) fn append_text(
    job: &mut egui::text::LayoutJob,
    text: &str,
    state: &DirectoryEntryState,
    changed: &[Range<usize>],
    palette: Palette,
    font: FontId,
    line_height: Option<f32>,
) {
    let color = state_color(state, palette);
    let mut start = 0;
    for range in changed {
        if range.start < start
            || range.end > text.len()
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
        {
            continue;
        }
        append_piece(
            job,
            &text[start..range.start],
            color,
            Color32::TRANSPARENT,
            font.clone(),
            line_height,
        );
        append_piece(
            job,
            &text[range.clone()],
            palette.text,
            color.gamma_multiply(0.3),
            font.clone(),
            line_height,
        );
        start = range.end;
    }
    append_piece(
        job,
        &text[start..],
        color,
        Color32::TRANSPARENT,
        font,
        line_height,
    );
}

/// Paint a line-ending badge immediately after the actual rendered text.
pub(super) fn paint_ending(
    painter: &egui::Painter,
    rect: Rect,
    end_x: f32,
    ending: versus::DisplayLineEnding,
    state: &DirectoryEntryState,
) {
    let palette = Palette::for_context(painter.ctx());
    let color = state_color(state, palette);
    let badge = Rect::from_min_size(
        egui::pos2(end_x + 8.0, rect.top() + 3.0),
        egui::vec2(58.0, 16.0),
    );
    painter.rect_filled(badge, 2, color.gamma_multiply(0.3));
    painter.text(
        badge.center(),
        Align2::CENTER_CENTER,
        ending.label(),
        FontId::monospace(9.0),
        palette.text,
    );
}

fn append_piece(
    job: &mut egui::text::LayoutJob,
    text: &str,
    color: Color32,
    background: Color32,
    font: FontId,
    line_height: Option<f32>,
) {
    // TextEdit cursor ranges are expressed in bytes of the original buffer.
    // Keep tabs intact here: egui expands their advance during layout, while a
    // replacement would make the galley byte indices disagree with the editor.
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: font,
            color,
            background,
            line_height,
            ..Default::default()
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_text_uses_the_same_state_palette_as_a_file_row() {
        let context = egui::Context::default();
        let palette = Palette::for_context(&context);
        let mut job = egui::text::LayoutJob::default();
        append_text(
            &mut job,
            "left\tright",
            &DirectoryEntryState::LeftOnly,
            &[4..9],
            palette,
            FontId::monospace(12.0),
            Some(22.0),
        );
        assert_eq!(job.text, "left\tright");
        assert_eq!(job.sections.len(), 3);
        assert_eq!(job.sections[1].format.color, palette.text);
        assert_eq!(
            job.sections[1].format.background,
            palette.left_only.gamma_multiply(0.3)
        );
        assert_eq!(job.sections[1].format.line_height, Some(22.0));
    }

    #[test]
    fn tabs_keep_editor_byte_offsets_while_using_eguis_tab_advance() {
        let context = egui::Context::default();
        context
            .run_ui(egui::RawInput::default(), |_| {})
            .drop_without_applying_deltas();
        let palette = Palette::for_context(&context);
        let font = FontId::monospace(12.0);
        let mut tabs = egui::text::LayoutJob::default();
        let mut spaces = egui::text::LayoutJob::default();
        append_text(
            &mut tabs,
            "a\tb",
            &DirectoryEntryState::Same,
            &[],
            palette,
            font.clone(),
            None,
        );
        append_text(
            &mut spaces,
            "a    b",
            &DirectoryEntryState::Same,
            &[],
            palette,
            font,
            None,
        );
        assert_eq!(tabs.text, "a\tb");
        let tab_width = context.fonts_mut(|fonts| fonts.layout_job(tabs.clone()).size().x);
        let space_width = context.fonts_mut(|fonts| fonts.layout_job(spaces).size().x);
        assert!((tab_width - space_width).abs() < 0.1);
        // epaint lays tabs out using its configured four-space tab stop; the
        // essential invariant here is that the source byte offsets stay raw.
        assert_ne!(tabs.text.len(), "a    b".len());
    }

    #[test]
    fn invalid_highlight_ranges_do_not_break_plain_text() {
        let context = egui::Context::default();
        let mut job = egui::text::LayoutJob::default();
        append_text(
            &mut job,
            "é",
            &DirectoryEntryState::Different,
            &[1..2],
            Palette::for_context(&context),
            FontId::monospace(12.0),
            None,
        );
        assert_eq!(job.text, "é");
        assert_eq!(job.sections.len(), 1);
    }
}
