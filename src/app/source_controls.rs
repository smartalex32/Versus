//! Source selection, mode controls and pane headers.

use super::*;
use crate::app::visuals::icons::{ToolbarIcon, icon_button};
use crate::app::visuals::panels::{
    comparison_mode_button, left_elided_path, paint_side_label, split_rect,
};
use crate::app::visuals::theme::{Palette, comparison_font, control_extent, set_theme};

impl VersusApp {
    pub(super) fn header(&mut self, ui: &mut egui::Ui) {
        let modes = [ComparisonMode::Folder, ComparisonMode::File];
        let labels = modes.map(|mode| {
            egui::WidgetText::from(
                RichText::new(mode.workspace_label(self.session.merge_mode))
                    .size(14.0)
                    .strong(),
            )
            .into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                FontId::proportional(14.0),
            )
        });
        let padding = ui.spacing().button_padding;
        let widths = labels
            .each_ref()
            .map(|label| label.size().x + 22.0 + padding.x * 2.0);
        let gap = ui.spacing().item_spacing.x;
        let group_width = widths.iter().sum::<f32>() + gap;
        let (header, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 32.0_f32.max(control_extent(ui))),
            Sense::hover(),
        );
        let group = Rect::from_center_size(header.center(), egui::vec2(group_width, 28.0));

        let mut brand_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    header.min,
                    egui::pos2(group.left() - gap, header.bottom()),
                ))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        if header.width() >= 700.0 {
            if let Some((_, logo)) = &self.logo_texture {
                brand_ui.image((logo.id(), egui::vec2(28.0, 28.0)));
            }
            brand_ui.label(RichText::new("Versus").size(18.0).strong());
            brand_ui.add_space(16.0);
        } else {
            brand_ui.spacing_mut().item_spacing.x = 4.0;
        }
        self.workspace_mode_toggle(&mut brand_ui);

        let mut x = group.left();
        for (index, mode) in modes.into_iter().enumerate() {
            let rect = Rect::from_min_size(
                egui::pos2(x, group.top()),
                egui::vec2(widths[index], group.height()),
            );
            if comparison_mode_button(
                ui,
                rect,
                labels[index].clone(),
                mode,
                self.session.comparison_mode == mode,
            )
            .clicked()
            {
                self.session.switch_comparison_mode(mode);
                ui.ctx().request_repaint();
            }
            x += widths[index] + gap;
        }

        let mut controls_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    egui::pos2(group.right() + gap, header.top()),
                    header.max,
                ))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        let dark = ui.visuals().dark_mode;
        if icon_button(
            &mut controls_ui,
            if dark {
                ToolbarIcon::Sun
            } else {
                ToolbarIcon::Moon
            },
            true,
            if dark {
                "Switch to light mode"
            } else {
                "Switch to dark mode"
            },
        )
        .clicked()
        {
            set_theme(
                ui.ctx(),
                if dark {
                    egui::Theme::Light
                } else {
                    egui::Theme::Dark
                },
            );
            ui.ctx().request_repaint();
        }
        if self.session.interactive.is_some()
            && (self.session.merge_mode || self.session.merge_file_open)
        {
            if self.session.merge_mode {
                let mut enabled = self
                    .session
                    .interactive
                    .as_ref()
                    .unwrap()
                    .run_diff_enabled();
                if enabled
                    && icon_button(
                        &mut controls_ui,
                        ToolbarIcon::Refresh,
                        self.session
                            .interactive
                            .as_ref()
                            .unwrap()
                            .can_refresh_diff()
                            || self.session.merge_folders.is_some(),
                        "Refresh comparison",
                    )
                    .clicked()
                {
                    if let Some(browser) = &mut self.session.merge_folders {
                        browser.refresh_listing();
                    }
                    self.session.interactive.as_mut().unwrap().refresh_diff();
                    self.session.folder_diff_settings = None;
                }
                let view = self.session.interactive.as_mut().unwrap();
                if controls_ui
                    .checkbox(&mut enabled, "Run Diff")
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Compare loaded files to assist merging")
                    .changed()
                {
                    view.set_run_diff(enabled);
                }
            } else {
                let view = self.session.interactive.as_mut().unwrap();
                if icon_button(
                    &mut controls_ui,
                    ToolbarIcon::Refresh,
                    view.can_refresh_diff(),
                    "Refresh comparison",
                )
                .clicked()
                {
                    view.refresh_diff();
                }
            }
        } else {
            match self.session.mode() {
                SelectionMode::Folder => self.controls(&mut controls_ui),
                SelectionMode::File => self.file_controls(&mut controls_ui),
                _ => {}
            }
        }
    }

    pub(super) fn folder_inputs(&mut self, ui: &mut egui::Ui) {
        let kinds = self.session.source_kinds;
        self.folder_inputs_with_picker(ui, |side, current_path, mode| {
            selection::pick_path(side, current_path, kinds[side], mode)
        });
    }

    pub(super) fn folder_inputs_with_picker(
        &mut self,
        ui: &mut egui::Ui,
        pick_source: impl FnMut(usize, &str, ComparisonMode) -> Option<PathBuf>,
    ) {
        self.source_inputs(ui, pick_source, None);
    }

    pub(super) fn merge_inputs(
        &mut self,
        ui: &mut egui::Ui,
        view: &mut InteractiveView,
    ) -> [bool; 2] {
        let files = view.current_sources();
        let roots = self
            .session
            .merge_folders
            .as_ref()
            .map(FolderBrowser::source_roots)
            .unwrap_or([None, None]);
        let header = MergePaneHeaders {
            files: files.clone(),
            back: std::array::from_fn(|side| {
                self.session.merge_file_sides[side] && roots[side].is_some()
            }),
            save: std::array::from_fn(|side| view.can_save_side(side)),
            dirty: std::array::from_fn(|side| view.is_side_dirty(side)),
        };
        let kinds = std::array::from_fn::<_, 2, _>(|side| {
            if files[side].is_some() {
                Some(SourceKind::File)
            } else {
                self.session.source_kinds[side]
            }
        });
        let (back, save) = self.source_inputs(
            ui,
            |side, path, mode| selection::pick_path(side, path, kinds[side], mode),
            Some(&header),
        );
        for side in 0..2 {
            if save[side] {
                view.request_side_save(side);
            }
        }
        back
    }

    pub(super) fn source_inputs(
        &mut self,
        ui: &mut egui::Ui,
        mut pick_source: impl FnMut(usize, &str, ComparisonMode) -> Option<PathBuf>,
        merge: Option<&MergePaneHeaders>,
    ) -> ([bool; 2], [bool; 2]) {
        let palette = Palette::for_context(ui.ctx());
        let extent = control_extent(ui);
        let gap = ui.spacing().item_spacing.x.max(4.0);
        let width = ui.available_width();
        let paths: [String; 2] = std::array::from_fn(|side| {
            merge
                .and_then(|header| header.files[side].as_ref())
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.session.paths[side].clone())
        });
        let full_paths = paths.each_ref().map(|path| {
            if path.is_empty() {
                format!("Choose a {}", self.session.comparison_mode.source_label())
            } else {
                absolute_path(path).display().to_string()
            }
        });
        let label_widths = std::array::from_fn::<_, 2, _>(|side| {
            if merge.is_some() {
                ui.painter()
                    .layout_no_wrap(
                        if side == 0 {
                            "LEFT (Incoming)"
                        } else {
                            "RIGHT (Current)"
                        }
                        .into(),
                        FontId::monospace(10.0),
                        palette.muted,
                    )
                    .size()
                    .x
                    + gap
            } else {
                40.0
            }
        });
        let path_widths = std::array::from_fn::<_, 2, _>(|side| {
            let back = merge.is_some_and(|header| header.back[side]);
            (width / 2.0
                - 14.0
                - label_widths[side]
                - extent
                - gap
                - if merge.is_some() { extent + gap } else { 0.0 }
                - if back { extent + gap } else { 0.0 })
            .max(1.0)
        });
        let galleys = std::array::from_fn::<_, 2, _>(|side| {
            if merge.is_some() || self.session.source_kinds[side] == Some(SourceKind::File) {
                left_elided_path(
                    ui.painter(),
                    &full_paths[side],
                    path_widths[side],
                    palette.text,
                )
            } else {
                ui.painter().layout(
                    full_paths[side].clone(),
                    comparison_font(ui.ctx()),
                    palette.text,
                    path_widths[side],
                )
            }
        });
        let height = galleys
            .iter()
            .map(|galley| galley.size().y)
            .fold(extent, f32::max)
            + 14.0;
        let (header, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
        self.source_header_rects = split_rect(header);
        let mut back = [false; 2];
        let mut save = [false; 2];
        for (side, half) in split_rect(header).into_iter().enumerate() {
            let rect = half.shrink(7.0);
            let painter = ui.painter().with_clip_rect(rect);
            let mut label_x = rect.left();
            if merge.is_some_and(|header| header.back[side]) {
                let button = Rect::from_min_size(
                    egui::pos2(label_x, rect.center().y - extent / 2.0),
                    egui::vec2(extent, extent),
                );
                let mut button_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(button)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                back[side] = icon_button(
                    &mut button_ui,
                    ToolbarIcon::Back,
                    true,
                    &format!(
                        "Back to {} folder",
                        if side == 0 { "left" } else { "right" }
                    ),
                )
                .clicked();
                label_x += extent + gap;
            }
            if merge.is_some() {
                painter.text(
                    egui::pos2(label_x, rect.center().y),
                    Align2::LEFT_CENTER,
                    if side == 0 {
                        "LEFT (Incoming)"
                    } else {
                        "RIGHT (Current)"
                    },
                    FontId::monospace(10.0),
                    if side == 0 {
                        palette.left_only
                    } else {
                        palette.right_only
                    },
                );
                ui.interact(
                    Rect::from_min_size(
                        egui::pos2(label_x, rect.top()),
                        egui::vec2(label_widths[side], rect.height()),
                    ),
                    egui::Id::new(("merge-side-label", side)),
                    Sense::hover(),
                );
            } else {
                paint_side_label(&painter, rect, side, palette);
            }
            let position = egui::pos2(
                label_x + label_widths[side],
                rect.center().y - galleys[side].size().y / 2.0,
            );
            let path_rect = Rect::from_min_size(
                egui::pos2(position.x, rect.top()),
                egui::vec2(path_widths[side], rect.height()),
            );
            painter.galley_with_override_text_color(
                position,
                galleys[side].clone(),
                if merge.is_some_and(|header| header.dirty[side]) {
                    palette.changed
                } else {
                    palette.text
                },
            );
            let kind = if merge.is_some_and(|header| header.files[side].is_some()) {
                "file"
            } else {
                self.session.comparison_mode.source_label()
            };
            let browse_label =
                format!("Browse {} {kind}", if side == 0 { "left" } else { "right" });
            let path_response = ui
                .interact(
                    path_rect,
                    ui.id().with(("selected-folder", side)),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!(
                    "{}\nClick to browse for a {kind}",
                    full_paths[side]
                ));
            path_response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    format!("{browse_label} path"),
                )
            });
            let save_space = if merge.is_some() { extent + gap } else { 0.0 };
            let browse_rect = Rect::from_center_size(
                rect.right_center() - egui::vec2(extent / 2.0 + save_space, 0.0),
                egui::vec2(extent, extent),
            );
            let mut browse_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(browse_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            let browse = icon_button(&mut browse_ui, ToolbarIcon::Browse, true, &browse_label);
            if let Some(state) = merge {
                let save_rect = Rect::from_center_size(
                    rect.right_center() - egui::vec2(extent / 2.0, 0.0),
                    egui::vec2(extent, extent),
                );
                let mut save_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(save_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                save[side] = icon_button(
                    &mut save_ui,
                    ToolbarIcon::Save,
                    state.save[side],
                    if side == 0 { "Save Left" } else { "Save Right" },
                )
                .clicked();
            }
            if browse.clicked() || path_response.clicked() {
                if let Some(path) = pick_source(side, &paths[side], self.session.comparison_mode) {
                    self.session.select_path(side, path);
                    ui.ctx().request_repaint();
                }
            }
        }
        ui.painter().line_segment(
            [header.center_top(), header.center_bottom()],
            Stroke::new(1.0, palette.border),
        );
        (back, save)
    }
}
