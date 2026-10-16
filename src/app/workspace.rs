//! Compose Diff and Merge workspaces within the native window.

use super::*;
use crate::app::visuals::footer::{DIFF_LEGEND_ICONS, workspace_footer_height};
use crate::app::visuals::panels::{
    comparison_panel, empty_display, file_path_headers, paint_panel_divider, split_rect,
    workspace_mode_button,
};
use crate::app::visuals::theme::{MAX_ZOOM, MIN_ZOOM, Palette, comparison_row_height};

impl VersusApp {
    pub(super) fn render(&mut self, ui: &mut egui::Ui) {
        self.dismiss_errors_on_pane_input(ui);
        self.preferences_context = Some(ui.ctx().clone());
        // egui separates modified wheel input from scrolling and smooths it.
        // Apply each delta once, including its tail across repaint frames.
        if ui.ctx().current_pass_index() == 0 {
            let delta = ui.input(|input| input.zoom_delta());
            if delta.is_finite() && delta > 0.0 && delta != 1.0 {
                ui.ctx()
                    .set_zoom_factor((ui.ctx().zoom_factor() * delta).clamp(MIN_ZOOM, MAX_ZOOM));
            }
        }
        if self.session.file_view.is_none()
            && self.session.tree_focus_id.is_some()
            && ui.memory(|memory| memory.focused()) == self.session.tree_focus_id
            && ui.input(|input| input.key_pressed(egui::Key::Tab))
        {
            let destination = if ui.input(|input| input.events.iter().any(|event| matches!(event,
                egui::Event::Key { key: egui::Key::Tab, pressed: true, modifiers, .. } if modifiers.shift))) {
                "Browse right folder"
            } else {
                "Folder Diff"
            };
            ui.memory_mut(|memory| {
                memory.move_focus(egui::FocusDirection::None);
                memory.request_focus(egui::Id::new(destination));
            });
            self.session.tree_keyboard_focus = None;
            self.session.tree_focus_id = None;
        }
        let row_height = comparison_row_height(ui);
        if (row_height - self.session.row_height).abs() > 0.1 {
            self.session.tree_scroll_y *= row_height / self.session.row_height;
            self.session.row_height = row_height;
        }
        if let Some(view) = &mut self.session.file_view {
            view.scroll_y *= row_height / view.row_height;
            view.row_height = row_height;
        }
        let palette = Palette::for_context(ui.ctx());
        let dark = ui.visuals().dark_mode;
        if self
            .logo_texture
            .as_ref()
            .is_none_or(|(theme, _)| *theme != dark)
        {
            let icon = crate::logo::themed_icon(dark);
            // PNG/window-icon bytes have straight alpha; egui textures need
            // premultiplied alpha, including nearly transparent edge pixels.
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            );
            self.logo_texture = Some((
                dark,
                ui.ctx()
                    .load_texture("versus-mark", image, egui::TextureOptions::LINEAR),
            ));
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Icon(Some(Arc::new(icon))));
        }
        egui::Frame::default()
            .fill(palette.background)
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                self.header(ui);
                ui.add_space(5.0);
                if self.session.interactive.is_some()
                    && (self.session.merge_mode || self.session.merge_file_open)
                {
                    self.interactive_area(ui);
                    return;
                }
                self.legend(ui);
                ui.add_space(5.0);
                let available = ui.available_rect_before_wrap();
                let footer_height = workspace_footer_height(
                    ui,
                    false,
                    false,
                    &DIFF_LEGEND_ICONS,
                    self.session.comparison_counts(),
                );
                let footer_rect = Rect::from_min_max(
                    egui::pos2(
                        available.left(),
                        (available.bottom() - footer_height).max(available.top()),
                    ),
                    available.max,
                );
                let tree_rect = Rect::from_min_max(
                    available.min,
                    egui::pos2(
                        available.right(),
                        (footer_rect.top() - 5.0).max(available.top()),
                    ),
                );
                self.pane_rects = split_rect(tree_rect);
                ui.scope_builder(egui::UiBuilder::new().max_rect(tree_rect), |ui| {
                    ui.set_clip_rect(ui.clip_rect().intersect(tree_rect));
                    if self.session.file_view.is_some() {
                        self.file_area(ui);
                    } else {
                        self.tree_area(ui, (tree_rect.height() - 2.0).max(0.0));
                    }
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(footer_rect), |ui| {
                    ui.set_clip_rect(ui.clip_rect().intersect(footer_rect));
                    self.footer(ui);
                });
            });
        self.session.sync_folder_merge_diff();
        self.session.sync_working_folder_diff(ui.ctx());
        self.drag_and_drop(ui);
        self.merge_exit_dialog(ui.ctx());
    }

    pub(super) fn workspace_mode_toggle(&mut self, ui: &mut egui::Ui) {
        let mut merge = self.session.merge_mode;
        for (enabled, label, tooltip) in [
            (
                false,
                "Diff",
                "Inspect differences using comparison preferences",
            ),
            (
                true,
                "Merge",
                "Edit and transfer content; comparison is optional",
            ),
        ] {
            if workspace_mode_button(ui, label, merge == enabled)
                .on_hover_text(tooltip)
                .clicked()
            {
                merge = enabled;
            }
        }
        if merge != self.session.merge_mode {
            self.session.set_workspace_merge(merge);
            ui.ctx().request_repaint();
        }
    }

    pub(super) fn interactive_area(&mut self, ui: &mut egui::Ui) {
        if self.session.merge_mode {
            self.independent_merge_area(ui);
            return;
        }
        let saving = self
            .session
            .merge_folders
            .as_ref()
            .is_some_and(FolderBrowser::is_saving)
            || self
                .session
                .merge_file_search
                .as_ref()
                .is_some_and(FolderBrowser::is_saving);
        let prefs = [
            self.session.show_only_differences,
            self.session.ignore_whitespace,
            self.session.ignore_line_endings,
        ];
        if let Some(view) = &mut self.session.interactive {
            view.set_external_save_busy(saving);
            view.set_editing(self.session.merge_mode, prefs);
        }
        if self.session.merge_file_open {
            if let Some(mut view) = self.session.interactive.take() {
                view.set_from_folders(self.session.merge_folders.is_some());
                let folder_file_paths = self
                    .session
                    .merge_folders
                    .as_ref()
                    .map(|_| view.current_sources());
                let (panes, status) = view.render_with_sources(ui, |ui| {
                    if self.session.merge_folders.is_none() {
                        self.folder_inputs(ui);
                    } else if let Some([Some(left), Some(right)]) = &folder_file_paths {
                        file_path_headers(ui, &[left.clone(), right.clone()]);
                    }
                    paint_panel_divider(ui);
                });
                self.pane_rects = panes;
                ui.scope_builder(egui::UiBuilder::new().max_rect(status), |ui| {
                    self.render_bottom_status(ui, Some(&mut view));
                });
                if self.session.merge_folders.is_none() {
                    if let Some(paths) = view
                        .take_loaded_paths()
                        .filter(|_| self.session.source_jobs.iter().all(Option::is_none))
                    {
                        self.session.paths = paths
                            .clone()
                            .map(|path| path.to_string_lossy().into_owned());
                        self.session.source_paths = paths.clone().map(Some);
                        self.session.source_kinds = [Some(SourceKind::File); 2];
                    }
                }
                if !self.session.merge_mode {
                    let options = view.diff_preferences();
                    self.session.show_only_differences = options[0];
                    self.session.ignore_whitespace = options[1];
                    self.session.ignore_line_endings = options[2];
                }
                if view.take_back_requested() {
                    self.session.merge_file_open = false;
                    self.session.merge_file_sides = [false; 2];
                    view.open_sources([None, None]);
                    if !self.session.merge_mode && self.session.merge_folders.is_some() {
                        self.session.start_comparison();
                    }
                }
                self.session.interactive = Some(view);
            }
            if self
                .session
                .interactive
                .as_mut()
                .is_some_and(InteractiveView::take_new_requested)
            {
                self.session.new_comparison();
            }
        } else {
            if let Some(view) = &mut self.session.interactive {
                view.set_from_folders(false);
                let differences = match (&self.session.tree, &mut self.session.merge_folders) {
                    (Some(tree), Some(browser)) => browser.difference_paths(tree, view),
                    _ => Arc::new(Vec::new()),
                };
                view.set_folder_differences(differences);
                view.render_toolbar(ui);
                if let (Some(path), Some(browser)) =
                    (view.take_folder_jump(), &mut self.session.merge_folders)
                {
                    browser.reveal_difference(path);
                }
            }
            comparison_panel(ui).show(ui, |ui| {
                ui.set_min_height((ui.available_height() - 2.0).max(0.0));
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                self.folder_inputs(ui);
                paint_panel_divider(ui);
                self.pane_rects = split_rect(ui.available_rect_before_wrap());
                if let (Some(browser), Some(view)) = (
                    &mut self.session.merge_folders,
                    &mut self.session.interactive,
                ) {
                    if let Some(paths) = browser.render(ui, view, self.session.tree.as_ref()) {
                        view.open_pair(paths);
                        self.session.merge_file_open = true;
                    }
                } else {
                    empty_display(
                        ui,
                        ui.available_height().max(60.0),
                        "Choose files or folders",
                        "Use Browse or drop a source into each pane.",
                    );
                }
            });
            if self
                .session
                .interactive
                .as_mut()
                .is_some_and(InteractiveView::take_new_requested)
            {
                self.session.new_comparison();
            }
        }
    }

    pub(super) fn independent_merge_area(&mut self, ui: &mut egui::Ui) {
        let Some(mut view) = self.session.interactive.take() else {
            return;
        };
        view.set_external_save_busy(
            self.session
                .merge_folders
                .as_ref()
                .is_some_and(FolderBrowser::is_saving)
                || self
                    .session
                    .merge_file_search
                    .as_ref()
                    .is_some_and(FolderBrowser::is_saving),
        );
        view.set_editing(
            true,
            [
                self.session.show_only_differences,
                self.session.ignore_whitespace,
                self.session.ignore_line_endings,
            ],
        );
        view.set_from_folders(self.session.merge_folders.is_some());
        let differences = match (&self.session.tree, &mut self.session.merge_folders) {
            (Some(tree), Some(browser)) => browser.difference_paths(tree, &view),
            _ => Arc::new(Vec::new()),
        };
        view.set_folder_differences(differences);
        view.render_workspace_toolbar(ui, !self.session.merge_file_sides.iter().any(|open| *open));
        if view.take_search_requested() {
            if let Some(browser) = &mut self.session.merge_folders {
                browser.open_search();
            } else {
                let sources = view.current_sources();
                let browser = self
                    .session
                    .merge_file_search
                    .get_or_insert_with(|| FolderBrowser::new_search(sources.clone()));
                browser.set_search_sources(sources);
                browser.open_search();
            }
        }
        if self.session.merge_folders.is_none()
            && let Some(browser) = &mut self.session.merge_file_search
        {
            browser.set_search_sources(view.current_sources());
            browser.render_search_controls(ui, &mut view);
        }
        if let (Some(path), Some(browser)) =
            (view.take_folder_jump(), &mut self.session.merge_folders)
        {
            browser.reveal_difference(path);
        }
        if view.take_back_requested() {
            self.session.merge_file_sides = [false; 2];
            self.session.merge_file_open = false;
            view.open_sources([None, None]);
        }
        ui.add_space(5.0);
        let area = ui.available_rect_before_wrap();
        let footer_height = view.footer_height(ui) + 4.0;
        let body = Rect::from_min_max(
            area.min,
            egui::pos2(
                area.right(),
                (area.bottom() - footer_height).max(area.top()),
            ),
        );
        let footer = Rect::from_min_max(egui::pos2(area.left(), body.bottom() + 4.0), area.max);
        let mut opened: [Option<PathBuf>; 2] = [None, None];
        let mut back = [false; 2];
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            comparison_panel(ui).show(ui, |ui| {
                ui.set_min_height((ui.available_height() - 2.0).max(0.0));
                let source_spacing = ui.spacing().item_spacing.y;
                ui.spacing_mut().item_spacing = egui::vec2(
                    0.0,
                    if self.session.merge_file_sides.iter().any(|open| *open) {
                        source_spacing
                    } else {
                        0.0
                    },
                );
                back = self.merge_inputs(ui, &mut view);
                for side in 0..2 {
                    if self.session.source_jobs[side].is_some() {
                        view.open_side(side, None);
                    }
                }
                paint_panel_divider(ui);
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                if let Some(browser) = &mut self.session.merge_folders {
                    browser.render_controls(ui, &mut view);
                }
                let pane_area = ui.available_rect_before_wrap();
                self.pane_rects = split_rect(pane_area);
                ui.painter().line_segment(
                    [pane_area.center_top(), pane_area.center_bottom()],
                    Stroke::new(1.0, Palette::for_context(ui.ctx()).border),
                );
                ui.columns(2, |columns| {
                    for (side, pane) in columns.iter_mut().enumerate() {
                        pane.set_clip_rect(pane.clip_rect().intersect(self.pane_rects[side]));
                        if self.session.merge_file_sides[side] {
                            let height = pane.available_height();
                            view.render_side(pane, side, height);
                        } else if let Some(browser) = self
                            .session
                            .merge_folders
                            .as_mut()
                            .filter(|browser| browser.source_roots()[side].is_some())
                        {
                            if let Some(path) = browser.render_side(
                                pane,
                                &mut view,
                                self.session.tree.as_ref(),
                                side,
                            ) {
                                opened[side] = Some(path);
                            }
                        } else {
                            let (rect, _) = pane.allocate_exact_size(
                                egui::vec2(
                                    pane.available_width(),
                                    pane.available_height().max(80.0),
                                ),
                                Sense::hover(),
                            );
                            paint_empty_drop_target(pane, rect, side, true);
                        }
                    }
                });
            });
        });
        for side in 0..2 {
            if back[side] {
                self.session.merge_file_sides[side] = false;
                view.open_side(side, None);
            } else if let Some(path) = opened[side].take() {
                self.session.merge_file_sides[side] = true;
                view.open_side(side, Some(path));
            }
        }
        self.session.merge_file_open = self.session.merge_file_sides.iter().any(|open| *open);
        ui.scope_builder(egui::UiBuilder::new().max_rect(footer), |ui| {
            let status = view.render_footer(ui);
            ui.scope_builder(egui::UiBuilder::new().max_rect(status), |ui| {
                self.render_bottom_status(ui, Some(&mut view));
            });
        });
        if view.take_save_all_requested() {
            if let Some(browser) = self.session.merge_folders.as_mut() {
                browser.begin_save(&view);
            } else {
                let browser = self
                    .session
                    .merge_file_search
                    .get_or_insert_with(|| FolderBrowser::new_search(view.current_sources()));
                browser.begin_save(&view);
            }
        }
        view.render_previews(ui.ctx());
        if self.session.merge_folders.is_none() {
            if let Some(paths) = view.take_loaded_sources() {
                for side in 0..2 {
                    if self.session.source_jobs[side].is_none() {
                        self.session.paths[side] = paths[side]
                            .as_ref()
                            .map(|path| path.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        self.session.source_paths[side] = paths[side].clone();
                        self.session.source_kinds[side] =
                            paths[side].as_ref().map(|_| SourceKind::File);
                    }
                }
            }
        }
        let new = view.take_new_requested();
        self.session.interactive = Some(view);
        if new {
            self.session.new_comparison();
        }
    }
}
