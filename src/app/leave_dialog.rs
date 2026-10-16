//! Explicit unsaved-change protection before leaving a Merge session.

use super::*;

impl VersusApp {
    pub(super) fn merge_exit_dialog(&mut self, ctx: &egui::Context) {
        self.session.sync_file_search_when_clean();
        if ctx.input(|input| input.viewport().close_requested())
            && (self
                .session
                .interactive
                .as_ref()
                .is_some_and(InteractiveView::any_dirty)
                || self
                    .session
                    .merge_folders
                    .as_ref()
                    .is_some_and(FolderBrowser::any_dirty)
                || self
                    .session
                    .interactive
                    .as_ref()
                    .is_some_and(InteractiveView::is_saving)
                || self
                    .session
                    .merge_folders
                    .as_ref()
                    .is_some_and(FolderBrowser::is_saving)
                || self
                    .session
                    .merge_file_search
                    .as_ref()
                    .is_some_and(FolderBrowser::is_saving))
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.session.pending_merge_exit = Some(MergeExit::Close);
        }
        let Some(destination) = self.session.pending_merge_exit.clone() else {
            return;
        };
        let saving = self
            .session
            .interactive
            .as_ref()
            .is_some_and(InteractiveView::is_saving)
            || self
                .session
                .merge_folders
                .as_ref()
                .is_some_and(FolderBrowser::is_saving)
            || self
                .session
                .merge_file_search
                .as_ref()
                .is_some_and(FolderBrowser::is_saving);
        let mut discard = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("leave-merge")).show(ctx, |ui| {
            ui.heading("Leave merge?");
            ui.label(if saving {
                "Wait for the save to finish before leaving."
            } else {
                "The result has unsaved changes. Save it before leaving, or discard those changes."
            });
            ui.horizontal(|ui| {
                cancel = ui.button("Keep merging").clicked();
                discard = ui
                    .add_enabled(!saving, egui::Button::new("Discard and leave"))
                    .clicked();
            });
        });
        if cancel {
            self.session.pending_merge_exit = None;
        } else if discard {
            self.session.pending_merge_exit = None;
            self.session.interactive = None;
            self.session.merge_folders = None;
            self.session.merge_file_search = None;
            self.session.merge_file_open = false;
            self.session.merge_file_sides = [false; 2];
            match destination {
                MergeExit::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                MergeExit::New => self.session.new_comparison(),
                MergeExit::Mode(mode) => self.session.switch_comparison_mode(mode),
                MergeExit::Recent(entry) => self.session.open_recent(entry),
            }
        }
    }
}
