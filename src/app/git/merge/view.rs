//! Focused presentation for one Git mergetool invocation.
//!
//! This deliberately owns no file loading, saving, or process lifecycle.  The
//! surrounding Git app supplies a [`GitMergeModel`] for a single invocation;
//! keeping those concerns outside this view prevents Git controls from leaking
//! into the ordinary two-working-copy Merge workspace.

use crate::app::interactive_view::projection::editor_byte_edit;
use crate::app::visuals::icons::{ToolbarIcon, icon_button};
use crate::app::visuals::panels::{comparison_panel, paint_panel_divider};
use crate::app::visuals::theme::{Palette, comparison_font};
use crate::app::*;
use versus::core::git_merge::{GitMergeModel, MergeChoice};

mod navigation;
mod panes;
mod text;
mod toolbar;
use navigation::scroll_offset;

/// Stateful, per-window presentation data for Git Merge.
#[derive(Default)]
pub(super) struct MergeView {
    selected_region: Option<usize>,
    show_base: bool,
    error: Option<String>,
    pane_scrolls: [f32; 3],
    base_scroll: f32,
    scroll_to_region: bool,
}

#[cfg(test)]
#[path = "../../../../tests/unit/app/git/merge/view/mod.rs"]
mod tests;

impl MergeView {
    /// Paint the one-file Git Merge workspace.  `true` means the caller should
    /// save the current Result and report a successful mergetool exit code.
    pub(super) fn render(
        &mut self,
        ui: &mut egui::Ui,
        model: &mut GitMergeModel,
        label: &str,
        saving: bool,
    ) -> bool {
        self.consume_history_shortcuts(ui, model, saving);
        self.clamp_selected_region(model);
        let palette = Palette::for_context(ui.ctx());
        let mut save_requested = false;

        egui::Frame::default()
            .fill(palette.background)
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                self.toolbar(ui, model, label, saving, &mut save_requested);
                if let Some(error) = &self.error {
                    ui.add_space(4.0);
                    ui.colored_label(palette.error, error);
                }
                ui.add_space(6.0);
                self.workspace(ui, model, saving);
            });
        save_requested
    }
}
