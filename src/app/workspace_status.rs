//! Workspace guidance, activity, errors and footer presentation.

use super::*;
use crate::app::visuals::footer::{
    DIFF_LEGEND_ICONS, bottom_legend, bottom_status_area, error_summary, footer_guidance,
    header_progress_label, header_status_label, workspace_footer_layout,
};
use crate::app::visuals::icons::{ToolbarIcon, icon_button, icon_button_state};
use crate::app::visuals::panels::status_label;
use crate::app::visuals::theme::{Palette, control_extent, zoom_controls};

impl VersusApp {
    pub(super) fn progress_indicator(&self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let file_job = self
            .session
            .file_view
            .as_ref()
            .and_then(|view| view.job.as_ref());
        let job = file_job
            .map(|job| (&job.progress, &job.cancellation, "Cancel file comparison"))
            .or_else(|| {
                self.session
                    .job
                    .as_ref()
                    .map(|job| (&job.progress, &job.cancellation, "Cancel folder comparison"))
            });
        if let Some((progress, cancel, tooltip)) = job {
            let snapshot = progress.snapshot();
            ui.horizontal(|ui| {
                let extent = control_extent(ui);
                let reserve = extent + ui.spacing().item_spacing.x;
                if ui.available_width() > 2.0 * extent {
                    ui.spinner();
                }
                let width = (ui.available_width() - reserve).max(0.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(width, extent),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        header_progress_label(ui, snapshot, palette.text);
                    },
                );
                if icon_button(
                    ui,
                    ToolbarIcon::Cancel,
                    !cancel.load(Ordering::Relaxed),
                    tooltip,
                )
                .clicked()
                {
                    cancel.store(true, Ordering::Relaxed);
                }
            });
            ui.ctx().request_repaint_after(Duration::from_millis(60));
        } else if self.session.source_jobs.iter().any(Option::is_some) {
            header_status_label(ui, "Opening selected sources…", palette.text);
            ui.ctx().request_repaint_after(Duration::from_millis(60));
        }
    }

    pub(super) fn legend(&mut self, ui: &mut egui::Ui) {
        let back = self
            .session
            .file_view
            .as_ref()
            .is_some_and(|view| view.from_folders);
        let palette = Palette::for_context(ui.ctx());
        let available = ui.available_width();
        let gap = ui.spacing().item_spacing.x;
        let extent = control_extent(ui);
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
        let right_width = zoom_width + 5.0 * (extent + gap);
        ui.horizontal(|ui| {
            let left_width = (available - right_width - gap).max(extent);
            ui.allocate_ui_with_layout(
                egui::vec2(left_width, extent),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(left_width);
                    ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));
                    if back && icon_button(ui, ToolbarIcon::Back, true, "Back to folders").clicked()
                    {
                        self.session.file_view = None;
                    }
                    if icon_button(ui, ToolbarIcon::New, true, "New comparison").clicked() {
                        self.session.new_comparison();
                        ui.ctx().request_repaint();
                    }
                    status_label(ui, &self.session.navigation_status(), palette.muted);
                },
            );
            zoom_controls(ui);
            if icon_button_state(
                ui,
                ToolbarIcon::Differences,
                true,
                self.session.show_only_differences,
                "Show only differences",
            )
            .clicked()
            {
                self.session
                    .set_differences_only(!self.session.show_only_differences);
                ui.ctx().request_repaint();
            }
            let mut changed = false;
            if icon_button_state(
                ui,
                ToolbarIcon::Whitespace,
                true,
                self.session.ignore_whitespace,
                "Ignore whitespace",
            )
            .clicked()
            {
                self.session.ignore_whitespace = !self.session.ignore_whitespace;
                changed = true;
            }
            if icon_button_state(
                ui,
                ToolbarIcon::LineEndings,
                true,
                self.session.ignore_line_endings,
                "Ignore line endings",
            )
            .clicked()
            {
                self.session.ignore_line_endings = !self.session.ignore_line_endings;
                changed = true;
            }
            if changed {
                self.session.recompare();
                ui.ctx().request_repaint();
            }
            let has_differences = self.session.file_view.as_ref().map_or_else(
                || !self.session.folder_differences.is_empty(),
                |view| !view.difference_rows.is_empty(),
            );
            for (icon, forward, label) in [
                (ToolbarIcon::Previous, false, "Previous difference"),
                (ToolbarIcon::Next, true, "Next difference"),
            ] {
                if icon_button(ui, icon, has_differences, label).clicked() {
                    self.session.jump_difference(forward);
                    ui.ctx().request_repaint();
                }
            }
        });
    }

    pub(super) fn error_details(&self, view: Option<&InteractiveView>) -> Vec<String> {
        let mut errors = self
            .session
            .source_errors
            .iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        errors.extend(self.session.error.iter().cloned());
        errors.extend(self.session.drop_message.iter().cloned());
        errors.extend(
            self.session
                .file_view
                .as_ref()
                .and_then(|file| file.error.clone()),
        );
        if let Some(error) = view.and_then(InteractiveView::error_details) {
            errors.extend(error.lines().map(str::to_owned));
        }
        if let Some(browser) = &self.session.merge_folders {
            errors.extend(browser.error_details());
        }
        if let Some(browser) = &self.session.merge_file_search {
            errors.extend(browser.error_details());
        }
        errors.sort();
        errors.dedup();
        errors
    }

    pub(super) fn dismiss_errors_on_pane_input(&mut self, ui: &egui::Ui) {
        let moved = ui.input(|input| {
            input.events.iter().any(|event| {
                let position = match event {
                    egui::Event::PointerMoved(position) => Some(*position),
                    egui::Event::PointerButton {
                        pos, pressed: true, ..
                    } => Some(*pos),
                    _ => None,
                };
                position.is_some_and(|position| {
                    self.pane_rects
                        .iter()
                        .chain(self.source_header_rects.iter())
                        .any(|rect| rect.contains(position))
                })
            })
        });
        if moved {
            if !self
                .error_details(self.session.interactive.as_ref())
                .is_empty()
            {
                self.session.message.clear();
            }
            self.session.error = None;
            self.session.drop_message = None;
            self.session.source_errors = [None, None];
            if let Some(file) = &mut self.session.file_view {
                file.error = None;
            }
            if let Some(view) = &mut self.session.interactive {
                view.clear_error();
            }
            if let Some(browser) = &mut self.session.merge_folders {
                browser.clear_error();
            }
            if let Some(browser) = &mut self.session.merge_file_search {
                browser.clear_error();
            }
        }
    }

    pub(super) fn render_bottom_status(
        &mut self,
        ui: &mut egui::Ui,
        view: Option<&mut InteractiveView>,
    ) {
        let errors = self.error_details(view.as_deref());
        for error in &errors {
            if !self.logged_errors.contains(error) {
                crate::diagnostics::error(format!("Status error: {error}"));
            }
        }
        self.logged_errors = errors.clone();
        bottom_status_area(ui, |ui| {
            if !errors.is_empty() {
                error_summary(ui, &errors);
            } else if self.session.drop_message.is_some()
                || self.session.source_jobs.iter().any(Option::is_some)
                || self.session.job.is_some()
            {
                self.progress_indicator(ui);
            } else if self.session.merge_mode
                && self
                    .session
                    .merge_folders
                    .as_ref()
                    .is_some_and(|browser| browser.render_activity_status(ui))
            {
            } else if self.session.merge_mode
                && self
                    .session
                    .merge_file_search
                    .as_ref()
                    .is_some_and(|browser| browser.render_activity_status(ui))
            {
            } else if view.is_none_or(|view| !view.render_activity_status(ui)) {
                self.progress_indicator(ui);
            }
        });
    }

    pub(super) fn footer(&mut self, ui: &mut egui::Ui) {
        let row = workspace_footer_layout(
            ui,
            false,
            false,
            &DIFF_LEGEND_ICONS,
            self.session.comparison_counts(),
        );
        let guidance = self
            .session
            .selected
            .as_ref()
            .filter(|_| self.session.tree.is_some() && self.session.file_view.is_none())
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| self.session.message.clone());
        ui.scope_builder(egui::UiBuilder::new().max_rect(row.info), |ui| {
            footer_guidance(ui, &guidance);
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(row.legend), |ui| {
            bottom_legend(ui, &DIFF_LEGEND_ICONS, self.session.comparison_counts());
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(row.status), |ui| {
            self.render_bottom_status(ui, None);
        });
    }
}
