//! Shared line coordinates and selection paint for the three Git merge panes.

use super::*;
use std::ops::Range;

pub(super) fn display_range(original: &str, range: Range<usize>) -> Range<usize> {
    // Only CRLF removes bytes during display normalization. UTF-8 byte offsets
    // otherwise remain unchanged, including lone CR rendered as LF.
    let offset = |end: usize| {
        end - original.as_bytes()[..end]
            .windows(2)
            .filter(|pair| *pair == b"\r\n")
            .count()
    };
    offset(range.start)..offset(range.end)
}

pub(super) fn line_number(text: &str, byte: usize) -> usize {
    let prefix = &text[..byte.min(text.len())];
    1 + prefix
        .bytes()
        .filter(|byte| *byte == b'\n' || *byte == b'\r')
        .count()
        - prefix
            .as_bytes()
            .windows(2)
            .filter(|pair| *pair == b"\r\n")
            .count()
}

pub(super) fn line_scope(text: &str, range: &Range<usize>) -> String {
    let start = line_number(text, range.start);
    if range.is_empty() {
        return format!("insert at line {start}");
    }
    let end_byte = text[..range.end]
        .char_indices()
        .last()
        .map_or(0, |(byte, _)| byte);
    let end = line_number(text, end_byte).max(start);
    if end == start {
        format!("line {start}")
    } else {
        format!("lines {start}–{end}")
    }
}

pub(super) fn layout(
    ui: &egui::Ui,
    text: &str,
    selected: Option<&Range<usize>>,
) -> std::sync::Arc<egui::Galley> {
    let palette = Palette::for_context(ui.ctx());
    let font = comparison_font(ui.ctx());
    let height = Some(crate::app::visuals::theme::comparison_row_height(ui));
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let selected = selected.filter(|range| {
        range.start <= range.end
            && range.end <= text.len()
            && text.is_char_boundary(range.start)
            && text.is_char_boundary(range.end)
    });
    if let Some(range) = selected {
        crate::app::file_style::append_text(
            &mut job,
            &text[..range.start],
            &DirectoryEntryState::Same,
            &[],
            palette,
            font.clone(),
            height,
        );
        crate::app::file_style::append_text(
            &mut job,
            &text[range.clone()],
            &DirectoryEntryState::Different,
            &[0..range.len()],
            palette,
            font.clone(),
            height,
        );
        crate::app::file_style::append_text(
            &mut job,
            &text[range.end..],
            &DirectoryEntryState::Same,
            &[],
            palette,
            font,
            height,
        );
    } else {
        crate::app::file_style::append_text(
            &mut job,
            text,
            &DirectoryEntryState::Same,
            &[],
            palette,
            font,
            height,
        );
    }
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}
