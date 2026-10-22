//! Region selection, model actions, and history shortcuts for Git Merge.

use super::*;

impl MergeView {
    pub(super) fn choose(&mut self, model: &mut GitMergeModel, choice: MergeChoice) {
        if let Some(id) = self.selected_region {
            self.try_action(model.choose(id, model.revision(), choice));
            self.clamp_selected_region(model);
        }
    }

    pub(super) fn step_region(&mut self, model: &GitMergeModel, forward: bool) {
        let regions = model.regions();
        if regions.is_empty() {
            self.selected_region = None;
            return;
        }
        let current = self
            .selected_region
            .and_then(|id| regions.iter().position(|region| region.id == id))
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % regions.len()
        } else {
            (current + regions.len() - 1) % regions.len()
        };
        self.selected_region = Some(regions[next].id);
        self.scroll_to_region = true;
    }

    pub(super) fn clamp_selected_region(&mut self, model: &GitMergeModel) {
        if self
            .selected_region
            .is_some_and(|id| model.regions().iter().all(|region| region.id != id))
        {
            self.selected_region = None;
        }
        if self.selected_region.is_none() {
            self.selected_region = model.regions().first().map(|region| region.id);
            self.scroll_to_region = self.selected_region.is_some();
        }
    }

    pub(super) fn consume_history_shortcuts(
        &mut self,
        ui: &mut egui::Ui,
        model: &mut GitMergeModel,
        saving: bool,
    ) {
        if saving {
            return;
        }
        if ui.input_mut(|input| {
            input.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::Z,
            ) || input.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::Z)
                || input.consume_key(egui::Modifiers::CTRL, egui::Key::Y)
        }) {
            model.redo();
            self.error = None;
        } else if ui.input_mut(|input| {
            input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)
                || input.consume_key(egui::Modifiers::CTRL, egui::Key::Z)
        }) {
            model.undo();
            self.error = None;
        }
    }

    pub(super) fn try_action<E: std::fmt::Display>(&mut self, result: Result<(), E>) {
        match result {
            Ok(()) => self.error = None,
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}

pub(super) fn scroll_offset(text: &str, byte: usize, ui: &egui::Ui) -> f32 {
    let line = super::text::line_number(text, byte) - 1;
    line as f32 * crate::app::visuals::theme::comparison_row_height(ui)
}
