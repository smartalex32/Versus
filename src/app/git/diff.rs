//! State and lifecycle for the isolated Git difftool view.
mod file;
mod folder;
mod loading;

use crate::app::visuals::panels::split_rect;
use crate::app::visuals::theme::Palette;
use crate::cli::GitDiffRequest;
use eframe::egui::{self, RichText, Sense, Stroke};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, TryRecvError},
    },
    time::Duration,
};
use versus::{DirectoryEntryState, FileComparison, FolderTree};

pub(super) struct GitDiffView {
    request: GitDiffRequest,
    options: Options,
    screen: Screen,
    job: Option<Job>,
    search: String,
}
#[derive(Clone, Copy)]
pub(super) struct Options {
    differences_only: bool,
    ignore_whitespace: bool,
    ignore_line_endings: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            differences_only: true,
            ignore_whitespace: false,
            ignore_line_endings: true,
        }
    }
}
enum Screen {
    Loading,
    Error(String),
    File(FileScreen),
    Folder(FolderScreen),
}
struct FileScreen {
    paths: [PathBuf; 2],
    comparison: FileComparison,
    navigation: Option<usize>,
    back: Option<Box<FolderScreen>>,
    sources: [bool; 2],
    scroll_y: f32,
    pending_navigation: bool,
    content_columns: Option<[usize; 2]>,
}
struct FolderScreen {
    tree: FolderTree,
    selected: Option<PathBuf>,
    error: Option<String>,
}
struct Job {
    receiver: Receiver<Result<Loaded, String>>,
    cancellation: Arc<AtomicBool>,
    target: Option<Target>,
}
enum Target {
    Initial,
    FolderFile {
        previous: FolderScreen,
        paths: [PathBuf; 2],
        sources: [bool; 2],
    },
}
enum Loaded {
    File(FileComparison),
    Folder(FolderTree),
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancellation.store(true, Ordering::Relaxed);
    }
}

impl GitDiffView {
    pub(super) fn new(request: GitDiffRequest, ctx: &egui::Context) -> Self {
        let mut view = Self {
            request,
            options: Options::default(),
            screen: Screen::Loading,
            job: None,
            search: String::new(),
        };
        view.start_initial(ctx);
        view
    }
    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.job else { return };
        match job.receiver.try_recv() {
            Ok(result) => {
                let mut job = self.job.take().expect("Git Diff job was present");
                if job.cancellation.load(Ordering::Relaxed) {
                    self.screen = Screen::Error("Comparison cancelled.".into());
                    return;
                }
                let target = job.target.take().expect("Git Diff job target was present");
                match (target, result) {
                    (Target::Initial, Ok(Loaded::File(comparison))) => {
                        self.screen = Screen::File(FileScreen {
                            paths: self.request.paths.clone(),
                            comparison,
                            navigation: None,
                            back: None,
                            sources: loading::source_presence(&self.request.paths)
                                .map(|x| x.is_some()),
                            scroll_y: 0.0,
                            pending_navigation: false,
                            content_columns: None,
                        })
                    }
                    (Target::Initial, Ok(Loaded::Folder(tree))) => {
                        self.screen = Screen::Folder(FolderScreen {
                            tree,
                            selected: None,
                            error: None,
                        })
                    }
                    (
                        Target::FolderFile {
                            paths,
                            previous,
                            sources,
                        },
                        Ok(Loaded::File(comparison)),
                    ) => {
                        self.screen = Screen::File(FileScreen {
                            paths,
                            comparison,
                            navigation: None,
                            back: Some(Box::new(previous)),
                            sources,
                            scroll_y: 0.0,
                            pending_navigation: false,
                            content_columns: None,
                        })
                    }
                    (Target::FolderFile { mut previous, .. }, Err(error)) => {
                        previous.error = Some(error.clone());
                        self.search.clear();
                        crate::diagnostics::error(format!(
                            "Git Diff file comparison failed: {error}"
                        ));
                        self.screen = Screen::Folder(previous)
                    }
                    (Target::Initial, Err(error)) => self.screen = Screen::Error(error),
                    _ => {
                        self.screen = Screen::Error(
                            "The comparison worker returned an unexpected result.".into(),
                        )
                    }
                }
            }
            Err(TryRecvError::Disconnected) => {
                self.job.take();
                self.screen = Screen::Error("The comparison worker stopped unexpectedly.".into())
            }
            Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(60)),
        }
    }
    pub(super) fn render(&mut self, ui: &mut egui::Ui) {
        self.toolbar(ui);
        let screen = std::mem::replace(&mut self.screen, Screen::Loading);
        self.screen = match screen {
            Screen::Loading => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Loading Git comparison…");
                });
                Screen::Loading
            }
            Screen::Error(error) => {
                ui.colored_label(Palette::for_context(ui.ctx()).error, &error);
                Screen::Error(error)
            }
            Screen::File(mut file) => {
                if self.render_file(ui, &mut file) {
                    file.back
                        .take()
                        .map_or(Screen::File(file), |folder| Screen::Folder(*folder))
                } else {
                    Screen::File(file)
                }
            }
            Screen::Folder(mut folder) => match self.render_folder(ui, &mut folder) {
                Some((paths, exists)) => {
                    self.start_folder_file(paths, exists, folder, ui.ctx());
                    Screen::Loading
                }
                None => Screen::Folder(folder),
            },
        }
    }
    pub(super) fn exit_code(&self) -> u8 {
        u8::from(matches!(self.screen, Screen::Error(_)))
    }
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        ui.horizontal_wrapped(|ui| {
            ui.strong("Git Diff");
            if let Some(label) = &self.request.label {
                ui.label(RichText::new(label).color(palette.muted));
            }
            ui.separator();
            ui.checkbox(&mut self.options.differences_only, "Differences only");
            let whitespace = ui
                .checkbox(&mut self.options.ignore_whitespace, "Ignore whitespace")
                .changed();
            let endings = ui
                .checkbox(&mut self.options.ignore_line_endings, "Ignore line endings")
                .changed();
            if whitespace || endings {
                self.reload_current_comparison(ui.ctx())
            }
            ui.separator();
            ui.label("Search");
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .desired_width(180.0)
                    .hint_text("Text or path"),
            );
        });
        ui.add_space(6.0)
    }
    fn start_initial(&mut self, ctx: &egui::Context) {
        if let Some(job) = self.job.take() {
            job.cancellation.store(true, Ordering::Relaxed)
        }
        self.screen = Screen::Loading;
        self.job = Some(loading::spawn_initial_job(
            self.request.paths.clone(),
            self.request.mode,
            self.options,
        ));
        ctx.request_repaint()
    }
    fn start_folder_file(
        &mut self,
        paths: [PathBuf; 2],
        exists: [bool; 2],
        previous: FolderScreen,
        ctx: &egui::Context,
    ) {
        self.screen = Screen::Loading;
        self.job = Some(loading::spawn_folder_file_job(
            paths,
            exists,
            self.options,
            previous,
        ));
        ctx.request_repaint()
    }
    fn reload_current_comparison(&mut self, ctx: &egui::Context) {
        let screen = std::mem::replace(&mut self.screen, Screen::Loading);
        if let Screen::File(mut file) = screen {
            if let Some(previous) = file.back.take() {
                self.start_folder_file(file.paths, file.sources, *previous, ctx);
                return;
            }
        }
        self.start_initial(ctx)
    }
}

fn source_headers(ui: &mut egui::Ui, paths: &[PathBuf; 2], label: Option<&str>) {
    let palette = Palette::for_context(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), Sense::hover());
    for (side, half) in split_rect(rect).into_iter().enumerate() {
        let role = if side == 0 { "LEFT" } else { "RIGHT" };
        let name = label.map_or_else(
            || paths[side].display().to_string(),
            |label| format!("{label} — {}", paths[side].display()),
        );
        ui.painter().with_clip_rect(half.shrink(6.0)).text(
            half.left_center() + egui::vec2(7.0, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("{role}  {name}"),
            egui::FontId::proportional(12.0),
            palette.text,
        );
    }
    ui.painter().line_segment(
        [rect.center_top(), rect.center_bottom()],
        Stroke::new(1.0, palette.border),
    );
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, palette.border),
    );
}
fn state_label(state: &DirectoryEntryState) -> &'static str {
    match state {
        DirectoryEntryState::Same => "identical",
        DirectoryEntryState::Different => "different",
        DirectoryEntryState::LeftOnly => "only on left",
        DirectoryEntryState::RightOnly => "only on right",
        DirectoryEntryState::TypeMismatch => "type mismatch",
        DirectoryEntryState::Error(_) => "read error",
    }
}
#[cfg(test)]
#[path = "../../../tests/unit/app/git/diff/mod.rs"]
mod tests;
