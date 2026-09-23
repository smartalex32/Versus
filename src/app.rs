use eframe::egui::{self, Color32, RichText, TextEdit};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool, mpsc},
    thread,
};
use versus::{
    DirectoryCompareOptions, DirectoryDiff, DirectoryEntry, DirectoryEntryKind,
    DirectoryEntryState, FileCompareOptions, FileDiff, FileDiffKind, LineEnding, MergeChoice,
    MergeHunkKind, SaveOptions, TextDiff, ThreeWayDiff, compare_directories, compare_files,
    compare_texts, compare_three_texts, save_text_safely,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Directory,
    TwoWay,
    ThreeWay,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum DirectoryFilter {
    All,
    Different,
    Same,
    LeftOnly,
    RightOnly,
}
struct DirectoryJob {
    receiver: mpsc::Receiver<Result<DirectoryDiff, String>>,
    cancelled: Arc<AtomicBool>,
    left_root: String,
    right_root: String,
}
enum FileJobResult {
    Two(FileDiff, String, String, String, String),
    Three(String, String, String, ThreeWayDiff, String, String, String),
}
struct FileJob {
    receiver: mpsc::Receiver<Result<FileJobResult, String>>,
}
#[derive(Default)]
struct TextUndo {
    undo: Vec<String>,
    redo: Vec<String>,
}
impl TextUndo {
    fn changed(&mut self, before: String) {
        self.undo.push(before);
        self.redo.clear();
    }
    fn undo(&mut self, value: &mut String) {
        if let Some(previous) = self.undo.pop() {
            self.redo.push(std::mem::replace(value, previous));
        }
    }
    fn redo(&mut self, value: &mut String) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(value, next));
        }
    }
}

pub struct VersusApp {
    mode: Mode,
    left_path: String,
    right_path: String,
    base_path: String,
    directory_left_path: String,
    directory_right_path: String,
    compared_directory_left: String,
    compared_directory_right: String,
    two_loaded_left_path: String,
    two_loaded_right_path: String,
    three_loaded_base_path: String,
    three_loaded_left_path: String,
    three_loaded_right_path: String,
    directory: Option<DirectoryDiff>,
    directory_job: Option<DirectoryJob>,
    file_job: Option<FileJob>,
    directory_filter: DirectoryFilter,
    two_diff: Option<TextDiff>,
    two_kind: Option<FileDiffKind>,
    two_equal: Option<bool>,
    two_left_ending: LineEnding,
    two_right_ending: LineEnding,
    three_result_ending: LineEnding,
    left_text: String,
    right_text: String,
    two_selected: usize,
    left_undo: TextUndo,
    right_undo: TextUndo,
    three_diff: Option<ThreeWayDiff>,
    resolved_merge_hunks: Vec<bool>,
    three_selected: usize,
    base_text: String,
    merge_left: String,
    merge_right: String,
    merged_text: String,
    merged_undo: TextUndo,
    ignore_line_endings: bool,
    ignore_whitespace: bool,
    confirm_overwrite: bool,
    status: String,
    save_as_open: bool,
    save_as_path: String,
    save_as_contents: String,
    save_as_line_ending: LineEnding,
    pending_overwrite: Option<(PathBuf, String, LineEnding)>,
    two_scroll: f32,
}

impl VersusApp {
    pub fn new(_: &eframe::CreationContext<'_>) -> Self {
        let settings = load_settings();
        Self {
            mode: Mode::Directory,
            left_path: String::new(),
            right_path: String::new(),
            base_path: String::new(),
            directory_left_path: String::new(),
            directory_right_path: String::new(),
            compared_directory_left: String::new(),
            compared_directory_right: String::new(),
            two_loaded_left_path: String::new(),
            two_loaded_right_path: String::new(),
            three_loaded_base_path: String::new(),
            three_loaded_left_path: String::new(),
            three_loaded_right_path: String::new(),
            directory: None,
            directory_job: None,
            file_job: None,
            directory_filter: DirectoryFilter::Different,
            two_diff: None,
            two_kind: None,
            two_equal: None,
            two_left_ending: LineEnding::Lf,
            two_right_ending: LineEnding::Lf,
            three_result_ending: LineEnding::Lf,
            left_text: String::new(),
            right_text: String::new(),
            two_selected: 0,
            left_undo: TextUndo::default(),
            right_undo: TextUndo::default(),
            three_diff: None,
            resolved_merge_hunks: Vec::new(),
            three_selected: 0,
            base_text: String::new(),
            merge_left: String::new(),
            merge_right: String::new(),
            merged_text: String::new(),
            merged_undo: TextUndo::default(),
            ignore_line_endings: settings.ignore_line_endings,
            ignore_whitespace: settings.ignore_whitespace,
            confirm_overwrite: settings.confirm_overwrite,
            status: "Choose paths and compare.".into(),
            save_as_open: false,
            save_as_path: String::new(),
            save_as_contents: String::new(),
            save_as_line_ending: LineEnding::Lf,
            pending_overwrite: None,
            two_scroll: 0.0,
        }
    }
    fn options(&self) -> FileCompareOptions {
        FileCompareOptions {
            ignore_line_endings: self.ignore_line_endings,
            ignore_whitespace: self.ignore_whitespace,
            ..Default::default()
        }
    }
    fn persist_settings(&mut self) {
        if let Err(error) = save_settings(Settings {
            ignore_line_endings: self.ignore_line_endings,
            ignore_whitespace: self.ignore_whitespace,
            confirm_overwrite: self.confirm_overwrite,
        }) {
            self.status = format!("Unable to save settings: {error}");
        }
    }
    fn selected_hunk_scroll(&mut self) {
        if let Some(diff) = &self.two_diff {
            if let Some(hunk) = diff.hunks.get(self.two_selected) {
                self.two_scroll = hunk.left_range.start.min(hunk.right_range.start) as f32 * 18.0;
            }
        }
    }
    fn poll_file_job(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.file_job {
            match job.receiver.try_recv() {
                Ok(Ok(FileJobResult::Two(file, left, right, left_path, right_path))) => {
                    self.two_kind = Some(file.kind.clone());
                    self.two_equal = Some(file.equal);
                    self.two_diff = file.text;
                    if file.kind == FileDiffKind::Text {
                        self.left_text = left;
                        self.right_text = right;
                        self.two_left_ending = detected_line_ending(&self.left_text);
                        self.two_right_ending = detected_line_ending(&self.right_text);
                        self.left_undo = TextUndo::default();
                        self.right_undo = TextUndo::default();
                        self.two_selected = 0;
                    }
                    self.two_loaded_left_path = left_path;
                    self.two_loaded_right_path = right_path;
                    self.status = if file.equal {
                        "Files match.".into()
                    } else {
                        "File comparison ready.".into()
                    };
                    self.file_job = None;
                }
                Ok(Ok(FileJobResult::Three(
                    base,
                    left,
                    right,
                    diff,
                    base_path,
                    left_path,
                    right_path,
                ))) => {
                    self.three_result_ending = detected_line_ending(&right);
                    self.base_text = base;
                    self.merge_left = left;
                    self.merge_right = right;
                    self.merged_text = diff.merged.clone();
                    self.three_diff = Some(diff);
                    self.three_loaded_base_path = base_path;
                    self.three_loaded_left_path = left_path;
                    self.three_loaded_right_path = right_path;
                    self.resolved_merge_hunks =
                        vec![false; self.three_diff.as_ref().map_or(0, |diff| diff.hunks.len())];
                    self.merged_undo = TextUndo::default();
                    self.three_selected = 0;
                    self.status = "Three-way merge ready.".into();
                    self.file_job = None;
                }
                Ok(Err(error)) => {
                    self.status = error;
                    self.file_job = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100))
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.status = "File comparison worker stopped unexpectedly.".into();
                    self.file_job = None;
                }
            }
        }
    }
    fn poll_directory_job(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.directory_job {
            match job.receiver.try_recv() {
                Ok(Ok(result)) => {
                    self.status = if result.cancelled {
                        "Directory comparison cancelled.".into()
                    } else {
                        format!("Compared {} entries.", result.entries.len())
                    };
                    self.directory = Some(result);
                    self.compared_directory_left = job.left_root.clone();
                    self.compared_directory_right = job.right_root.clone();
                    self.directory_job = None;
                }
                Ok(Err(error)) => {
                    self.status = error;
                    self.directory_job = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100))
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.status = "Directory comparison worker stopped unexpectedly.".into();
                    self.directory_job = None;
                }
            }
        }
    }
    fn start_directory_compare(&mut self) {
        let left = PathBuf::from(self.directory_left_path.trim());
        let right = PathBuf::from(self.directory_right_path.trim());
        if left.as_os_str().is_empty() || right.as_os_str().is_empty() {
            self.status = "Enter both directory paths.".into();
            return;
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancelled);
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let options = DirectoryCompareOptions {
                cancellation: Some(worker_cancel),
                ..Default::default()
            };
            let _ =
                sender.send(compare_directories(left, right, &options).map_err(|e| e.to_string()));
        });
        self.directory_job = Some(DirectoryJob {
            receiver,
            cancelled,
            left_root: self.directory_left_path.trim().to_owned(),
            right_root: self.directory_right_path.trim().to_owned(),
        });
        self.status = "Scanning directories…".into();
    }
    fn compare_two_paths(&mut self) {
        self.two_diff = None;
        self.two_kind = None;
        self.two_equal = None;
        self.two_loaded_left_path.clear();
        self.two_loaded_right_path.clear();
        let left = self.left_path.trim().to_owned();
        let right = self.right_path.trim().to_owned();
        let options = self.options();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = compare_files(&left, &right, &options)
                .map_err(|e| e.to_string())
                .and_then(|file| {
                    if file.kind != FileDiffKind::Text {
                        return Ok(FileJobResult::Two(
                            file,
                            String::new(),
                            String::new(),
                            left,
                            right,
                        ));
                    }
                    let read = |p: &str| {
                        let metadata = std::fs::metadata(p).map_err(|e| e.to_string())?;
                        if metadata.len() > versus::DEFAULT_TEXT_SIZE_LIMIT {
                            return Err(format!(
                                "{p}: file exceeds the text limit of {} bytes",
                                versus::DEFAULT_TEXT_SIZE_LIMIT
                            ));
                        }
                        std::fs::read_to_string(p)
                            .map(|s| s.strip_prefix('\u{feff}').unwrap_or(&s).to_owned())
                            .map_err(|e| e.to_string())
                    };
                    Ok(FileJobResult::Two(
                        file,
                        read(&left)?,
                        read(&right)?,
                        left,
                        right,
                    ))
                });
            let _ = sender.send(result);
        });
        self.file_job = Some(FileJob { receiver });
        self.status = "Comparing files…".into();
    }
    fn refresh_two_diff(&mut self) {
        self.two_diff = Some(compare_texts(
            &self.left_text,
            &self.right_text,
            &self.options(),
        ));
    }
    fn compare_three_paths(&mut self) {
        self.three_diff = None;
        self.three_loaded_base_path.clear();
        self.three_loaded_left_path.clear();
        self.three_loaded_right_path.clear();
        let base_path = self.base_path.trim().to_owned();
        let left_path = self.left_path.trim().to_owned();
        let right_path = self.right_path.trim().to_owned();
        let options = self.options();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let read = |p: &str| {
                let metadata = std::fs::metadata(p).map_err(|e| e.to_string())?;
                if metadata.len() > versus::DEFAULT_TEXT_SIZE_LIMIT {
                    return Err(format!(
                        "{p}: file exceeds the text limit of {} bytes",
                        versus::DEFAULT_TEXT_SIZE_LIMIT
                    ));
                }
                std::fs::read_to_string(p)
                    .map(|s| s.strip_prefix('\u{feff}').unwrap_or(&s).to_owned())
                    .map_err(|e| e.to_string())
            };
            let result = (|| {
                let base = read(&base_path)?;
                let left = read(&left_path)?;
                let right = read(&right_path)?;
                let diff = compare_three_texts(&base, &left, &right, &options)
                    .map_err(|e| e.to_string())?;
                Ok(FileJobResult::Three(
                    base, left, right, diff, base_path, left_path, right_path,
                ))
            })();
            let _ = sender.send(result);
        });
        self.file_job = Some(FileJob { receiver });
        self.status = "Comparing three files…".into();
    }
    fn save(&mut self, target: &str, contents: &str, line_ending: LineEnding) {
        let path = PathBuf::from(target.trim());
        if path.as_os_str().is_empty() {
            self.save_as_open = true;
            return;
        }
        let target_exists = path.exists() || std::fs::symlink_metadata(&path).is_ok();
        if target_exists && self.confirm_overwrite {
            self.pending_overwrite = Some((path, contents.to_owned(), line_ending));
            return;
        }
        match save_text_safely(
            &path,
            contents,
            &SaveOptions {
                overwrite: target_exists,
                line_ending,
            },
        ) {
            Ok(()) => self.status = format!("Saved {}.", path.display()),
            Err(error) => self.status = error.to_string(),
        }
    }
    fn open_save_as(&mut self, contents: String, line_ending: LineEnding) {
        self.save_as_contents = contents;
        self.save_as_line_ending = line_ending;
        self.save_as_open = true;
    }
    fn copy_hunk(&mut self, left_to_right: bool) {
        let Some(diff) = &self.two_diff else { return };
        let Some(hunk) = diff.hunks.get(self.two_selected) else {
            return;
        };
        let source = if left_to_right {
            &self.left_text
        } else {
            &self.right_text
        };
        let replacement: Vec<String> = source
            .lines()
            .skip(if left_to_right {
                hunk.left_range.start
            } else {
                hunk.right_range.start
            })
            .take(if left_to_right {
                hunk.left_range.len()
            } else {
                hunk.right_range.len()
            })
            .map(str::to_owned)
            .collect();
        let source_range = if left_to_right {
            hunk.left_range.clone()
        } else {
            hunk.right_range.clone()
        };
        let source_line_count = source.lines().count();
        let source_terminal_newline = source.ends_with('\n');
        let target = if left_to_right {
            &mut self.right_text
        } else {
            &mut self.left_text
        };
        let undo = if left_to_right {
            &mut self.right_undo
        } else {
            &mut self.left_undo
        };
        let range = if left_to_right {
            hunk.right_range.clone()
        } else {
            hunk.left_range.clone()
        };
        let terminal_newline =
            if source_range.end == source_line_count && range.end == target.lines().count() {
                source_terminal_newline
            } else {
                target.ends_with('\n')
            };
        let before = target.clone();
        *target = replace_lines_preserving_style(target, range, replacement, terminal_newline);
        undo.changed(before);
        self.refresh_two_diff();
        self.status = "Copied selected difference. Save explicitly to write the file.".into();
    }
    fn path_field(ui: &mut egui::Ui, label: &str, path: &mut String, directory: bool) {
        ui.label(label);
        ui.horizontal(|ui| {
            ui.add(TextEdit::singleline(path).desired_width(ui.available_width() - 72.0));
            if ui.button("Browse").clicked() {
                let pick = if directory {
                    rfd::FileDialog::new().pick_folder()
                } else {
                    rfd::FileDialog::new().pick_file()
                };
                if let Some(path_buf) = pick {
                    *path = path_buf.display().to_string();
                }
            }
        });
    }
    fn accept_drops(&mut self, ctx: &egui::Context) {
        for file in ctx.input(|input| input.raw.dropped_files.clone()) {
            let path = file.path();
            let target = match self.mode {
                Mode::Directory if self.directory_left_path.is_empty() => {
                    &mut self.directory_left_path
                }
                Mode::Directory => &mut self.directory_right_path,
                Mode::ThreeWay if self.base_path.is_empty() => &mut self.base_path,
                _ if self.left_path.is_empty() => &mut self.left_path,
                _ => &mut self.right_path,
            };
            *target = path.display().to_string();
            self.status = "Added dropped path.".into();
        }
    }
}

impl eframe::App for VersusApp {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.accept_drops(ctx);
        self.poll_directory_job(ctx);
        self.poll_file_job(ctx);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        ui.horizontal(|ui| {
            ui.heading("Versus");
            ui.separator();
            for (mode, label) in [
                (Mode::Directory, "Directory"),
                (Mode::TwoWay, "2-Way"),
                (Mode::ThreeWay, "3-Way"),
            ] {
                if ui.selectable_label(self.mode == mode, label).clicked() {
                    self.mode = mode;
                }
            }
            ui.separator();
            ui.menu_button("Settings", |ui| {
                let mut changed = ui
                    .checkbox(
                        &mut self.ignore_line_endings,
                        "Ignore line-ending differences",
                    )
                    .changed();
                changed |= ui
                    .checkbox(&mut self.ignore_whitespace, "Ignore whitespace")
                    .changed();
                changed |= ui
                    .checkbox(&mut self.confirm_overwrite, "Confirm before overwrite")
                    .changed();
                if changed {
                    self.persist_settings();
                }
            });
        });
        ui.separator();
        match self.mode {
            Mode::Directory => self.directory_ui(ui),
            Mode::TwoWay => self.two_way_ui(ui),
            Mode::ThreeWay => self.three_way_ui(ui),
        };
        ui.separator();
        ui.label(&self.status);
        self.save_dialog(ui.ctx());
    }
}

impl VersusApp {
    fn directory_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Directory comparison");
        Self::path_field(ui, "Left directory", &mut self.directory_left_path, true);
        Self::path_field(ui, "Right directory", &mut self.directory_right_path, true);
        ui.horizontal(|ui| {
            if self.directory_job.is_some() {
                if ui.button("Cancel").clicked() {
                    if let Some(job) = &self.directory_job {
                        job.cancelled
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            } else if ui.button("Compare directories").clicked() {
                self.start_directory_compare();
            }
            for (filter, label) in [
                (DirectoryFilter::All, "All"),
                (DirectoryFilter::Different, "Different"),
                (DirectoryFilter::Same, "Same"),
                (DirectoryFilter::LeftOnly, "Left Only"),
                (DirectoryFilter::RightOnly, "Right Only"),
            ] {
                ui.selectable_value(&mut self.directory_filter, filter, label);
            }
        });
        let entries: Vec<DirectoryEntry> = self
            .directory
            .as_ref()
            .map(|d| {
                d.entries
                    .iter()
                    .filter(|e| match self.directory_filter {
                        DirectoryFilter::All => true,
                        DirectoryFilter::Different => matches!(
                            e.state,
                            DirectoryEntryState::Different
                                | DirectoryEntryState::LeftOnly
                                | DirectoryEntryState::RightOnly
                                | DirectoryEntryState::TypeMismatch
                                | DirectoryEntryState::Error(_)
                        ),
                        DirectoryFilter::Same => matches!(e.state, DirectoryEntryState::Same),
                        DirectoryFilter::LeftOnly => {
                            matches!(e.state, DirectoryEntryState::LeftOnly)
                        }
                        DirectoryFilter::RightOnly => {
                            matches!(e.state, DirectoryEntryState::RightOnly)
                        }
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("directory-results")
                .striped(true)
                .min_col_width(150.0)
                .show(ui, |ui| {
                    ui.strong("Status");
                    ui.strong("Type");
                    ui.strong("Path");
                    ui.end_row();
                    for entry in entries {
                        let state = directory_state(&entry.state);
                        let color = state_color(&entry.state);
                        ui.colored_label(color, state);
                        ui.label(format!("{:?}", entry.kind));
                        let response =
                            ui.selectable_label(false, entry.relative_path.display().to_string());
                        if response.double_clicked()
                            && matches!(entry.kind, DirectoryEntryKind::File)
                            && matches!(
                                entry.state,
                                DirectoryEntryState::Different | DirectoryEntryState::Same
                            )
                        {
                            self.left_path = Path::new(&self.compared_directory_left)
                                .join(&entry.relative_path)
                                .display()
                                .to_string();
                            self.right_path = Path::new(&self.compared_directory_right)
                                .join(&entry.relative_path)
                                .display()
                                .to_string();
                            self.mode = Mode::TwoWay;
                            self.compare_two_paths();
                        }
                        ui.end_row();
                    }
                });
        });
    }
    fn two_way_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("2-Way file comparison");
        Self::path_field(ui, "Left file", &mut self.left_path, false);
        Self::path_field(ui, "Right file", &mut self.right_path, false);
        ui.horizontal(|ui| {
            if ui.button("Compare / Reload").clicked() {
                self.compare_two_paths();
            }
            if let Some(diff) = &self.two_diff {
                let count = diff.hunks.len();
                ui.label(format!(
                    "Difference {} of {}",
                    if count == 0 { 0 } else { self.two_selected + 1 },
                    count
                ));
                if ui.button("Previous").clicked() && count > 0 {
                    self.two_selected = (self.two_selected + count - 1) % count;
                    self.selected_hunk_scroll();
                }
                if ui.button("Next").clicked() && count > 0 {
                    self.two_selected = (self.two_selected + 1) % count;
                    self.selected_hunk_scroll();
                }
                if ui.button("← Copy").clicked() {
                    self.copy_hunk(false);
                }
                if ui.button("Copy →").clicked() {
                    self.copy_hunk(true);
                }
            }
        });
        if let Some(kind) = &self.two_kind {
            if !matches!(kind, FileDiffKind::Text) {
                ui.colored_label(
                    Color32::YELLOW,
                    match kind {
                        FileDiffKind::Binary if self.two_equal == Some(true) => {
                            "Binary files match; text display is unavailable."
                        }
                        FileDiffKind::Binary => "Binary files differ; text display is unavailable.",
                        FileDiffKind::TooLarge if self.two_equal == Some(true) => {
                            "Large files match; text display is unavailable."
                        }
                        FileDiffKind::TooLarge => {
                            "Large files differ; text display is unavailable."
                        }
                        FileDiffKind::Text => "",
                    },
                );
                return;
            }
        }
        let diff = self.two_diff.clone();
        ui.columns(2, |columns| {
            let Some(diff) = diff.as_ref() else { return };
            let requested_offset = self.two_scroll;
            let left_offset = diff_pane(
                &mut columns[0],
                "LEFT",
                &self.left_text,
                diff,
                true,
                self.two_selected,
                self.two_scroll,
            );
            let right_offset = diff_pane(
                &mut columns[1],
                "RIGHT",
                &self.right_text,
                diff,
                false,
                self.two_selected,
                left_offset,
            );
            self.two_scroll = if (left_offset - requested_offset).abs() > 0.1 {
                left_offset
            } else {
                right_offset
            };
        });
        ui.separator();
        ui.label("Edit buffers");
        ui.columns(2, |columns| {
            text_editor(
                &mut columns[0],
                "LEFT",
                &mut self.left_text,
                &mut self.left_undo,
            );
            text_editor(
                &mut columns[1],
                "RIGHT",
                &mut self.right_text,
                &mut self.right_undo,
            );
        });
        if ui.button("Recalculate edited diff").clicked() {
            self.refresh_two_diff();
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.two_loaded_left_path.is_empty(),
                    egui::Button::new("Save left"),
                )
                .clicked()
            {
                let path = self.two_loaded_left_path.clone();
                let content = self.left_text.clone();
                self.save(&path, &content, self.two_left_ending);
            }
            if ui
                .add_enabled(
                    !self.two_loaded_right_path.is_empty(),
                    egui::Button::new("Save right"),
                )
                .clicked()
            {
                let path = self.two_loaded_right_path.clone();
                let content = self.right_text.clone();
                self.save(&path, &content, self.two_right_ending);
            }
            if ui
                .add_enabled(
                    !self.two_loaded_right_path.is_empty(),
                    egui::Button::new("Save right as…"),
                )
                .clicked()
            {
                self.open_save_as(self.right_text.clone(), self.two_right_ending);
            }
        });
    }
    fn three_way_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("3-Way merge");
        Self::path_field(ui, "Base file", &mut self.base_path, false);
        Self::path_field(ui, "Left file", &mut self.left_path, false);
        Self::path_field(ui, "Right file", &mut self.right_path, false);
        ui.horizontal(|ui| {
            if ui.button("Compare / Reload").clicked() {
                self.compare_three_paths();
            }
            if let Some(diff) = &self.three_diff {
                let conflicts: Vec<usize> = diff
                    .hunks
                    .iter()
                    .enumerate()
                    .filter_map(|(i, h)| (h.kind == MergeHunkKind::Conflict).then_some(i))
                    .collect();
                ui.label(format!(
                    "Conflict {} of {}",
                    conflicts
                        .iter()
                        .position(|&i| i == self.three_selected)
                        .map(|i| i + 1)
                        .unwrap_or(0),
                    conflicts.len()
                ));
                if ui.button("Previous conflict").clicked() {
                    if let Some(&index) = conflicts
                        .iter()
                        .rev()
                        .find(|&&i| i < self.three_selected)
                        .or_else(|| conflicts.last())
                    {
                        self.three_selected = index;
                    }
                }
                if ui.button("Next conflict").clicked() {
                    if let Some(&index) = conflicts
                        .iter()
                        .find(|&&i| i > self.three_selected)
                        .or_else(|| conflicts.first())
                    {
                        self.three_selected = index;
                    }
                }
                if ui.button("Use Left").clicked() {
                    self.apply_merge_choice(MergeChoice::Left);
                }
                if ui.button("Use Right").clicked() {
                    self.apply_merge_choice(MergeChoice::Right);
                }
            }
        });
        ui.columns(3, |columns| {
            readonly_lines(&mut columns[0], "BASE", &self.base_text);
            readonly_lines(&mut columns[1], "LEFT", &self.merge_left);
            readonly_lines(&mut columns[2], "RIGHT", &self.merge_right);
        });
        ui.separator();
        text_editor(
            ui,
            "MERGED RESULT",
            &mut self.merged_text,
            &mut self.merged_undo,
        );
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.three_diff.is_some(),
                    egui::Button::new("Save result as…"),
                )
                .clicked()
            {
                self.open_save_as(self.merged_text.clone(), self.three_result_ending);
            }
            if ui
                .add_enabled(
                    !self.three_loaded_right_path.is_empty(),
                    egui::Button::new("Save result to right"),
                )
                .clicked()
            {
                let path = self.three_loaded_right_path.clone();
                let content = self.merged_text.clone();
                self.save(&path, &content, self.three_result_ending);
            }
        });
    }
    fn apply_merge_choice(&mut self, choice: MergeChoice) {
        let Some(diff) = &self.three_diff else { return };
        let Some(hunk) = diff.hunks.get(self.three_selected) else {
            return;
        };
        if hunk.kind != MergeHunkKind::Conflict {
            return;
        }
        let before = self.merged_text.clone();
        let marker = conflict_marker(hunk);
        let occurrence = diff.hunks[..self.three_selected]
            .iter()
            .enumerate()
            .filter(|(index, earlier)| {
                earlier.kind == MergeHunkKind::Conflict
                    && !self
                        .resolved_merge_hunks
                        .get(*index)
                        .copied()
                        .unwrap_or(false)
                    && conflict_marker(earlier) == marker
            })
            .count();
        self.merged_text = resolve_current_conflict(&self.merged_text, hunk, choice, occurrence);
        if let Some(resolved) = self.resolved_merge_hunks.get_mut(self.three_selected) {
            *resolved = true;
        }
        self.merged_undo.changed(before);
        self.status = "Applied conflict choice; save explicitly to write the result.".into();
    }
    fn save_dialog(&mut self, ctx: &egui::Context) {
        if self.save_as_open {
            egui::Window::new("Save As")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label("Destination path");
                    ui.text_edit_singleline(&mut self.save_as_path);
                    if ui.button("Browse").clicked() {
                        if let Some(path) = rfd::FileDialog::new().save_file() {
                            self.save_as_path = path.display().to_string();
                        }
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            let path = self.save_as_path.clone();
                            let contents = self.save_as_contents.clone();
                            self.save(&path, &contents, self.save_as_line_ending);
                            self.save_as_open = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.save_as_open = false;
                        }
                    });
                });
        }
        if let Some((path, contents, line_ending)) = self.pending_overwrite.clone() {
            egui::Window::new("Overwrite existing file?")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(format!("{} already exists.", path.display()));
                    ui.horizontal(|ui| {
                        if ui.button("Overwrite").clicked() {
                            match save_text_safely(
                                &path,
                                &contents,
                                &SaveOptions {
                                    overwrite: true,
                                    line_ending,
                                },
                            ) {
                                Ok(()) => self.status = format!("Saved {}.", path.display()),
                                Err(error) => self.status = error.to_string(),
                            }
                            self.pending_overwrite = None;
                        }
                        if ui.button("Cancel").clicked() {
                            self.pending_overwrite = None;
                        }
                    });
                });
        }
    }
}
fn text_editor(ui: &mut egui::Ui, title: &str, text: &mut String, undo: &mut TextUndo) {
    ui.label(RichText::new(title).strong());
    ui.horizontal(|ui| {
        if ui.button("Undo").clicked() {
            undo.undo(text);
        }
        if ui.button("Redo").clicked() {
            undo.redo(text);
        }
    });
    let before = text.clone();
    let response = ui.add(
        TextEdit::multiline(text)
            .code_editor()
            .desired_rows(22)
            .desired_width(f32::INFINITY),
    );
    if response.changed() {
        undo.changed(before);
    }
}
fn diff_pane(
    ui: &mut egui::Ui,
    title: &str,
    text: &str,
    diff: &TextDiff,
    left: bool,
    selected: usize,
    offset: f32,
) -> f32 {
    ui.label(RichText::new(title).strong());
    let lines: Vec<&str> = text.lines().collect();
    let output = egui::ScrollArea::vertical()
        .id_salt(if left {
            "two-left-diff"
        } else {
            "two-right-diff"
        })
        .max_height(260.0)
        .vertical_scroll_offset(offset)
        .show_rows(
            ui,
            ui.text_style_height(&egui::TextStyle::Monospace),
            lines.len(),
            |ui, rows| {
                for index in rows {
                    let line = lines[index];
                    let matching = diff.hunks.iter().enumerate().find(|(_, hunk)| {
                        let range = if left {
                            &hunk.left_range
                        } else {
                            &hunk.right_range
                        };
                        range.contains(&index)
                    });
                    let color = match matching.map(|(_, hunk)| &hunk.kind) {
                        Some(versus::HunkKind::Added) => Color32::from_rgb(38, 100, 62),
                        Some(versus::HunkKind::Removed) => Color32::from_rgb(110, 46, 46),
                        Some(versus::HunkKind::Changed) => Color32::from_rgb(105, 84, 35),
                        None => Color32::TRANSPARENT,
                    };
                    egui::Frame::default().fill(color).show(ui, |ui| {
                        let marker = matching.is_some_and(|(hunk_index, _)| hunk_index == selected);
                        ui.monospace(format!(
                            "{} {:>5} | {}",
                            if marker { ">" } else { " " },
                            index + 1,
                            line
                        ));
                    });
                }
            },
        );
    output.state.offset.y
}
fn detected_line_ending(text: &str) -> LineEnding {
    if text.matches("\r\n").count() * 2 >= text.matches('\n').count() {
        LineEnding::Crlf
    } else {
        LineEnding::Lf
    }
}
fn replace_lines_preserving_style(
    target: &str,
    range: std::ops::Range<usize>,
    replacement: Vec<String>,
    source_terminal_newline: bool,
) -> String {
    let ending = if target.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut lines: Vec<String> = if target.is_empty() {
        Vec::new()
    } else {
        target
            .lines()
            .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned())
            .collect()
    };
    lines.splice(
        range,
        replacement
            .into_iter()
            .map(|line| line.strip_suffix('\r').unwrap_or(&line).to_owned()),
    );
    let mut result = lines.join(ending);
    if source_terminal_newline && !result.ends_with('\n') {
        result.push_str(ending);
    }
    result
}
fn conflict_marker(hunk: &versus::MergeHunk) -> String {
    format!(
        "<<<<<<< LEFT\n{}\n=======\n{}\n>>>>>>> RIGHT",
        hunk.left.join("\n"),
        hunk.right.join("\n")
    )
}
fn resolve_current_conflict(
    current: &str,
    hunk: &versus::MergeHunk,
    choice: MergeChoice,
    occurrence: usize,
) -> String {
    let marker = conflict_marker(hunk);
    let replacement = match choice {
        MergeChoice::Left => hunk.left.join("\n"),
        MergeChoice::Right => hunk.right.join("\n"),
    };
    let mut search_start = 0;
    for index in 0..=occurrence {
        let Some(relative) = current[search_start..].find(&marker) else {
            return current.to_owned();
        };
        search_start += relative;
        if index == occurrence {
            return format!(
                "{}{}{}",
                &current[..search_start],
                replacement,
                &current[search_start + marker.len()..]
            );
        }
        search_start += marker.len();
    }
    current.to_owned()
}
fn readonly_lines(ui: &mut egui::Ui, title: &str, text: &str) {
    ui.label(RichText::new(title).strong());
    egui::ScrollArea::vertical()
        .max_height(180.0)
        .show(ui, |ui| {
            for (number, line) in text.lines().enumerate() {
                ui.monospace(format!("{:>5} | {}", number + 1, line));
            }
        });
}
fn directory_state(state: &DirectoryEntryState) -> String {
    match state {
        DirectoryEntryState::Same => "Same".into(),
        DirectoryEntryState::Different => "Different".into(),
        DirectoryEntryState::LeftOnly => "Left Only".into(),
        DirectoryEntryState::RightOnly => "Right Only".into(),
        DirectoryEntryState::TypeMismatch => "Type Mismatch".into(),
        DirectoryEntryState::Error(error) => format!("Error: {error}"),
    }
}
fn state_color(state: &DirectoryEntryState) -> Color32 {
    match state {
        DirectoryEntryState::Same => Color32::GRAY,
        DirectoryEntryState::Different | DirectoryEntryState::TypeMismatch => Color32::YELLOW,
        DirectoryEntryState::LeftOnly => Color32::LIGHT_RED,
        DirectoryEntryState::RightOnly => Color32::LIGHT_GREEN,
        DirectoryEntryState::Error(_) => Color32::RED,
    }
}
#[derive(Clone, Copy)]
struct Settings {
    ignore_line_endings: bool,
    ignore_whitespace: bool,
    confirm_overwrite: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            ignore_line_endings: true,
            ignore_whitespace: false,
            confirm_overwrite: true,
        }
    }
}
fn settings_path() -> Option<PathBuf> {
    let root = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".config").into_os_string())
        })?;
    Some(PathBuf::from(root).join("Versus").join("settings.conf"))
}
fn load_settings() -> Settings {
    let Ok(contents) = settings_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .ok_or(())
    else {
        return Settings::default();
    };
    let mut settings = Settings::default();
    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value == "true";
        match key {
            "ignore_line_endings" => settings.ignore_line_endings = value,
            "ignore_whitespace" => settings.ignore_whitespace = value,
            "confirm_overwrite" => settings.confirm_overwrite = value,
            _ => {}
        }
    }
    settings
}
fn save_settings(settings: Settings) -> Result<(), std::io::Error> {
    let Some(path) = settings_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        format!(
            "ignore_line_endings={}\nignore_whitespace={}\nconfirm_overwrite={}\n",
            settings.ignore_line_endings, settings.ignore_whitespace, settings.confirm_overwrite
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Range;

    fn conflict(left: &[&str], right: &[&str]) -> versus::MergeHunk {
        versus::MergeHunk {
            base_range: Range { start: 0, end: 1 },
            merged_range: Range { start: 0, end: 0 },
            left: left.iter().map(|line| (*line).into()).collect(),
            right: right.iter().map(|line| (*line).into()).collect(),
            kind: MergeHunkKind::Conflict,
        }
    }

    #[test]
    fn resolving_one_conflict_preserves_other_conflicts_and_edits() {
        let first = conflict(&["left one"], &["right one"]);
        let second = conflict(&["left two"], &["right two"]);
        let initial = "manual header\n<<<<<<< LEFT\nleft one\n=======\nright one\n>>>>>>> RIGHT\n<<<<<<< LEFT\nleft two\n=======\nright two\n>>>>>>> RIGHT";
        let after_first = resolve_current_conflict(initial, &first, MergeChoice::Left, 0);
        let final_text = resolve_current_conflict(&after_first, &second, MergeChoice::Right, 0);
        assert_eq!(final_text, "manual header\nleft one\nright two");
    }

    #[test]
    fn resolving_second_identical_conflict_keeps_the_first() {
        let conflict = conflict(&["left"], &["right"]);
        let marker = conflict_marker(&conflict);
        let current = format!("{marker}\nunchanged\n{marker}");
        assert_eq!(
            resolve_current_conflict(&current, &conflict, MergeChoice::Right, 1),
            format!("{marker}\nunchanged\nright")
        );
    }

    #[test]
    fn detects_predominant_line_ending() {
        assert_eq!(detected_line_ending("a\r\nb\r\nc\n"), LineEnding::Crlf);
        assert_eq!(detected_line_ending("a\nb\r\nc\n"), LineEnding::Lf);
    }

    #[test]
    fn copying_lines_preserves_target_newline_and_crlf() {
        assert_eq!(
            replace_lines_preserving_style("one\r\ntwo\r\n", 1..2, vec!["changed".into()], true),
            "one\r\nchanged\r\n"
        );
    }

    #[test]
    fn copying_into_empty_target_does_not_add_a_newline() {
        assert_eq!(
            replace_lines_preserving_style("", 0..0, vec!["new".into()], false),
            "new"
        );
    }

    #[test]
    fn copying_eof_block_can_remove_terminal_newline() {
        assert_eq!(
            replace_lines_preserving_style("old\n", 0..1, vec!["new".into()], false),
            "new"
        );
    }
}
