//! Synchronized vertical navigation and independent horizontal file scrolling.

use super::*;
use crate::app::file_diff::paint_file_line;
use crate::app::visuals::theme::comparison_font;

impl FileScreen {
    pub(super) fn paint_rows(
        &mut self,
        ui: &mut egui::Ui,
        visible: &[usize],
        gutter_extra: f32,
        ignore_endings: bool,
        row_height: f32,
    ) {
        let columns = *self
            .content_columns
            .get_or_insert_with(|| content_columns(&self.comparison));
        let widths = columns.map(|count| {
            file_style::TEXT_OFFSET
                + gutter_extra
                + 90.0
                + count as f32 * comparison_font(ui.ctx()).size
        });
        let height = ui.available_height().max(0.0);
        let active_side = usize::from(
            ui.input(|input| input.pointer.hover_pos())
                .is_some_and(|point| point.x > ui.available_rect_before_wrap().center().x),
        );
        let mut scroll_y = self.scroll_y;
        let mut viewports = [egui::Rect::NOTHING; 2];
        let mut horizontal = [0.0; 2];
        ui.columns(2, |columns| {
            for side in [active_side, 1 - active_side] {
                let output = egui::ScrollArea::both()
                    .id_salt(("git-diff-lines", &self.paths, side))
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .max_height(height)
                    .vertical_scroll_offset(scroll_y)
                    .show_viewport(&mut columns[side], |ui, _| {
                        ui.set_min_size(egui::vec2(
                            ui.available_width().max(widths[side]),
                            row_height * visible.len() as f32,
                        ));
                    });
                scroll_y = output.state.offset.y;
                viewports[side] = output.inner_rect;
                horizontal[side] = output.state.offset.x;
            }
        });
        for side in 0..2 {
            let viewport = viewports[side];
            let painter = ui.painter().with_clip_rect(viewport);
            let first = (scroll_y / row_height).floor() as usize;
            let last = ((scroll_y + viewport.height()) / row_height).ceil() as usize;
            for position in first..last.min(visible.len()) {
                let row_index = visible[position];
                let row = &self.comparison.rows[row_index];
                let line = if side == 0 { &row.left } else { &row.right };
                let changed = if side == 0 {
                    &row.left_changed
                } else {
                    &row.right_changed
                };
                let ending = if side == 0 {
                    row.left_ending
                } else {
                    row.right_ending
                };
                let ending_diff = !ignore_endings && row.left_ending != row.right_ending;
                let rect = egui::Rect::from_min_size(
                    egui::pos2(
                        viewport.left() - horizontal[side],
                        viewport.top() + position as f32 * row_height - scroll_y,
                    ),
                    egui::vec2(viewport.width().max(widths[side]), row_height),
                );
                paint_file_line(
                    &painter,
                    rect,
                    line.as_ref(),
                    &row.state,
                    position % 2 == 1,
                    changed,
                    ending_diff.then_some(ending).flatten(),
                    self.navigation == Some(row_index),
                    gutter_extra,
                );
            }
        }
        self.scroll_y = scroll_y;
    }
}

pub(in crate::app::git::diff) fn content_columns(comparison: &FileComparison) -> [usize; 2] {
    let mut columns = [0; 2];
    for row in &comparison.rows {
        for (side, line) in [&row.left, &row.right].into_iter().enumerate() {
            if let Some((_, text)) = line {
                columns[side] = columns[side].max(text.len() + text.matches('\t').count() * 3);
            }
        }
    }
    columns
}
