//! Fixed-input Git views. Ordinary desktop sessions never enter this app owner.

mod diff;
mod merge;

use crate::app::visuals::theme::{Palette, apply_theme, restore_zoom, set_theme, zoom_controls};
use crate::cli::{GitDiffRequest, GitMergeRequest};
use eframe::egui;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

pub(crate) enum GitLaunch {
    Diff(GitDiffRequest),
    Merge(GitMergeRequest),
}

impl GitLaunch {
    pub(crate) fn initial_exit_code(&self) -> u8 {
        // Closing a read-only Diff advances Git. Merge needs a confirmed save.
        u8::from(matches!(self, Self::Merge(_)))
    }
}

enum GitView {
    Diff(diff::GitDiffView),
    Merge(merge::GitMergeWindow),
}

pub(crate) struct GitApp {
    view: GitView,
    completion: Arc<AtomicU8>,
    context: egui::Context,
    close_prompt: bool,
    closing: bool,
    error: Option<String>,
}

impl GitApp {
    pub(crate) fn new(
        cc: &eframe::CreationContext<'_>,
        launch: GitLaunch,
        completion: Arc<AtomicU8>,
        error: Option<String>,
    ) -> Self {
        apply_theme(&cc.egui_ctx);
        if let Some(storage) = cc.storage {
            restore_zoom(&cc.egui_ctx, storage);
        }
        Self::start(&cc.egui_ctx, launch, completion, error)
    }

    fn start(
        ctx: &egui::Context,
        launch: GitLaunch,
        completion: Arc<AtomicU8>,
        error: Option<String>,
    ) -> Self {
        completion.store(launch.initial_exit_code(), Ordering::Release);
        let view = match launch {
            GitLaunch::Diff(request) => GitView::Diff(diff::GitDiffView::new(request, ctx)),
            GitLaunch::Merge(request) => GitView::Merge(merge::GitMergeWindow::new(request, ctx)),
        };
        Self {
            view,
            completion,
            context: ctx.clone(),
            close_prompt: false,
            closing: false,
            error,
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        match &mut self.view {
            GitView::Diff(view) => {
                view.poll(ctx);
                self.completion.store(view.exit_code(), Ordering::Release);
            }
            GitView::Merge(view) => {
                if view.poll(ctx) {
                    self.completion.store(0, Ordering::Release);
                    self.closing = true;
                    self.close_prompt = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn render(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Versus");
            zoom_controls(ui);
            let dark = ui.ctx().theme() == egui::Theme::Dark;
            if ui
                .button(if dark { "Light theme" } else { "Dark theme" })
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
            }
        });
        if let Some(error) = &self.error {
            ui.colored_label(Palette::for_context(ui.ctx()).error, error);
        }
        ui.separator();
        match &mut self.view {
            GitView::Diff(view) => view.render(ui),
            GitView::Merge(view) => view.render(ui),
        }
        self.close_dialog(ui.ctx());
    }

    fn close_dialog(&mut self, ctx: &egui::Context) {
        let GitView::Merge(view) = &self.view else {
            return;
        };
        if !self.closing && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_prompt = true;
        }
        if !self.close_prompt {
            return;
        }
        egui::Modal::new(egui::Id::new("git-cancel-merge")).show(ctx, |ui| {
            ui.heading("Cancel Git merge?");
            ui.label(if view.is_saving() {
                "Wait for Save and Resolve to finish."
            } else {
                "Cancel returns failure to Git. Git can restore its pre-tool file; edits from this window will not be kept."
            });
            ui.horizontal(|ui| {
                if ui.button("Keep editing").clicked() { self.close_prompt = false; }
                if ui.add_enabled(!view.is_saving(), egui::Button::new("Cancel merge")).clicked() {
                    self.close_prompt = false;
                    self.closing = true;
                    // Completion remains nonzero, even if the file had no initial conflicts.
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        });
    }
}

impl eframe::App for GitApp {
    fn persist_egui_memory(&self) -> bool {
        false
    }
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string("versus.zoom", self.context.zoom_factor().to_string());
    }
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        Palette::new(visuals.dark_mode)
            .background
            .to_normalized_gamma_f32()
    }
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if let Some(error) = crate::diagnostics::take_error() {
            self.error = Some(error);
        }
        self.poll(ctx);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.render(ui);
    }
}

#[cfg(test)]
#[path = "../../tests/unit/app/git/mod.rs"]
mod tests;
