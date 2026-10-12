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
}

impl MergeView {
    pub(super) fn set_sources(&mut self, sources: [Option<PathBuf>; 2]) {
        for (index, path) in [0, 2].into_iter().zip(sources) {
            if let Some(path) = path {
                self.paths[index] = path.to_string_lossy().into_owned();
                self.native_paths[index] = Some(path);
            }
        }
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
        let base = (!self.paths[1].is_empty()).then(|| self.source_path(1));
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
            self.load = None;
            match result {
                Ok(Some(workspace)) => {
                    self.workspace = Some(workspace);
                    self.loaded_paths = Some(display_paths);
                    self.output = None;
                    self.generation += 1;
                    self.scroll_y = 0.0;
                    self.message = "Choose arrows or edit the result, then Save.".into();
                }
                Ok(None) => self.message = "Merge loading cancelled.".into(),
                Err(error) => self.error = Some(error.to_string()),
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
        if self.load.is_some() || self.save.is_some() {
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
            }
            SaveEvent::Failed(error) => self.error = Some(error),
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

    pub(super) fn render(&mut self, ui: &mut egui::Ui) {
        self.poll(ui.ctx());
        let palette = Palette::for_context(ui.ctx());
        self.inputs(ui);
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            let ready = !self.paths[0].is_empty()
                && !self.paths[2].is_empty()
                && self.load.is_none()
                && !self.is_saving();
            if ui
                .add_enabled(ready, egui::Button::new("Open merge"))
                .clicked()
            {
                self.request_replace(ReplaceMerge::Open, ui.ctx());
            }
            if ui
                .add_enabled(!self.is_saving(), egui::Button::new("New merge"))
                .clicked()
            {
                self.request_replace(ReplaceMerge::New, ui.ctx());
            }
            if self.load.is_some() {
                ui.spinner();
                if ui.button("Cancel loading").clicked() {
                    self.load = None;
                    self.message = "Merge loading cancelled.".into();
                }
            }
            let safe = self.workspace.as_ref().is_some_and(|workspace| {
                workspace.actions().iter().any(|action| {
                    action.is_pending
                        && action.kind == MergeActionKind::Safe
                        && action.target_range.is_some()
                })
            });
            if ui
                .add_enabled(
                    safe && self.load.is_none(),
                    egui::Button::new("Auto merge nonconflicting"),
                )
                .clicked()
            {
                if let Some(workspace) = &mut self.workspace {
                    match workspace.auto_merge() {
                        Ok(_) => self.error = None,
                        Err(error) => self.error = Some(error.to_string()),
                    }
                }
            }
            let undo = self
                .workspace
                .as_ref()
                .is_some_and(MergeWorkspace::can_undo);
            let undo_shortcut = self.workspace.is_some()
                && self.load.is_none()
                && ui.memory(|memory| {
                    memory.has_focus(egui::Id::new(("merge-result", self.generation)))
                })
                && ui.input_mut(|input| {
                    input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)
                        || input.consume_key(egui::Modifiers::CTRL, egui::Key::Z)
                });
            if ui.add_enabled(undo, egui::Button::new("Undo")).clicked() || undo_shortcut {
                if let Some(workspace) = &mut self.workspace {
                    workspace.undo();
                }
                self.error = None;
            }
            let can_save = self.workspace.is_some() && !self.is_saving() && self.load.is_none();
            let save_shortcut = can_save
                && ui.input_mut(|input| {
                    input.consume_key(egui::Modifiers::COMMAND, egui::Key::S)
                        || input.consume_key(egui::Modifiers::CTRL, egui::Key::S)
                });
            if ui
                .add_enabled(can_save, egui::Button::new("Save"))
                .clicked()
                || save_shortcut
            {
                self.save_result(ui.ctx(), false);
            }
            if ui
                .add_enabled(can_save, egui::Button::new("Save As…"))
                .clicked()
            {
                self.save_result(ui.ctx(), true);
            }
            zoom_controls(ui);
        });
        ui.horizontal(|ui| {
            ui.colored_label(palette.error, "! Conflict");
            ui.colored_label(palette.changed, "Nonconflicting block");
            if let Some(workspace) = &self.workspace {
                let conflicts = workspace
                    .actions()
                    .iter()
                    .filter(|action| action.kind == MergeActionKind::Conflict && action.is_pending)
                    .count();
                ui.label(format!("{conflicts} conflict lines to review"));
                if self.loaded_paths.as_ref().unwrap_or(&self.paths)[1].is_empty() {
                    ui.label("No base: choose differing lines manually.");
                }
            }
        });
        if let Some(error) = &self.error {
            status_label(ui, error, palette.error);
        } else if self.is_saving() {
            status_label(ui, "Saving merged result…", palette.muted);
        } else {
            status_label(ui, &self.message, palette.muted);
        }
        ui.separator();
        if self.workspace.is_some() {
            ui.add_enabled_ui(self.load.is_none(), |ui| self.panes(ui));
        } else {
            let height = ui.available_height().max(0.0);
            ui.columns(3, |columns| {
                for (column, label) in columns.iter_mut().zip(["INCOMING", "RESULT · EDITABLE", "CURRENT"]) {
                    egui::Frame::new().fill(palette.panel).inner_margin(10).show(column, |ui| {
                        ui.set_min_height((height - 22.0).max(0.0));
                        ui.label(RichText::new(label).strong());
                        ui.add_space(30.0);
                        ui.label(if label.starts_with("RESULT") { "Choose Incoming and Current, then Open merge. Add their common ancestor as Base to auto merge safe changes." } else { "Browse or paste a file path above." });
                    });
                }
            });
        }
        self.dialogs(ui.ctx());
    }

    fn inputs(&mut self, ui: &mut egui::Ui) {
        ui.columns(3, |columns| {
            for (index, column) in columns.iter_mut().enumerate() {
                let label = ["Incoming", "Base (optional common ancestor)", "Current"][index];
                column.label(RichText::new(label).strong());
                column.horizontal(|ui| {
                    let width = (ui.available_width() - 58.0).max(20.0);
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut self.paths[index])
                                .id_salt(("merge-path", index))
                                .desired_width(width)
                                .hint_text("Paste a file path"),
                        )
                        .changed()
                    {
                        self.native_paths[index] = None;
                    }
                    if ui.button("Browse").clicked() {
                        let mut dialog =
                            rfd::FileDialog::new().set_title(format!("Choose {label} file"));
                        if let Some(parent) = self
                            .source_path(index)
                            .parent()
                            .filter(|parent| !parent.as_os_str().is_empty())
                        {
                            dialog = dialog.set_directory(parent);
                        }
                        if let Some(path) = dialog.pick_file() {
                            self.paths[index] = path.to_string_lossy().into_owned();
                            self.native_paths[index] = Some(path);
                        }
                    }
                });
            }
        });
    }

    fn panes(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let workspace = self.workspace.as_mut().unwrap();
        let font = comparison_font(ui.ctx());
        let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
        ui.columns(3, |columns| {
            for (index, column) in columns.iter_mut().enumerate() {
                let (label, color) = match index {
                    0 => ("INCOMING", palette.left_only),
                    1 => (
                        if workspace.is_dirty() {
                            "RESULT · EDITABLE · UNSAVED"
                        } else {
                            "RESULT · EDITABLE"
                        },
                        palette.accent,
                    ),
                    _ => ("CURRENT", palette.right_only),
                };
                column.colored_label(color, RichText::new(label).strong());
                let path = if index == 1 {
                    self.output
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "Choose destination with Save".into())
                } else {
                    self.loaded_paths.as_ref().unwrap_or(&self.paths)[index].clone()
                };
                column.add(
                    egui::Label::new(RichText::new(path).monospace().color(palette.muted))
                        .truncate(),
                );
            }
        });
        let mut choice = None;
        let mut result = workspace.result().to_owned();
        let before = result.clone();
        let actions = workspace.actions().to_vec();
        let height = ui.available_height().max(0.0);
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
        self.scroll_y = outer.state.offset.y;
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
            let incoming = text_center(&output, "INCOMING");
            let result = text_center(&output, "RESULT · EDITABLE");
            let current = text_center(&output, "CURRENT");
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
            let output = render(&mut view, &ctx, Vec::new());
            click(&mut view, &ctx, text_center(&output, "Undo"));
            assert_eq!(
                view.workspace.as_ref().unwrap().result(),
                "heading\na\nb\nmode=current\nend\n"
            );
            let output = render(&mut view, &ctx, Vec::new());
            click(
                &mut view,
                &ctx,
                text_center(&output, "Auto merge nonconflicting"),
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
        while view.load.is_some() || view.save.is_some() {
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
}
