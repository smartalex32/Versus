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

pub(super) const TEXT_OFFSET: f32 = 56.0;

/// Keep the normal gutter compact and reserve extra space for wide line numbers.
pub(super) fn gutter_extra(line_count: usize) -> f32 {
    let digits = line_count.max(1).ilog10() + 1;
    digits.saturating_sub(4) as f32 * 6.0
}

/// Paint the line number and status icon at the fixed File Diff gutter slots.
pub(super) fn paint_gutter(
    painter: &egui::Painter,
    rect: Rect,
    number: usize,
    state: &DirectoryEntryState,
    extra: f32,
) {
    let palette = Palette::for_context(painter.ctx());
    painter.text(
        rect.left_center() + egui::vec2(30.0 + extra, 0.0),
        Align2::RIGHT_CENTER,
        number.to_string(),
        FontId::monospace(10.0),
        palette.muted,
    );
    if let Some(icon) = status_icon(state) {
        paint_status_icon(
            painter,
            rect.left_center() + egui::vec2(42.0 + extra, 0.0),
            icon,
        );
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
    append_text_inner(job, text, state, changed, palette, font, line_height, true);
}

fn append_text_inner(
    job: &mut egui::text::LayoutJob,
    text: &str,
    state: &DirectoryEntryState,
    changed: &[Range<usize>],
    palette: Palette,
    font: FontId,
    line_height: Option<f32>,
    inline_background: bool,
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
            1.0,
        );
        append_piece(
            job,
            &text[range.clone()],
            palette.text,
            if inline_background {
                color.gamma_multiply(0.3)
            } else {
                Color32::TRANSPARENT
            },
            font.clone(),
            line_height,
            1.0,
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
        1.0,
    );
}

/// Append File Diff colors without a text-format background. The editable
/// Merge renderer paints its inline background separately so it can use the
/// native glyph-height rectangle within its taller editor row.
pub(super) fn append_text_without_inline_background(
    job: &mut egui::text::LayoutJob,
    text: &str,
    state: &DirectoryEntryState,
    changed: &[Range<usize>],
    palette: Palette,
    font: FontId,
    line_height: Option<f32>,
) {
    append_text_inner(job, text, state, changed, palette, font, line_height, false);
}

/// Compute inline changed spans at native glyph height, centered in an
/// editable comparison row. The caller reserves their shapes before TextEdit
/// paints, so highlights remain behind text even after horizontal scrolling.
pub(super) fn inline_highlight_rects(
    painter: &egui::Painter,
    text_origin: Pos2,
    text: &str,
    changed: &[Range<usize>],
    font: FontId,
    row_height: f32,
) -> Vec<Rect> {
    // Derive geometry from File Diff's native galley, including its one-point
    // background expansion. This keeps short spans, tabs and fallback glyphs
    // identical while reserving the editor's padded height for cursor layout.
    let mut job = egui::text::LayoutJob::default();
    append_text(
        &mut job,
        text,
        &DirectoryEntryState::Different,
        changed,
        Palette::for_context(painter.ctx()),
        font,
        None,
    );
    let galley = painter.layout_job(job);
    let offset =
        text_origin.to_vec2() + egui::vec2(0.0, ((row_height - galley.size().y) / 2.0).max(0.0));
    galley
        .rows
        .iter()
        .flat_map(|row| {
            row.visuals.mesh.vertices[..row.visuals.glyph_vertex_range.start]
                .chunks_exact(4)
                .map(move |vertices| {
                    let mut rect = Rect::NOTHING;
                    for vertex in vertices {
                        rect.extend_with(vertex.pos + row.pos.to_vec2() + offset);
                    }
                    rect
                })
        })
        .collect()
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
    background_expand: f32,
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
            expand_bg: background_expand,
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
        assert_eq!(job.sections[1].format.expand_bg, 1.0);
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
    fn inline_highlights_match_native_height_keep_short_spans_and_follow_scroll() {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            for zoom in [1.0, 2.0] {
                let ctx = egui::Context::default();
                set_theme(&ctx, theme);
                ctx.set_zoom_factor(zoom);
                ctx.run_ui(egui::RawInput::default(), |ui| {
                    let font = comparison_font(ui.ctx());
                    let row_height = comparison_row_height(ui);
                    let font_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
                    let origin = egui::pos2(100.0, 80.0);
                    let ranges = [0..1, 4..5];
                    let rects = inline_highlight_rects(
                        ui.painter(),
                        origin,
                        "a\tb c",
                        &ranges,
                        font.clone(),
                        row_height,
                    );
                    assert_eq!(rects.len(), 2, "all disjoint ranges are present");
                    for rect in &rects {
                        assert!(
                            rect.width() > 1.0,
                            "one-character spans must retain their width"
                        );
                        assert!((rect.height() - (font_height + 2.0)).abs() < 0.6);
                        assert!(rect.top() >= origin.y && rect.bottom() <= origin.y + row_height);
                    }
                    let shifted = inline_highlight_rects(
                        ui.painter(),
                        origin - egui::vec2(60.0, 0.0),
                        "a\tb c",
                        &ranges,
                        font.clone(),
                        row_height,
                    );
                    for (before, after) in rects.iter().zip(shifted) {
                        assert!((before.left() - after.left() - 60.0).abs() < 0.1);
                        assert_eq!(before.height(), after.height());
                    }
                    let mut native = egui::text::LayoutJob::default();
                    append_text(
                        &mut native,
                        "a\tb c",
                        &DirectoryEntryState::Different,
                        &ranges,
                        Palette::for_context(ui.ctx()),
                        font,
                        None,
                    );
                    for section in native
                        .sections
                        .iter()
                        .filter(|section| section.format.background != Color32::TRANSPARENT)
                    {
                        assert_eq!(
                            section.format.expand_bg, 1.0,
                            "preserve native File Diff highlights"
                        );
                        assert_eq!(section.format.line_height, None);
                    }
                })
                .drop_without_applying_deltas();
            }
        }
    }

    #[test]
    fn compact_gutter_keeps_small_and_wide_line_numbers_clear_of_the_diff_bar() {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            for zoom in [1.0, 2.0] {
                for number in [1usize, 9999, 10_000, 99_999_999] {
                    let ctx = egui::Context::default();
                    set_theme(&ctx, theme);
                    ctx.set_zoom_factor(zoom);
                    let rect =
                        Rect::from_min_size(egui::pos2(100.0, 80.0), egui::vec2(300.0, 24.0));
                    let extra = gutter_extra(number);
                    let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                        paint_background(
                            ui.painter(),
                            rect,
                            &DirectoryEntryState::Different,
                            false,
                            false,
                        );
                        paint_gutter(
                            ui.painter(),
                            rect,
                            number,
                            &DirectoryEntryState::Different,
                            extra,
                        );
                    });
                    let label = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) if text.galley.text() == number.to_string() => {
                                Some(text)
                            }
                            _ => None,
                        })
                        .unwrap();
                    assert!(
                        label.pos.x >= rect.left() + 5.0,
                        "wide numbers must clear the three-pixel bar"
                    );
                    assert!(
                        label.pos.x + label.galley.size().x < rect.left() + 42.0 + extra - 5.0,
                        "numbers must clear the status icon"
                    );
                    if number == 1 {
                        assert!(
                            label.pos.x < rect.left() + 25.0,
                            "ordinary numbers move closer to the diff bar"
                        );
                    }
                    output.drop_without_applying_deltas();
                }
            }
        }
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
