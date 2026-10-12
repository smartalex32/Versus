//! GUI merge workflow. Input reads and output writes run on cancellable workers.
use super::*;
use std::ops::Range;
use versus::{
    LineEnding, MergeAction, MergeActionKind, MergeSide, MergeWorkspace, SaveOptions,
    load_merge_workspace, save_text_safely,
};

struct LoadJob {
    receiver: Receiver<Result<Option<MergeWorkspace>, CompareError>>,
    cancellation: Arc<AtomicBool>,
    display_paths: [String; 3],
    native_paths: [Option<PathBuf>; 3],
}

impl Drop for LoadJob {
    fn drop(&mut self) {
        self.cancellation.store(true, Ordering::Relaxed);
    }
}

enum SaveEvent {
    Overwrite(PathBuf, String),
    Complete(PathBuf, String),
    Failed(String),
}

#[derive(Clone, Copy)]
enum ReplaceMerge {
    Open,
    New,
}

#[derive(Default)]
pub(super) struct MergeView {
    // Incoming, optional base, Current. Native picker paths retain non-UTF8 bytes.
    paths: [String; 3],
    native_paths: [Option<PathBuf>; 3],
    loaded_paths: Option<[String; 3]>,
    loaded_native_paths: Option<[Option<PathBuf>; 3]>,
    workspace: Option<MergeWorkspace>,
    load: Option<LoadJob>,
    save: Option<Receiver<SaveEvent>>,
    overwrite: Option<(PathBuf, String)>,
    replace: Option<ReplaceMerge>,
    output: Option<PathBuf>,
    message: String,
    error: Option<String>,
    generation: u64,
    scroll_y: f32,
    start_requested: bool,
    path_edits: [bool; 3],
    finish_requested: bool,
    finished: bool,
    pane_rects: Option<[Rect; 3]>,
    drop_hover_side: Option<usize>,
    drop_job: Option<(SourceJob, usize, PathBuf)>,
    navigation_row: Option<usize>,
    navigation_scroll_pending: bool,
    row_height: f32,
}

impl MergeView {
    pub(super) fn set_sources(&mut self, sources: [Option<PathBuf>; 2]) {
        for (index, path) in [0, 2].into_iter().zip(sources) {
            if let Some(path) = path {
                self.paths[index] = path.to_string_lossy().into_owned();
                self.native_paths[index] = Some(path);
            }
        }
        self.start_requested = true;
    }

    pub(super) fn is_finished(&self) -> bool {
        self.finished
    }

    pub(super) fn is_saving(&self) -> bool {
        self.save.is_some() || self.overwrite.is_some()
    }

    pub(super) fn needs_guard(&self) -> bool {
        self.is_saving()
            || self
                .workspace
                .as_ref()
                .is_some_and(MergeWorkspace::is_dirty)
    }

    fn source_path(&self, index: usize) -> PathBuf {
        self.native_paths[index]
            .clone()
            .unwrap_or_else(|| PathBuf::from(&self.paths[index]))
    }

    fn open(&mut self, ctx: &egui::Context) {
        let incoming = self.source_path(0);
        let current = self.source_path(2);
        let base = Some(if self.paths[1].is_empty() {
            current.clone()
        } else {
            self.source_path(1)
        });
        let cancellation = Arc::new(AtomicBool::new(false));
        let cancelled = cancellation.clone();
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = load_merge_workspace(base.as_deref(), &incoming, &current, &cancelled);
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.load = Some(LoadJob {
            receiver,
            cancellation,
            display_paths: self.paths.clone(),
            native_paths: self.native_paths.clone(),
        });
        self.error = None;
        self.message = "Loading merge inputs…".into();
    }

    fn request_replace(&mut self, operation: ReplaceMerge, ctx: &egui::Context) {
        if self.needs_guard() {
            self.replace = Some(operation);
        } else {
            self.replace_now(operation, ctx);
        }
    }

    fn replace_now(&mut self, operation: ReplaceMerge, ctx: &egui::Context) {
        match operation {
            ReplaceMerge::Open => self.open(ctx),
            ReplaceMerge::New => *self = Self::default(),
        }
    }

    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        let dropped =
            self.drop_job
                .as_ref()
                .and_then(|(job, _, _)| match job.receiver.try_recv() {
                    Ok(result) => Some(result),
                    Err(TryRecvError::Disconnected) => {
                        Some(Err("Cannot inspect dropped file: worker stopped.".into()))
                    }
                    Err(TryRecvError::Empty) => None,
                });
        if let Some(result) = dropped {
            let (_, side, path) = self.drop_job.take().unwrap();
            match result {
                Ok(SourceKind::File) => {
                    self.load = None;
                    self.paths[side] = path.to_string_lossy().into_owned();
                    self.native_paths[side] = Some(path);
                    self.path_edits[side] = false;
                    self.start_requested = true;
                    self.error = None;
                }
                Ok(_) => {
                    self.error =
                        Some("Merge requires a regular file, not a folder or symlink.".into())
                }
                Err(error) => self.error = Some(error),
            }
        }
        let loaded = self
            .load
            .as_ref()
            .and_then(|job| match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => Some(Err(CompareError {
                    path: None,
                    kind: versus::CompareErrorKind::Io,
                    message: "Merge worker stopped.".into(),
                })),
                Err(TryRecvError::Empty) => None,
            });
        if let Some(result) = loaded {
            let display_paths = self.load.as_ref().unwrap().display_paths.clone();
            let native_paths = self.load.as_ref().unwrap().native_paths.clone();
            self.load = None;
            match result {
                Ok(Some(workspace)) => {
                    self.workspace = Some(workspace);
                    self.loaded_paths = Some(display_paths);
                    self.loaded_native_paths = Some(native_paths);
                    self.output = None;
                    self.generation += 1;
                    self.scroll_y = 0.0;
                    self.navigation_row = None;
                    self.navigation_scroll_pending = false;
                    self.message = "Choose arrows or edit the result, then Save.".into();
                }
                Ok(None) => {
                    self.restore_loaded_sources();
                    self.message = "Merge loading cancelled.".into();
                }
                Err(error) => {
                    self.restore_loaded_sources();
                    self.error = Some(error.to_string());
                }
            }
        }
        let saved = self
            .save
            .as_ref()
            .and_then(|receiver| match receiver.try_recv() {
                Ok(event) => Some(event),
                Err(TryRecvError::Disconnected) => {
                    Some(SaveEvent::Failed("Save worker stopped.".into()))
                }
                Err(TryRecvError::Empty) => None,
            });
        if let Some(event) = saved {
            self.save = None;
            self.accept_save_event(event);
        }
        if self.load.is_some() || self.save.is_some() || self.drop_job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    fn accept_save_event(&mut self, event: SaveEvent) {
        match event {
            SaveEvent::Overwrite(path, contents) => self.overwrite = Some((path, contents)),
            SaveEvent::Complete(path, contents) => {
                if let Some(workspace) = &mut self.workspace {
                    workspace.mark_saved_result(&contents);
                }
                self.message = format!("Saved {}", path.display());
                self.output = Some(path);
                self.error = None;
                if self.finish_requested {
                    self.finished = self
                        .workspace
                        .as_ref()
                        .is_some_and(|workspace| !workspace.is_dirty());
                    self.finish_requested = false;
                    if !self.finished {
                        self.message =
                            "Saved snapshot. Finish Merge again to save the latest edits.".into();
                    }
                }
            }
            SaveEvent::Failed(error) => {
                self.error = Some(error);
                self.finish_requested = false;
            }
        }
    }

    fn save_result(&mut self, ctx: &egui::Context, save_as: bool) {
        if self.is_saving() {
            return;
        }
        let Some(workspace) = &self.workspace else {
            return;
        };
        let path = if save_as || self.output.is_none() {
            let mut dialog = rfd::FileDialog::new().set_title("Save merged result");
            if let Some(path) = &self.output {
                if let Some(parent) = path.parent() {
                    dialog = dialog.set_directory(parent);
                }
                if let Some(name) = path.file_name() {
                    dialog = dialog.set_file_name(name.to_string_lossy());
                }
            }
            dialog.save_file()
        } else {
            self.output.clone()
        };
        if let Some(path) = path {
            self.start_save(ctx, path, workspace.result().to_owned(), false);
        }
    }

    fn start_save(
        &mut self,
        ctx: &egui::Context,
        path: PathBuf,
        contents: String,
        overwrite: bool,
    ) {
        self.error = None;
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let event = if !overwrite && std::fs::symlink_metadata(&path).is_ok() {
                SaveEvent::Overwrite(path, contents)
            } else {
                match save_text_safely(
                    &path,
                    &contents,
                    &SaveOptions {
                        overwrite,
                        line_ending: LineEnding::Preserve,
                    },
                ) {
                    Ok(()) => SaveEvent::Complete(path, contents),
                    Err(error) => SaveEvent::Failed(error.to_string()),
                }
            };
            let _ = sender.send(event);
            ctx.request_repaint();
        });
        self.save = Some(receiver);
    }

    fn ready(&self) -> bool {
        !self.paths[0].is_empty()
            && !self.paths[2].is_empty()
            && self.load.is_none()
            && !self.is_saving()
            && self.drop_job.is_none()
    }

    fn restore_loaded_sources(&mut self) {
        if let Some(paths) = &self.loaded_paths {
            self.paths = paths.clone();
            self.native_paths = self.loaded_native_paths.clone().unwrap_or_default();
            self.path_edits = [false; 3];
            self.start_requested = false;
        }
    }

    pub(super) fn drag_and_drop(&mut self, ui: &mut egui::Ui, native_position: Option<Pos2>) {
        let (hovering, position, dropped) = native_drop_frame(ui, native_position);
        let side = position.and_then(|position| {
            self.pane_rects
                .and_then(|rects| rects.iter().position(|rect| rect.contains(position)))
        });
        if hovering {
            if cfg!(windows) {
                ui.ctx().request_repaint_after(Duration::from_millis(33));
            }
            if position.is_some() {
                self.drop_hover_side = side;
            }
            if let Some(side) = side.or(self.drop_hover_side) {
                if let Some(rects) = self.pane_rects {
                    paint_native_drop_highlight(
                        ui.ctx(),
                        rects[side],
                        &format!(
                            "Drop a file on {}",
                            ["LEFT (Incoming)", "BASE", "RIGHT (Current)"][side]
                        ),
                    );
                }
            }
        }
        if !dropped.is_empty() {
            let target = if position.is_some() {
                side
            } else {
                self.drop_hover_side
            };
            if self.is_saving() {
                self.error =
                    Some("Wait for the save to finish before replacing a merge input.".into());
            } else if dropped.len() != 1 {
                self.error = Some("Drop one file at a time.".into());
            } else if dropped[0].path().as_os_str().is_empty() {
                self.error =
                    Some("This drag did not include a file path. Use Browse to select it.".into());
            } else if let Some(side) = target {
                let path = dropped[0].path().to_path_buf();
                let inspected = path.clone();
                let cancellation = Arc::new(AtomicBool::new(false));
                let cancelled = cancellation.clone();
                let (sender, receiver) = mpsc::channel();
                let ctx = ui.ctx().clone();
                std::thread::spawn(move || {
                    let result = selection::classify_path(&inspected);
                    if !cancelled.load(Ordering::Relaxed) {
                        let _ = sender.send(result);
                        ctx.request_repaint();
                    }
                });
                self.drop_job = Some((
                    SourceJob {
                        receiver,
                        cancellation,
                    },
                    side,
                    path,
                ));
            } else {
                self.error = Some("Drop into the LEFT, BASE, or RIGHT pane.".into());
            }
            ui.ctx().request_repaint();
        }
        if !hovering {
            self.drop_hover_side = None;
        }
    }

    fn jump_difference(&mut self, forward: bool) {
        let Some(workspace) = &self.workspace else {
            return;
        };
        let top = (self.scroll_y / self.row_height.max(1.0)).floor() as usize;
        let anchor = self.navigation_row.unwrap_or(top);
        let actions = workspace.actions();
        let target = if forward {
            actions.iter().find(|action| {
                if self.navigation_row.is_some() {
                    action.row_range.start > anchor
                } else {
                    action.row_range.start >= anchor
                }
            })
        } else {
            actions
                .iter()
                .rev()
                .find(|action| action.row_range.start < anchor)
        };
        if let Some(target) = target {
            self.navigation_row = Some(target.row_range.start);
            self.navigation_scroll_pending = true;
        }
    }

    pub(super) fn header_controls(&mut self, ui: &mut egui::Ui) {
        if icon_button(ui, ToolbarIcon::Refresh, self.ready(), "Refresh merge").clicked() {
            self.request_replace(ReplaceMerge::Open, ui.ctx());
        }
        if self.load.is_some()
            && icon_button(ui, ToolbarIcon::Cancel, true, "Cancel merge loading").clicked()
        {
            self.load = None;
            self.restore_loaded_sources();
            self.message = "Merge loading cancelled.".into();
        }
    }

    fn finish_merge(&mut self, ctx: &egui::Context) {
        if self.workspace.is_none()
            || self.is_saving()
            || self.load.is_some()
            || self.drop_job.is_some()
        {
            return;
        }
        if self.output.is_some() && !self.workspace.as_ref().unwrap().is_dirty() {
            self.finished = true;
        } else {
            self.finish_requested = true;
            self.save_result(ctx, false);
            if !self.is_saving() {
                self.finish_requested = false;
            }
        }
    }

    pub(super) fn render(&mut self, ui: &mut egui::Ui) {
        self.poll(ui.ctx());
        let palette = Palette::for_context(ui.ctx());
        let can_edit = self.workspace.is_some() && self.load.is_none();
        let undo_shortcut = can_edit
            && ui.memory(|memory| {
                memory.has_focus(egui::Id::new(("merge-result", self.generation)))
            })
            && ui.input_mut(|input| {
                input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)
                    || input.consume_key(egui::Modifiers::CTRL, egui::Key::Z)
            });
        if undo_shortcut {
            self.workspace.as_mut().unwrap().undo();
            self.error = None;
        }
        if can_edit
            && !self.is_saving()
            && ui.input_mut(|input| {
                input.consume_key(egui::Modifiers::COMMAND, egui::Key::S)
                    || input.consume_key(egui::Modifiers::CTRL, egui::Key::S)
            })
        {
            self.save_result(ui.ctx(), false);
        }
        self.legend(ui);
        ui.add_space(5.0);
        let available = ui.available_rect_before_wrap();
        let footer_height = control_extent(ui) + 10.0;
        let footer = Rect::from_min_max(
            egui::pos2(
                available.left(),
                (available.bottom() - footer_height).max(available.top()),
            ),
            available.max,
        );
        let body = Rect::from_min_max(
            available.min,
            egui::pos2(available.right(), (footer.top() - 5.0).max(available.top())),
        );
        let gap = ui.spacing().item_spacing.x;
        let pane_width = (body.width() - 2.0 * gap) / 3.0;
        self.pane_rects = Some(std::array::from_fn(|index| {
            Rect::from_min_size(
                body.min + egui::vec2(index as f32 * (pane_width + gap), 0.0),
                egui::vec2(pane_width, body.height()),
            )
        }));
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(body));
            egui::Frame::new()
                .fill(palette.panel)
                .stroke(Stroke::new(1.0, palette.border))
                .show(ui, |ui| {
                    self.inputs(ui);
                    ui.separator();
                    if self.start_requested && self.ready() {
                        self.start_requested = false;
                        self.request_replace(ReplaceMerge::Open, ui.ctx());
                    }
                    if self.workspace.is_some() {
                        ui.add_enabled_ui(self.load.is_none(), |ui| self.panes(ui));
                    } else {
                        let height = (ui.available_height() - 2.0).max(0.0);
                        ui.columns(3, |columns| {
                            for (index, column) in columns.iter_mut().enumerate() {
                                let (rect, _) = column.allocate_exact_size(
                                    egui::vec2(column.available_width(), height),
                                    Sense::hover(),
                                );
                                let target = rect.shrink(8.0);
                                paint_drop_outline(column.painter(), target, palette.border);
                                column.painter().text(
                                    target.center() - egui::vec2(0.0, 12.0),
                                    Align2::CENTER_CENTER,
                                    if index == 1 {
                                        "Editable merged result"
                                    } else {
                                        "Choose a file"
                                    },
                                    FontId::proportional(14.0),
                                    palette.muted,
                                );
                                column.painter().text(
                                    target.center() + egui::vec2(0.0, 12.0),
                                    Align2::CENTER_CENTER,
                                    if index == 1 {
                                        "Drop a base file, or use Current"
                                    } else {
                                        "Drop a file here, or use Browse above"
                                    },
                                    FontId::proportional(11.0),
                                    palette.muted,
                                );
                            }
                        });
                    }
                });
        });
        let gap = ui.spacing().item_spacing.x;
        let width = (body.width() - gap * 2.0) / 3.0;
        for x in [
            body.left() + width + gap / 2.0,
            body.left() + width * 2.0 + gap * 1.5,
        ] {
            ui.painter().line_segment(
                [egui::pos2(x, body.top()), egui::pos2(x, body.bottom())],
                Stroke::new(1.0, palette.border),
            );
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(footer), |ui| {
            self.footer(ui)
        });
        self.dialogs(ui.ctx());
    }

    fn legend(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let extent = control_extent(ui);
        let gap = ui.spacing().item_spacing.x;
        let (row, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), extent), Sense::hover());
        let actions =
            Rect::from_center_size(row.center(), egui::vec2(4.0 * extent + 3.0 * gap, extent));
        let left = Rect::from_min_max(row.min, egui::pos2(actions.left() - gap, row.bottom()));
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(left)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(left));
                for icon in [
                    StatusIcon::Different,
                    StatusIcon::LeftOnly,
                    StatusIcon::RightOnly,
                    StatusIcon::Conflict,
                ] {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), Sense::hover());
                    paint_status_icon(ui.painter(), rect.center(), icon);
                    ui.label(
                        RichText::new(icon.label())
                            .size(10.0)
                            .color(icon.color(palette)),
                    );
                    ui.add_space(5.0);
                }
                if icon_button(ui, ToolbarIcon::New, !self.is_saving(), "New merge").clicked() {
                    self.request_replace(ReplaceMerge::New, ui.ctx());
                }
            },
        );
        let ready = self.workspace.is_some() && self.load.is_none();
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(actions)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                for (icon, side, label) in [
                    (
                        ToolbarIcon::AcceptAllLeft,
                        MergeSide::Incoming,
                        "Accept All Left",
                    ),
                    (
                        ToolbarIcon::AcceptAllRight,
                        MergeSide::Current,
                        "Accept All Right",
                    ),
                ] {
                    if icon_button(ui, icon, ready, label).clicked() {
                        self.workspace.as_mut().unwrap().accept_all(side);
                        self.error = None;
                    }
                }
                let safe = ready
                    && self
                        .workspace
                        .as_ref()
                        .unwrap()
                        .actions()
                        .iter()
                        .any(|action| {
                            action.is_pending
                                && action.kind == MergeActionKind::Safe
                                && action.target_range.is_some()
                        });
                if icon_button(ui, ToolbarIcon::AutoMerge, safe, "Auto Merge Non-Conflicts")
                    .clicked()
                {
                    match self.workspace.as_mut().unwrap().auto_merge() {
                        Ok(_) => self.error = None,
                        Err(error) => self.error = Some(error.to_string()),
                    }
                }
                let undo = ready && self.workspace.as_ref().unwrap().can_undo();
                if icon_button(ui, ToolbarIcon::Undo, undo, "Undo").clicked() {
                    self.workspace.as_mut().unwrap().undo();
                    self.error = None;
                }
            },
        );
        let right = Rect::from_min_max(egui::pos2(actions.right() + gap, row.top()), row.max);
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(right)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                let zoom_width = ui
                    .painter()
                    .layout_no_wrap(
                        format!("Zoom {:.0}%", ui.ctx().zoom_factor() * 100.0),
                        egui::TextStyle::Button.resolve(ui.style()),
                        palette.muted,
                    )
                    .size()
                    .x
                    + 2.0 * ui.spacing().button_padding.x
                    + 2.0 * (extent + gap);
                let width = (ui.available_width() - zoom_width - 2.0 * extent - 3.0 * gap).max(0.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(width, extent),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_width(width);
                        ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));
                        if let Some(error) = &self.error {
                            status_label(ui, error, palette.error);
                        } else if self.load.is_some() {
                            ui.spinner();
                            status_label(ui, "Loading merge…", palette.muted);
                        } else if self.drop_job.is_some() {
                            ui.spinner();
                            status_label(ui, "Opening dropped file…", palette.muted);
                        } else if self.is_saving() {
                            ui.spinner();
                            status_label(ui, "Saving merge…", palette.muted);
                        } else if let Some(workspace) = &self.workspace {
                            let count = workspace
                                .actions()
                                .iter()
                                .filter(|action| {
                                    action.kind == MergeActionKind::Conflict && action.is_pending
                                })
                                .count();
                            let actions = workspace.actions();
                            let navigation = self.navigation_row.and_then(|row| {
                                actions
                                    .iter()
                                    .position(|action| action.row_range.start == row)
                            });
                            status_label(
                                ui,
                                &navigation.map_or_else(
                                    || format!("{count} conflict lines to review"),
                                    |index| format!("Change {} of {}", index + 1, actions.len()),
                                ),
                                palette.muted,
                            );
                        }
                    },
                );
                zoom_controls(ui);
                let has_differences =
                    ready && !self.workspace.as_ref().unwrap().actions().is_empty();
                for (icon, forward, label) in [
                    (ToolbarIcon::Previous, false, "Previous difference"),
                    (ToolbarIcon::Next, true, "Next difference"),
                ] {
                    if icon_button(ui, icon, has_differences, label).clicked() {
                        self.jump_difference(forward);
                        ui.ctx().request_repaint();
                    }
                }
            },
        );
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let ready = self.workspace.is_some()
            && self.load.is_none()
            && !self.is_saving()
            && self.drop_job.is_none();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(ready, egui::Button::new("Finish Merge"))
                .on_hover_text("Save any outstanding edits and finish this merge")
                .clicked()
            {
                self.finish_merge(ui.ctx());
            }
            let save = ui
                .add_enabled(ready, egui::Button::new("Save"))
                .on_hover_text("Save the merged result. Right-click for Save As.");
            if save.clicked() {
                self.save_result(ui.ctx(), false);
            }
            save.context_menu(|ui| {
                if ui
                    .add_enabled(ready, egui::Button::new("Save As…"))
                    .clicked()
                {
                    self.save_result(ui.ctx(), true);
                    ui.close();
                }
            });
            let path = self
                .output
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "Merged result".into());
            let dirty = self
                .workspace
                .as_ref()
                .is_some_and(MergeWorkspace::is_dirty);
            status_label(
                ui,
                &format!("{}{}", path, if dirty { " · Unsaved changes" } else { "" }),
                palette.muted,
            );
        });
    }

    fn inputs(&mut self, ui: &mut egui::Ui) {
        let mut commit = false;
        ui.columns(3, |columns| {
            for (index, column) in columns.iter_mut().enumerate() {
                egui::Frame::new().inner_margin(7).show(column, |ui| {
                    ui.horizontal(|ui| {
                        let palette = Palette::for_context(ui.ctx());
                        let title = ["LEFT (Incoming)", "BASE", "RIGHT (Current)"][index];
                        let label_font = FontId::monospace(10.0);
                        let label_width = ui.painter().layout_no_wrap(title.into(), label_font.clone(), palette.text).size().x.max(40.0);
                        let (label, _) = ui.allocate_exact_size(egui::vec2(label_width, control_extent(ui)), Sense::hover());
                        ui.painter().text(label.left_center(), Align2::LEFT_CENTER, title, label_font, [palette.left_only, palette.accent, palette.right_only][index]);
                        let width = (ui.available_width() - control_extent(ui) - ui.spacing().item_spacing.x).max(20.0);
                        let response = ui.add(egui::TextEdit::singleline(&mut self.paths[index]).id_salt(("merge-path", index)).font(comparison_font(ui.ctx())).margin(0).frame(egui::Frame::NONE).desired_width(width).hint_text(if index == 1 { "Defaults to Current" } else { "Choose a file" })).on_hover_text(if index == 1 { "Common ancestor for conflict detection. When blank, Current is used as Base. The pane below is the editable merged result." } else if index == 0 { "Incoming file (Left)" } else { "Current file (Right)" });
                        if response.changed() { self.native_paths[index] = None; self.path_edits[index] = true; self.load = None; self.drop_job = None; }
                        if self.path_edits[index] && (response.lost_focus() || (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))) {
                            self.path_edits[index] = false;
                            commit = true;
                        }
                        let browse = ["Browse left merge file", "Browse base file", "Browse right merge file"][index];
                        if icon_button(ui, ToolbarIcon::Browse, true, browse).clicked() {
                            let mut dialog = rfd::FileDialog::new().set_title(browse);
                            if let Some(parent) = self.source_path(index).parent().filter(|parent| !parent.as_os_str().is_empty()) { dialog = dialog.set_directory(parent); }
                            if let Some(path) = dialog.pick_file() {
                                self.paths[index] = path.to_string_lossy().into_owned(); self.native_paths[index] = Some(path); self.drop_job = None; self.load = None; commit = true;
                            }
                        }
                    });
                });
            }
        });
        if commit {
            self.start_requested = true;
        }
    }

    fn panes(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let workspace = self.workspace.as_mut().unwrap();
        let font = comparison_font(ui.ctx());
        let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
        if self.row_height > 0.0 {
            self.scroll_y *= row_height / self.row_height;
        }
        self.row_height = row_height;
        let mut choice = None;
        let mut result = workspace.result().to_owned();
        let before = result.clone();
        let actions = workspace.actions().to_vec();
        let height = ui.available_height().max(0.0);
        if self.navigation_scroll_pending {
            let rows = workspace
                .rows()
                .len()
                .max(display_result_text(&result).split('\n').count());
            self.scroll_y = (self.navigation_row.unwrap_or(0) as f32 * row_height)
                .clamp(0.0, (rows as f32 * row_height + 8.0 - height).max(0.0));
        }
        let min_content_height = height.max(workspace.rows().len() as f32 * row_height);
        let outer = egui::ScrollArea::vertical()
            .id_salt(("merge-shared-scroll", self.generation))
            .auto_shrink([false, false])
            .max_height(height)
            .vertical_scroll_offset(self.scroll_y)
            .show_viewport(ui, |ui, viewport| {
                ui.columns(3, |columns| {
                    for (index, column) in columns.iter_mut().enumerate() {
                        column.spacing_mut().item_spacing.y = 0.0;
                        egui::Frame::new().fill(palette.panel).inner_margin(egui::Margin::symmetric(3, 4)).show(column, |ui| {
                            egui::ScrollArea::horizontal().id_salt(("merge-horizontal", index, self.generation)).auto_shrink([false, false]).show(ui, |ui| {
                                ui.set_min_height(min_content_height);
                                if index == 1 {
                                    let conflict_ranges: Vec<_> = actions.iter().filter(|action| action.is_pending && action.kind == MergeActionKind::Conflict).filter_map(|action| action.target_range.clone()).collect();
                                    let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, _: f32| {
                                        ui.fonts_mut(|fonts| fonts.layout_job(result_layout(text.as_str(), &conflict_ranges, font.clone(), palette)))
                                    };
                                    ui.horizontal_top(|ui| {
                                        ui.spacing_mut().item_spacing.x = 3.0;
                                        let line_count = display_result_text(&result).split('\n').count();
                                        let (numbers, _) = ui.allocate_exact_size(egui::vec2(38.0, line_count as f32 * row_height), Sense::hover());
                                        let first = (viewport.min.y / row_height).floor().max(0.0) as usize;
                                        let last = ((viewport.max.y / row_height).ceil() as usize + 2).min(line_count);
                                        for line in first..last {
                                            ui.painter().text(numbers.min + egui::vec2(32.0, line as f32 * row_height), Align2::RIGHT_TOP, (line + 1).to_string(), font.clone(), palette.muted);
                                        }
                                        ui.add(egui::TextEdit::multiline(&mut result)
                                            .id(egui::Id::new(("merge-result", self.generation)))
                                            .code_editor().font(font.clone()).desired_width(f32::INFINITY)
                                            .desired_rows(1).margin(0).frame(egui::Frame::NONE)
                                            .layouter(&mut layouter));
                                    });
                                } else {
                                    let side = if index == 0 { MergeSide::Incoming } else { MergeSide::Current };
                                    let first = (viewport.min.y / row_height).floor().max(0.0) as usize;
                                    let last = ((viewport.max.y / row_height).ceil() as usize + 2).min(workspace.rows().len());
                                    let first = first.min(last);
                                    ui.add_space(first as f32 * row_height);
                                    for (row_index, row) in workspace.rows().iter().enumerate().take(last).skip(first) {
                                        let action = actions.get(actions.partition_point(|action| action.row_range.end <= row_index)).filter(|action| action.row_range.contains(&row_index));
                                        let line = if index == 0 { &row.incoming } else { &row.current };
                                        let text = line.as_ref().map_or("", |line| line.text.as_str());
                                        let galley = ui.painter().layout_no_wrap(text.to_owned(), font.clone(), palette.text);
                                        let row_width = ui.available_width().max(galley.size().x + 100.0);
                                        let (rect, response) = ui.allocate_exact_size(egui::vec2(row_width, row_height), Sense::hover());
                                        let color = action.map_or(palette.muted, |action| if action.kind == MergeActionKind::Conflict { palette.error } else { palette.changed });
                                        if action.is_some() {
                                            ui.painter().rect_filled(rect, 0, color.gamma_multiply(0.08));
                                        } else if row_index % 2 == 1 {
                                            ui.painter().rect_filled(rect, 0, palette.alternate);
                                        }
                                        // Keep arrows at the inner edge while scrolling long lines.
                                        let gutter_x = if index == 0 { ui.clip_rect().right() - 12.0 } else { ui.clip_rect().left() + 12.0 };
                                        let text_x = rect.left() + if index == 0 { 56.0 } else { 80.0 };
                                        if let Some(line) = line {
                                            ui.painter().text(egui::pos2(text_x - 12.0, rect.top()), Align2::RIGHT_TOP, line.number.to_string(), font.clone(), palette.muted);
                                        }
                                        ui.painter().galley(egui::pos2(text_x, rect.top()), galley, palette.text);
                                        ui.painter().rect_filled(Rect::from_center_size(egui::pos2(gutter_x, rect.center().y), egui::vec2(26.0, row_height)), 0, palette.panel);
                                        if let Some(action) = action {
                                            let conflict = action.kind == MergeActionKind::Conflict;
                                            if action.is_pending && conflict {
                                                ui.painter().text(egui::pos2(text_x - 4.0, rect.top()), Align2::RIGHT_TOP, "!", font.clone(), color);
                                            } else {
                                                let point = egui::pos2(text_x - 7.0, rect.center().y);
                                                if action.is_pending { ui.painter().circle_filled(point, 2.0, color); }
                                                else { ui.painter().line_segment([point - egui::vec2(3.0, 0.0), point], Stroke::new(1.0, color)); ui.painter().line_segment([point, point + egui::vec2(4.0, -4.0)], Stroke::new(1.0, color)); }
                                            }
                                            if action.row_range.start == row_index {
                                                let arrow_rect = Rect::from_center_size(egui::pos2(gutter_x, rect.center().y), egui::vec2(22.0, row_height));
                                                let enabled = action.target_range.is_some();
                                                let arrow = ui.interact(arrow_rect, egui::Id::new(("merge-arrow", self.generation, action.id, index)), if enabled { Sense::click() } else { Sense::hover() });
                                                let label = action_label(action, side);
                                                arrow.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &label));
                                                if arrow.hovered() { ui.painter().rect_filled(arrow_rect, 2, palette.hover); }
                                                paint_merge_arrow(ui.painter(), arrow_rect.center(), side, if enabled { color } else { palette.muted });
                                                if arrow.clicked() { choice = Some((action.id, side)); }
                                                arrow.on_hover_text(if enabled { label } else { "This result range was edited manually. Undo that edit to use its source arrow, or finish the result by typing.".into() });
                                            }
                                        }
                                        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("{} line {}: {}{}", if index == 0 { "Incoming" } else { "Current" }, line.as_ref().map_or("absent".into(), |line| line.number.to_string()), if action.is_some_and(|action| action.kind == MergeActionKind::Conflict) { "Conflict. " } else { "" }, text)));
                                    }
                                    ui.add_space(workspace.rows().len().saturating_sub(last) as f32 * row_height);
                                }
                            });
                        });
                    }
                });
            });
        if !self.navigation_scroll_pending && (self.scroll_y - outer.state.offset.y).abs() > 0.1 {
            self.navigation_row = None;
        }
        self.scroll_y = outer.state.offset.y;
        self.navigation_scroll_pending = false;
        if result != before {
            workspace.edit_result(result);
        }
        if let Some((id, side)) = choice {
            match workspace.apply_action(id, side) {
                Ok(_) => self.error = None,
                Err(error) => self.error = Some(error.to_string()),
            }
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(operation) = self.replace {
            let mut discard = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("replace-merge")).show(ctx, |ui| {
                ui.heading("Discard unsaved merge?");
                ui.label("Save the result first, or discard it to open another merge.");
                ui.horizontal(|ui| {
                    cancel = ui.button("Keep merging").clicked();
                    discard = ui
                        .add_enabled(!self.is_saving(), egui::Button::new("Discard and continue"))
                        .clicked();
                });
            });
            if cancel {
                self.replace = None;
                if matches!(operation, ReplaceMerge::Open) {
                    self.restore_loaded_sources();
                }
            }
            if discard {
                self.replace = None;
                self.replace_now(operation, ctx);
            }
        }
        if let Some((path, _)) = &self.overwrite {
            let mut confirm = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("overwrite-merge")).show(ctx, |ui| {
                ui.heading("Replace existing file?");
                ui.label(path.display().to_string());
                ui.label("The saved result will replace this file's contents.");
                ui.horizontal(|ui| {
                    cancel = ui.button("Cancel").clicked();
                    confirm = ui.button("Replace file").clicked();
                });
            });
            if cancel {
                self.overwrite = None;
                self.finish_requested = false;
            }
            if confirm {
                let (path, contents) = self.overwrite.take().unwrap();
                self.start_save(ctx, path, contents, true);
            }
        }
    }
}

fn action_label(action: &MergeAction, side: MergeSide) -> String {
    format!(
        "Use {} {} {} in result",
        if side == MergeSide::Incoming {
            "Incoming"
        } else {
            "Current"
        },
        if action.kind == MergeActionKind::Conflict {
            "conflict line"
        } else {
            "nonconflicting block"
        },
        action.row_range.start + 1
    )
}

fn paint_merge_arrow(painter: &egui::Painter, center: Pos2, side: MergeSide, color: Color32) {
    let direction = if side == MergeSide::Incoming {
        1.0
    } else {
        -1.0
    };
    let tip = center + egui::vec2(6.0 * direction, 0.0);
    let stroke = Stroke::new(1.5, color);
    painter.line_segment([center - egui::vec2(6.0 * direction, 0.0), tip], stroke);
    painter.line_segment([tip - egui::vec2(4.0 * direction, 4.0), tip], stroke);
    painter.line_segment([tip - egui::vec2(4.0 * direction, -4.0), tip], stroke);
}

fn result_layout(
    text: &str,
    conflicts: &[Range<usize>],
    font: FontId,
    palette: Palette,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let mut offset = 0;
    for line in display_result_text(text).split_inclusive('\n') {
        let range = offset..offset + line.len();
        let conflict = conflicts
            .iter()
            .any(|conflict| conflict.start < range.end && range.start < conflict.end);
        job.append(
            line,
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color: palette.text,
                background: if conflict {
                    palette.error.gamma_multiply(0.09)
                } else {
                    Color32::TRANSPARENT
                },
                ..Default::default()
            },
        );
        offset = range.end;
    }
    job.wrap.max_width = f32::INFINITY;
    job
}

// egui hides CR. Render a standalone CR as a newline while retaining identical
// character/byte offsets for editor cursors and action targets. CRLF stays intact.
fn display_result_text(text: &str) -> String {
    let mut characters = text.chars().peekable();
    let mut display = String::with_capacity(text.len());
    while let Some(character) = characters.next() {
        display.push(if character == '\r' && characters.peek() != Some(&'\n') {
            '\n'
        } else {
            character
        });
    }
    display
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct TestDrop(PathBuf);
    impl egui::DroppedFile for TestDrop {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            panic!("Drop handler must not read contents on the UI thread")
        }
    }

    fn temp_directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "versus-merge-input-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn drop_frame(
        view: &mut MergeView,
        ctx: &egui::Context,
        position: Pos2,
        paths: Vec<PathBuf>,
        hover: bool,
        native: Option<Pos2>,
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerGone,
                ],
                hovered_files: if hover {
                    vec![egui::HoveredFile {
                        path: paths.first().cloned(),
                        ..Default::default()
                    }]
                } else {
                    Vec::new()
                },
                dropped_files: if hover {
                    Vec::new()
                } else {
                    paths
                        .into_iter()
                        .map(|path| Arc::new(TestDrop(path)) as egui::DroppedFileHandle)
                        .collect()
                },
                ..Default::default()
            },
            |ui| {
                view.render(ui);
                view.drag_and_drop(ui, native);
            },
        );
        output.textures_delta.clear();
        output
    }

    fn fixture() -> MergeView {
        MergeView {
            workspace: Some(MergeWorkspace::new(
                Some("heading\na\nb\nmode=base\nend\n"),
                "heading\nA\nB\nmode=incoming\nend\n",
                "heading\na\nb\nmode=current\nend\n",
            )),
            paths: [
                "incoming.txt".into(),
                "base.txt".into(),
                "current.txt".into(),
            ],
            ..Default::default()
        }
    }

    fn render(
        view: &mut MergeView,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                events,
                ..Default::default()
            },
            |ui| view.render(ui),
        );
        output.textures_delta.clear();
        output
    }

    fn click(view: &mut MergeView, ctx: &egui::Context, position: Pos2) {
        for pressed in [true, false] {
            render(
                view,
                ctx,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            )
            .drop_without_applying_deltas();
        }
    }

    fn text_center(output: &egui::FullOutput, value: &str) -> Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == value => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing UI text: {value}"))
    }

    #[test]
    fn three_panes_grouped_safe_arrow_line_conflict_and_undo_work_in_both_themes() {
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let ctx = egui::Context::default();
            set_theme(&ctx, theme);
            let mut view = fixture();
            let output = render(&mut view, &ctx, Vec::new());
            let incoming = text_center(&output, "LEFT (Incoming)");
            let result = text_center(&output, "BASE");
            let current = text_center(&output, "RIGHT (Current)");
            assert!(incoming.x < result.x && result.x < current.x);
            let workspace = view.workspace.as_ref().unwrap();
            let actions = workspace.actions();
            assert_eq!(actions.len(), 2);
            assert_eq!(actions[0].kind, MergeActionKind::Safe);
            assert_eq!(actions[0].row_range.len(), 2);
            assert_eq!(actions[1].kind, MergeActionKind::Conflict);
            assert_eq!(actions[1].row_range.len(), 1);
            let arrow = ctx
                .read_response(egui::Id::new((
                    "merge-arrow",
                    0_u64,
                    actions[0].id,
                    0_usize,
                )))
                .unwrap();
            let result_rect = ctx
                .read_response(egui::Id::new(("merge-result", 0_u64)))
                .unwrap()
                .rect;
            assert!(arrow.rect.right() < result_rect.left());
            click(&mut view, &ctx, arrow.rect.center());
            assert_eq!(
                view.workspace.as_ref().unwrap().result(),
                "heading\nA\nB\nmode=current\nend\n"
            );
            render(&mut view, &ctx, Vec::new()).drop_without_applying_deltas();
            click(
                &mut view,
                &ctx,
                ctx.read_response(egui::Id::new("Undo"))
                    .unwrap()
                    .rect
                    .center(),
            );
            assert_eq!(
                view.workspace.as_ref().unwrap().result(),
                "heading\na\nb\nmode=current\nend\n"
            );
            render(&mut view, &ctx, Vec::new()).drop_without_applying_deltas();
            click(
                &mut view,
                &ctx,
                ctx.read_response(egui::Id::new("Auto Merge Non-Conflicts"))
                    .unwrap()
                    .rect
                    .center(),
            );
            assert_eq!(
                view.workspace.as_ref().unwrap().result(),
                "heading\nA\nB\nmode=current\nend\n"
            );
            let output = render(&mut view, &ctx, Vec::new());
            let arrow = ctx
                .read_response(egui::Id::new((
                    "merge-arrow",
                    0_u64,
                    actions[1].id,
                    0_usize,
                )))
                .unwrap();
            assert!(text_center(&output, "1 conflict lines to review").x > 0.0);
            click(&mut view, &ctx, arrow.rect.center());
            assert_eq!(
                view.workspace.as_ref().unwrap().result(),
                "heading\nA\nB\nmode=incoming\nend\n"
            );
        }
    }

    #[test]
    fn middle_editor_accepts_free_typing_and_command_z_undoes_it() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut view = fixture();
        render(&mut view, &ctx, Vec::new()).drop_without_applying_deltas();
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new(("merge-result", 0_u64))));
        render(
            &mut view,
            &ctx,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                },
                egui::Event::Paste("自由に編集\nhand typed\n".into()),
            ],
        )
        .drop_without_applying_deltas();
        assert_eq!(
            view.workspace.as_ref().unwrap().result(),
            "自由に編集\nhand typed\n"
        );
        render(
            &mut view,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Z,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
        )
        .drop_without_applying_deltas();
        assert_eq!(
            view.workspace.as_ref().unwrap().result(),
            "heading\na\nb\nmode=current\nend\n"
        );
    }

    fn settle(view: &mut MergeView, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while view.load.is_some() || view.save.is_some() || view.drop_job.is_some() {
            assert!(Instant::now() < deadline, "merge worker did not finish");
            view.poll(ctx);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn save_worker_requires_overwrite_and_tracks_the_saved_snapshot() {
        let directory = std::env::temp_dir().join(format!(
            "versus-merge-save-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("result.txt");
        std::fs::write(&path, "original").unwrap();
        let ctx = egui::Context::default();
        let mut view = fixture();
        view.workspace
            .as_mut()
            .unwrap()
            .edit_result("α\r\nβ\nlast".into());
        view.start_save(&ctx, path.clone(), "α\r\nβ\nlast".into(), false);
        settle(&mut view, &ctx);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        assert!(view.overwrite.is_some());
        let (path, contents) = view.overwrite.take().unwrap();
        view.start_save(&ctx, path.clone(), contents, true);
        view.workspace
            .as_mut()
            .unwrap()
            .edit_result("later edit".into());
        settle(&mut view, &ctx);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "α\r\nβ\nlast");
        assert_eq!(view.output.as_ref(), Some(&path));
        assert!(view.workspace.as_ref().unwrap().is_dirty());
        assert!(view.workspace.as_mut().unwrap().undo());
        assert!(!view.workspace.as_ref().unwrap().is_dirty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unsaved_result_requires_discard_to_replace_and_cancellation_keeps_it() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut view = fixture();
        view.workspace
            .as_mut()
            .unwrap()
            .edit_result("unsaved".into());
        view.request_replace(ReplaceMerge::New, &ctx);
        assert!(view.replace.is_some());
        // Modal Areas use their first pass to measure before painting.
        render(&mut view, &ctx, Vec::new()).drop_without_applying_deltas();
        let output = render(&mut view, &ctx, Vec::new());
        click(&mut view, &ctx, text_center(&output, "Keep merging"));
        assert!(view.replace.is_none());
        assert_eq!(view.workspace.as_ref().unwrap().result(), "unsaved");
        view.request_replace(ReplaceMerge::New, &ctx);
        let output = render(&mut view, &ctx, Vec::new());
        click(
            &mut view,
            &ctx,
            text_center(&output, "Discard and continue"),
        );
        assert!(view.workspace.is_none());
    }

    #[test]
    fn line_ending_display_preserves_cursor_offsets() {
        let text = "α\rβ\r\nγ\n";
        let display = display_result_text(text);
        assert_eq!(display, "α\nβ\r\nγ\n");
        assert_eq!(display.len(), text.len());
        assert_eq!(display.chars().count(), text.chars().count());
    }

    #[test]
    fn merge_icons_share_centered_legend_and_save_finish_stay_bottom_right() {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            for (width, height) in [(900.0, 650.0), (1200.0, 800.0)] {
                let ctx = egui::Context::default();
                set_theme(&ctx, theme);
                let mut view = fixture();
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            egui::vec2(width, height),
                        )),
                        ..Default::default()
                    },
                    |ui| view.render(ui),
                );
                output.textures_delta.clear();
                let controls: Vec<_> = [
                    "Accept All Left",
                    "Accept All Right",
                    "Auto Merge Non-Conflicts",
                    "Undo",
                ]
                .iter()
                .map(|label| ctx.read_response(egui::Id::new(label)).unwrap().rect)
                .collect();
                for pair in controls.windows(2) {
                    assert!(pair[0].right() < pair[1].left());
                    assert!((pair[0].center().y - pair[1].center().y).abs() < 0.5);
                }
                assert!(
                    (controls[0].union(*controls.last().unwrap()).center().x - width / 2.0).abs()
                        < 1.0
                );
                let conflict = text_center(&output, "Conflict");
                assert!(conflict.x < controls[0].left());
                assert!((conflict.y - controls[0].center().y).abs() < 1.0);
                let save = text_center(&output, "Save");
                let finish = text_center(&output, "Finish Merge");
                assert!(save.x < finish.x && save.x > width * 0.7);
                assert!(save.y > height - 35.0 && (save.y - finish.y).abs() < 0.5);
                assert!(
                    ctx.read_response(egui::Id::new("Browse base file"))
                        .unwrap()
                        .rect
                        .bottom()
                        < save.y
                );
                let zoom = ctx.read_response(egui::Id::new("Zoom in")).unwrap().rect;
                let previous = ctx
                    .read_response(egui::Id::new("Previous difference"))
                    .unwrap()
                    .rect;
                let next = ctx
                    .read_response(egui::Id::new("Next difference"))
                    .unwrap()
                    .rect;
                assert!(controls.last().unwrap().right() < zoom.left());
                assert!(zoom.right() < previous.left() && previous.right() < next.left());
                assert!(next.right() <= width && next.right() > width - 15.0);
                let before = view.workspace.as_ref().unwrap().result().to_owned();
                click(&mut view, &ctx, controls[0].center());
                assert_eq!(
                    view.workspace.as_ref().unwrap().result(),
                    "heading\nA\nB\nmode=incoming\nend\n"
                );
                render(&mut view, &ctx, Vec::new()).drop_without_applying_deltas();
                click(
                    &mut view,
                    &ctx,
                    ctx.read_response(egui::Id::new("Undo"))
                        .unwrap()
                        .rect
                        .center(),
                );
                assert_eq!(view.workspace.as_ref().unwrap().result(), before);
                render(&mut view, &ctx, Vec::new()).drop_without_applying_deltas();
                click(
                    &mut view,
                    &ctx,
                    ctx.read_response(egui::Id::new("Accept All Right"))
                        .unwrap()
                        .rect
                        .center(),
                );
                assert!(
                    view.workspace
                        .as_ref()
                        .unwrap()
                        .actions()
                        .iter()
                        .all(|action| !action.is_pending)
                );
            }
        }
    }

    #[test]
    fn finishing_requires_successful_current_snapshot_and_keeps_edits_on_failure() {
        let ctx = egui::Context::default();
        let mut view = fixture();
        view.finish_requested = true;
        view.accept_save_event(SaveEvent::Failed("write failed".into()));
        assert!(!view.is_finished() && !view.finish_requested);
        let snapshot = view.workspace.as_ref().unwrap().result().to_owned();
        view.finish_requested = true;
        view.workspace
            .as_mut()
            .unwrap()
            .edit_result("newer edits".into());
        view.accept_save_event(SaveEvent::Complete(PathBuf::from("result.txt"), snapshot));
        assert!(!view.is_finished());
        assert!(view.workspace.as_ref().unwrap().is_dirty());
        view.finish_requested = true;
        view.accept_save_event(SaveEvent::Complete(
            PathBuf::from("result.txt"),
            "newer edits".into(),
        ));
        assert!(view.is_finished());
        let mut view = fixture();
        view.output = Some(PathBuf::from("already-saved.txt"));
        view.finish_merge(&ctx);
        assert!(view.is_finished() && !view.is_saving());
    }

    #[test]
    fn rejected_or_failed_source_replacement_restores_active_path_headers() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut view = fixture();
        let active = view.paths.clone();
        let native = active.clone().map(|path| Some(PathBuf::from(path)));
        view.loaded_paths = Some(active.clone());
        view.loaded_native_paths = Some(native.clone());
        view.native_paths = native.clone();
        view.workspace
            .as_mut()
            .unwrap()
            .edit_result("unsaved result".into());
        view.paths[0] = "different.txt".into();
        view.native_paths[0] = Some(PathBuf::from("different.txt"));
        view.request_replace(ReplaceMerge::Open, &ctx);
        render(&mut view, &ctx, Vec::new()).drop_without_applying_deltas();
        let output = render(&mut view, &ctx, Vec::new());
        click(&mut view, &ctx, text_center(&output, "Keep merging"));
        assert_eq!(view.paths, active);
        assert_eq!(view.native_paths, native);
        view.paths[0] = "different.txt".into();
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Err(CompareError {
                path: None,
                kind: versus::CompareErrorKind::Io,
                message: "Cannot read replacement source".into(),
            }))
            .unwrap();
        view.load = Some(LoadJob {
            receiver,
            cancellation: Arc::new(AtomicBool::new(false)),
            display_paths: view.paths.clone(),
            native_paths: view.native_paths.clone(),
        });
        view.poll(&ctx);
        assert_eq!(view.paths, active);
        assert_eq!(view.native_paths, native);
        assert_eq!(view.workspace.as_ref().unwrap().result(), "unsaved result");
        assert!(view.error.is_some());
    }

    #[test]
    fn every_inserted_conflict_line_has_two_visible_arrows_even_with_long_text() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let incoming = format!("{}\nsecond\nthird\n", "long text ".repeat(100));
        let mut view = MergeView {
            workspace: Some(MergeWorkspace::new(None, &incoming, "")),
            ..Default::default()
        };
        render(&mut view, &ctx, Vec::new()).drop_without_applying_deltas();
        let actions = view.workspace.as_ref().unwrap().actions();
        assert_eq!(actions.len(), 3);
        for action in actions {
            assert_eq!(action.row_range.len(), 1);
            for side in [0_usize, 2] {
                let response = ctx
                    .read_response(egui::Id::new(("merge-arrow", 0_u64, action.id, side)))
                    .unwrap();
                assert!(response.enabled());
                assert!(
                    Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))
                        .contains_rect(response.rect)
                );
            }
        }
    }

    #[test]
    fn app_hands_file_sources_to_merge_and_guards_mode_and_window_exit() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = VersusApp::default();
        let paths: [PathBuf; 2] = ["incoming.txt".into(), "current.txt".into()];
        app.comparison_mode = ComparisonMode::File;
        app.file_view = Some(VersusApp::create_file_view(
            paths.clone(),
            paths.map(Some),
            false,
            false,
            false,
            app.file_options(),
        ));
        let frame = |app: &mut VersusApp, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                    events,
                    ..Default::default()
                },
                |ui| app.render(ui),
            );
            output.textures_delta.clear();
            output
        };
        let output = frame(&mut app, Vec::new());
        let merge = text_center(&output, "File Merge");
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(merge),
                    egui::Event::PointerButton {
                        pos: merge,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            )
            .drop_without_applying_deltas();
        }
        let view = app.merge_view.as_mut().unwrap();
        assert_eq!(view.paths, ["incoming.txt", "", "current.txt"]);
        view.workspace = Some(MergeWorkspace::new(None, "incoming", "current"));
        view.workspace
            .as_mut()
            .unwrap()
            .edit_result("unsaved".into());
        app.switch_comparison_mode(ComparisonMode::Folder);
        assert!(matches!(
            app.pending_merge_exit,
            Some(MergeExit::Mode(ComparisonMode::Folder))
        ));
        assert!(app.merge_view.is_some());
        frame(&mut app, Vec::new()).drop_without_applying_deltas();
        let output = frame(&mut app, Vec::new());
        let keep = text_center(&output, "Keep merging");
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(keep),
                    egui::Event::PointerButton {
                        pos: keep,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            )
            .drop_without_applying_deltas();
        }
        assert!(app.pending_merge_exit.is_none());
        assert!(app.merge_view.is_some());
        let mut input = egui::RawInput::default();
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .events
            .push(egui::ViewportEvent::Close);
        let mut output = ctx.run_ui(input, |ui| app.render(ui));
        output.textures_delta.clear();
        assert!(matches!(app.pending_merge_exit, Some(MergeExit::Close)));
        assert!(
            output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::CancelClose))
        );
        assert_eq!(
            app.merge_view
                .as_ref()
                .unwrap()
                .workspace
                .as_ref()
                .unwrap()
                .result(),
            "unsaved"
        );
    }

    #[test]
    fn blank_base_uses_current_file_snapshot_and_auto_merges_incoming() {
        let directory = temp_directory();
        let incoming = directory.join("incoming.txt");
        let current = directory.join("current.txt");
        std::fs::write(&incoming, "A\nb\n").unwrap();
        std::fs::write(&current, "a\nb\n").unwrap();
        let ctx = egui::Context::default();
        let mut view = MergeView::default();
        view.set_sources([Some(incoming), Some(current)]);
        view.open(&ctx);
        settle(&mut view, &ctx);
        assert!(view.paths[1].is_empty());
        assert!(
            view.workspace
                .as_ref()
                .unwrap()
                .actions()
                .iter()
                .all(|action| action.kind == MergeActionKind::Safe)
        );
        view.workspace.as_mut().unwrap().auto_merge().unwrap();
        assert_eq!(view.workspace.as_ref().unwrap().result(), "A\nb\n");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn navigation_buttons_follow_zoom_and_jump_both_directions_even_when_lines_fit() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut view = fixture();
        render(&mut view, &ctx, Vec::new()).drop_without_applying_deltas();
        let zoom = ctx.read_response(egui::Id::new("Zoom in")).unwrap().rect;
        let previous = ctx
            .read_response(egui::Id::new("Previous difference"))
            .unwrap()
            .rect;
        let next = ctx
            .read_response(egui::Id::new("Next difference"))
            .unwrap()
            .rect;
        assert!(zoom.right() < previous.left() && previous.right() < next.left());
        assert!(next.right() > 1190.0);
        click(&mut view, &ctx, next.center());
        assert_eq!(view.navigation_row, Some(1));
        click(&mut view, &ctx, next.center());
        assert_eq!(view.navigation_row, Some(3));
        click(&mut view, &ctx, previous.center());
        assert_eq!(view.navigation_row, Some(1));
        let current: String = (0..250).map(|index| format!("line {index}\n")).collect();
        let incoming = current
            .replace("line 120\n", "changed 120\n")
            .replace("line 240\n", "changed 240\n");
        view.workspace = Some(MergeWorkspace::new(Some(&current), &incoming, &current));
        view.navigation_row = None;
        view.scroll_y = 0.0;
        render(&mut view, &ctx, Vec::new()).drop_without_applying_deltas();
        click(&mut view, &ctx, next.center());
        assert_eq!(view.navigation_row, Some(120));
        assert!(view.scroll_y > 1000.0);
        click(&mut view, &ctx, next.center());
        assert_eq!(view.navigation_row, Some(240));
        assert!(view.scroll_y > 2000.0);
        click(&mut view, &ctx, previous.center());
        assert_eq!(view.navigation_row, Some(120));
    }

    #[test]
    fn drops_target_all_merge_panes_and_share_empty_outlines_and_hover_style() {
        let directory = temp_directory();
        let file = directory.join("example.txt");
        std::fs::write(&file, "content\n").unwrap();
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            for side in 0..3 {
                let ctx = egui::Context::default();
                set_theme(&ctx, theme);
                let mut view = MergeView::default();
                let empty = render(&mut view, &ctx, Vec::new());
                let palette = Palette::for_context(&ctx);
                assert!(empty.shapes.iter().filter(|shape| matches!(&shape.shape, egui::Shape::LineSegment { stroke, .. } if stroke.width == 1.0 && stroke.color == palette.border)).count() > 20);
                let panes = view.pane_rects.unwrap();
                let position = panes[side].center();
                let hover = drop_frame(
                    &mut view,
                    &ctx,
                    position,
                    vec![file.clone()],
                    true,
                    Some(panes[(side + 1) % 3].center()),
                );
                assert_eq!(view.drop_hover_side, Some(side));
                let border = hover
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect)
                            if rect.stroke.width == 3.0 && rect.stroke.color == palette.accent =>
                        {
                            Some(rect.rect)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert_eq!(border, panes[side].shrink(3.0));
                drop_frame(&mut view, &ctx, position, vec![file.clone()], false, None)
                    .drop_without_applying_deltas();
                settle(&mut view, &ctx);
                assert_eq!(view.source_path(side), file);
                assert!(view.native_paths[side].is_some());
                assert!(view.drop_hover_side.is_none());
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn invalid_drops_preserve_merge_and_valid_dropped_replacement_keeps_discard_guard() {
        let directory = temp_directory();
        let file = directory.join("replacement.txt");
        std::fs::write(&file, "new contents\n").unwrap();
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut view = fixture();
        let active = view.paths.clone();
        view.loaded_paths = Some(active.clone());
        view.workspace
            .as_mut()
            .unwrap()
            .edit_result("unsaved".into());
        render(&mut view, &ctx, Vec::new()).drop_without_applying_deltas();
        let position = view.pane_rects.unwrap()[0].center();
        for paths in [vec![file.clone(), file.clone()], vec![directory.clone()]] {
            drop_frame(&mut view, &ctx, position, paths, false, None)
                .drop_without_applying_deltas();
            settle(&mut view, &ctx);
            assert_eq!(view.paths, active);
            assert_eq!(view.workspace.as_ref().unwrap().result(), "unsaved");
            assert!(view.error.is_some());
        }
        drop_frame(
            &mut view,
            &ctx,
            egui::pos2(5.0, 790.0),
            vec![file.clone()],
            false,
            None,
        )
        .drop_without_applying_deltas();
        assert_eq!(view.paths, active);
        drop_frame(&mut view, &ctx, position, vec![file], false, None)
            .drop_without_applying_deltas();
        settle(&mut view, &ctx);
        render(&mut view, &ctx, Vec::new()).drop_without_applying_deltas();
        assert!(view.replace.is_some());
        assert_eq!(view.workspace.as_ref().unwrap().result(), "unsaved");
        let output = render(&mut view, &ctx, Vec::new());
        click(&mut view, &ctx, text_center(&output, "Keep merging"));
        assert_eq!(view.paths, active);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn app_forwards_native_drop_position_to_each_merge_pane_without_changing_compare_sources() {
        let directory = temp_directory();
        let file = directory.join("source.txt");
        std::fs::write(&file, "contents\n").unwrap();
        for side in 0..3 {
            let ctx = egui::Context::default();
            apply_theme(&ctx);
            let mut app = VersusApp::default();
            app.merge_view = Some(MergeView::default());
            let mut initial = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                    ..Default::default()
                },
                |ui| app.render(ui),
            );
            initial.textures_delta.clear();
            app.native_drag_position =
                Some(app.merge_view.as_ref().unwrap().pane_rects.unwrap()[side].center());
            let mut dropped =
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            egui::vec2(1200.0, 800.0),
                        )),
                        events: vec![egui::Event::PointerGone],
                        dropped_files: vec![
                            Arc::new(TestDrop(file.clone())) as egui::DroppedFileHandle
                        ],
                        ..Default::default()
                    },
                    |ui| app.render(ui),
                );
            dropped.textures_delta.clear();
            settle(app.merge_view.as_mut().unwrap(), &ctx);
            assert_eq!(app.merge_view.as_ref().unwrap().source_path(side), file);
            assert_eq!(app.source_paths, [None, None]);
            assert!(app.native_drag_position.is_none());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
