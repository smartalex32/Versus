//! Git merge controls and explicit region actions.

use super::*;

impl MergeView {
    pub(super) fn toolbar(
        &mut self,
        ui: &mut egui::Ui,
        model: &mut GitMergeModel,
        label: &str,
        saving: bool,
        save_requested: &mut bool,
    ) {
        let count = model.regions().len();
        let unresolved = model.unresolved_count();
        let selected = self.selected_region;
        let region_label = selected
            .and_then(|id| model.regions().iter().position(|region| region.id == id))
            .map(|index| format!("Region {} of {count}", index + 1))
            .unwrap_or_else(|| {
                if count == 0 {
                    "No merge regions".to_owned()
                } else {
                    format!("{count} merge regions")
                }
            });
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Git Merge").strong());
            if !label.is_empty() {
                ui.label(egui::RichText::new(label).color(Palette::for_context(ui.ctx()).muted));
            }
            ui.separator();
            let restore_enabled = !saving && model.base().is_some();
            if ui
                .add_enabled(restore_enabled, egui::Button::new("Restore Base"))
                .on_hover_text(if model.base().is_some() {
                    "Replace Result with the original Base as one undoable action"
                } else {
                    "Git did not supply an original Base for this merge"
                })
                .clicked()
            {
                self.try_action(model.restore_base());
            }
            if icon_button(ui, ToolbarIcon::Undo, !saving && model.can_undo(), "Undo").clicked() {
                model.undo();
                self.error = None;
            }
            if icon_button(ui, ToolbarIcon::Redo, !saving && model.can_redo(), "Redo").clicked() {
                model.redo();
                self.error = None;
            }
            ui.separator();
            if icon_button(
                ui,
                ToolbarIcon::Previous,
                count > 0,
                "Previous merge region",
            )
            .clicked()
            {
                self.step_region(model, false);
            }
            ui.label(region_label);
            if icon_button(ui, ToolbarIcon::Next, count > 0, "Next merge region").clicked() {
                self.step_region(model, true);
            }
            ui.separator();
            let conflicts = if unresolved == 1 {
                "1 unresolved conflict".to_owned()
            } else {
                format!("{unresolved} unresolved conflicts")
            };
            ui.label(egui::RichText::new(conflicts).color(if unresolved == 0 {
                Palette::for_context(ui.ctx()).muted
            } else {
                Palette::for_context(ui.ctx()).changed
            }));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !saving && model.can_finish(),
                        egui::Button::new("Save and Resolve"),
                    )
                    .on_hover_text(if model.can_finish() {
                        "Write Result and return success to Git"
                    } else {
                        "Resolve every known conflict before saving"
                    })
                    .clicked()
                {
                    *save_requested = true;
                }
                if ui
                    .button(if self.show_base {
                        "Hide Base"
                    } else {
                        "Show Base"
                    })
                    .clicked()
                {
                    self.show_base = !self.show_base;
                }
            });
        });
    }

    pub(super) fn pane_headers(
        &mut self,
        ui: &mut egui::Ui,
        model: &mut GitMergeModel,
        saving: bool,
    ) {
        let mapping_available = model.mapping_available();
        let selected = self
            .selected_region
            .and_then(|id| model.regions().iter().find(|region| region.id == id))
            .cloned();
        let selected_description = selected.as_ref().map_or_else(
            || "Select a merge region to replace its matching Result range.".to_owned(),
            |region| {
                if region.unresolved {
                    "Unresolved change. Take either source, or edit Result and mark it resolved."
                        .to_owned()
                } else {
                    "Reviewed change. Take either source to replace the highlighted Result."
                        .to_owned()
                }
            },
        );
        let unresolved = selected.as_ref().is_some_and(|region| region.unresolved);
        let scope = selected.as_ref().map(|region| {
            format!(
                "Scope — Incoming {} · Result {} · Current {}",
                super::text::line_scope(model.incoming(), &region.incoming),
                super::text::line_scope(model.result(), &region.result),
                super::text::line_scope(model.current(), &region.current),
            )
        });
        ui.columns(3, |columns| {
            columns[0].horizontal(|ui| {
                ui.label(egui::RichText::new("Incoming (Git Remote)").strong());
                if icon_button(
                    ui,
                    ToolbarIcon::AcceptAllLeft,
                    !saving && mapping_available && self.selected_region.is_some(),
                    "Replace selected Result region with Incoming",
                )
                .clicked()
                {
                    self.choose(model, MergeChoice::Incoming);
                }
            });
            columns[1].vertical(|ui| {
                ui.label(egui::RichText::new("Result (editable)").strong());
                ui.label(
                    egui::RichText::new(selected_description)
                        .small()
                        .color(Palette::for_context(ui.ctx()).muted),
                );
                if ui
                    .add_enabled(!saving && unresolved, egui::Button::new("Mark Resolved"))
                    .on_hover_text(
                        "Confirm that this selected conflict was resolved by manual editing",
                    )
                    .clicked()
                {
                    if let Some(id) = self.selected_region {
                        self.try_action(model.mark_resolved(id, model.revision()));
                    }
                }
            });
            columns[2].horizontal(|ui| {
                if icon_button(
                    ui,
                    ToolbarIcon::AcceptAllRight,
                    !saving && mapping_available && self.selected_region.is_some(),
                    "Replace selected Result region with Current",
                )
                .clicked()
                {
                    self.choose(model, MergeChoice::Current);
                }
                ui.label(egui::RichText::new("Current (Git Local)").strong());
            });
        });
        if let Some(scope) = scope {
            ui.label(
                egui::RichText::new(scope)
                    .small()
                    .color(Palette::for_context(ui.ctx()).accent),
            );
        }
        if self.selected_region.is_some() {
            ui.horizontal_wrapped(|ui| {
                ui.label("Take both:");
                if ui
                    .add_enabled(
                        !saving && mapping_available,
                        egui::Button::new("Incoming then Current"),
                    )
                    .clicked()
                {
                    self.choose(model, MergeChoice::IncomingThenCurrent);
                }
                if ui
                    .add_enabled(
                        !saving && mapping_available,
                        egui::Button::new("Current then Incoming"),
                    )
                    .clicked()
                {
                    self.choose(model, MergeChoice::CurrentThenIncoming);
                }
            });
        }
    }
}
