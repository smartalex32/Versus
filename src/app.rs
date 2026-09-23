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
#[derive(Clone, Copy, PartialEq, Eq)]
enum FileViewFilter {
    All,
    Differences,
    Same,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AlignedRow {
    left: Option<usize>,
    right: Option<usize>,
    hunk: Option<usize>,
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
    logo_texture: egui::TextureHandle,
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
    file_filter: FileViewFilter,
    two_rows: Vec<AlignedRow>,
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
    dark_mode: bool,
    status: String,
    save_as_open: bool,
    save_as_path: String,
    save_as_contents: String,
    save_as_line_ending: LineEnding,
    pending_overwrite: Option<(PathBuf, String, LineEnding)>,
    two_scroll: f32,
    two_edit_open: bool,
    two_diff_dirty: bool,
}

impl VersusApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = load_settings();
        apply_theme(&cc.egui_ctx, settings.dark_mode);
        let logo_image: egui::ColorImage = (&crate::logo::icon_data()).into();
        let logo_texture =
            cc.egui_ctx
                .load_texture("versus-mark", logo_image, egui::TextureOptions::LINEAR);
        Self {
            logo_texture,
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
            file_filter: FileViewFilter::All,
            two_rows: Vec::new(),
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
            dark_mode: settings.dark_mode,
            status: "Choose paths and compare.".into(),
            save_as_open: false,
            save_as_path: String::new(),
            save_as_contents: String::new(),
            save_as_line_ending: LineEnding::Lf,
            pending_overwrite: None,
            two_scroll: 0.0,
            two_edit_open: false,
            two_diff_dirty: false,
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
            dark_mode: self.dark_mode,
        }) {
            self.status = format!("Unable to save settings: {error}");
        }
    }
    fn selected_hunk_scroll(&mut self) {
        if let Some(index) = self
            .two_rows
            .iter()
            .position(|row| row.hunk == Some(self.two_selected))
        {
            self.two_scroll = index as f32 * 18.0;
        }
    }
    fn refresh_two_rows(&mut self) {
        self.two_rows = self
            .two_diff
            .as_ref()
            .map_or_else(Vec::new, |diff| aligned_rows(diff, self.file_filter));
    }
    fn poll_file_job(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.file_job {
            match job.receiver.try_recv() {
                Ok(Ok(FileJobResult::Two(file, left, right, left_path, right_path))) => {
                    self.two_kind = Some(file.kind.clone());
                    self.two_equal = Some(file.equal);
                    self.two_diff = file.text;
                    self.refresh_two_rows();
                    if file.kind == FileDiffKind::Text {
                        self.left_text = left;
                        self.right_text = right;
                        self.two_left_ending = detected_line_ending(&self.left_text);
                        self.two_right_ending = detected_line_ending(&self.right_text);
                        self.left_undo = TextUndo::default();
                        self.right_undo = TextUndo::default();
                        self.two_selected = 0;
                        self.two_diff_dirty = false;
                    } else {
                        self.left_text.clear();
                        self.right_text.clear();
                        self.left_undo = TextUndo::default();
                        self.right_undo = TextUndo::default();
                        self.two_diff_dirty = false;
                        self.two_edit_open = false;
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
                    self.three_selected = self
                        .three_diff
                        .as_ref()
                        .and_then(|diff| {
                            diff.hunks
                                .iter()
                                .position(|h| h.kind == MergeHunkKind::Conflict)
                        })
                        .unwrap_or(0);
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
        self.two_rows.clear();
        self.two_kind = None;
        self.two_equal = None;
        self.two_scroll = 0.0;
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
        let count = self.two_diff.as_ref().map_or(0, |diff| diff.hunks.len());
        self.two_selected = self.two_selected.min(count.saturating_sub(1));
        self.refresh_two_rows();
        self.two_diff_dirty = false;
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
        ui.label(RichText::new(label).strong());
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 60.0).max(80.0);
            ui.add(
                TextEdit::singleline(path)
                    .desired_width(width)
                    .background_color(if ui.visuals().dark_mode {
                        Color32::from_rgb(37, 43, 52)
                    } else {
                        Color32::WHITE
                    })
                    .frame(egui::Frame::group(ui.style())),
            );
            if ui.button("…").on_hover_text("Browse for a path").clicked() {
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
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        let fill = if self.dark_mode {
            Color32::from_rgb(24, 28, 35)
        } else {
            Color32::from_rgb(249, 251, 254)
        };
        fill.to_normalized_gamma_f32()
    }
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.accept_drops(ctx);
        self.poll_directory_job(ctx);
        self.poll_file_job(ctx);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        egui::Frame::default()
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                self.header(ui);
                self.toolbar(ui);
                ui.add_space(8.0);
                match self.mode {
                    Mode::Directory => self.directory_ui(ui),
                    Mode::TwoWay => self.two_way_ui(ui),
                    Mode::ThreeWay => self.three_way_ui(ui),
                };
                self.status_bar(ui);
            });
        self.save_dialog(ui.ctx());
    }
}

impl VersusApp {
    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.image((self.logo_texture.id(), egui::vec2(30.0, 30.0)));
            ui.label(RichText::new("Versus").size(23.0).strong());
            ui.separator();
            let mode_name = match self.mode {
                Mode::Directory => "Directory Compare",
                Mode::TwoWay => "Text Compare",
                Mode::ThreeWay => "3-Way Merge",
            };
            ui.label(RichText::new(mode_name).size(17.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if self.dark_mode {
                    "Light mode"
                } else {
                    "Dark mode"
                };
                if ui
                    .button(label)
                    .on_hover_text("Switch appearance")
                    .clicked()
                {
                    self.dark_mode = !self.dark_mode;
                    apply_theme(ui.ctx(), self.dark_mode);
                    self.persist_settings();
                }
            });
        });
        ui.add_space(5.0);
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Frame::default()
            .fill(toolbar_fill(self.dark_mode))
            .inner_margin(egui::Margin::symmetric(8, 7))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (mode, label) in [(Mode::Directory, "Directory"), (Mode::TwoWay, "2-Way")] {
                        ui.selectable_value(&mut self.mode, mode, label);
                    }
                    ui.separator();
                    let busy = self.directory_job.is_some() || self.file_job.is_some();
                    if self.mode == Mode::Directory && self.directory_job.is_some() {
                        if ui.button("Cancel").clicked() {
                            if let Some(job) = &self.directory_job {
                                job.cancelled
                                    .store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                    } else if ui
                        .add_enabled(!busy, egui::Button::new("Compare"))
                        .clicked()
                    {
                        match self.mode {
                            Mode::Directory => self.start_directory_compare(),
                            Mode::TwoWay => self.compare_two_paths(),
                            Mode::ThreeWay => self.compare_three_paths(),
                        }
                    }
                    if ui
                        .add_enabled(!busy, egui::Button::new("Refresh"))
                        .clicked()
                    {
                        match self.mode {
                            Mode::Directory => self.start_directory_compare(),
                            Mode::TwoWay => self.compare_two_paths(),
                            Mode::ThreeWay => self.compare_three_paths(),
                        }
                    }
                    if self.mode == Mode::TwoWay {
                        self.two_way_actions(ui);
                    }
                    if self.mode == Mode::ThreeWay {
                        self.three_way_actions(ui);
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
            });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.add_space(5.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.colored_label(accent(self.dark_mode), "●");
            ui.label(&self.status);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(if self.directory_job.is_some() || self.file_job.is_some() {
                    "Working…"
                } else {
                    "Ready"
                });
            });
        });
    }

    fn directory_summary(&self, ui: &mut egui::Ui) {
        let Some(directory) = &self.directory else {
            ui.add_space(10.0);
            ui.label("Choose two folders, then select Compare. Double-click a changed file to inspect it.");
            return;
        };
        let mut same = 0;
        let mut different = 0;
        let mut left_only = 0;
        let mut right_only = 0;
        let mut issues = 0;
        for entry in &directory.entries {
            match entry.state {
                DirectoryEntryState::Same => same += 1,
                DirectoryEntryState::Different => different += 1,
                DirectoryEntryState::LeftOnly => left_only += 1,
                DirectoryEntryState::RightOnly => right_only += 1,
                DirectoryEntryState::TypeMismatch | DirectoryEntryState::Error(_) => issues += 1,
            }
        }
        ui.add_space(7.0);
        ui.label(
            RichText::new("Comparison Summary")
                .strong()
                .color(accent(self.dark_mode)),
        );
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("{} items compared", directory.entries.len()));
            ui.separator();
            ui.label(format!("{different} different"));
            ui.label(format!("{left_only} left only"));
            ui.label(format!("{right_only} right only"));
            ui.label(format!("{same} same"));
            if issues > 0 {
                ui.colored_label(Color32::RED, format!("{issues} issues"));
            }
        });
    }

    fn two_way_actions(&mut self, ui: &mut egui::Ui) {
        let text_ready = self.two_kind == Some(FileDiffKind::Text)
            && self.two_diff.is_some()
            && self.file_job.is_none();
        let diff_ready = text_ready && !self.two_diff_dirty;
        let count = self.two_diff.as_ref().map_or(0, |diff| diff.hunks.len());
        if ui
            .add_enabled(diff_ready && count > 0, egui::Button::new("Previous"))
            .clicked()
        {
            self.two_selected = (self.two_selected + count - 1) % count;
            self.file_filter = FileViewFilter::Differences;
            self.refresh_two_rows();
            self.selected_hunk_scroll();
        }
        if ui
            .add_enabled(diff_ready && count > 0, egui::Button::new("Next"))
            .clicked()
        {
            self.two_selected = (self.two_selected + 1) % count;
            self.file_filter = FileViewFilter::Differences;
            self.refresh_two_rows();
            self.selected_hunk_scroll();
        }
        if ui
            .add_enabled(diff_ready && count > 0, egui::Button::new("Copy left"))
            .clicked()
        {
            self.copy_hunk(false);
        }
        if ui
            .add_enabled(diff_ready && count > 0, egui::Button::new("Copy right"))
            .clicked()
        {
            self.copy_hunk(true);
        }
        if ui
            .add_enabled(
                diff_ready,
                egui::Button::new(if self.two_edit_open {
                    "View diff"
                } else {
                    "Edit buffers"
                }),
            )
            .clicked()
        {
            self.two_edit_open = !self.two_edit_open;
        }
        ui.menu_button("Save", |ui| {
            if ui
                .add_enabled(
                    text_ready && !self.two_loaded_left_path.is_empty(),
                    egui::Button::new("Save left"),
                )
                .clicked()
            {
                self.save(
                    &self.two_loaded_left_path.clone(),
                    &self.left_text.clone(),
                    self.two_left_ending,
                );
                ui.close();
            }
            if ui
                .add_enabled(
                    text_ready && !self.two_loaded_right_path.is_empty(),
                    egui::Button::new("Save right"),
                )
                .clicked()
            {
                self.save(
                    &self.two_loaded_right_path.clone(),
                    &self.right_text.clone(),
                    self.two_right_ending,
                );
                ui.close();
            }
            if ui
                .add_enabled(
                    text_ready && !self.two_loaded_right_path.is_empty(),
                    egui::Button::new("Save right as…"),
                )
                .clicked()
            {
                self.open_save_as(self.right_text.clone(), self.two_right_ending);
                ui.close();
            }
        });
    }

    fn three_way_actions(&mut self, ui: &mut egui::Ui) {
        let conflicts: Vec<usize> = self
            .three_diff
            .as_ref()
            .map(|diff| {
                diff.hunks
                    .iter()
                    .enumerate()
                    .filter_map(|(i, h)| {
                        (h.kind == MergeHunkKind::Conflict
                            && !self.resolved_merge_hunks.get(i).copied().unwrap_or(false))
                        .then_some(i)
                    })
                    .collect()
            })
            .unwrap_or_default();
        if ui
            .add_enabled(!conflicts.is_empty(), egui::Button::new("Previous"))
            .clicked()
        {
            if let Some(&index) = conflicts
                .iter()
                .rev()
                .find(|&&i| i < self.three_selected)
                .or_else(|| conflicts.last())
            {
                self.three_selected = index;
            }
        }
        if ui
            .add_enabled(!conflicts.is_empty(), egui::Button::new("Next"))
            .clicked()
        {
            if let Some(&index) = conflicts
                .iter()
                .find(|&&i| i > self.three_selected)
                .or_else(|| conflicts.first())
            {
                self.three_selected = index;
            }
        }
        if ui
            .add_enabled(!conflicts.is_empty(), egui::Button::new("Use left"))
            .clicked()
        {
            self.apply_merge_choice(MergeChoice::Left);
        }
        if ui
            .add_enabled(!conflicts.is_empty(), egui::Button::new("Use right"))
            .clicked()
        {
            self.apply_merge_choice(MergeChoice::Right);
        }
        ui.menu_button("Save result", |ui| {
            if ui
                .add_enabled(self.three_diff.is_some(), egui::Button::new("Save as…"))
                .clicked()
            {
                self.open_save_as(self.merged_text.clone(), self.three_result_ending);
                ui.close();
            }
            if ui
                .add_enabled(
                    !self.three_loaded_right_path.is_empty(),
                    egui::Button::new("Save to right file"),
                )
                .clicked()
            {
                self.save(
                    &self.three_loaded_right_path.clone(),
                    &self.merged_text.clone(),
                    self.three_result_ending,
                );
                ui.close();
            }
        });
    }
    fn directory_ui(&mut self, ui: &mut egui::Ui) {
        ui.columns(2, |columns| {
            Self::path_field(
                &mut columns[0],
                "LEFT  •  Folder",
                &mut self.directory_left_path,
                true,
            );
            Self::path_field(
                &mut columns[1],
                "RIGHT  •  Folder",
                &mut self.directory_right_path,
                true,
            );
        });
        ui.horizontal(|ui| {
            if ui.small_button("Swap sides").clicked() {
                std::mem::swap(
                    &mut self.directory_left_path,
                    &mut self.directory_right_path,
                );
            }
            ui.separator();
            ui.label("Show");
            for (filter, label) in [
                (DirectoryFilter::All, "All"),
                (DirectoryFilter::Different, "Diffs"),
                (DirectoryFilter::Same, "Same"),
            ] {
                ui.selectable_value(&mut self.directory_filter, filter, label);
            }
            ui.menu_button("More ▾", |ui| {
                for (filter, label) in [
                    (DirectoryFilter::LeftOnly, "Left only"),
                    (DirectoryFilter::RightOnly, "Right only"),
                ] {
                    if ui
                        .selectable_value(&mut self.directory_filter, filter, label)
                        .clicked()
                    {
                        ui.close();
                    }
                }
            });
            if self.directory_job.is_some() {
                ui.spinner();
            }
        });
        ui.add_space(4.0);
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
        let table_height = (ui.available_height() - 165.0).max(160.0);
        let side_width = ((ui.available_width() - 190.0) / 2.0).max(80.0);
        ui.label(format!(
            "{} of {} items shown",
            entries.len(),
            self.directory.as_ref().map_or(0, |d| d.entries.len())
        ));
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_height(table_height);
            ui.horizontal(|ui| {
                ui.add_sized(
                    [side_width, 20.0],
                    egui::Label::new(RichText::new("LEFT  •  Name").strong()),
                );
                ui.add_sized(
                    [145.0, 20.0],
                    egui::Label::new(RichText::new("RESULT").strong()),
                );
                ui.add_sized(
                    [side_width, 20.0],
                    egui::Label::new(RichText::new("RIGHT  •  Name").strong()),
                );
            });
            ui.separator();
            if entries.is_empty() {
                ui.label(if self.directory.is_some() {
                    "No items match this view. Choose another filter to see more."
                } else {
                    "Comparison results will appear here."
                });
            }
            egui::ScrollArea::vertical()
                .max_height(table_height)
                .show(ui, |ui| {
                    egui::Grid::new("directory-results")
                        .striped(true)
                        .num_columns(3)
                        .min_col_width(0.0)
                        .show(ui, |ui| {
                            for entry in entries {
                                let name = entry.relative_path.display().to_string();
                                let icon = if matches!(entry.kind, DirectoryEntryKind::Directory) {
                                    "Folder"
                                } else {
                                    "File"
                                };
                                let left_visible =
                                    !matches!(entry.state, DirectoryEntryState::RightOnly);
                                let right_visible =
                                    !matches!(entry.state, DirectoryEntryState::LeftOnly);
                                let left_response = ui.add_sized(
                                    [side_width, 22.0],
                                    egui::Label::new(if left_visible {
                                        format!("{icon}  {name}")
                                    } else {
                                        String::new()
                                    })
                                    .truncate()
                                    .sense(egui::Sense::click()),
                                );
                                let color = state_color(&entry.state, self.dark_mode);
                                ui.add_sized(
                                    [145.0, 22.0],
                                    egui::Label::new(
                                        RichText::new(format!(
                                            "●  {}",
                                            directory_state(&entry.state)
                                        ))
                                        .color(color),
                                    )
                                    .truncate(),
                                );
                                let right_response = ui.add_sized(
                                    [side_width, 22.0],
                                    egui::Label::new(if right_visible {
                                        format!("{icon}  {name}")
                                    } else {
                                        String::new()
                                    })
                                    .truncate()
                                    .sense(egui::Sense::click()),
                                );
                                if (left_response.double_clicked()
                                    || right_response.double_clicked())
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
        });
        self.directory_summary(ui);
    }
    fn two_way_ui(&mut self, ui: &mut egui::Ui) {
        ui.columns(2, |columns| {
            Self::path_field(&mut columns[0], "LEFT  •  File", &mut self.left_path, false);
            Self::path_field(
                &mut columns[1],
                "RIGHT  •  File",
                &mut self.right_path,
                false,
            );
        });
        ui.horizontal(|ui| {
            ui.label(format!("{} bytes  •  UTF-8", self.left_text.len()));
            ui.separator();
            ui.label(format!("{} bytes  •  UTF-8", self.right_text.len()));
            if self.file_job.is_some() {
                ui.spinner();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Show");
            for (filter, label) in [
                (FileViewFilter::All, "All"),
                (FileViewFilter::Differences, "Diffs"),
                (FileViewFilter::Same, "Same"),
            ] {
                if ui
                    .selectable_value(&mut self.file_filter, filter, label)
                    .clicked()
                {
                    self.two_scroll = 0.0;
                    self.refresh_two_rows();
                }
            }
            if let Some(diff) = &self.two_diff {
                ui.separator();
                ui.label(format!("{} differences", diff.hunks.len()));
            }
        });
        ui.add_space(7.0);
        if let Some(kind) = &self.two_kind {
            if !matches!(kind, FileDiffKind::Text) {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.label(match kind {
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
                    });
                });
                return;
            }
        }
        let pane_height = (ui.available_height() - 235.0).max(150.0);
        if self.two_edit_open {
            let edited = ui.columns(2, |columns| {
                let left_changed = text_editor(
                    &mut columns[0],
                    "LEFT  •  Editable buffer",
                    &mut self.left_text,
                    &mut self.left_undo,
                );
                let right_changed = text_editor(
                    &mut columns[1],
                    "RIGHT  •  Editable buffer",
                    &mut self.right_text,
                    &mut self.right_undo,
                );
                left_changed || right_changed
            });
            if edited {
                self.two_diff_dirty = true;
            }
            if ui.button("Recalculate edited diff").clicked() {
                self.refresh_two_diff();
            }
        } else if let Some(diff) = self.two_diff.as_ref() {
            let rows = &self.two_rows;
            let requested_offset = self.two_scroll;
            let selected = self.two_selected;
            let (scroll, clicked) = ui.columns(2, |columns| {
                let (left_offset, left_clicked) = diff_pane(
                    &mut columns[0],
                    "LEFT",
                    diff,
                    &rows,
                    true,
                    selected,
                    requested_offset,
                    pane_height,
                    self.dark_mode,
                );
                let (right_offset, right_clicked) = diff_pane(
                    &mut columns[1],
                    "RIGHT",
                    diff,
                    &rows,
                    false,
                    selected,
                    left_offset,
                    pane_height,
                    self.dark_mode,
                );
                let scroll = if (left_offset - requested_offset).abs() > 0.1 {
                    left_offset
                } else {
                    right_offset
                };
                (scroll, left_clicked.or(right_clicked))
            });
            self.two_scroll = scroll;
            if let Some(index) = clicked {
                self.two_selected = index;
            }
        } else {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.set_min_height(pane_height);
                ui.add_space(20.0);
                ui.label("Choose two files, then select Compare.");
            });
        }
        ui.add_space(7.0);
        ui.label(
            RichText::new("Difference Summary")
                .strong()
                .color(accent(self.dark_mode)),
        );
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            let count = self.two_diff.as_ref().map_or(0, |diff| diff.hunks.len());
            ui.strong(format!("{count} differences"));
            ui.separator();
            ui.label(format!(
                "Difference {} of {count}",
                if count == 0 { 0 } else { self.two_selected + 1 }
            ));
            ui.separator();
            ui.label(if self.two_diff_dirty {
                "Recalculate the diff before copying changes."
            } else {
                "Changes stay in editable buffers until you save."
            });
        });
    }
    fn three_way_ui(&mut self, ui: &mut egui::Ui) {
        ui.columns(3, |columns| {
            Self::path_field(
                &mut columns[0],
                "BASE  •  Common ancestor",
                &mut self.base_path,
                false,
            );
            Self::path_field(
                &mut columns[1],
                "LEFT  •  Branch A",
                &mut self.left_path,
                false,
            );
            Self::path_field(
                &mut columns[2],
                "RIGHT  •  Branch B",
                &mut self.right_path,
                false,
            );
        });
        let source_height = (ui.available_height() * 0.35).clamp(125.0, 280.0);
        ui.add_space(6.0);
        ui.columns(3, |columns| {
            readonly_lines(&mut columns[0], "Base", &self.base_text, source_height);
            readonly_lines(
                &mut columns[1],
                "Left (Branch A)",
                &self.merge_left,
                source_height,
            );
            readonly_lines(
                &mut columns[2],
                "Right (Branch B)",
                &self.merge_right,
                source_height,
            );
        });
        ui.add_space(8.0);
        let remaining = (ui.available_height() - 110.0).max(150.0);
        let result_width = (ui.available_width() * 0.73).max(250.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(result_width);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_min_height(remaining);
                    ui.label(
                        RichText::new("Merged Result  •  Editable")
                            .strong()
                            .color(accent(self.dark_mode)),
                    );
                    let rows = ((remaining - 55.0) / 18.0).max(5.0) as usize;
                    text_editor_sized(ui, &mut self.merged_text, &mut self.merged_undo, rows);
                });
            });
            ui.vertical(|ui| {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_min_height(remaining);
                    ui.strong("Conflicts");
                    ui.separator();
                    if let Some(diff) = &self.three_diff {
                        let unresolved = diff
                            .hunks
                            .iter()
                            .enumerate()
                            .filter(|(i, h)| {
                                h.kind == MergeHunkKind::Conflict
                                    && !self.resolved_merge_hunks.get(*i).copied().unwrap_or(false)
                            })
                            .count();
                        ui.label(format!("{unresolved} remaining"));
                        if let Some(hunk) = diff.hunks.get(self.three_selected) {
                            if hunk.kind == MergeHunkKind::Conflict {
                                ui.add_space(8.0);
                                ui.label(format!(
                                    "Conflict at base line {}",
                                    hunk.base_range.start + 1
                                ));
                                ui.label(format!("Left: {} line(s)", hunk.left.len()));
                                ui.label(format!("Right: {} line(s)", hunk.right.len()));
                                ui.add_space(8.0);
                                let selected_unresolved = !self
                                    .resolved_merge_hunks
                                    .get(self.three_selected)
                                    .copied()
                                    .unwrap_or(false);
                                if ui
                                    .add_enabled(selected_unresolved, egui::Button::new("Use Left"))
                                    .clicked()
                                {
                                    self.apply_merge_choice(MergeChoice::Left);
                                }
                                if ui
                                    .add_enabled(
                                        selected_unresolved,
                                        egui::Button::new("Use Right"),
                                    )
                                    .clicked()
                                {
                                    self.apply_merge_choice(MergeChoice::Right);
                                }
                            }
                        }
                    } else {
                        ui.label("Compare three files to inspect conflicts.");
                    }
                });
            });
        });
        ui.add_space(5.0);
        ui.label(
            RichText::new("Merge Output")
                .strong()
                .color(accent(self.dark_mode)),
        );
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
        if let Some(next) = diff
            .hunks
            .iter()
            .enumerate()
            .filter(|(i, h)| {
                h.kind == MergeHunkKind::Conflict
                    && !self.resolved_merge_hunks.get(*i).copied().unwrap_or(false)
            })
            .map(|(i, _)| i)
            .find(|&i| i > self.three_selected)
            .or_else(|| {
                diff.hunks
                    .iter()
                    .enumerate()
                    .find(|(i, h)| {
                        h.kind == MergeHunkKind::Conflict
                            && !self.resolved_merge_hunks.get(*i).copied().unwrap_or(false)
                    })
                    .map(|(i, _)| i)
            })
        {
            self.three_selected = next;
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
fn text_editor(ui: &mut egui::Ui, title: &str, text: &mut String, undo: &mut TextUndo) -> bool {
    ui.label(RichText::new(title).strong());
    text_editor_sized(ui, text, undo, 22)
}
fn text_editor_sized(
    ui: &mut egui::Ui,
    text: &mut String,
    undo: &mut TextUndo,
    rows: usize,
) -> bool {
    let mut history_changed = false;
    ui.horizontal(|ui| {
        if ui.button("Undo").clicked() {
            history_changed = !undo.undo.is_empty();
            undo.undo(text);
        }
        if ui.button("Redo").clicked() {
            history_changed |= !undo.redo.is_empty();
            undo.redo(text);
        }
    });
    let before = text.clone();
    let response = ui.add(
        TextEdit::multiline(text)
            .code_editor()
            .desired_rows(rows)
            .desired_width(f32::INFINITY),
    );
    if response.changed() {
        undo.changed(before);
    }
    history_changed || response.changed()
}
fn aligned_rows(diff: &TextDiff, filter: FileViewFilter) -> Vec<AlignedRow> {
    let mut rows = Vec::new();
    let mut left = 0;
    let mut right = 0;
    for (index, hunk) in diff.hunks.iter().enumerate() {
        while left < hunk.left_range.start && right < hunk.right_range.start {
            if filter != FileViewFilter::Differences {
                rows.push(AlignedRow {
                    left: Some(left),
                    right: Some(right),
                    hunk: None,
                });
            }
            left += 1;
            right += 1;
        }
        let count = hunk.left_range.len().max(hunk.right_range.len());
        if filter != FileViewFilter::Same {
            for offset in 0..count {
                rows.push(AlignedRow {
                    left: (offset < hunk.left_range.len())
                        .then_some(hunk.left_range.start + offset),
                    right: (offset < hunk.right_range.len())
                        .then_some(hunk.right_range.start + offset),
                    hunk: Some(index),
                });
            }
        }
        left = hunk.left_range.end;
        right = hunk.right_range.end;
    }
    while left < diff.left_lines.len() && right < diff.right_lines.len() {
        if filter != FileViewFilter::Differences {
            rows.push(AlignedRow {
                left: Some(left),
                right: Some(right),
                hunk: None,
            });
        }
        left += 1;
        right += 1;
    }
    rows
}

fn diff_pane(
    ui: &mut egui::Ui,
    title: &str,
    diff: &TextDiff,
    rows: &[AlignedRow],
    left: bool,
    selected: usize,
    offset: f32,
    max_height: f32,
    dark_mode: bool,
) -> (f32, Option<usize>) {
    let mut clicked_hunk = None;
    let output = egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.set_min_height(max_height);
        ui.label(RichText::new(title).strong().color(accent(dark_mode)));
        ui.separator();
        if rows.is_empty() {
            ui.label("No lines match this view.");
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::both()
            .id_salt(if left {
                "two-left-diff"
            } else {
                "two-right-diff"
            })
            .max_height(max_height)
            .vertical_scroll_offset(offset)
            .show_rows(
                ui,
                ui.text_style_height(&egui::TextStyle::Monospace),
                rows.len(),
                |ui, visible| {
                    for index in visible {
                        let row = rows[index];
                        let line_index = if left { row.left } else { row.right };
                        let line = line_index.and_then(|i| {
                            if left {
                                diff.left_lines.get(i)
                            } else {
                                diff.right_lines.get(i)
                            }
                        });
                        let kind = row
                            .hunk
                            .and_then(|i| diff.hunks.get(i))
                            .map(|hunk| &hunk.kind);
                        let color = match kind {
                            Some(versus::HunkKind::Added) if dark_mode => {
                                Color32::from_rgb(38, 80, 62)
                            }
                            Some(versus::HunkKind::Removed) if dark_mode => {
                                Color32::from_rgb(90, 42, 50)
                            }
                            Some(versus::HunkKind::Changed) if dark_mode && left => {
                                Color32::from_rgb(90, 42, 50)
                            }
                            Some(versus::HunkKind::Changed) if dark_mode => {
                                Color32::from_rgb(38, 80, 62)
                            }
                            Some(versus::HunkKind::Added) => Color32::from_rgb(222, 247, 230),
                            Some(versus::HunkKind::Removed) => Color32::from_rgb(255, 228, 230),
                            Some(versus::HunkKind::Changed) if left => {
                                Color32::from_rgb(255, 228, 230)
                            }
                            Some(versus::HunkKind::Changed) => Color32::from_rgb(222, 247, 230),
                            None => Color32::TRANSPARENT,
                        };
                        let color = if line_index.is_none() {
                            Color32::TRANSPARENT
                        } else {
                            color
                        };
                        egui::Frame::default().fill(color).show(ui, |ui| {
                            let marker = row.hunk == Some(selected);
                            let label = match (line_index, line) {
                                (Some(number), Some(line)) => format!(
                                    "{} {:>5} │ {}",
                                    if marker { "▸" } else { " " },
                                    number + 1,
                                    line
                                ),
                                _ => "        │".to_owned(),
                            };
                            let response = ui.add(
                                egui::Label::new(RichText::new(label).monospace())
                                    .wrap_mode(egui::TextWrapMode::Extend)
                                    .sense(egui::Sense::click()),
                            );
                            if response.clicked() {
                                clicked_hunk = row.hunk;
                            }
                        });
                    }
                },
            )
    });
    (output.inner.state.offset.y, clicked_hunk)
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
fn readonly_lines(ui: &mut egui::Ui, title: &str, text: &str, height: f32) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.strong(title);
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt(title)
            .max_height(height)
            .show(ui, |ui| {
                ui.set_min_height(height);
                for (number, line) in text.lines().enumerate() {
                    ui.monospace(format!("{:>4}  {}", number + 1, line));
                }
            });
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
fn state_color(state: &DirectoryEntryState, dark_mode: bool) -> Color32 {
    match state {
        DirectoryEntryState::Same => Color32::GRAY,
        DirectoryEntryState::Different | DirectoryEntryState::TypeMismatch => {
            if dark_mode {
                Color32::YELLOW
            } else {
                Color32::from_rgb(190, 45, 48)
            }
        }
        DirectoryEntryState::LeftOnly => {
            if dark_mode {
                Color32::from_rgb(202, 157, 239)
            } else {
                Color32::from_rgb(123, 72, 164)
            }
        }
        DirectoryEntryState::RightOnly => {
            if dark_mode {
                Color32::from_rgb(202, 157, 239)
            } else {
                Color32::from_rgb(123, 72, 164)
            }
        }
        DirectoryEntryState::Error(_) => Color32::RED,
    }
}
fn accent(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgb(115, 175, 245)
    } else {
        Color32::from_rgb(36, 101, 183)
    }
}
fn toolbar_fill(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgb(35, 40, 49)
    } else {
        Color32::from_rgb(242, 247, 253)
    }
}
fn apply_theme(ctx: &egui::Context, dark_mode: bool) {
    ctx.set_theme(if dark_mode {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    ctx.set_visuals(if dark_mode {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    });
    ctx.all_styles_mut(|style| {
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(15.0));
        style
            .text_styles
            .insert(egui::TextStyle::Monospace, egui::FontId::monospace(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Small, egui::FontId::proportional(13.0));
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.interact_size.y = 29.0;
    });
}
#[derive(Clone, Copy)]
struct Settings {
    ignore_line_endings: bool,
    ignore_whitespace: bool,
    confirm_overwrite: bool,
    dark_mode: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            ignore_line_endings: true,
            ignore_whitespace: false,
            confirm_overwrite: true,
            dark_mode: false,
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
    parse_settings(&contents)
}
fn parse_settings(contents: &str) -> Settings {
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
            "dark_mode" => settings.dark_mode = value,
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
            "ignore_line_endings={}\nignore_whitespace={}\nconfirm_overwrite={}\ndark_mode={}\n",
            settings.ignore_line_endings,
            settings.ignore_whitespace,
            settings.confirm_overwrite,
            settings.dark_mode
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Range;

    #[test]
    fn theme_setting_loads_and_old_settings_default_to_light() {
        assert!(parse_settings("dark_mode=true\nignore_whitespace=true\n").dark_mode);
        assert!(parse_settings("dark_mode=true\nignore_whitespace=true\n").ignore_whitespace);
        assert!(!parse_settings("ignore_whitespace=true\n").dark_mode);
    }

    #[test]
    fn file_view_filters_keep_two_sides_aligned_across_uneven_hunks() {
        let diff = TextDiff {
            left_lines: ["same", "old one", "old two", "end"]
                .map(str::to_owned)
                .to_vec(),
            right_lines: ["same", "new", "end"].map(str::to_owned).to_vec(),
            hunks: vec![versus::DiffHunk {
                left_range: 1..3,
                right_range: 1..2,
                kind: versus::HunkKind::Changed,
            }],
        };
        assert_eq!(
            aligned_rows(&diff, FileViewFilter::All),
            vec![
                AlignedRow {
                    left: Some(0),
                    right: Some(0),
                    hunk: None
                },
                AlignedRow {
                    left: Some(1),
                    right: Some(1),
                    hunk: Some(0)
                },
                AlignedRow {
                    left: Some(2),
                    right: None,
                    hunk: Some(0)
                },
                AlignedRow {
                    left: Some(3),
                    right: Some(2),
                    hunk: None
                },
            ]
        );
        assert_eq!(aligned_rows(&diff, FileViewFilter::Differences).len(), 2);
        assert_eq!(aligned_rows(&diff, FileViewFilter::Same).len(), 2);
    }

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
