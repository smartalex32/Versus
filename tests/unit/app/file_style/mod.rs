use super::*;
use crate::app::visuals::theme::{Palette, comparison_font, comparison_row_height, set_theme};

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
                let rect = Rect::from_min_size(egui::pos2(100.0, 80.0), egui::vec2(300.0, 24.0));
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
