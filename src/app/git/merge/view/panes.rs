//! Read-only source panes and the byte-preserving Result editor.

use super::*;

impl MergeView {
    pub(super) fn workspace(&mut self, ui: &mut egui::Ui, model: &mut GitMergeModel, saving: bool) {
        comparison_panel(ui).show(ui, |ui| {
            self.pane_headers(ui, model, saving);
            if !model.mapping_available() {
                ui.colored_label(
                    Palette::for_context(ui.ctx()).error,
                    "Source-to-Result mapping is unavailable for this content. Edit Result directly or undo to a mappable revision.",
                );
            }
            paint_panel_divider(ui);
            let height =
                (ui.available_height() - if self.show_base { 190.0 } else { 4.0 }).max(120.0);
            let selected = self
                .selected_region
                .and_then(|id| model.regions().iter().find(|region| region.id == id))
                .cloned();
            let jump = self.scroll_to_region;
            let targets = selected.as_ref().map(|region| {
                [
                    scroll_offset(model.incoming(), region.incoming.start, ui),
                    scroll_offset(model.result(), region.result.start, ui),
                    scroll_offset(model.current(), region.current.start, ui),
                ]
            });
            let mut scrolls = self.pane_scrolls;
            let mut transfer = None;
            let arrows_enabled = !saving && model.mapping_available() && selected.is_some();
            ui.columns(3, |columns| {
                if Self::readonly_pane(
                    &mut columns[0],
                    "git-merge-incoming",
                    model.incoming(),
                    height,
                    "Incoming (Git Remote)",
                    &mut scrolls[0],
                    jump.then(|| targets.map(|target| target[0])).flatten(),
                    selected.as_ref().map(|region| region.incoming.clone()),
                    arrows_enabled.then_some(0),
                ) { transfer = Some(MergeChoice::Incoming); }
                self.result_pane(
                    &mut columns[1],
                    model,
                    height,
                    saving,
                    &mut scrolls[1],
                    jump.then(|| targets.map(|target| target[1])).flatten(),
                );
                if Self::readonly_pane(
                    &mut columns[2],
                    "git-merge-current",
                    model.current(),
                    height,
                    "Current (Git Local)",
                    &mut scrolls[2],
                    jump.then(|| targets.map(|target| target[2])).flatten(),
                    selected.as_ref().map(|region| region.current.clone()),
                    arrows_enabled.then_some(1),
                ) { transfer = Some(MergeChoice::Current); }
            });
            if let Some(choice) = transfer { self.choose(model, choice); }
            self.pane_scrolls = scrolls;
            if self.show_base {
                ui.add_space(6.0);
                paint_panel_divider(ui);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Original Base (read-only reference)").strong());
                    if model.base().is_none() {
                        ui.label(
                            egui::RichText::new("Git did not provide a Base file.")
                                .color(Palette::for_context(ui.ctx()).muted),
                        );
                    }
                });
                if let Some(base) = model.base() {
                    let target = selected.as_ref().and_then(|region| {
                        region
                            .base
                            .as_ref()
                            .map(|range| scroll_offset(base, range.start, ui))
                    });
                    Self::readonly_pane(
                        ui,
                        "git-merge-base",
                        base,
                        150.0,
                        "Original Base",
                        &mut self.base_scroll,
                        jump.then_some(target).flatten(),
                        selected.as_ref().and_then(|region| region.base.clone()),
                        None,
                    );
                }
            }
            self.scroll_to_region = false;
        });
    }

    pub(super) fn readonly_pane(
        ui: &mut egui::Ui,
        id: &'static str,
        text: &str,
        height: f32,
        label: &str,
        scroll: &mut f32,
        target: Option<f32>,
        selected: Option<std::ops::Range<usize>>,
        transfer_side: Option<usize>,
    ) -> bool {
        let output = egui::ScrollArea::both()
            .id_salt((id, "scroll"))
            .auto_shrink([false, false])
            .max_height(height)
            .vertical_scroll_offset(target.unwrap_or(*scroll))
            .show(ui, |ui| {
                let selected = selected.map(|range| super::text::display_range(text, range));
                let mut display = text.replace("\r\n", "\n").replace('\r', "\n");
                let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _: f32| {
                    super::text::layout(ui, buffer.as_str(), selected.as_ref())
                };
                let width = ui.available_width() - if transfer_side.is_some() { 34.0 } else { 0.0 };
                let mut arrow_clicked = false;
                ui.horizontal_top(|ui| {
                    let left_gutter = (transfer_side == Some(1)).then(|| {
                        ui.allocate_exact_size(egui::vec2(30.0, height), egui::Sense::hover())
                            .0
                    });
                    let response = ui
                        .add(
                            egui::TextEdit::multiline(&mut display)
                                .id_source(id)
                                .font(comparison_font(ui.ctx()))
                                .code_editor()
                                .layouter(&mut layouter)
                                .interactive(false)
                                .desired_width(width.max(80.0))
                                .desired_rows(1)
                                .min_size(egui::vec2(80.0, height)),
                        )
                        .on_hover_text(format!("{label}; read-only"));
                    let gutter = left_gutter.or_else(|| {
                        (transfer_side == Some(0)).then(|| {
                            ui.allocate_exact_size(egui::vec2(30.0, height), egui::Sense::hover())
                                .0
                        })
                    });
                    if let (Some(side), Some(gutter), Some(range)) =
                        (transfer_side, gutter, selected.as_ref())
                    {
                        let y = response.rect.top()
                            + (super::text::line_number(&display, range.start) - 1) as f32
                                * crate::app::visuals::theme::comparison_row_height(ui);
                        let rect = egui::Rect::from_min_size(
                            egui::pos2(gutter.left(), y),
                            egui::vec2(30.0, crate::app::visuals::theme::comparison_row_height(ui)),
                        );
                        arrow_clicked = crate::app::interactive_view::transfer_action(
                            ui,
                            rect,
                            egui::Id::new((id, "take-selected")),
                            side,
                            if side == 0 {
                                "Take Incoming for the highlighted change"
                            } else {
                                "Take Current for the highlighted change"
                            },
                        )
                        .clicked();
                    }
                });
                arrow_clicked
            });
        *scroll = output.state.offset.y;
        output.inner
    }

    pub(super) fn result_pane(
        &mut self,
        ui: &mut egui::Ui,
        model: &mut GitMergeModel,
        height: f32,
        saving: bool,
        scroll: &mut f32,
        target: Option<f32>,
    ) {
        let original = model.result().to_owned();
        let mut displayed = original.replace("\r\n", "\n").replace('\r', "\n");
        // The model owns the complete Result/conflict history.  Clear egui's
        // buffer-local undoer after every model publication so Cmd/Ctrl-Z is
        // always handled by `consume_history_shortcuts` above.
        let editor_id = egui::Id::new("git-merge-result");
        let mut prior_selection = None;
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), editor_id) {
            prior_selection = state.cursor.char_range().map(|range| {
                let range = range.as_sorted_char_range();
                usize::from(range.start)..usize::from(range.end)
            });
            state.clear_undoer();
            state.store(ui.ctx(), editor_id);
        }
        let selected = self
            .selected_region
            .and_then(|id| model.regions().iter().find(|region| region.id == id))
            .map(|region| region.result.clone());
        let highlight = selected.map(|range| super::text::display_range(&original, range));
        let font = comparison_font(ui.ctx());
        let response = egui::ScrollArea::both()
            .id_salt("git-merge-result-scroll")
            .auto_shrink([false, false])
            .max_height(height)
            .vertical_scroll_offset(target.unwrap_or(*scroll))
            .show(ui, |ui| {
                let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _: f32| {
                    super::text::layout(ui, buffer.as_str(), highlight.as_ref())
                };
                ui.add_enabled_ui(!saving, |ui| {
                    egui::TextEdit::multiline(&mut displayed)
                        .id(editor_id)
                        .font(font.clone())
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(1)
                        .min_size(egui::vec2(80.0, height))
                        .layouter(&mut layouter)
                        .show(ui)
                })
                .inner
            });
        *scroll = response.state.offset.y;
        if response.inner.response.changed() {
            let (range, replacement) = editor_byte_edit(&original, &displayed, prior_selection);
            let mut updated = original;
            updated.replace_range(range, &replacement);
            self.try_action(model.edit_result(updated));
        }
    }
}
