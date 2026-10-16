//! Find and Replace modal controls, fields, and streamed result painting.

use super::*;
use crate::app::visuals::icons::{ToolbarIcon, icon_button, icon_button_state};
use crate::app::visuals::theme::{Palette, control_extent};
use search_state::scope_includes;
use versus::SearchScope;

impl FolderBrowser {
    pub(super) fn search_window(&mut self, ctx: &egui::Context, view: &mut InteractiveView) {
        if !self.search_state.is_open() {
            return;
        }
        if self.search_state.focus_pending() {
            self.working = view.export_working_files();
        }
        if !self.search_state.is_waiting()
            && !self.search_state.is_searching()
            && self.search_state.results().is_none()
            && !self.search_state.query().pattern.is_empty()
        {
            self.schedule_search(ctx.input(|input| input.time));
        }
        let mut close = false;
        let width = (ctx.content_rect().width() - 48.0).clamp(200.0, 576.0);
        let max_height = (ctx.content_rect().height() - 96.0).clamp(120.0, 360.0);
        let response = egui::Modal::new(egui::Id::new("folder-find-replace-modal"))
            .frame(
                egui::Frame::popup(&ctx.global_style())
                    .inner_margin(12)
                    .corner_radius(6),
            )
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Find and Replace").size(14.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close =
                            icon_button(ui, ToolbarIcon::Cancel, true, "Close Find and Replace")
                                .clicked();
                    });
                });
                ui.add_space(6.0);
                ui.columns(2, |columns| {
                    egui::ScrollArea::vertical()
                        .id_salt("folder-find-replace-content")
                        .max_height(max_height)
                        .auto_shrink([false, true])
                        .show(&mut columns[0], |ui| {
                            let editing_field = ui.memory(|memory| {
                                matches!(memory.focused(), Some(id)
                    if id == egui::Id::new("folder-find-pattern")
                        || id == egui::Id::new("folder-replacement-text"))
                            });
                            if !editing_field {
                                if ui.input_mut(|input| {
                                    input.consume_key(
                                        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                                        egui::Key::Z,
                                    ) || input.consume_key(egui::Modifiers::CTRL, egui::Key::Y)
                                }) {
                                    self.undo_batch(view, true);
                                } else if ui.input_mut(|input| {
                                    input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)
                                        || input.consume_key(egui::Modifiers::CTRL, egui::Key::Z)
                                }) {
                                    self.undo_batch(view, false);
                                }
                            }
                            let previous_query = self.search_state.query().clone();
                            let request_find_focus = self.search_state.take_focus();
                            let query = self.search_state.query_mut();
                            let find = search_text_field(
                                ui,
                                &mut query.pattern,
                                "folder-find-pattern",
                                "Find",
                                Some([
                                    &mut query.case_sensitive,
                                    &mut query.whole_word,
                                    &mut query.regex,
                                ]),
                            );
                            if request_find_focus {
                                find.request_focus();
                            }
                            let extent = control_extent(ui);
                            let gap = ui.spacing().item_spacing.x;
                            let (row, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), SEARCH_FIELD_HEIGHT),
                                Sense::hover(),
                            );
                            let field = Rect::from_min_max(
                                row.min,
                                egui::pos2(row.right() - 2.0 * (extent + gap), row.bottom()),
                            );
                            let mut replacement_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(field)
                                    .layout(egui::Layout::top_down(egui::Align::Min)),
                            );
                            search_text_field(
                                &mut replacement_ui,
                                self.search_state.replacement_mut(),
                                "folder-replacement-text",
                                "Replace",
                                None,
                            );
                            let enabled =
                                self.search_state.replacement_allowed() && !view.is_saving();
                            for (index, icon, label, next) in [
                                (0, ToolbarIcon::ReplaceNext, "Replace Next", true),
                                (1, ToolbarIcon::ReplaceAll, "Replace All", false),
                            ] {
                                let button = Rect::from_center_size(
                                    egui::pos2(
                                        field.right()
                                            + gap
                                            + extent / 2.0
                                            + index as f32 * (extent + gap),
                                        row.center().y,
                                    ),
                                    egui::vec2(extent, extent),
                                );
                                let mut button_ui = ui.new_child(
                                    egui::UiBuilder::new()
                                        .max_rect(button)
                                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                                );
                                if icon_button(&mut button_ui, icon, enabled, label).clicked() {
                                    self.replace_matches(view, next);
                                }
                            }
                            ui.add_space(6.0);
                            let mut refresh = false;
                            let mut scope_clicked = false;
                            let scope_width: f32 = ["LEFT", "RIGHT", "BOTH"]
                                .into_iter()
                                .map(|label| {
                                    ui.painter()
                                        .layout_no_wrap(
                                            label.into(),
                                            FontId::monospace(11.0),
                                            Palette::for_context(ui.ctx()).text,
                                        )
                                        .size()
                                        .x
                                        + 16.0
                                })
                                .sum();
                            let fits =
                                scope_width + 3.0 * extent + 5.0 * 3.0 <= ui.available_width();
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 3.0;
                                let palette = Palette::for_context(ui.ctx());
                                for (scope, label, color) in [
                                    (SearchScope::Left, "LEFT", palette.left_only),
                                    (SearchScope::Right, "RIGHT", palette.right_only),
                                    (SearchScope::Both, "BOTH", palette.text),
                                ] {
                                    if search_scope_button(
                                        ui,
                                        label,
                                        color,
                                        self.search_state.scope() == scope,
                                    )
                                    .clicked()
                                    {
                                        scope_clicked = true;
                                        self.search_state.set_scope(scope);
                                    }
                                }
                                if fits {
                                    refresh = self.search_action_buttons(ui, view);
                                }
                            });
                            if !fits {
                                ui.horizontal(|ui| {
                                    refresh = self.search_action_buttons(ui, view);
                                });
                            }
                            let options_changed = self.search_state.query().case_sensitive
                                != previous_query.case_sensitive
                                || self.search_state.query().whole_word
                                    != previous_query.whole_word
                                || self.search_state.query().regex != previous_query.regex;
                            if refresh
                                || options_changed
                                || (scope_clicked
                                    && (self.search_state.is_waiting()
                                        || self.search_state.query().pattern
                                            != previous_query.pattern
                                        || (!self.search_state.is_searching()
                                            && !self.search_state.results_current())))
                            {
                                self.start_search(view);
                            } else if self.search_state.query().pattern != previous_query.pattern {
                                self.schedule_search(ctx.input(|input| input.time));
                            }
                            ui.horizontal(|ui| {
                                if self.search_state.is_searching() {
                                    ui.spinner();
                                    let status = format!(
                                        "Searching… {} matches",
                                        self.search_state.selected_count()
                                    );
                                    ui.add_sized(
                                        egui::vec2(
                                            (ui.available_width()
                                                - extent
                                                - ui.spacing().item_spacing.x)
                                                .max(1.0),
                                            extent,
                                        ),
                                        egui::Label::new(&status).truncate(),
                                    )
                                    .on_hover_text(status);
                                    if icon_button(ui, ToolbarIcon::Cancel, true, "Cancel search")
                                        .clicked()
                                    {
                                        self.search_state.cancel();
                                    }
                                } else if self.search_state.is_waiting() {
                                    ui.label("Waiting for typing…");
                                } else if let Some(results) = self.search_state.results() {
                                    ui.label(if results.cancelled {
                                        "Search stopped".into()
                                    } else {
                                        format!(
                                            "Found {} matches",
                                            self.search_state.selected_count()
                                        )
                                    });
                                }
                            });
                            if let Some(error) = &self.error {
                                ui.colored_label(Palette::for_context(ctx).error, error);
                            }
                        });
                    let right = &mut columns[1];
                    let counts = self.search_match_counts();
                    right.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        let palette = Palette::for_context(ui.ctx());
                        let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
                        for (side, label) in [(0, "LEFT"), (1, "RIGHT")] {
                            let enabled = scope_includes(self.search_state.scope(), side);
                            if ui
                                .add_enabled_ui(enabled, |ui| {
                                    search_choice_button(
                                        ui,
                                        &format!("{label} ({})", counts[side]),
                                        palette.text,
                                        self.search_state.results_side() == side,
                                        egui::Id::new(("folder-search-view", side)),
                                        if side == 0 {
                                            "Show left results"
                                        } else {
                                            "Show right results"
                                        },
                                        Some(width),
                                    )
                                })
                                .inner
                                .clicked()
                            {
                                self.search_state.set_results_side(side);
                            }
                        }
                    });
                    right.add_space(3.0);
                    let results_height = (max_height - control_extent(right) - 6.0).max(60.0);
                    self.search_results_panel(right, results_height);
                });
            });
        if close || response.should_close() {
            self.search_state.close();
        } else {
            self.run_scheduled_search(ctx, view, ctx.input(|input| input.time));
        }
    }
    fn search_action_buttons(&mut self, ui: &mut egui::Ui, view: &mut InteractiveView) -> bool {
        if icon_button(
            ui,
            ToolbarIcon::Undo,
            view.can_undo_batch(),
            "Undo replacement batch",
        )
        .clicked()
        {
            self.undo_batch(view, false);
        }
        if icon_button(
            ui,
            ToolbarIcon::Redo,
            view.can_redo_batch(),
            "Redo replacement batch",
        )
        .clicked()
        {
            self.undo_batch(view, true);
        }
        return icon_button(
            ui,
            ToolbarIcon::Refresh,
            !self.search_state.query().pattern.is_empty() && !view.is_saving() && !self.is_saving(),
            "Refresh search",
        )
        .clicked();
    }

    pub(super) fn search_match_counts(&self) -> [usize; 2] {
        self.search_state.match_counts()
    }

    fn search_results_panel(&mut self, ui: &mut egui::Ui, height: f32) {
        let palette = Palette::for_context(ui.ctx());
        let mut removed = Vec::new();
        egui::Frame::new()
            .fill(palette.panel)
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(6)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_height((height - 18.0).max(0.0));
                ui.interact(
                    ui.max_rect(),
                    egui::Id::new("folder-search-results"),
                    Sense::hover(),
                );
                if let Some(results) = self.search_state.results() {
                    enum Row<'a> {
                        File(&'a versus::SearchFileResults),
                        Hit(&'a versus::SearchFileResults, &'a versus::SearchHit),
                    }
                    let mut rows = Vec::new();
                    for file in results
                        .files
                        .iter()
                        .filter(|file| file.side == self.search_state.results_side())
                    {
                        let hits: Vec<_> = file
                            .hits
                            .iter()
                            .filter(|hit| self.search_state.selected().contains(&hit.id))
                            .collect();
                        if hits.is_empty() {
                            continue;
                        }
                        rows.push(Row::File(file));
                        rows.extend(hits.into_iter().map(|hit| Row::Hit(file, hit)));
                    }
                    if !rows.is_empty() {
                        let row_height = control_extent(ui);
                        egui::ScrollArea::vertical()
                            .id_salt((
                                "folder-search-results-list",
                                self.search_state.results_side(),
                            ))
                            .max_height((height - 18.0).max(30.0))
                            .auto_shrink([false, false])
                            .show_rows(ui, row_height, rows.len(), |ui, range| {
                                for index in range {
                                    match rows[index] {
                                        Row::File(file) => {
                                            let path = self.sources[file.side]
                                                .as_deref()
                                                .and_then(|root| file.path.strip_prefix(root).ok())
                                                .filter(|relative| !relative.as_os_str().is_empty())
                                                .unwrap_or(&file.path);
                                            let name = if path == file.path
                                                && self.sources[file.side].as_ref()
                                                    == Some(&file.path)
                                            {
                                                file.path
                                                    .file_name()
                                                    .unwrap_or(file.path.as_os_str())
                                                    .to_string_lossy()
                                            } else {
                                                path.as_os_str().to_string_lossy()
                                            };
                                            ui.add_sized(
                                                egui::vec2(ui.available_width(), row_height),
                                                egui::Label::new(RichText::new(name).strong())
                                                    .truncate()
                                                    .halign(egui::Align::Min),
                                            )
                                            .on_hover_text(file.path.display().to_string());
                                        }
                                        Row::Hit(file, hit) => {
                                            if search_result_rows::result_row(
                                                ui,
                                                hit,
                                                &file.working_file.text,
                                            ) {
                                                removed.push(hit.id);
                                            }
                                        }
                                    }
                                }
                            });
                        return;
                    }
                }
                let rect = ui.available_rect_before_wrap();
                let mut job = egui::text::LayoutJob::simple(
                    if self.search_state.is_searching() || self.search_state.results().is_none() {
                        "Search results will appear here"
                    } else {
                        "No results on this side"
                    }
                    .into(),
                    FontId::proportional(14.0),
                    palette.muted,
                    rect.width(),
                );
                job.halign = egui::Align::Center;
                let galley = ui.painter().layout_job(job);
                ui.painter().with_clip_rect(rect).galley(
                    rect.center() - galley.rect.center().to_vec2(),
                    galley,
                    palette.muted,
                );
            });
        for id in removed {
            self.search_state.remove_selected(id);
        }
    }
}
const SEARCH_ENTRY_HEIGHT: f32 = 34.0;
// Four-point padding plus a one-point border on each side.
const SEARCH_FIELD_HEIGHT: f32 = SEARCH_ENTRY_HEIGHT + 10.0;

fn search_text_field(
    ui: &mut egui::Ui,
    text: &mut String,
    id: &str,
    placeholder: &str,
    options: Option<[&mut bool; 3]>,
) -> egui::Response {
    let palette = Palette::for_context(ui.ctx());
    let focused = ui.memory(|memory| memory.has_focus(egui::Id::new(id)));
    let frame = egui::Frame::new()
        .fill(palette.background)
        .corner_radius(3)
        .stroke(Stroke::new(
            1.0,
            if focused {
                palette.accent
            } else {
                palette.border
            },
        ))
        .inner_margin(4);
    let response = frame.show(ui, |ui| {
        let extent = control_extent(ui);
        let gap = ui.spacing().item_spacing.x;
        let reserve = if options.is_some() {
            3.0 * (extent + gap)
        } else {
            0.0
        };
        let (row, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), SEARCH_ENTRY_HEIGHT),
            Sense::hover(),
        );
        let field = Rect::from_min_max(
            row.min,
            egui::pos2((row.right() - reserve).max(row.left() + 20.0), row.bottom()),
        );
        let edit = ui
            .scope_builder(egui::UiBuilder::new().max_rect(field), |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(field));
                // TextEdit uses the scoped weak-text color for its hint atoms.
                ui.visuals_mut().weak_text_color = Some(palette.muted.gamma_multiply(0.8));
                ui.add_sized(
                    field.size(),
                    egui::TextEdit::singleline(text)
                        .id(egui::Id::new(id))
                        .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(4, 8)))
                        .font(FontId::proportional(14.0))
                        .hint_text(
                            RichText::new(placeholder)
                                .font(FontId::proportional(14.0))
                                .italics()
                                .color(palette.muted.gamma_multiply(0.8)),
                        ),
                )
            })
            .inner;
        if let Some(options) = options {
            for (index, (value, (icon, label))) in options
                .into_iter()
                .zip([
                    (ToolbarIcon::MatchCase, "Match Case"),
                    (ToolbarIcon::MatchWholeWord, "Match Whole Word"),
                    (ToolbarIcon::UseRegularExpression, "Use Regular Expression"),
                ])
                .enumerate()
            {
                let button = Rect::from_center_size(
                    egui::pos2(
                        field.right() + gap + extent / 2.0 + index as f32 * (extent + gap),
                        row.center().y,
                    ),
                    egui::vec2(extent, extent),
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(button)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        if icon_button_state(ui, icon, true, *value, label).clicked() {
                            *value = !*value;
                        }
                    },
                );
            }
        }
        edit
    });
    ui.interact(
        response.response.rect,
        egui::Id::new((id, "frame")),
        Sense::hover(),
    );
    response.inner
}

fn search_scope_button(
    ui: &mut egui::Ui,
    label: &str,
    color: Color32,
    selected: bool,
) -> egui::Response {
    search_choice_button(
        ui,
        label,
        color,
        selected,
        egui::Id::new(("folder-search-scope", label)),
        &format!("Replace on {label}"),
        None,
    )
}

fn search_choice_button(
    ui: &mut egui::Ui,
    label: &str,
    color: Color32,
    selected: bool,
    id: egui::Id,
    tooltip: &str,
    width: Option<f32>,
) -> egui::Response {
    let enabled = ui.is_enabled();
    let color = if enabled {
        color
    } else {
        Palette::for_context(ui.ctx()).muted.gamma_multiply(0.4)
    };
    let text = ui
        .painter()
        .layout_no_wrap(label.into(), FontId::monospace(11.0), color);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(width.unwrap_or(text.size().x + 16.0), control_extent(ui)),
        Sense::hover(),
    );
    let response = ui.interact(rect, id, Sense::click());
    let palette = Palette::for_context(ui.ctx());
    if selected {
        ui.painter().rect(
            rect,
            3,
            color.gamma_multiply(0.12),
            Stroke::new(1.0, color),
            egui::StrokeKind::Inside,
        );
    } else if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 3, palette.border);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            3,
            Stroke::new(2.0, palette.accent),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter()
        .galley(rect.center() - text.size() * 0.5, text, color);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, label)
    });
    response
        .on_hover_cursor(if enabled {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::Default
        })
        .on_hover_text(tooltip)
}
