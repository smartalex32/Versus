//! Read-only File Diff-style pane presentation and painting.

use super::*;
use crate::app::visuals::status::{file_line_accessible_label, side_name};
use crate::app::visuals::theme::{Palette, comparison_font, comparison_row_height};

impl InteractiveView {
    pub(in crate::app) fn render_side(&mut self, ui: &mut egui::Ui, side: usize, height: f32) {
        debug_assert!(side < 2);
        if self.side_sources[side].is_none() && self.paths.is_none() {
            let rect = ui.available_rect_before_wrap();
            paint_empty_drop_target(ui, rect, side, true);
            return;
        }
        if self.side_loading[side].is_some() {
            let rect = ui.available_rect_before_wrap();
            paint_empty_drop_target(ui, rect, side, true);
            return;
        }
        if self.workspace.is_none() || !self.side_has_file(side) {
            let rect = ui.available_rect_before_wrap();
            paint_empty_drop_target(ui, rect, side, true);
            return;
        }
        if !self.editing {
            self.readonly_editor(ui, side, height);
            return;
        }
        let pane = ui.available_rect_before_wrap();
        let track = self.has_actual_pair().then(|| {
            let width = 38.0;
            let rect = if side == 0 {
                Rect::from_min_max(egui::pos2(pane.right() - width, pane.top()), pane.max)
            } else {
                Rect::from_min_max(pane.min, egui::pos2(pane.left() + width, pane.bottom()))
            };
            let palette = Palette::for_context(ui.ctx());
            ui.painter().rect_filled(rect, 0.0, palette.background);
            for x in [rect.left(), rect.right()] {
                ui.painter().line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    Stroke::new(1.0, palette.border),
                );
            }
            ui.interact(
                rect,
                egui::Id::new(("merge-transfer-track", side)),
                Sense::hover(),
            );
            rect
        });
        let content = track.map_or(pane, |track| {
            if side == 0 {
                Rect::from_min_max(pane.min, egui::pos2(track.left(), pane.bottom()))
            } else {
                Rect::from_min_max(egui::pos2(track.right(), pane.top()), pane.max)
            }
        });
        let pane_clip = ui.clip_rect();
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
            ui.set_clip_rect(content.intersect(pane_clip));
            self.editor(
                ui,
                side,
                height,
                track.map(|rect| rect.intersect(pane_clip)),
            );
        });
    }
    pub(super) fn ensure_readonly_presentation(&mut self, ui: &egui::Ui) {
        let font = comparison_font(ui.ctx());
        let stale = self
            .readonly_presentation
            .as_ref()
            .is_none_or(|cached| cached.preferences != self.diff_options || cached.font != font);
        if !stale {
            return;
        }
        let previous = self.readonly_presentation.take();
        let row_height = comparison_row_height(ui);
        let previous_fraction = self.scroll[0] % row_height;
        let previous_anchor = previous.as_ref().and_then(|cached| {
            cached
                .rows
                .get((self.scroll[0] / row_height).floor() as usize)
                .copied()
        });
        let Some(diff) = self.diff.as_ref() else {
            return;
        };
        let mut rows = Vec::with_capacity(diff.rows.len());
        // Cache only intrinsic line widths. Each pane applies its own viewport
        // minimum when it renders, so resizing cannot leave a full-body width
        // stuck on a short file.
        let mut content_widths = [0.0_f32; 2];
        let mut max_line_number = 1;
        for (index, row) in diff.rows.iter().enumerate() {
            if !self.diff_options[0] || row.state != DirectoryEntryState::Same {
                rows.push(index);
            }
            for (side, line) in [&row.left, &row.right].into_iter().enumerate() {
                if let Some((number, text)) = line {
                    max_line_number = max_line_number.max(*number);
                    // This is deliberately the same conservative width budget
                    // as native File Diff: tabs occupy four character slots.
                    let expanded =
                        text.len() + text.bytes().filter(|byte| *byte == b'\t').count() * 3;
                    content_widths[side] = content_widths[side]
                        .max(super::file_style::TEXT_OFFSET + 28.0 + expanded as f32 * font.size);
                }
            }
        }
        let gutter_extra = super::file_style::gutter_extra(max_line_number);
        for width in &mut content_widths {
            if *width > 0.0 {
                *width += gutter_extra;
            }
        }
        self.readonly_presentation = Some(ReadonlyPresentation {
            rows,
            content_widths,
            preferences: self.diff_options,
            font,
            row_height,
            gutter_extra,
        });
        if let Some((anchor, fraction)) = self.readonly_pending_anchor.take() {
            let rows = &self.readonly_presentation.as_ref().unwrap().rows;
            let top = rows
                .iter()
                .position(|index| {
                    let row = &diff.rows[*index];
                    [
                        row.left.as_ref().map(|line| line.0),
                        row.right.as_ref().map(|line| line.0),
                    ]
                    .iter()
                    .enumerate()
                    .any(|(side, number)| number.is_some() && *number == anchor[side])
                })
                .unwrap_or_else(|| rows.len().saturating_sub(1));
            self.scroll = [top as f32 * row_height + fraction; 2];
        } else if let Some(anchor) = previous_anchor {
            let rows = &self.readonly_presentation.as_ref().unwrap().rows;
            let top = rows
                .iter()
                .position(|row| *row >= anchor)
                .unwrap_or_else(|| rows.len().saturating_sub(1));
            self.scroll = [top as f32 * row_height + previous_fraction; 2];
        }
    }
    pub(super) fn readonly_display_index(&self, row_index: usize) -> Option<usize> {
        self.readonly_presentation
            .as_ref()
            .and_then(|cached| cached.rows.iter().position(|index| *index == row_index))
            .or_else(|| {
                if self.diff_options[0] {
                    self.diff_rows.iter().position(|index| *index == row_index)
                } else {
                    self.diff
                        .as_ref()
                        .and_then(|diff| (row_index < diff.rows.len()).then_some(row_index))
                }
            })
    }
    /// Read-only Diff mode deliberately bypasses TextEdit.  It uses the same
    /// aligned row model and row primitives as File Diff, while retained merge
    /// documents remain untouched for a later switch back to Merge.
    pub(super) fn readonly_editor(&mut self, ui: &mut egui::Ui, side: usize, height: f32) {
        if self.diff.is_none() {
            if self.diff_job.is_some() || self.diff_dirty {
                ui.centered_and_justified(|ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading file comparison…");
                    });
                });
            }
            return;
        }
        self.ensure_readonly_presentation(ui);
        let Some(diff) = self.diff.as_ref() else {
            return;
        };
        if let Some(message) = &diff.message {
            ui.label(RichText::new(message).color(Palette::for_context(ui.ctx()).muted));
            return;
        }
        let (visible_len, content_width) = self
            .readonly_presentation
            .as_ref()
            .map(|cached| (cached.rows.len(), cached.content_widths[side]))
            .unwrap_or_default();
        if visible_len == 0 {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("No differences with the current options.")
                        .color(Palette::for_context(ui.ctx()).muted),
                );
            });
            return;
        }
        let row_height = comparison_row_height(ui);
        let palette = Palette::for_context(ui.ctx());
        let output = egui::ScrollArea::both()
            .id_salt(("two-way-readonly", self.side_generation[side], side))
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .auto_shrink([false, false])
            .max_height(height)
            .vertical_scroll_offset(self.scroll[side])
            .show_viewport(ui, |ui, _| {
                ui.set_min_size(egui::vec2(
                    ui.available_width().max(content_width),
                    row_height * visible_len as f32,
                ));
            });
        // File Diff keeps aligned rows together.  Merge's editable renderer
        // below intentionally retains its independent per-side offsets.
        self.scroll = [output.state.offset.y; 2];
        let viewport = output.inner_rect;
        let painter = ui.painter().with_clip_rect(viewport);
        let first = (self.scroll[side] / row_height).floor() as usize;
        let last = ((self.scroll[side] + viewport.height()) / row_height).ceil() as usize;
        let navigation = ui
            .ctx()
            .memory(|memory| {
                memory
                    .data
                    .get_temp::<usize>(egui::Id::new("interactive-diff-navigation"))
            })
            .and_then(|index| self.diff_rows.get(index).copied());
        for display_index in first..last.min(visible_len) {
            let row_index = self.readonly_presentation.as_ref().unwrap().rows[display_index];
            let row = &diff.rows[row_index];
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
            let ending_diff = !self.diff_options[2] && row.left_ending != row.right_ending;
            let rect = Rect::from_min_size(
                egui::pos2(
                    viewport.left() - output.state.offset.x,
                    viewport.top() + display_index as f32 * row_height - self.scroll[side],
                ),
                egui::vec2(viewport.width().max(content_width), row_height),
            );
            super::paint_file_line(
                &painter,
                rect,
                if side == 0 {
                    row.left.as_ref()
                } else {
                    row.right.as_ref()
                },
                &row.state,
                display_index % 2 == 1,
                changed,
                ending_diff.then_some(ending).flatten(),
                navigation == Some(row_index),
                self.readonly_presentation.as_ref().unwrap().gutter_extra,
            );
            let line = if side == 0 { &row.left } else { &row.right };
            ui.interact(
                rect.intersect(viewport),
                ui.id().with(("interactive-readonly-line", side, row_index)),
                Sense::hover(),
            )
            .widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Label,
                    true,
                    file_line_accessible_label(side, line.as_ref(), &row.state),
                )
            });
        }
        let pane = ui.interact(
            viewport,
            egui::Id::new((
                "interactive-readonly-pane",
                self.side_generation[side],
                side,
            )),
            Sense::click(),
        );
        pane.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Panel,
                true,
                format!(
                    "{} file comparison pane. Use Up and Down Arrow to scroll lines, Page Up or Page Down for pages, Home or End for the first or last line.",
                    side_name(side)
                ),
            )
        });
        if pane.clicked() {
            pane.request_focus();
        }
        if pane.has_focus() {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    pane.id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        ..Default::default()
                    },
                )
            });
            let delta = ui.input(|input| {
                if input.key_pressed(egui::Key::ArrowDown) {
                    row_height
                } else if input.key_pressed(egui::Key::ArrowUp) {
                    -row_height
                } else if input.key_pressed(egui::Key::PageDown) {
                    viewport.height()
                } else if input.key_pressed(egui::Key::PageUp) {
                    -viewport.height()
                } else if input.key_pressed(egui::Key::Home) {
                    -self.scroll[side]
                } else if input.key_pressed(egui::Key::End) {
                    row_height * visible_len as f32 - self.scroll[side]
                } else {
                    0.0
                }
            });
            if delta != 0.0 {
                let next = (self.scroll[side] + delta).clamp(
                    0.0,
                    (row_height * visible_len as f32 - viewport.height()).max(0.0),
                );
                self.scroll = [next; 2];
                ui.ctx().request_repaint();
            }
            ui.painter().rect_stroke(
                viewport.shrink(1.0),
                4.0,
                Stroke::new(2.0, palette.accent),
                egui::StrokeKind::Inside,
            );
        }
    }
}
