//! Read-only aligned file painting, scrolling and comparison controls.

use super::*;
use crate::app::visuals::icons::{ToolbarIcon, icon_button};
use crate::app::visuals::panels::{comparison_panel, empty_display, file_path_headers};
use crate::app::visuals::status::{
    file_line_accessible_label, paint_status_icon, side_name, state_color,
};
use crate::app::visuals::theme::{Palette, comparison_font};

impl VersusApp {
    pub(super) fn file_controls(&mut self, ui: &mut egui::Ui) {
        let ready = self.session.source_jobs.iter().all(Option::is_none)
            && self.session.source_errors.iter().all(Option::is_none)
            && self.session.file_view.as_ref().is_some_and(|view| {
                view.job.is_none()
                    && (view.from_folders
                        || self.session.source_kinds == [Some(SourceKind::File); 2])
            });
        if icon_button(ui, ToolbarIcon::Refresh, ready, "Refresh comparison").clicked() {
            self.session.refresh_file();
        }
        if let Some(job) = self
            .session
            .file_view
            .as_ref()
            .and_then(|view| view.job.as_ref())
        {
            let cancelling = job.cancellation.load(Ordering::Relaxed);
            if icon_button(
                ui,
                ToolbarIcon::Cancel,
                !cancelling,
                if cancelling {
                    "Cancelling…"
                } else {
                    "Cancel comparison"
                },
            )
            .clicked()
            {
                job.cancellation.store(true, Ordering::Relaxed);
            }
        }
    }

    pub(super) fn file_area(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let row_height = self.session.row_height;
        let mut focused_file_side = self.session.file_keyboard_focus_side;
        let from_folders = self.session.file_view.as_ref().unwrap().from_folders;
        comparison_panel(ui).show(ui, |ui| {
                ui.set_min_height((ui.available_height() - 2.0).max(0.0));
                if !from_folders {
                    self.folder_inputs(ui);
                }
                let Some(view) = self.session.file_view.as_mut() else {
                    ui.label("Opening selected source…");
                    return;
                };
                if from_folders {
                    file_path_headers(ui, &view.paths);
                }
                let (divider, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter().line_segment(
                    [divider.left_center(), divider.right_center()],
                    Stroke::new(1.0, palette.border),
                );
                if from_folders && view.sources.iter().any(Option::is_none) {
                    ui.columns(2, |columns| {
                        for (side, column) in columns.iter_mut().enumerate() {
                            if view.sources[side].is_none() {
                                column.label(
                                    RichText::new("Not present on this side").color(palette.muted),
                                );
                            }
                        }
                    });
                }
                if let Some(error) = &view.error
                    && view.comparison.is_none()
                {
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(14.0, 14.0), Sense::hover());
                        paint_status_icon(ui.painter(), rect.center(), view.error_icon);
                        ui.colored_label(palette.error, error);
                    });
                    return;
                }
                let Some(comparison) = &view.comparison else {
                    if view.job.is_some() {
                        ui.horizontal(|ui| { ui.spinner(); ui.label("Loading file comparison…"); });
                    } else {
                        empty_display(ui, (ui.available_height() - 2.0).max(0.0), "Choose files", "Use Browse above to select each file.");
                    }
                    return;
                };
                if let Some(message) = &comparison.message {
                    ui.label(RichText::new(message).color(palette.muted));
                }
                if comparison.rows.is_empty() {
                    if comparison.message.is_none() {
                        empty_display(ui, (ui.available_height() - 2.0).max(0.0), "Both files are empty", "There are no lines to compare.");
                    }
                    return;
                }
                if view.visible_rows.is_empty() {
                    empty_display(ui, (ui.available_height() - 2.0).max(0.0), "No differences", "No differences with the current options.");
                    return;
                }
                let height = (ui.available_height() - 2.0).max(0.0);
                let previous_y = view.scroll_y;
                let mut next_y = previous_y;
                // Process the pane under the pointer first, then paint both panes
                // using the resulting shared offset (scroll events apply during layout).
                let active_side =
                    usize::from(ui.input(|input| input.pointer.hover_pos()).is_some_and(
                        |position| position.x > ui.available_rect_before_wrap().center().x,
                    ));
                let mut viewports = [Rect::NOTHING; 2];
                let mut horizontal_offsets = [0.0; 2];
                ui.columns(2, |columns| {
                    for side in [active_side, 1 - active_side] {
                        let column = &mut columns[side];
                        column.spacing_mut().item_spacing = Vec2::ZERO;
                        let output = egui::ScrollArea::both()
                            .id_salt(("file-lines", &view.paths, side))
                            .scroll_bar_visibility(
                                egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                            )
                            .auto_shrink([false, false])
                            .max_height(height)
                            .vertical_scroll_offset(next_y)
                            .show_viewport(column, |ui, _| {
                                ui.set_min_size(egui::vec2(
                                    ui.available_width().max(view.content_widths[side]),
                                    row_height * view.visible_rows.len() as f32,
                                ));
                            });
                        next_y = output.state.offset.y;
                        viewports[side] = output.inner_rect;
                        horizontal_offsets[side] = output.state.offset.x;
                    }
                });
                for side in 0..2 {
                    let viewport = viewports[side];
                    let painter = ui.painter().with_clip_rect(viewport);
                    let first = (next_y / row_height).floor() as usize;
                    let last = ((next_y + viewport.height()) / row_height).ceil() as usize;
                    for index in first..last.min(view.visible_rows.len()) {
                        let original_index = view.visible_rows[index];
                        let row = &comparison.rows[original_index];
                        let line = if side == 0 { &row.left } else { &row.right };
                        let rect = Rect::from_min_size(
                            egui::pos2(
                                viewport.left() - horizontal_offsets[side],
                                viewport.top() + index as f32 * row_height - next_y,
                            ),
                            egui::vec2(
                                viewport.width().max(view.content_widths[side]),
                                row_height,
                            ),
                        );
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
                        let ending_diff =
                            !view.ignore_line_endings && row.left_ending != row.right_ending;
                        paint_file_line(
                            &painter,
                            rect,
                            line.as_ref(),
                            &row.state,
                            index % 2 == 1,
                            changed,
                            ending_diff.then_some(ending).flatten(),
                            view.navigation_row == Some(original_index),
                            view.gutter_extra,
                        );
                        let line_response = ui.interact(
                            rect.intersect(viewport),
                            ui.id().with(("file-line", &view.paths, side, original_index)),
                            Sense::hover(),
                        );
                        line_response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Label,
                                true,
                                file_line_accessible_label(side, line.as_ref(), &row.state),
                            )
                        });
                    }
                }
                let mut keyboard_scroll = 0.0;
                for side in 0..2 {
                    let pane_response = ui.interact(
                        viewports[side],
                        ui.id().with(("file-pane", &view.paths, side)),
                        Sense::click(),
                    );
                    pane_response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Panel,
                            true,
                            format!(
                                "{} file comparison pane: {}. Use Up and Down Arrow to scroll lines, Page Up or Page Down for pages, Home or End for the first or last line.",
                                side_name(side), view.paths[side].display()
                            ),
                        )
                    });
                    if pane_response.clicked() {
                        pane_response.request_focus();
                        focused_file_side = Some(side);
                    }
                    if pane_response.has_focus() {
                        ui.memory_mut(|memory| memory.set_focus_lock_filter(pane_response.id, egui::EventFilter {
                            horizontal_arrows: true, vertical_arrows: true, ..Default::default()
                        }));
                        focused_file_side = Some(side);
                        keyboard_scroll = ui.input(|input| {
                            if input.key_pressed(egui::Key::ArrowDown) {
                                row_height
                            } else if input.key_pressed(egui::Key::ArrowUp) {
                                -row_height
                            } else if input.key_pressed(egui::Key::PageDown) {
                                viewports[side].height()
                            } else if input.key_pressed(egui::Key::PageUp) {
                                -viewports[side].height()
                            } else if input.key_pressed(egui::Key::Home) {
                                -next_y
                            } else if input.key_pressed(egui::Key::End) {
                                (view.visible_rows.len() as f32 * row_height) - next_y
                            } else {
                                0.0
                            }
                        });
                        ui.painter().rect_stroke(
                            viewports[side].shrink(1.0),
                            4.0,
                            Stroke::new(2.0, palette.accent),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
                if keyboard_scroll != 0.0 {
                    next_y = (next_y + keyboard_scroll).clamp(
                        0.0,
                        (view.visible_rows.len() as f32 * row_height
                            - viewports[0].height())
                            .max(0.0),
                    );
                    view.navigation_row = None;
                    ui.ctx().request_repaint();
                }
                if (next_y - previous_y).abs() > 0.1 {
                    view.scroll_y = next_y;
                    // Scrolling by hand starts navigation from the new viewport.
                    if !view.navigation_scroll_pending {
                        view.navigation_row = None;
                    }
                    ui.ctx().request_repaint();
                }
                view.navigation_scroll_pending = false;
                self.session.file_keyboard_focus_side = focused_file_side;
            });
    }
}
pub(in crate::app) fn paint_file_line(
    painter: &egui::Painter,
    rect: Rect,
    line: Option<&(usize, String)>,
    state: &DirectoryEntryState,
    alternate: bool,
    changed: &[std::ops::Range<usize>],
    ending: Option<versus::DisplayLineEnding>,
    selected: bool,
    gutter_extra: f32,
) {
    let palette = Palette::for_context(painter.ctx());
    let painter = painter.with_clip_rect(rect);
    let color = state_color(state, palette);
    file_style::paint_background(&painter, rect, state, alternate, selected);
    let Some((number, text)) = line else {
        painter.text(
            rect.left_center() + egui::vec2(file_style::TEXT_OFFSET - 2.0 + gutter_extra, 0.0),
            Align2::LEFT_CENTER,
            "—",
            comparison_font(painter.ctx()),
            palette.muted,
        );
        return;
    };
    file_style::paint_gutter(&painter, rect, *number, state, gutter_extra);
    let mut job = egui::text::LayoutJob::default();
    file_style::append_text(
        &mut job,
        text,
        state,
        changed,
        palette,
        comparison_font(painter.ctx()),
        None,
    );
    let galley = painter.layout_job(job);
    let position = egui::pos2(
        rect.left() + file_style::TEXT_OFFSET + gutter_extra,
        rect.center().y - galley.size().y / 2.0,
    );
    let end_x = position.x + galley.size().x;
    painter.galley(position, galley, color);
    if let Some(ending) = ending {
        file_style::paint_ending(&painter, rect, end_x, ending, state);
    }
}
