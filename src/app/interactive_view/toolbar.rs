//! Workspace toolbar, footer orchestration, and difference navigation.

use super::*;
use crate::app::visuals::footer::{
    header_progress_label, header_status_label, workspace_footer_height,
};
use crate::app::visuals::icons::{ToolbarIcon, icon_button, icon_button_state};
use crate::app::visuals::panels::{comparison_panel, empty_display, split_rect};
use crate::app::visuals::theme::{
    Palette, ROW_HEIGHT, comparison_font, control_extent, zoom_controls,
};

impl InteractiveView {
    pub(in crate::app) fn render_toolbar(&mut self, ui: &mut egui::Ui) {
        self.render_workspace_toolbar(ui, true);
    }
    pub(in crate::app) fn render_workspace_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        folder_overview: bool,
    ) {
        self.folder_overview = folder_overview;
        self.poll(ui.ctx());
        self.shortcuts(ui);
        self.toolbar(ui);
    }
    pub(in crate::app) fn change_label(&self) -> String {
        if self.folder_overview {
            let count = self.folder_differences.len();
            return self
                .folder_difference_index
                .map(|index| format!("Change {} of {count}", index + 1))
                .unwrap_or_else(|| format!("{count} changes"));
        }
        if let Some(changes) = self.presentation.pending_changes() {
            return format!("{changes} changes");
        }
        let current = self.editor_context.as_ref().and_then(|ctx| {
            ctx.memory(|memory| {
                memory
                    .data
                    .get_temp::<usize>(egui::Id::new("interactive-diff-navigation"))
            })
        });
        current
            .map(|index| format!("Change {} of {}", index + 1, self.diff_rows.len()))
            .unwrap_or_else(|| format!("{} changes", self.diff_rows.len()))
    }
    pub(in crate::app) fn render_activity_status(&mut self, ui: &mut egui::Ui) -> bool {
        let palette = Palette::for_context(ui.ctx());
        if let Some(job) = &self.diff_job {
            if ui.available_width() > 2.0 * control_extent(ui) {
                ui.spinner();
            }
            let cancel_width = control_extent(ui) + ui.spacing().item_spacing.x;
            ui.allocate_ui_with_layout(
                egui::vec2(
                    (ui.available_width() - cancel_width).max(0.0),
                    control_extent(ui),
                ),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| header_progress_label(ui, job.progress.snapshot(), palette.muted),
            );
            if icon_button(ui, ToolbarIcon::Cancel, true, "Cancel file comparison").clicked() {
                self.run_diff = false;
                self.invalidate_diff();
            }
            return true;
        }
        if self.save_job.is_some() {
            ui.spinner();
            header_status_label(ui, "Saving…", palette.muted);
            return true;
        }
        if self.loading.is_some() || self.side_loading.iter().any(Option::is_some) {
            header_status_label(ui, "Loading selected files…", palette.muted);
            ui.ctx().request_repaint_after(Duration::from_millis(25));
            return true;
        }
        false
    }
    pub(in crate::app) fn render_footer(&mut self, ui: &mut egui::Ui) -> Rect {
        self.footer(ui)
    }
    pub(super) fn guidance_message(&self) -> &str {
        if !self.has_actual_pair() {
            return if self.side_has_file(0) || self.side_has_file(1) {
                "Choose a file on the other side to begin."
            } else {
                "Choose a file on each side to begin."
            };
        }
        if !self.status.is_empty()
            && !self.status.starts_with("Loading")
            && !self.status.starts_with("Saving")
        {
            &self.status
        } else if self.editing {
            "Use chunk or line arrows to transfer content."
        } else {
            "Inspect differences between the open files."
        }
    }
    pub(in crate::app) fn footer_height(&self, ui: &egui::Ui) -> f32 {
        workspace_footer_height(
            ui,
            self.editing,
            self.any_dirty(),
            &self.footer_legend_icons(),
            self.footer_legend_counts(),
        )
    }
    pub(in crate::app) fn render_previews(&mut self, ctx: &egui::Context) {
        self.save_preview(ctx);
    }
    pub(in crate::app) fn set_folder_differences(
        &mut self,
        differences: impl Into<Arc<Vec<PathBuf>>>,
    ) {
        let differences = differences.into();
        if Arc::ptr_eq(&self.folder_differences, &differences) {
            return;
        }
        if self.folder_differences != differences {
            self.folder_difference_index = None;
        }
        self.folder_differences = differences;
    }
    pub(in crate::app) fn take_folder_jump(&mut self) -> Option<PathBuf> {
        self.folder_jump.take()
    }
    pub(in crate::app) fn render_with_sources(
        &mut self,
        ui: &mut egui::Ui,
        sources: impl FnOnce(&mut egui::Ui),
    ) -> ([Rect; 2], Rect) {
        self.folder_overview = false;
        self.render_workspace_toolbar(ui, false);
        ui.add_space(5.);
        let area = ui.available_rect_before_wrap();
        let footer_height = self.footer_height(ui);
        let foot = footer_height + 8.;
        let body = Rect::from_min_max(
            area.min,
            egui::pos2(area.right(), (area.bottom() - foot).max(area.top())),
        );
        let footer = Rect::from_min_size(
            egui::pos2(area.left(), body.bottom() + 4.),
            egui::vec2(area.width(), footer_height),
        );
        let mut panes = split_rect(body);
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            comparison_panel(ui).show(ui, |ui| {
                ui.set_min_height((ui.available_height() - 2.0).max(0.0));
                sources(ui);
                panes = split_rect(ui.available_rect_before_wrap());
                self.panes(ui);
            });
        });
        let status = ui
            .scope_builder(egui::UiBuilder::new().max_rect(footer), |ui| {
                self.render_footer(ui)
            })
            .inner;
        self.render_previews(ui.ctx());
        (panes, status)
    }
    pub(super) fn toolbar(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let has_pair = self.has_actual_pair();
        let extent = control_extent(ui);
        let width = ui.available_width();
        let gap = ui.spacing().item_spacing.x;
        let compact = width < 700.0;
        let height = if compact {
            3.0 * extent + 2.0 * gap
        } else {
            extent
        };
        let (center, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
        let gap = ui.spacing().item_spacing.x;
        let side_width = if compact {
            width
        } else {
            (width / 2.0 - (4.0 * extent + 3.0 * gap) / 2.0 - gap).max(110.0)
        };
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    center.min,
                    egui::pos2(center.left() + side_width, center.top() + extent),
                ))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                if !self.editing
                    && self.from_folders
                    && icon_button(ui, ToolbarIcon::Back, true, "Back to folders").clicked()
                {
                    self.back = true;
                }
                if icon_button(ui, ToolbarIcon::New, true, "New comparison").clicked() {
                    self.new_requested = true;
                }
                if self.editing
                    && icon_button(ui, ToolbarIcon::Search, true, "Find and replace").clicked()
                {
                    self.search_requested = true;
                }
                if self.engine_enabled() {
                    ui.label(
                        RichText::new(self.change_label())
                            .size(11.0)
                            .color(palette.muted),
                    );
                }
            },
        );
        let actions_width = 4.0 * extent + 3.0 * gap;
        let actions = Rect::from_center_size(
            egui::pos2(
                center.center().x,
                if compact {
                    center.top() + extent + gap + extent / 2.0
                } else {
                    center.center().y
                },
            ),
            egui::vec2(actions_width, extent),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(actions)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                if self.editing {
                    if icon_button(
                        ui,
                        ToolbarIcon::AcceptAllLeft,
                        has_pair,
                        "Accept All Left — replace Right with Left",
                    )
                    .clicked()
                    {
                        self.copy_all(TwoWaySide::Left);
                    }
                    if icon_button(
                        ui,
                        ToolbarIcon::AcceptAllRight,
                        has_pair,
                        "Accept All Right — replace Left with Right",
                    )
                    .clicked()
                    {
                        self.copy_all(TwoWaySide::Right);
                    }
                    if icon_button(
                        ui,
                        ToolbarIcon::Undo,
                        self.workspace
                            .as_ref()
                            .is_some_and(TwoWayWorkspace::can_undo),
                        "Undo",
                    )
                    .clicked()
                    {
                        self.move_history(false);
                    }
                    if icon_button(
                        ui,
                        ToolbarIcon::Redo,
                        self.workspace
                            .as_ref()
                            .is_some_and(TwoWayWorkspace::can_redo),
                        "Redo",
                    )
                    .clicked()
                    {
                        self.move_history(true);
                    }
                }
            },
        );
        let zoom_width = ui
            .painter()
            .layout_no_wrap(
                format!("Zoom {:.0}%", ui.ctx().zoom_factor() * 100.0),
                egui::TextStyle::Button.resolve(ui.style()),
                palette.muted,
            )
            .size()
            .x
            + 2.0 * ui.spacing().button_padding.x
            + 2.0 * (extent + gap);
        let right_width = zoom_width
            + if self.engine_enabled() {
                5.0 * (extent + gap)
            } else {
                0.0
            };
        let right = Rect::from_min_max(
            egui::pos2(
                if compact {
                    (center.right() - right_width).max(center.left())
                } else {
                    (center.right() - right_width).max(actions.right() + gap)
                },
                if compact {
                    center.top() + 2.0 * (extent + gap)
                } else {
                    center.top()
                },
            ),
            center.max,
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(right)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                zoom_controls(ui);
                if !self.engine_enabled() {
                    return;
                }
                let previous = if self.editing {
                    self.merge_options
                } else {
                    self.diff_options
                };
                let mut prefs = previous;
                for (index, icon, label) in [
                    (0, ToolbarIcon::Differences, "Show only differences"),
                    (1, ToolbarIcon::Whitespace, "Ignore whitespace"),
                    (2, ToolbarIcon::LineEndings, "Ignore line endings"),
                ] {
                    if icon_button_state(ui, icon, true, prefs[index], label).clicked() {
                        prefs[index] = !prefs[index];
                    }
                }
                if icon_button(
                    ui,
                    ToolbarIcon::Previous,
                    self.has_differences(),
                    "Previous difference",
                )
                .clicked()
                {
                    self.jump_diff(false, ui.ctx());
                }
                if icon_button(
                    ui,
                    ToolbarIcon::Next,
                    self.has_differences(),
                    "Next difference",
                )
                .clicked()
                {
                    self.jump_diff(true, ui.ctx());
                }
                if prefs != previous {
                    if self.editing {
                        self.merge_options = prefs;
                    } else {
                        self.diff_options = prefs;
                    }
                    if prefs[1..] != previous[1..] {
                        self.invalidate_diff();
                    }
                }
            },
        );
    }
    pub(super) fn jump_diff(&mut self, forward: bool, ctx: &egui::Context) {
        if self.folder_overview {
            let count = self.folder_differences.len();
            if count == 0 {
                return;
            }
            let index = match self.folder_difference_index {
                None => {
                    if forward {
                        0
                    } else {
                        count - 1
                    }
                }
                Some(index) => {
                    if forward {
                        (index + 1).min(count - 1)
                    } else {
                        index.saturating_sub(1)
                    }
                }
            };
            self.folder_difference_index = Some(index);
            self.folder_jump = Some(self.folder_differences[index].clone());
            ctx.request_repaint();
            return;
        }
        let Some(diff) = &self.diff else {
            return;
        };
        if self.diff_rows.is_empty() {
            return;
        }
        let id = egui::Id::new("interactive-diff-navigation");
        let current = ctx.memory(|m| m.data.get_temp::<usize>(id));
        let next = match current {
            None => {
                if forward {
                    0
                } else {
                    self.diff_rows.len() - 1
                }
            }
            Some(index) => {
                if forward {
                    (index + 1) % self.diff_rows.len()
                } else {
                    (index + self.diff_rows.len() - 1) % self.diff_rows.len()
                }
            }
        };
        ctx.memory_mut(|m| m.data.insert_temp(id, next));
        let row_index = self.diff_rows[next];
        let row = &diff.rows[row_index];
        let font = comparison_font(ctx);
        let row_height =
            ROW_HEIGHT.max(ctx.fonts_mut(|fonts| fonts.row_height(&font)).ceil() + 8.0);
        if self.editing {
            // Only move panes present on this comparison row. Matching rows
            // can have different source numbers, and the differences-only
            // projection has its own compact row positions on each side.
            for (side, line) in [&row.left, &row.right].into_iter().enumerate() {
                if let Some((number, _)) = line {
                    let display_line = if self.run_diff && self.merge_options[0] {
                        diff.rows[..row_index]
                            .iter()
                            .filter(|row| {
                                row.state != DirectoryEntryState::Same
                                    && if side == 0 {
                                        row.left.is_some()
                                    } else {
                                        row.right.is_some()
                                    }
                            })
                            .count()
                    } else {
                        number.saturating_sub(1)
                    };
                    self.scroll[side] = display_line.saturating_sub(1) as f32 * row_height;
                    self.pending_navigation_scroll[side] = true;
                }
            }
        } else if let Some(display_index) = self.readonly_display_index(row_index) {
            // Diff panes are aligned rows, including when the differences-only
            // preference filters out equal rows.
            self.scroll = [display_index.saturating_sub(1) as f32 * row_height; 2];
        }
        ctx.request_repaint();
    }
    pub(super) fn navigation_lines(&self, ctx: &egui::Context) -> [Option<usize>; 2] {
        ctx.memory(|memory| {
            memory
                .data
                .get_temp::<usize>(egui::Id::new("interactive-diff-navigation"))
        })
        .and_then(|index| self.diff_rows.get(index))
        .and_then(|index| self.diff.as_ref()?.rows.get(*index))
        .map(|row| {
            [
                row.left.as_ref().map(|line| line.0.saturating_sub(1)),
                row.right.as_ref().map(|line| line.0.saturating_sub(1)),
            ]
        })
        .unwrap_or([None; 2])
    }
    pub(super) fn has_differences(&self) -> bool {
        self.engine_enabled()
            && if self.folder_overview {
                !self.folder_differences.is_empty()
            } else {
                !self.diff_rows.is_empty()
            }
    }
    pub(in crate::app) fn diff_preferences(&self) -> [bool; 3] {
        self.diff_options
    }
    pub(super) fn panes(&mut self, ui: &mut egui::Ui) {
        let height = ui.available_height().max(1.);
        if !self.editing && self.diff.is_some() {
            self.ensure_readonly_presentation(ui);
            let diff = self.diff.as_ref().unwrap();
            if let Some(message) = &diff.message {
                ui.label(RichText::new(message).color(Palette::for_context(ui.ctx()).muted));
                return;
            }
            if diff.rows.is_empty() {
                empty_display(
                    ui,
                    height,
                    "Both files are empty",
                    "There are no lines to compare.",
                );
                return;
            }
            if self
                .readonly_presentation
                .as_ref()
                .is_some_and(|cached| cached.rows.is_empty())
            {
                empty_display(
                    ui,
                    height,
                    "No differences",
                    "No differences with the current options.",
                );
                return;
            }
        }
        let clip = ui.clip_rect().intersect(ui.available_rect_before_wrap());
        let active_side = usize::from(
            ui.input(|input| input.pointer.hover_pos())
                .is_some_and(|position| position.x > ui.available_rect_before_wrap().center().x),
        );
        ui.scope(|ui| {
            ui.set_clip_rect(clip);
            ui.columns(2, |columns| {
                // Process the hovered pane first.  Scroll input is consumed
                // during layout, so this makes a wheel event over the right
                // pane update the shared read-only alignment in this frame.
                for side in [active_side, 1 - active_side] {
                    self.render_side(&mut columns[side], side, height);
                }
            });
        });
    }
}
