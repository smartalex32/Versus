//! One Git merge invocation: immutable inputs, result save baseline, async IO.

mod view;

use crate::cli::GitMergeRequest;
use crate::selection::absolute_native_path;
use eframe::egui;
use std::{
    path::Path,
    sync::mpsc::{self, Receiver, TryRecvError},
};
use versus::core::git_merge::GitMergeModel;
use versus::{FileFingerprint, WorkingFile, load_working_file, save_working_file};

#[derive(Debug)]
struct MergeSession {
    model: GitMergeModel,
    output: WorkingFile,
}

impl MergeSession {
    fn load(request: GitMergeRequest) -> Result<Self, String> {
        let load = |path| required_file(&absolute_native_path(path));
        let base = request.base.map(load).transpose()?;
        let incoming = load(request.remote)?;
        let current = load(request.local)?;
        let output = load(request.output)?;
        for input in [&incoming, &current].into_iter().chain(base.as_ref()) {
            if input.identity == output.identity {
                return Err(
                    "Result must have a different destination from Base, Incoming, and Current."
                        .into(),
                );
            }
        }
        let model = GitMergeModel::new(
            base.map(|file| file.text),
            incoming.text,
            current.text,
            output.text.clone(),
        )
        .map_err(|error| error.to_string())?;
        Ok(Self { model, output })
    }

    fn save_candidate(&self) -> Result<WorkingFile, String> {
        if !self.model.can_finish() {
            return Err("Resolve every known conflict before saving the result.".into());
        }
        let mut candidate = self.output.clone();
        candidate.text = self.model.result().to_owned();
        Ok(candidate)
    }
}

fn required_file(path: &Path) -> Result<WorkingFile, String> {
    let file = load_working_file(Some(path)).map_err(|error| error.to_string())?;
    if file.fingerprint == FileFingerprint::Missing {
        return Err(format!(
            "Required Git input does not exist: {}",
            path.display()
        ));
    }
    Ok(file)
}

pub(super) struct GitMergeWindow {
    label: String,
    loading: Option<Receiver<Result<MergeSession, String>>>,
    session: Option<MergeSession>,
    saving: Option<Receiver<Result<WorkingFile, String>>>,
    view: view::MergeView,
    error: Option<String>,
}

impl GitMergeWindow {
    pub(super) fn new(request: GitMergeRequest, ctx: &egui::Context) -> Self {
        let label = request
            .label
            .clone()
            .unwrap_or_else(|| request.output.display().to_string());
        let (sender, receiver) = mpsc::channel();
        let context = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(MergeSession::load(request));
            context.request_repaint();
        });
        Self {
            label,
            loading: Some(receiver),
            session: None,
            saving: None,
            view: Default::default(),
            error: None,
        }
    }

    pub(super) fn is_saving(&self) -> bool {
        self.saving.is_some()
    }

    /// Returns true only once, after the protected save has actually succeeded.
    pub(super) fn poll(&mut self, ctx: &egui::Context) -> bool {
        if let Some(receiver) = &self.loading {
            match receiver.try_recv() {
                Ok(result) => {
                    self.loading = None;
                    match result {
                        Ok(session) => self.session = Some(session),
                        Err(error) => self.error = Some(error),
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.loading = None;
                    self.error = Some("The Git input loader stopped before completing.".into());
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(receiver) = &self.saving {
            match receiver.try_recv() {
                Ok(result) => {
                    self.saving = None;
                    match result {
                        Ok(output) => {
                            if let Some(session) = &mut self.session {
                                session.output = output;
                            }
                            return true;
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.saving = None;
                    self.error =
                        Some("The save worker stopped before confirming completion.".into());
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if self.loading.is_some() || self.saving.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        false
    }

    fn start_save(&mut self, ctx: &egui::Context) {
        if self.is_saving() {
            return;
        }
        let Some(session) = &self.session else {
            return;
        };
        let mut output = match session.save_candidate() {
            Ok(output) => output,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        let context = ctx.clone();
        self.error = None;
        self.saving = Some(receiver);
        std::thread::spawn(move || {
            let result = save_working_file(&mut output)
                .map(|()| output)
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
            context.request_repaint();
        });
    }

    pub(super) fn render(&mut self, ui: &mut egui::Ui) {
        if let Some(error) = &self.error {
            ui.colored_label(
                crate::app::visuals::theme::Palette::for_context(ui.ctx()).error,
                error,
            );
        }
        if self.loading.is_some() {
            ui.heading(format!("Git Merge — {}", self.label));
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading Git snapshots…");
            });
        }
        let saving = self.is_saving();
        if let Some(session) = &mut self.session {
            if self
                .view
                .render(ui, &mut session.model, &self.label, saving)
            {
                self.start_save(ui.ctx());
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/app/git/merge/mod.rs"]
mod tests;
