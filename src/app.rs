use crate::selection::{self, ComparisonMode, SelectionMode, SourceKind};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::{Duration, Instant},
};
use versus::{
    CompareError, ComparisonProgress, DirectoryCompareOptions, DirectoryEntryKind,
    DirectoryEntryState, FileComparison, FileViewOptions, FolderTree, ProgressStage, TreeNode,
    load_file_comparison_with_options,
};

#[derive(Clone, Copy)]
struct Palette {
    background: Color32,
    panel: Color32,
    border: Color32,
    muted: Color32,
    text: Color32,
    accent: Color32,
    changed: Color32,
    left_only: Color32,
    right_only: Color32,
    error: Color32,
    alternate: Color32,
    hover: Color32,
}

impl Palette {
    fn for_context(ctx: &egui::Context) -> Self {
        Self::new(ctx.theme() == egui::Theme::Dark)
    }
    fn new(dark: bool) -> Self {
        if dark {
            Self {
                background: Color32::from_rgb(16, 21, 29),
                panel: Color32::from_rgb(23, 30, 40),
                border: Color32::from_rgb(48, 59, 74),
                muted: Color32::from_rgb(145, 161, 181),
                text: Color32::from_rgb(221, 230, 240),
                accent: Color32::from_rgb(95, 163, 255),
                changed: Color32::from_rgb(244, 191, 98),
                left_only: Color32::from_rgb(94, 211, 221),
                right_only: Color32::from_rgb(189, 157, 255),
                error: Color32::from_rgb(255, 127, 137),
                alternate: Color32::from_rgb(26, 34, 45),
                hover: Color32::from_rgb(34, 44, 58),
            }
        } else {
            Self {
                background: Color32::from_rgb(244, 246, 249),
                panel: Color32::WHITE,
                border: Color32::from_rgb(207, 215, 225),
                muted: Color32::from_rgb(88, 101, 119),
                text: Color32::from_rgb(32, 43, 58),
                accent: Color32::from_rgb(26, 102, 185),
                changed: Color32::from_rgb(142, 86, 10),
                left_only: Color32::from_rgb(0, 112, 128),
                right_only: Color32::from_rgb(112, 65, 176),
                error: Color32::from_rgb(179, 38, 58),
                alternate: Color32::from_rgb(244, 247, 251),
                hover: Color32::from_rgb(230, 237, 246),
            }
        }
    }
}

const ROW_HEIGHT: f32 = 22.0;

struct ComparisonJob {
    receiver: Receiver<Result<FolderTree, CompareError>>,
    cancellation: Arc<AtomicBool>,
    roots: [PathBuf; 2],
    started: Instant,
    progress: Arc<ComparisonProgress>,
}

struct FileJob {
    receiver: Receiver<Result<Option<FileComparison>, CompareError>>,
    cancellation: Arc<AtomicBool>,
    started: Instant,
    progress: Arc<ComparisonProgress>,
}

struct SourceJob {
    receiver: Receiver<Result<SourceKind, String>>,
    cancellation: Arc<AtomicBool>,
}

impl Drop for SourceJob {
    fn drop(&mut self) {
        self.cancellation.store(true, Ordering::Relaxed);
    }
}

struct FileView {
    from_folders: bool,
    paths: [PathBuf; 2],
    sources: [Option<PathBuf>; 2],
    job: Option<FileJob>,
    comparison: Option<FileComparison>,
    error: Option<String>,
    error_icon: StatusIcon,
    scroll_y: f32,
    content_widths: [f32; 2],
    counts: [usize; 6],
    visible_rows: Vec<usize>,
    difference_rows: Vec<usize>,
    navigation_row: Option<usize>,
    navigation_scroll_pending: bool,
}

impl Drop for FileView {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancellation.store(true, Ordering::Relaxed);
        }
    }
}

pub struct VersusApp {
    comparison_mode: ComparisonMode,
    file_view: Option<FileView>,
    paths: [String; 2],
    // Keep filesystem paths separate from their potentially lossy UI text.
    source_paths: [Option<PathBuf>; 2],
    // Some(None) is a pending automatic launch; Some(Some(mode)) forces a mode.
    launch_mode: Option<Option<ComparisonMode>>,
    source_kinds: [Option<SourceKind>; 2],
    source_jobs: [Option<SourceJob>; 2],
    source_errors: [Option<String>; 2],
    roots: Option<[PathBuf; 2]>,
    tree: Option<FolderTree>,
    job: Option<ComparisonJob>,
    selected: Option<PathBuf>,
    message: String,
    error: Option<String>,
    elapsed: Option<Duration>,
    counts: [usize; 6],
    logo_texture: Option<(bool, egui::TextureHandle)>,
    scroll_generation: u64,
    show_only_differences: bool,
    ignore_whitespace: bool,
    ignore_line_endings: bool,
    tree_scroll_y: f32,
    folder_differences: Vec<PathBuf>,
    pane_rects: [Rect; 2],
    drop_hover_side: Option<usize>,
    drop_message: Option<String>,
}

impl Default for VersusApp {
    fn default() -> Self {
        Self {
            comparison_mode: ComparisonMode::Folder,
            file_view: None,
            paths: Default::default(),
            source_paths: [None, None],
            launch_mode: None,
            source_kinds: [None; 2],
            source_jobs: [None, None],
            source_errors: [None, None],
            roots: None,
            tree: None,
            job: None,
            selected: None,
            message: "Choose a folder on each side to begin.".into(),
            error: None,
            elapsed: None,
            counts: [0; 6],
            logo_texture: None,
            scroll_generation: 0,
            show_only_differences: false,
            ignore_whitespace: false,
            ignore_line_endings: true,
            tree_scroll_y: 0.0,
            folder_differences: Vec::new(),
            pane_rects: [Rect::NOTHING; 2],
            drop_hover_side: None,
            drop_message: None,
        }
    }
}

impl VersusApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        launch: Option<crate::cli::LaunchRequest>,
    ) -> Self {
        apply_theme(&cc.egui_ctx);
        let mut app = Self::default();
        if let Some(launch) = launch {
            app.open_launch_request(launch);
            cc.egui_ctx.request_repaint();
        }
        app
    }

    fn open_launch_request(&mut self, launch: crate::cli::LaunchRequest) {
        self.new_comparison();
        self.comparison_mode = launch.mode.unwrap_or(ComparisonMode::File);
        for (side, path) in launch.paths.into_iter().enumerate() {
            if launch.mode != Some(ComparisonMode::Folder) && empty_diff_side(&path) {
                // Git represents an added/deleted side with a null-device name.
                // Model it as absent; never open the device itself.
                self.paths[side] = path.to_string_lossy().into_owned();
                self.source_paths[side] = Some(path);
                self.source_kinds[side] = Some(SourceKind::File);
            } else {
                self.select_path(side, path);
            }
        }
        self.launch_mode = Some(launch.mode);
        self.start_selected_comparison();
    }

    fn comparison_paths(&self) -> [PathBuf; 2] {
        std::array::from_fn(|side| {
            self.source_paths[side]
                .clone()
                .unwrap_or_else(|| absolute_path(&self.paths[side]))
        })
    }

    fn invalidate(&mut self) {
        self.file_view = None;
        if let Some(job) = self.job.take() {
            job.cancellation.store(true, Ordering::Relaxed);
        }
        self.tree = None;
        self.roots = None;
        self.selected = None;
        self.error = None;
        self.elapsed = None;
        self.counts = [0; 6];
        self.folder_differences.clear();
        self.tree_scroll_y = 0.0;
        self.message = "Source selection changed.".into();
        self.scroll_generation += 1;
    }

    fn select_path(&mut self, side: usize, path: PathBuf) {
        self.invalidate();
        self.drop_message = None;
        self.launch_mode = None;
        let path = absolute_native_path(path);
        self.paths[side] = path.to_string_lossy().into_owned();
        self.source_paths[side] = Some(path.clone());
        self.source_kinds[side] = None;
        self.source_errors[side] = None;
        self.source_jobs[side] = None;
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker_cancellation = cancellation.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            if !worker_cancellation.load(Ordering::Relaxed) {
                let _ = sender.send(selection::classify_path(&path));
            }
        });
        self.source_jobs[side] = Some(SourceJob {
            receiver,
            cancellation,
        });
        self.message = "Opening selected source…".into();
    }

    fn poll_sources(&mut self, ctx: &egui::Context) {
        let mut changed = false;
        for side in 0..2 {
            let Some(job) = &self.source_jobs[side] else {
                continue;
            };
            let result = match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => {
                    Some(Err("The source worker stopped unexpectedly.".into()))
                }
                Err(TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.source_jobs[side] = None;
                match result {
                    Ok(kind) => self.source_kinds[side] = Some(kind),
                    Err(error) => self.source_errors[side] = Some(error),
                }
                changed = true;
            }
        }
        if changed {
            self.start_selected_comparison();
        }
        if self.source_jobs.iter().any(Option::is_some) {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    fn mode(&self) -> SelectionMode {
        if self.file_view.is_some() {
            SelectionMode::File
        } else if self.tree.is_some() || self.job.is_some() {
            SelectionMode::Folder
        } else {
            selection::selection_mode(self.source_kinds)
        }
    }

    fn switch_comparison_mode(&mut self, mode: ComparisonMode) {
        if self.comparison_mode == mode {
            if mode == ComparisonMode::Folder
                && self
                    .file_view
                    .as_ref()
                    .is_some_and(|view| view.from_folders)
            {
                self.file_view = None;
            }
            return;
        }
        self.comparison_mode = mode;
        self.new_comparison();
    }

    fn new_comparison(&mut self) {
        self.invalidate();
        self.paths = Default::default();
        self.source_paths = [None, None];
        self.launch_mode = None;
        self.source_kinds = [None; 2];
        self.source_jobs = [None, None];
        self.source_errors = [None, None];
        self.drop_message = None;
        self.message = format!(
            "Choose a {} on each side to begin.",
            self.comparison_mode.source_label()
        );
    }

    fn start_selected_comparison(&mut self) {
        self.error = self.source_errors.iter().flatten().next().cloned();
        if self.error.is_some() {
            self.message = "Cannot open selected source. Choose another file or folder.".into();
            return;
        }
        if self.source_jobs.iter().any(Option::is_some) {
            return;
        }
        if let Some(requested_mode) = self.launch_mode.take() {
            if let Some(mode) = requested_mode {
                let expected = match mode {
                    ComparisonMode::File => SourceKind::File,
                    ComparisonMode::Folder => SourceKind::Folder,
                };
                if self.source_kinds.iter().any(|kind| *kind != Some(expected)) {
                    self.error = Some(format!(
                        "{} requires two {}. Choose matching sources.",
                        mode.label(),
                        match mode {
                            ComparisonMode::File => "regular files",
                            ComparisonMode::Folder => "folders",
                        }
                    ));
                    self.message = "Cannot compare these command-line sources.".into();
                    return;
                }
            } else {
                self.comparison_mode = match selection::selection_mode(self.source_kinds) {
                    SelectionMode::Folder => ComparisonMode::Folder,
                    _ => ComparisonMode::File,
                };
            }
        }
        match selection::selection_mode(self.source_kinds) {
            SelectionMode::File => {
                let paths = self.comparison_paths();
                let sources = std::array::from_fn(|side| {
                    (self.source_kinds[side] == Some(SourceKind::File)
                        && !empty_diff_side(&paths[side]))
                    .then(|| paths[side].clone())
                });
                let ready = self
                    .source_kinds
                    .iter()
                    .all(|kind| *kind == Some(SourceKind::File));
                self.file_view = Some(Self::create_file_view(
                    paths,
                    sources,
                    false,
                    false,
                    ready,
                    self.file_options(),
                ));
                self.message = "Choose a file on the other side to compare.".into();
            }
            SelectionMode::Folder => {
                if self
                    .source_kinds
                    .iter()
                    .all(|kind| *kind == Some(SourceKind::Folder))
                {
                    self.start_comparison();
                } else {
                    self.message = "Choose a folder on the other side to compare.".into();
                }
            }
            SelectionMode::Incompatible => {
                self.message = if self.source_kinds.contains(&Some(SourceKind::Other)) {
                    "No comparison can be made with this source type. Select regular files or folders."
                } else {
                    "No comparison can be made between a file and a folder. Select matching types."
                }.into();
            }
            SelectionMode::Empty => {}
        }
    }

    fn start_comparison(&mut self) {
        if self.paths.iter().any(|path| path.trim().is_empty()) {
            return;
        }
        if let Some(job) = self.job.take() {
            job.cancellation.store(true, Ordering::Relaxed);
        }
        let roots = self.comparison_paths();
        let worker_roots = roots.clone();
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker_cancellation = cancellation.clone();
        let (sender, receiver) = mpsc::channel();
        let progress = Arc::new(ComparisonProgress::default());
        let worker_progress = progress.clone();
        let ignore_whitespace = self.ignore_whitespace;
        let ignore_line_endings = self.ignore_line_endings;
        std::thread::spawn(move || {
            let result = versus::compare_directories(
                &worker_roots[0],
                &worker_roots[1],
                &DirectoryCompareOptions {
                    cancellation: Some(worker_cancellation),
                    ignore_whitespace,
                    ignore_line_endings,
                    progress: Some(worker_progress),
                    ..Default::default()
                },
            )
            .map(|diff| FolderTree::from_diff(&diff));
            let _ = sender.send(result);
        });
        self.job = Some(ComparisonJob {
            receiver,
            cancellation,
            roots,
            started: Instant::now(),
            progress,
        });
        self.error = None;
        self.message = "Scanning folders and comparing file contents…".into();
    }

    fn poll_comparison(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.job else {
            return;
        };
        let result = match job.receiver.try_recv() {
            Ok(result) => Some(result.map_err(|error| error.to_string())),
            Err(TryRecvError::Disconnected) => Some(Err(
                "The comparison worker stopped unexpectedly. Try comparing again.".into(),
            )),
            Err(TryRecvError::Empty) => None,
        };
        if let Some(result) = result {
            let job = self.job.take().unwrap();
            if job.cancellation.load(Ordering::Relaxed) {
                self.message = "Comparison cancelled.".into();
                return;
            }
            match result {
                Ok(tree) => {
                    self.folder_differences = tree
                        .all_rows()
                        .into_iter()
                        .filter(|row| row.node.state != DirectoryEntryState::Same)
                        .map(|row| row.node.relative_path.clone())
                        .collect();
                    self.counts = count_entries(&tree);
                    self.tree = Some(tree);
                    self.roots = Some(job.roots);
                    self.elapsed = Some(job.started.elapsed());
                    self.selected = None;
                    self.scroll_generation += 1;
                    self.message = "Comparison complete.".into();
                }
                Err(error) => {
                    self.error = Some(error);
                    self.message =
                        "Comparison failed. Check the folder paths and access permissions.".into();
                }
            }
        } else {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    fn open_file(&mut self, relative_path: PathBuf, present: [bool; 2], type_mismatch: bool) {
        let Some(roots) = &self.roots else { return };
        let paths = roots.clone().map(|root| root.join(&relative_path));
        let sources = std::array::from_fn(|side| present[side].then(|| paths[side].clone()));
        self.file_view = Some(Self::create_file_view(
            paths,
            sources,
            true,
            type_mismatch,
            true,
            self.file_options(),
        ));
    }

    fn create_file_view(
        paths: [PathBuf; 2],
        sources: [Option<PathBuf>; 2],
        from_folders: bool,
        type_mismatch: bool,
        ready: bool,
        mut options: FileViewOptions,
    ) -> FileView {
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker_cancellation = cancellation.clone();
        let worker_sources = sources.clone();
        let (sender, receiver) = mpsc::channel();
        let progress = Arc::new(ComparisonProgress::default());
        options.progress = Some(progress.clone());
        if ready && !type_mismatch {
            std::thread::spawn(move || {
                let result = load_file_comparison_with_options(
                    &worker_sources,
                    &worker_cancellation,
                    &options,
                );
                let _ = sender.send(result);
            });
        }
        FileView {
            from_folders,
            paths,
            sources,
            job: (ready && !type_mismatch).then_some(FileJob {
                receiver,
                cancellation,
                started: Instant::now(),
                progress,
            }),
            comparison: None,
            error: type_mismatch.then(|| "Entry types differ. Line comparison requires regular files; folders and symlink targets are not opened.".into()),
            error_icon: if type_mismatch { StatusIcon::TypeMismatch } else { StatusIcon::Error },
            scroll_y: 0.0,
            content_widths: [0.0; 2],
            counts: [0; 6],
            visible_rows: Vec::new(),
            difference_rows: Vec::new(),
            navigation_row: None,
            navigation_scroll_pending: false,
        }
    }

    fn poll_file_comparison(&mut self, ctx: &egui::Context) {
        let Some(view) = &mut self.file_view else {
            return;
        };
        let Some(job) = &view.job else { return };
        let result = match job.receiver.try_recv() {
            Ok(result) => Some(result.map_err(|error| error.to_string())),
            Err(TryRecvError::Disconnected) => Some(Err(
                "The file comparison worker stopped unexpectedly.".into(),
            )),
            Err(TryRecvError::Empty) => None,
        };
        if let Some(result) = result {
            view.job = None;
            match result {
                Ok(Some(comparison)) => {
                    for row in &comparison.rows {
                        view.counts[state_index(&row.state)] += 1;
                        for (side, line) in [&row.left, &row.right].into_iter().enumerate() {
                            if let Some((_, text)) = line {
                                view.content_widths[side] = view.content_widths[side].max(
                                    96.0 + (text.len() + text.matches('\t').count() * 3) as f32
                                        * 8.0,
                                );
                            }
                        }
                    }
                    view.comparison = Some(comparison);
                    update_visible_file_rows(view, self.show_only_differences);
                }
                Ok(None) => view.error = Some("File comparison cancelled.".into()),
                Err(error) => view.error = Some(error),
            }
        } else {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    fn file_options(&self) -> FileViewOptions {
        FileViewOptions {
            ignore_whitespace: self.ignore_whitespace,
            ignore_line_endings: self.ignore_line_endings,
            progress: None,
        }
    }

    fn recompare(&mut self) {
        if self.source_jobs.iter().any(Option::is_some) {
            return; // Pending source classification will use the latest options.
        }
        if self
            .source_kinds
            .iter()
            .all(|kind| *kind == Some(SourceKind::Folder))
        {
            self.tree = None;
            self.folder_differences.clear();
            self.counts = [0; 6];
            self.start_comparison();
        }
        if let Some(view) = self.file_view.take() {
            let ready = view.from_folders
                || self
                    .source_kinds
                    .iter()
                    .all(|kind| *kind == Some(SourceKind::File));
            self.file_view = Some(Self::create_file_view(
                view.paths.clone(),
                view.sources.clone(),
                view.from_folders,
                matches!(view.error_icon, StatusIcon::TypeMismatch),
                ready,
                self.file_options(),
            ));
        }
    }

    fn jump_difference(&mut self, forward: bool) {
        if let Some(view) = &mut self.file_view {
            let top = (view.scroll_y / ROW_HEIGHT).floor() as usize;
            let anchor = view
                .navigation_row
                .or_else(|| view.visible_rows.get(top).copied());
            let target = if forward {
                view.difference_rows.iter().copied().find(|row| {
                    anchor.is_none_or(|anchor| {
                        if view.navigation_row.is_some() {
                            *row > anchor
                        } else {
                            *row >= anchor
                        }
                    })
                })
            } else {
                view.difference_rows
                    .iter()
                    .rev()
                    .copied()
                    .find(|row| anchor.is_some_and(|anchor| *row < anchor))
            };
            if let Some(target) = target {
                if let Some(index) = view.visible_rows.iter().position(|row| *row == target) {
                    view.scroll_y = index as f32 * ROW_HEIGHT;
                    view.navigation_row = Some(target);
                    view.navigation_scroll_pending = true;
                }
            }
            return;
        }
        let Some(tree) = &mut self.tree else { return };
        let all = tree.all_rows();
        let anchor = self
            .selected
            .as_ref()
            .and_then(|path| all.iter().position(|row| &row.node.relative_path == path))
            .or_else(|| {
                let visible: Vec<_> = tree
                    .visible_rows()
                    .into_iter()
                    .filter(|row| {
                        !self.show_only_differences || row.node.state != DirectoryEntryState::Same
                    })
                    .collect();
                let top = (self.tree_scroll_y / ROW_HEIGHT).floor() as usize;
                visible.get(top).and_then(|row| {
                    all.iter()
                        .position(|item| item.node.relative_path == row.node.relative_path)
                })
            });
        let selected = self.selected.is_some();
        let target = if forward {
            all.iter().enumerate().find(|(index, row)| {
                row.node.state != DirectoryEntryState::Same
                    && anchor.is_none_or(|anchor| {
                        if selected {
                            *index > anchor
                        } else {
                            *index >= anchor
                        }
                    })
            })
        } else {
            all.iter().enumerate().rev().find(|(index, row)| {
                row.node.state != DirectoryEntryState::Same
                    && anchor.is_some_and(|anchor| *index < anchor)
            })
        }
        .map(|(_, row)| row.node.relative_path.clone());
        if let Some(path) = target {
            tree.expand_parents(&path);
            let visible: Vec<_> = tree
                .visible_rows()
                .into_iter()
                .filter(|row| {
                    !self.show_only_differences || row.node.state != DirectoryEntryState::Same
                })
                .collect();
            if let Some(index) = visible
                .iter()
                .position(|row| row.node.relative_path == path)
            {
                self.tree_scroll_y = index as f32 * ROW_HEIGHT;
            }
            self.selected = Some(path);
        }
    }

    fn progress_indicator(&self, ui: &mut egui::Ui) {
        let file_job = self.file_view.as_ref().and_then(|view| view.job.as_ref());
        let job = file_job
            .map(|job| (&job.progress, job.started))
            .or_else(|| self.job.as_ref().map(|job| (&job.progress, job.started)));
        if let Some((progress, started)) = job {
            let snapshot = progress.snapshot();
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(progress_text(snapshot, started, Instant::now())).size(11.0),
                );
                if let Some(total) = snapshot.total.filter(|total| *total > 0) {
                    ui.add(
                        egui::ProgressBar::new(snapshot.completed as f32 / total as f32)
                            .desired_width(90.0),
                    );
                }
                if let Some(job) = file_job {
                    if icon_button(
                        ui,
                        ToolbarIcon::Cancel,
                        !job.cancellation.load(Ordering::Relaxed),
                        "Cancel file comparison",
                    )
                    .clicked()
                    {
                        job.cancellation.store(true, Ordering::Relaxed);
                    }
                }
            });
            ui.ctx().request_repaint_after(Duration::from_millis(60));
        } else if self.source_jobs.iter().any(Option::is_some) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Opening selected sources…");
            });
            ui.ctx().request_repaint_after(Duration::from_millis(60));
        }
        if let Some(message) = &self.drop_message {
            ui.label(
                RichText::new(message)
                    .size(11.0)
                    .color(Palette::for_context(ui.ctx()).error),
            );
        }
    }

    fn drag_and_drop(&mut self, ui: &mut egui::Ui) {
        let (hovering, position) = ui.input(|input| {
            (
                !input.raw.hovered_files.is_empty(),
                input.pointer.latest_pos(),
            )
        });
        let dropped = ui
            .ctx()
            .input_mut(|input| std::mem::take(&mut input.raw.dropped_files));
        let side =
            position.and_then(|pos| self.pane_rects.iter().position(|rect| rect.contains(pos)));
        if hovering {
            if position.is_some() {
                self.drop_hover_side = side;
            }
            if let Some(side) = side.or(self.drop_hover_side) {
                let rect = self.pane_rects[side].shrink(3.0);
                let palette = Palette::for_context(ui.ctx());
                ui.painter()
                    .rect_filled(rect, 4, palette.accent.gamma_multiply(0.1));
                ui.painter().rect_stroke(
                    rect,
                    4,
                    Stroke::new(2.0, palette.accent),
                    egui::StrokeKind::Inside,
                );
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    format!(
                        "Drop a file or folder on {}",
                        if side == 0 { "LEFT" } else { "RIGHT" }
                    ),
                    FontId::proportional(16.0),
                    palette.text,
                );
            }
        }
        if !dropped.is_empty() {
            let target = if position.is_some() {
                side
            } else {
                self.drop_hover_side
            };
            if dropped.len() == 1 && !dropped[0].path().as_os_str().is_empty() && target.is_some() {
                let side = target.unwrap();
                let path = dropped[0].path().to_path_buf();
                // A drop in a drilled-down file view retains the other displayed
                // file, rather than accidentally pairing it with the folder root.
                if let Some(view) = self.file_view.as_ref().filter(|view| view.from_folders) {
                    let peer = view.sources[1 - side].clone();
                    self.new_comparison();
                    if let Some(peer) = peer {
                        self.select_path(1 - side, peer);
                    }
                }
                self.select_path(side, path);
                self.launch_mode = Some(None);
            } else {
                self.drop_message =
                    Some("Drop one file or folder onto the LEFT or RIGHT pane.".into());
            }
            ui.ctx().request_repaint();
        }
        if !hovering {
            self.drop_hover_side = None;
        }
    }

    fn render(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let dark = ui.visuals().dark_mode;
        if self
            .logo_texture
            .as_ref()
            .is_none_or(|(theme, _)| *theme != dark)
        {
            let icon = crate::logo::themed_icon(dark);
            // PNG/window-icon bytes have straight alpha; egui textures need
            // premultiplied alpha, including nearly transparent edge pixels.
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            );
            self.logo_texture = Some((
                dark,
                ui.ctx()
                    .load_texture("versus-mark", image, egui::TextureOptions::LINEAR),
            ));
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Icon(Some(Arc::new(icon))));
        }
        egui::Frame::default()
            .fill(palette.background)
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                self.header(ui);
                ui.add_space(5.0);
                self.legend(ui);
                ui.add_space(5.0);
                self.progress_indicator(ui);
                if self.file_view.is_some() {
                    self.pane_rects = split_rect(ui.available_rect_before_wrap());
                    self.file_area(ui);
                    return;
                }
                if let Some(error) = &self.error {
                    egui::Frame::default()
                        .fill(palette.error.gamma_multiply(0.12))
                        .inner_margin(egui::Margin::same(6))
                        .show(ui, |ui| {
                            ui.colored_label(palette.error, error);
                        });
                    ui.add_space(4.0);
                }
                let available = ui.available_rect_before_wrap();
                let footer_height = ui.spacing().interact_size.y
                    + ui.text_style_height(&egui::TextStyle::Monospace).ceil()
                    + ui.spacing().item_spacing.y;
                let footer_rect = Rect::from_min_max(
                    egui::pos2(
                        available.left(),
                        (available.bottom() - footer_height).max(available.top()),
                    ),
                    available.max,
                );
                let tree_rect = Rect::from_min_max(
                    available.min,
                    egui::pos2(
                        available.right(),
                        (footer_rect.top() - 5.0).max(available.top()),
                    ),
                );
                self.pane_rects = split_rect(tree_rect);
                ui.scope_builder(egui::UiBuilder::new().max_rect(tree_rect), |ui| {
                    ui.set_clip_rect(ui.clip_rect().intersect(tree_rect));
                    self.tree_area(ui, (tree_rect.height() - 2.0).max(0.0));
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(footer_rect), |ui| {
                    ui.set_clip_rect(ui.clip_rect().intersect(footer_rect));
                    self.footer(ui);
                });
            });
        self.drag_and_drop(ui);
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let modes = [ComparisonMode::Folder, ComparisonMode::File];
        let labels = modes.map(|mode| {
            egui::WidgetText::from(RichText::new(mode.label()).size(14.0).strong()).into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                FontId::proportional(14.0),
            )
        });
        let padding = ui.spacing().button_padding;
        let widths = labels
            .each_ref()
            .map(|label| label.size().x + 22.0 + padding.x * 2.0);
        let gap = ui.spacing().item_spacing.x;
        let group_width = widths.iter().sum::<f32>() + gap;
        let (header, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), Sense::hover());
        let group = Rect::from_center_size(header.center(), egui::vec2(group_width, 28.0));

        let mut brand_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    header.min,
                    egui::pos2(group.left() - gap, header.bottom()),
                ))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        if let Some((_, logo)) = &self.logo_texture {
            brand_ui.image((logo.id(), egui::vec2(28.0, 28.0)));
        }
        brand_ui.label(RichText::new("Versus").size(18.0).strong());

        let mut x = group.left();
        for (index, mode) in modes.into_iter().enumerate() {
            let rect = Rect::from_min_size(
                egui::pos2(x, group.top()),
                egui::vec2(widths[index], group.height()),
            );
            if comparison_mode_button(
                ui,
                rect,
                labels[index].clone(),
                mode,
                self.comparison_mode == mode,
            )
            .clicked()
            {
                self.switch_comparison_mode(mode);
                ui.ctx().request_repaint();
            }
            x += widths[index] + gap;
        }

        let mut controls_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    egui::pos2(group.right() + gap, header.top()),
                    header.max,
                ))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        let dark = ui.visuals().dark_mode;
        if icon_button(
            &mut controls_ui,
            if dark {
                ToolbarIcon::Sun
            } else {
                ToolbarIcon::Moon
            },
            true,
            if dark {
                "Switch to light mode"
            } else {
                "Switch to dark mode"
            },
        )
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
            ui.ctx().request_repaint();
        }
        if self.mode() == SelectionMode::Folder {
            self.controls(&mut controls_ui);
        }
    }

    fn folder_inputs(&mut self, ui: &mut egui::Ui) {
        let kinds = self.source_kinds;
        self.folder_inputs_with_picker(ui, |side, current_path, mode| {
            selection::pick_path(side, current_path, kinds[side], mode)
        });
    }

    fn folder_inputs_with_picker(
        &mut self,
        ui: &mut egui::Ui,
        mut pick_source: impl FnMut(usize, &str, ComparisonMode) -> Option<PathBuf>,
    ) {
        let palette = Palette::for_context(ui.ctx());
        let full_paths = self.paths.clone().map(|path| {
            if path.is_empty() {
                format!("Choose a {}", self.comparison_mode.source_label())
            } else {
                absolute_path(&path).display().to_string()
            }
        });
        let width = ui.available_width();
        let path_width = (width / 2.0 - 84.0).max(40.0);
        let galleys = std::array::from_fn::<_, 2, _>(|side| {
            if self.source_kinds[side] == Some(SourceKind::File) {
                left_elided_path(ui.painter(), &full_paths[side], path_width, palette.text)
            } else {
                ui.painter().layout(
                    full_paths[side].clone(),
                    FontId::monospace(11.0),
                    palette.text,
                    path_width,
                )
            }
        });
        let height = galleys
            .iter()
            .map(|galley| galley.size().y)
            .fold(24.0, f32::max)
            + 14.0;
        let (header, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
        for (side, half) in split_rect(header).into_iter().enumerate() {
            let browse_label = format!(
                "Browse {} {}",
                if side == 0 { "left" } else { "right" },
                self.comparison_mode.source_label()
            );
            let rect = half.shrink(7.0);
            let painter = ui.painter().with_clip_rect(rect);
            paint_side_label(&painter, rect, side, palette);
            let galley = galleys[side].clone();
            let position = egui::pos2(rect.left() + 40.0, rect.center().y - galley.size().y / 2.0);
            let path_rect = Rect::from_min_size(
                egui::pos2(position.x, rect.top()),
                egui::vec2(path_width, rect.height()),
            );
            painter.galley(position, galley, palette.text);
            let path_response = ui
                .interact(
                    path_rect,
                    ui.id().with(("selected-folder", side)),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!(
                    "{}\nClick to browse for a {}",
                    full_paths[side],
                    self.comparison_mode.source_label()
                ));
            path_response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    format!("{browse_label} path"),
                )
            });
            let browse_rect = Rect::from_center_size(
                rect.right_center() - egui::vec2(12.0, 0.0),
                egui::vec2(24.0, 24.0),
            );
            let mut browse_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(browse_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            let browse = icon_button(&mut browse_ui, ToolbarIcon::Browse, true, &browse_label);
            if browse.clicked() || path_response.clicked() {
                if let Some(path) = pick_source(side, &self.paths[side], self.comparison_mode) {
                    self.select_path(side, path);
                    ui.ctx().request_repaint();
                }
            }
        }
        ui.painter().line_segment(
            [header.center_top(), header.center_bottom()],
            Stroke::new(1.0, palette.border),
        );
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        if icon_button(
            ui,
            ToolbarIcon::Collapse,
            self.tree.is_some(),
            "Collapse all",
        )
        .clicked()
        {
            self.tree.as_mut().unwrap().collapse_all();
        }
        if icon_button(ui, ToolbarIcon::Expand, self.tree.is_some(), "Expand all").clicked() {
            self.tree.as_mut().unwrap().expand_all();
        }
        let ready = self.paths.iter().all(|path| !path.trim().is_empty())
            && self.source_jobs.iter().all(Option::is_none)
            && self.source_errors.iter().all(Option::is_none);
        if icon_button(
            ui,
            ToolbarIcon::Refresh,
            ready && self.job.is_none(),
            if self.tree.is_some() {
                "Refresh comparison"
            } else {
                "Compare folders"
            },
        )
        .clicked()
        {
            self.start_comparison();
        }
        if let Some(job) = &self.job {
            let cancelling = job.cancellation.load(Ordering::Relaxed);
            if icon_button(
                ui,
                ToolbarIcon::Cancel,
                !cancelling,
                if cancelling {
                    "Cancelling…"
                } else {
                    "Cancel comparison"
                },
            )
            .clicked()
            {
                job.cancellation.store(true, Ordering::Relaxed);
            }
            ui.spinner();
        }
    }

    fn legend(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let back = self
            .file_view
            .as_ref()
            .is_some_and(|view| view.from_folders);
        let counts = if let Some(view) = &self.file_view {
            view.comparison.as_ref().map(|_| view.counts)
        } else {
            self.tree.as_ref().map(|_| self.counts)
        };
        ui.horizontal(|ui| {
            let control_width = 6.0 * (24.0 + ui.spacing().item_spacing.x);
            let legend_width = (ui.available_width() - control_width).max(0.0);
            ui.allocate_ui_with_layout(
                egui::vec2(legend_width, 24.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if back
                            && icon_button(ui, ToolbarIcon::Back, true, "Back to folders").clicked()
                        {
                            self.file_view = None;
                        }
                        for (index, icon) in [
                            StatusIcon::Different,
                            StatusIcon::LeftOnly,
                            StatusIcon::RightOnly,
                            StatusIcon::TypeMismatch,
                            StatusIcon::Error,
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            let (rect, _) =
                                ui.allocate_exact_size(egui::vec2(14.0, 14.0), Sense::hover());
                            paint_status_icon(ui.painter(), rect.center(), icon);
                            let label = counts.map_or_else(
                                || icon.label().into(),
                                |counts| format!("{}  {}", icon.label(), counts[index + 1]),
                            );
                            ui.label(RichText::new(label).size(10.0).color(icon.color(palette)));
                            ui.add_space(5.0);
                        }
                    });
                },
            );
            if icon_button_state(
                ui,
                ToolbarIcon::Differences,
                true,
                self.show_only_differences,
                "Show only differences",
            )
            .clicked()
            {
                self.show_only_differences = !self.show_only_differences;
                if let Some(view) = &mut self.file_view {
                    update_visible_file_rows(view, self.show_only_differences);
                }
                self.tree_scroll_y = 0.0;
                ui.ctx().request_repaint();
            }
            let mut changed = false;
            if icon_button_state(
                ui,
                ToolbarIcon::Whitespace,
                true,
                self.ignore_whitespace,
                "Ignore whitespace",
            )
            .clicked()
            {
                self.ignore_whitespace = !self.ignore_whitespace;
                changed = true;
            }
            if icon_button_state(
                ui,
                ToolbarIcon::LineEndings,
                true,
                self.ignore_line_endings,
                "Ignore line endings",
            )
            .clicked()
            {
                self.ignore_line_endings = !self.ignore_line_endings;
                changed = true;
            }
            if changed {
                self.recompare();
                ui.ctx().request_repaint();
            }
            let has_differences = self.file_view.as_ref().map_or_else(
                || !self.folder_differences.is_empty(),
                |view| !view.difference_rows.is_empty(),
            );
            for (icon, forward, label) in [
                (ToolbarIcon::Previous, false, "Previous difference"),
                (ToolbarIcon::Next, true, "Next difference"),
            ] {
                if icon_button(ui, icon, has_differences, label).clicked() {
                    self.jump_difference(forward);
                    ui.ctx().request_repaint();
                }
            }
            if icon_button(ui, ToolbarIcon::New, true, "New comparison").clicked() {
                self.new_comparison();
                ui.ctx().request_repaint();
            }
        });
    }

    fn file_area(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let from_folders = self.file_view.as_ref().unwrap().from_folders;
        egui::Frame::default()
            .fill(palette.panel)
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(6)
            .show(ui, |ui| {
                ui.set_min_height((ui.available_height() - 2.0).max(0.0));
                if !from_folders {
                    self.folder_inputs(ui);
                }
                let Some(view) = self.file_view.as_mut() else {
                    ui.label("Opening selected source…");
                    return;
                };
                if from_folders {
                    let (header, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 38.0),
                        Sense::hover(),
                    );
                    for (side, half) in split_rect(header).into_iter().enumerate() {
                        let rect = half.shrink(7.0);
                        let painter = ui.painter().with_clip_rect(rect);
                        paint_side_label(&painter, rect, side, palette);
                        let path = view.paths[side].display().to_string();
                        let path_rect =
                            Rect::from_min_max(rect.min + egui::vec2(40.0, 0.0), rect.max);
                        let galley =
                            left_elided_path(&painter, &path, path_rect.width(), palette.text);
                        let position =
                            egui::pos2(path_rect.left(), rect.center().y - galley.size().y / 2.0);
                        painter.galley(position, galley, palette.text);
                        ui.interact(path_rect, ui.id().with(("file-path", side)), Sense::hover())
                            .on_hover_text(path);
                    }
                    ui.painter().line_segment(
                        [header.center_top(), header.center_bottom()],
                        Stroke::new(1.0, palette.border),
                    );
                }
                let (divider, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter().line_segment(
                    [divider.left_center(), divider.right_center()],
                    Stroke::new(1.0, palette.border),
                );
                if from_folders && view.sources.iter().any(Option::is_none) {
                    ui.columns(2, |columns| {
                        for (side, column) in columns.iter_mut().enumerate() {
                            if view.sources[side].is_none() {
                                column.label(
                                    RichText::new("Not present on this side").color(palette.muted),
                                );
                            }
                        }
                    });
                }
                if let Some(error) = &view.error {
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(14.0, 14.0), Sense::hover());
                        paint_status_icon(ui.painter(), rect.center(), view.error_icon);
                        ui.colored_label(palette.error, error);
                    });
                    return;
                }
                let Some(comparison) = &view.comparison else {
                    ui.horizontal(|ui| {
                        if view.job.is_some() {
                            ui.spinner();
                            ui.label("Loading file comparison…");
                        } else {
                            ui.label("Choose a file on the other side to compare.");
                        }
                    });
                    return;
                };
                if let Some(message) = &comparison.message {
                    ui.label(RichText::new(message).color(palette.muted));
                }
                if comparison.rows.is_empty() {
                    if comparison.message.is_none() {
                        ui.label(RichText::new("Both files are empty.").color(palette.muted));
                    }
                    return;
                }
                if view.visible_rows.is_empty() {
                    ui.label(
                        RichText::new("No differences with the current options.")
                            .color(palette.muted),
                    );
                    return;
                }
                let height = (ui.available_height() - 2.0).max(0.0);
                let previous_y = view.scroll_y;
                let mut next_y = previous_y;
                // Process the pane under the pointer first, then paint both panes
                // using the resulting shared offset (scroll events apply during layout).
                let active_side =
                    usize::from(ui.input(|input| input.pointer.hover_pos()).is_some_and(
                        |position| position.x > ui.available_rect_before_wrap().center().x,
                    ));
                let mut viewports = [Rect::NOTHING; 2];
                let mut horizontal_offsets = [0.0; 2];
                ui.columns(2, |columns| {
                    for side in [active_side, 1 - active_side] {
                        let column = &mut columns[side];
                        column.spacing_mut().item_spacing = Vec2::ZERO;
                        let output = egui::ScrollArea::both()
                            .id_salt(("file-lines", &view.paths, side))
                            .scroll_bar_visibility(
                                egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                            )
                            .auto_shrink([false, false])
                            .max_height(height)
                            .vertical_scroll_offset(next_y)
                            .show_viewport(column, |ui, _| {
                                ui.set_min_size(egui::vec2(
                                    ui.available_width().max(view.content_widths[side]),
                                    ROW_HEIGHT * view.visible_rows.len() as f32,
                                ));
                            });
                        next_y = output.state.offset.y;
                        viewports[side] = output.inner_rect;
                        horizontal_offsets[side] = output.state.offset.x;
                    }
                });
                for side in 0..2 {
                    let viewport = viewports[side];
                    let painter = ui.painter().with_clip_rect(viewport);
                    let first = (next_y / ROW_HEIGHT).floor() as usize;
                    let last = ((next_y + viewport.height()) / ROW_HEIGHT).ceil() as usize;
                    for index in first..last.min(view.visible_rows.len()) {
                        let original_index = view.visible_rows[index];
                        let row = &comparison.rows[original_index];
                        let line = if side == 0 { &row.left } else { &row.right };
                        let rect = Rect::from_min_size(
                            egui::pos2(
                                viewport.left() - horizontal_offsets[side],
                                viewport.top() + index as f32 * ROW_HEIGHT - next_y,
                            ),
                            egui::vec2(viewport.width().max(view.content_widths[side]), ROW_HEIGHT),
                        );
                        let changed = if side == 0 {
                            &row.left_changed
                        } else {
                            &row.right_changed
                        };
                        let ending = if side == 0 {
                            row.left_ending
                        } else {
                            row.right_ending
                        };
                        let ending_diff =
                            !self.ignore_line_endings && row.left_ending != row.right_ending;
                        paint_file_line(
                            &painter,
                            rect,
                            line.as_ref(),
                            &row.state,
                            index % 2 == 1,
                            changed,
                            ending_diff.then_some(ending).flatten(),
                            view.navigation_row == Some(original_index),
                        );
                    }
                }
                if (next_y - previous_y).abs() > 0.1 {
                    view.scroll_y = next_y;
                    // Scrolling by hand starts navigation from the new viewport.
                    if !view.navigation_scroll_pending {
                        view.navigation_row = None;
                    }
                    ui.ctx().request_repaint();
                }
                view.navigation_scroll_pending = false;
            });
    }

    fn tree_area(&mut self, ui: &mut egui::Ui, height: f32) {
        let palette = Palette::for_context(ui.ctx());
        let mut open = None;
        egui::Frame::default()
            .fill(palette.panel)
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(6)
            .show(ui, |ui| {
                ui.set_min_height(height);
                ui.set_max_height(height);
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let header_top = ui.cursor().top();
                self.folder_inputs(ui);
                let header_height = ui.cursor().top() - header_top;
                let (divider, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter().line_segment(
                    [divider.left_center(), divider.right_center()],
                    Stroke::new(1.0, palette.border),
                );
                let body_height = (height - header_height - 2.0).max(60.0);
                if let Some(tree) = &mut self.tree {
                    let rows: Vec<_> = tree
                        .visible_rows()
                        .into_iter()
                        .filter(|row| {
                            !self.show_only_differences
                                || row.node.state != DirectoryEntryState::Same
                        })
                        .collect();
                    if rows.is_empty() {
                        empty_display(
                            ui,
                            body_height,
                            if self.show_only_differences {
                                "No differences"
                            } else {
                                "Both folders are empty"
                            },
                            if self.show_only_differences {
                                "No files or folders differ with the current options."
                            } else {
                                "There are no files or folders to compare."
                            },
                        );
                        return;
                    }
                    let mut toggle = None;
                    let mut selected = None;
                    let output = egui::ScrollArea::vertical()
                        .id_salt(("linked-trees", self.scroll_generation))
                        .auto_shrink([false, false])
                        .max_height(body_height)
                        .vertical_scroll_offset(self.tree_scroll_y)
                        .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                            for index in range {
                                let row = &rows[index];
                                let node = row.node;
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), ROW_HEIGHT),
                                    Sense::hover(),
                                );
                                let is_selected =
                                    self.selected.as_ref() == Some(&node.relative_path);
                                for (side, half) in split_rect(rect).into_iter().enumerate() {
                                    let entry = if side == 0 { &node.left } else { &node.right };
                                    let present = entry.exists;
                                    let response = ui.interact(
                                        half,
                                        ui.id().with((&node.relative_path, side)),
                                        Sense::click(),
                                    );
                                    let response = if present {
                                        response.on_hover_cursor(egui::CursorIcon::PointingHand)
                                    } else {
                                        response
                                    };
                                    response.widget_info(|| {
                                        egui::WidgetInfo::selected(
                                            egui::WidgetType::SelectableLabel,
                                            true,
                                            is_selected,
                                            format!(
                                                "{}: {} ({})",
                                                if side == 0 { "Left" } else { "Right" },
                                                node.relative_path.display(),
                                                state_label(&node.state)
                                            ),
                                        )
                                    });
                                    paint_row(
                                        ui,
                                        half,
                                        node,
                                        side,
                                        RowAppearance {
                                            depth: row.depth,
                                            expanded: tree.is_expanded(&node.relative_path),
                                            alternate: index % 2 == 1,
                                            selected: is_selected,
                                            hovered: response.hovered() || response.has_focus(),
                                        },
                                    );
                                    let expandable = node.is_expandable()
                                        && entry.kind == Some(DirectoryEntryKind::Directory);
                                    let keyboard_toggle = response.has_focus()
                                        && ui.input(|input| {
                                            input.key_pressed(egui::Key::Space)
                                                || input.key_pressed(egui::Key::Enter)
                                        });
                                    if (response.double_clicked() || keyboard_toggle)
                                        && entry.kind == Some(DirectoryEntryKind::File)
                                    {
                                        open = Some((
                                            node.relative_path.clone(),
                                            [node.left.exists, node.right.exists],
                                            [&node.left, &node.right].into_iter().any(|entry| {
                                                entry.exists
                                                    && entry.kind != Some(DirectoryEntryKind::File)
                                            }),
                                        ));
                                    }
                                    if response.clicked() || keyboard_toggle {
                                        response.request_focus();
                                        selected = Some(node.relative_path.clone());
                                        if expandable {
                                            toggle = Some(node.relative_path.clone());
                                        }
                                    }
                                    if response.has_focus() && expandable {
                                        let expanded = tree.is_expanded(&node.relative_path);
                                        if ui.input(|input| {
                                            (input.key_pressed(egui::Key::ArrowRight) && !expanded)
                                                || (input.key_pressed(egui::Key::ArrowLeft)
                                                    && expanded)
                                        }) {
                                            toggle = Some(node.relative_path.clone());
                                        }
                                    }
                                    response.on_hover_ui(|ui| {
                                        ui.monospace(node.relative_path.display().to_string());
                                        ui.label(if present {
                                            state_label(&node.state)
                                        } else {
                                            "Not present on this side"
                                        });
                                        if let Some(roots) = &self.roots {
                                            ui.monospace(
                                                roots[side]
                                                    .join(&node.relative_path)
                                                    .display()
                                                    .to_string(),
                                            );
                                        }
                                        if let DirectoryEntryState::Error(error) = &node.state {
                                            ui.colored_label(palette.error, error.to_string());
                                        }
                                    });
                                }
                                ui.painter().line_segment(
                                    [rect.center_top(), rect.center_bottom()],
                                    Stroke::new(1.0, palette.border),
                                );
                            }
                        });
                    self.tree_scroll_y = output.state.offset.y;
                    if let Some(path) = toggle {
                        tree.toggle_expanded(path);
                    }
                    if let Some(path) = selected {
                        self.selected = Some(path);
                    }
                } else {
                    let title = if self.job.is_some() {
                        "Comparing folders…"
                    } else if self.source_jobs.iter().any(Option::is_some) {
                        "Opening selected source…"
                    } else if self.mode() == SelectionMode::Incompatible {
                        "Incompatible source types"
                    } else if self.error.is_some() {
                        "Cannot open selected source"
                    } else if self.mode() == SelectionMode::Folder {
                        "Choose another folder"
                    } else {
                        match self.comparison_mode {
                            ComparisonMode::Folder => "Choose folders to compare",
                            ComparisonMode::File => "Choose files to compare",
                        }
                    };
                    empty_display(ui, body_height, title, &self.message);
                }
            });
        if let Some((path, present, type_mismatch)) = open {
            self.open_file(path, present, type_mismatch);
        }
    }

    fn footer(&self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.message).size(11.0).color(palette.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(elapsed) = self.elapsed {
                    ui.label(
                        RichText::new(format!("{:.2}s", elapsed.as_secs_f64()))
                            .monospace()
                            .size(11.0)
                            .color(palette.muted),
                    );
                }
            });
        });
        if let Some(path) = &self.selected {
            ui.add(
                egui::Label::new(
                    RichText::new(path.display().to_string())
                        .monospace()
                        .size(11.0)
                        .color(palette.text),
                )
                .truncate(),
            );
        } else {
            ui.label(RichText::new("Click a folder to expand both trees.  •  Double-click a file to compare.  •  Symlinks are not traversed.").size(10.0).color(palette.muted));
        }
    }
}

impl Drop for VersusApp {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancellation.store(true, Ordering::Relaxed);
        }
    }
}

impl eframe::App for VersusApp {
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        Palette::new(visuals.dark_mode)
            .background
            .to_normalized_gamma_f32()
    }
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll_sources(ctx);
        self.poll_comparison(ctx);
        self.poll_file_comparison(ctx);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.render(ui);
    }
}

fn absolute_path(path: &str) -> PathBuf {
    absolute_native_path(PathBuf::from(path))
}

fn absolute_native_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&path))
            .unwrap_or(path)
    }
}

fn empty_diff_side(path: &std::path::Path) -> bool {
    path.as_os_str() == "/dev/null"
        || (cfg!(windows)
            && path
                .as_os_str()
                .to_str()
                .is_some_and(|name| name.eq_ignore_ascii_case("NUL")))
}

fn comparison_mode_button(
    ui: &egui::Ui,
    rect: Rect,
    label: Arc<egui::Galley>,
    mode: ComparisonMode,
    selected: bool,
) -> egui::Response {
    let response = ui
        .interact(rect, egui::Id::new(mode.label()), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            ui.is_enabled(),
            selected,
            mode.label(),
        )
    });
    let visuals = ui.style().interact_selectable(&response, selected);
    if selected
        || response.hovered()
        || response.has_focus()
        || response.is_pointer_button_down_on()
    {
        ui.painter().rect(
            rect,
            visuals.corner_radius,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
    }
    let icon_center = egui::pos2(
        rect.left() + ui.spacing().button_padding.x + 8.0,
        rect.center().y,
    );
    if mode == ComparisonMode::Folder {
        paint_icon(
            ui.painter(),
            icon_center,
            Some(&DirectoryEntryKind::Directory),
            visuals.fg_stroke.color,
        );
    } else {
        let point = |x, y| icon_center + egui::vec2(x, y);
        let stroke = Stroke::new(1.0, visuals.fg_stroke.color);
        ui.painter().add(egui::Shape::closed_line(
            vec![
                point(-5.0, -7.0),
                point(1.0, -7.0),
                point(5.0, -3.0),
                point(5.0, 7.0),
                point(-5.0, 7.0),
            ],
            stroke,
        ));
        ui.painter().line(
            vec![point(1.0, -7.0), point(1.0, -3.0), point(5.0, -3.0)],
            stroke,
        );
        for y in [1.0, 4.0] {
            ui.painter()
                .line_segment([point(-2.0, y), point(2.0, y)], stroke);
        }
    }
    let position = egui::pos2(icon_center.x + 14.0, rect.center().y - label.size().y / 2.0);
    ui.painter()
        .galley_with_override_text_color(position, label, visuals.fg_stroke.color);
    response
}

fn paint_side_label(painter: &egui::Painter, rect: Rect, side: usize, palette: Palette) {
    painter.text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        if side == 0 { "LEFT" } else { "RIGHT" },
        FontId::monospace(10.0),
        if side == 0 {
            palette.left_only
        } else {
            palette.right_only
        },
    );
}

fn left_elided_path(
    painter: &egui::Painter,
    path: &str,
    width: f32,
    color: Color32,
) -> Arc<egui::Galley> {
    let layout = |text: String| painter.layout_no_wrap(text, FontId::monospace(11.0), color);
    let full = layout(path.into());
    if full.size().x <= width {
        return full;
    }
    if layout("…".into()).size().x > width {
        return layout(String::new());
    }
    let boundaries: Vec<_> = path
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(path.len()))
        .collect();
    let mut first = 0;
    let mut last = boundaries.len() - 1;
    while first < last {
        let middle = (first + last) / 2;
        if layout(format!("…{}", &path[boundaries[middle]..])).size().x <= width {
            last = middle;
        } else {
            first = middle + 1;
        }
    }
    layout(format!("…{}", &path[boundaries[first]..]))
}

fn state_index(state: &DirectoryEntryState) -> usize {
    match state {
        DirectoryEntryState::Same => 0,
        DirectoryEntryState::Different => 1,
        DirectoryEntryState::LeftOnly => 2,
        DirectoryEntryState::RightOnly => 3,
        DirectoryEntryState::TypeMismatch => 4,
        DirectoryEntryState::Error(_) => 5,
    }
}

fn paint_file_line(
    painter: &egui::Painter,
    rect: Rect,
    line: Option<&(usize, String)>,
    state: &DirectoryEntryState,
    alternate: bool,
    changed: &[std::ops::Range<usize>],
    ending: Option<versus::DisplayLineEnding>,
    selected: bool,
) {
    let palette = Palette::for_context(painter.ctx());
    let painter = painter.with_clip_rect(rect);
    let color = state_color(state, palette);
    painter.rect_filled(
        rect,
        0,
        if alternate {
            palette.alternate
        } else {
            palette.panel
        },
    );
    if selected {
        painter.rect_filled(rect, 0, palette.accent.gamma_multiply(0.13));
    }
    if *state != DirectoryEntryState::Same {
        painter.rect_filled(
            Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
            0,
            color,
        );
    }
    let Some((number, text)) = line else {
        painter.text(
            rect.left_center() + egui::vec2(66.0, 0.0),
            Align2::LEFT_CENTER,
            "—",
            FontId::monospace(11.0),
            palette.muted,
        );
        return;
    };
    painter.text(
        rect.left_center() + egui::vec2(42.0, 0.0),
        Align2::RIGHT_CENTER,
        number.to_string(),
        FontId::monospace(10.0),
        palette.muted,
    );
    if let Some(icon) = status_icon(state) {
        paint_status_icon(&painter, rect.left_center() + egui::vec2(54.0, 0.0), icon);
    }
    let mut job = egui::text::LayoutJob::default();
    let mut start = 0;
    for range in changed {
        if range.start < start
            || range.end > text.len()
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
        {
            continue;
        }
        append_file_text(
            &mut job,
            &text[start..range.start],
            color,
            Color32::TRANSPARENT,
        );
        append_file_text(
            &mut job,
            &text[range.clone()],
            palette.text,
            color.gamma_multiply(0.3),
        );
        start = range.end;
    }
    append_file_text(&mut job, &text[start..], color, Color32::TRANSPARENT);
    let galley = painter.layout_job(job);
    let position = egui::pos2(rect.left() + 68.0, rect.center().y - galley.size().y / 2.0);
    let end_x = position.x + galley.size().x;
    painter.galley(position, galley, color);
    if let Some(ending) = ending {
        let badge = Rect::from_min_size(
            egui::pos2(end_x + 8.0, rect.top() + 3.0),
            egui::vec2(58.0, 16.0),
        );
        painter.rect_filled(badge, 2, color.gamma_multiply(0.3));
        painter.text(
            badge.center(),
            Align2::CENTER_CENTER,
            ending.label(),
            FontId::monospace(9.0),
            palette.text,
        );
    }
}

fn append_file_text(
    job: &mut egui::text::LayoutJob,
    text: &str,
    color: Color32,
    background: Color32,
) {
    job.append(
        &text.replace('\t', "    "),
        0.0,
        egui::TextFormat {
            font_id: FontId::monospace(11.0),
            color,
            background,
            ..Default::default()
        },
    );
}

fn update_visible_file_rows(view: &mut FileView, differences_only: bool) {
    let anchor = view
        .visible_rows
        .get((view.scroll_y / ROW_HEIGHT).floor() as usize)
        .copied();
    let Some(comparison) = &view.comparison else {
        return;
    };
    view.difference_rows = comparison
        .rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| (row.state != DirectoryEntryState::Same).then_some(index))
        .collect();
    view.visible_rows = if differences_only {
        view.difference_rows.clone()
    } else {
        (0..comparison.rows.len()).collect()
    };
    let top = anchor
        .and_then(|anchor| view.visible_rows.iter().position(|index| *index >= anchor))
        .unwrap_or(0);
    view.scroll_y = top as f32 * ROW_HEIGHT;
}

fn progress_text(snapshot: versus::ProgressSnapshot, started: Instant, now: Instant) -> String {
    let stage = match snapshot.stage {
        ProgressStage::Scanning => "Scanning folders",
        ProgressStage::Reading => "Reading files",
        ProgressStage::ComparingFiles => "Comparing files",
        ProgressStage::ComparingLines => "Comparing lines",
        ProgressStage::Highlighting => "Highlighting differences",
        ProgressStage::Finished => "Preparing comparison",
    };
    let work = match (snapshot.stage, snapshot.total) {
        (ProgressStage::Reading, Some(total)) => format!(
            " · {} / {}",
            format_size(Some(snapshot.completed)),
            format_size(Some(total))
        ),
        (ProgressStage::Scanning, _) => format!(" · {} entries found", snapshot.completed),
        (_, Some(total)) if total > 0 => format!(" · {} / {}", snapshot.completed, total),
        _ => String::new(),
    };
    let eta = snapshot
        .remaining_at(now)
        .map_or_else(String::new, |remaining| {
            format!(
                " · ~{}s left in this stage",
                remaining.as_secs_f64().ceil().max(1.0) as u64
            )
        });
    format!(
        "{stage}…{work} · {:.1}s elapsed{eta}",
        now.saturating_duration_since(started).as_secs_f64()
    )
}

fn count_entries(tree: &FolderTree) -> [usize; 6] {
    let mut counts = [0; 6];
    let mut nodes: Vec<_> = tree.root().children.iter().collect();
    while let Some(node) = nodes.pop() {
        let index = state_index(&node.state);
        counts[index] += 1;
        nodes.extend(&node.children);
    }
    counts
}

fn split_rect(rect: Rect) -> [Rect; 2] {
    [
        Rect::from_min_max(rect.min, rect.center_bottom()),
        Rect::from_min_max(rect.center_top(), rect.max),
    ]
}

#[derive(Clone, Copy)]
enum StatusIcon {
    Different,
    LeftOnly,
    RightOnly,
    TypeMismatch,
    Error,
}

impl StatusIcon {
    fn label(self) -> &'static str {
        match self {
            Self::Different => "Different",
            Self::LeftOnly => "Left only",
            Self::RightOnly => "Right only",
            Self::TypeMismatch => "Type mismatch",
            Self::Error => "Read error",
        }
    }
    fn color(self, palette: Palette) -> Color32 {
        match self {
            Self::Different => palette.changed,
            Self::LeftOnly => palette.left_only,
            Self::RightOnly => palette.right_only,
            Self::TypeMismatch | Self::Error => palette.error,
        }
    }
}

fn status_icon(state: &DirectoryEntryState) -> Option<StatusIcon> {
    match state {
        DirectoryEntryState::Same => None,
        DirectoryEntryState::Different => Some(StatusIcon::Different),
        DirectoryEntryState::LeftOnly => Some(StatusIcon::LeftOnly),
        DirectoryEntryState::RightOnly => Some(StatusIcon::RightOnly),
        DirectoryEntryState::TypeMismatch => Some(StatusIcon::TypeMismatch),
        DirectoryEntryState::Error(_) => Some(StatusIcon::Error),
    }
}

fn state_label(state: &DirectoryEntryState) -> &'static str {
    status_icon(state).map_or("Identical", StatusIcon::label)
}
fn state_color(state: &DirectoryEntryState, palette: Palette) -> Color32 {
    status_icon(state).map_or(palette.muted, |icon| icon.color(palette))
}

fn paint_status_icon(painter: &egui::Painter, center: Pos2, icon: StatusIcon) {
    let palette = Palette::for_context(painter.ctx());
    let stroke = Stroke::new(1.3, icon.color(palette));
    let point = |x, y| center + egui::vec2(x, y);
    match icon {
        StatusIcon::Different => {
            for y in [-2.0, 2.0] {
                painter.line_segment([point(-5.0, y), point(5.0, y)], stroke);
            }
            painter.line_segment([point(3.0, -6.0), point(-3.0, 6.0)], stroke);
        }
        StatusIcon::LeftOnly | StatusIcon::RightOnly => {
            let direction = if matches!(icon, StatusIcon::LeftOnly) {
                -1.0
            } else {
                1.0
            };
            painter.line_segment([point(-5.0, 0.0), point(5.0, 0.0)], stroke);
            painter.line(
                vec![
                    point(direction, -4.0),
                    point(direction * 5.0, 0.0),
                    point(direction, 4.0),
                ],
                stroke,
            );
        }
        StatusIcon::TypeMismatch => {
            painter.rect_stroke(
                Rect::from_center_size(center, egui::vec2(12.0, 12.0)),
                1,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.line_segment([point(-3.0, -3.0), point(3.0, 3.0)], stroke);
            painter.line_segment([point(-3.0, 3.0), point(3.0, -3.0)], stroke);
        }
        StatusIcon::Error => {
            painter.add(egui::Shape::closed_line(
                vec![point(0.0, -6.0), point(6.0, 5.0), point(-6.0, 5.0)],
                stroke,
            ));
            painter.line_segment([point(0.0, -2.0), point(0.0, 1.0)], stroke);
            painter.circle_filled(point(0.0, 3.0), 0.7, icon.color(palette));
        }
    }
}

#[derive(Clone, Copy)]
enum ToolbarIcon {
    New,
    Back,
    Refresh,
    Expand,
    Collapse,
    Browse,
    Cancel,
    Sun,
    Moon,
    Differences,
    Whitespace,
    LineEndings,
    Previous,
    Next,
}

fn icon_button(ui: &mut egui::Ui, icon: ToolbarIcon, enabled: bool, label: &str) -> egui::Response {
    icon_button_state(ui, icon, enabled, false, label)
}

fn icon_button_state(
    ui: &mut egui::Ui,
    icon: ToolbarIcon,
    enabled: bool,
    selected: bool,
    label: &str,
) -> egui::Response {
    let palette = Palette::for_context(ui.ctx());
    ui.add_enabled_ui(enabled, |ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), Sense::hover());
        let response = ui.interact(rect, egui::Id::new(label), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, label)
        });
        let color = if !enabled {
            palette.muted.gamma_multiply(0.4)
        } else if selected || response.hovered() || response.has_focus() {
            palette.accent
        } else {
            palette.text
        };
        if response.hovered() || response.has_focus() {
            ui.painter().rect_filled(rect, 3, palette.border);
        }
        if selected {
            ui.painter()
                .rect_filled(rect, 3, palette.accent.gamma_multiply(0.15));
            ui.painter().rect_stroke(
                rect,
                3,
                Stroke::new(1.0, palette.accent),
                egui::StrokeKind::Inside,
            );
        }
        let center = rect.center();
        let point = |x, y| center + egui::vec2(x, y);
        let stroke = Stroke::new(1.4, color);
        match icon {
            ToolbarIcon::Differences => {
                for y in [-4.0, 4.0] {
                    ui.painter()
                        .line_segment([point(-6.0, y), point(6.0, y)], stroke);
                }
                ui.painter()
                    .line_segment([point(3.0, -8.0), point(-3.0, 8.0)], stroke);
            }
            ToolbarIcon::Whitespace => {
                ui.painter().line(
                    vec![
                        point(-7.0, -2.0),
                        point(-7.0, 5.0),
                        point(7.0, 5.0),
                        point(7.0, -2.0),
                    ],
                    stroke,
                );
                ui.painter().circle_filled(point(0.0, -3.0), 1.3, color);
            }
            ToolbarIcon::LineEndings => {
                ui.painter().line(
                    vec![point(6.0, -6.0), point(6.0, 3.0), point(-6.0, 3.0)],
                    stroke,
                );
                ui.painter().line(
                    vec![point(-1.0, -2.0), point(-6.0, 3.0), point(-1.0, 8.0)],
                    stroke,
                );
            }
            ToolbarIcon::Previous | ToolbarIcon::Next => {
                let direction = if matches!(icon, ToolbarIcon::Next) {
                    1.0
                } else {
                    -1.0
                };
                ui.painter().line_segment(
                    [point(0.0, -7.0 * direction), point(0.0, 7.0 * direction)],
                    stroke,
                );
                ui.painter().line(
                    vec![
                        point(-5.0, 2.0 * direction),
                        point(0.0, 7.0 * direction),
                        point(5.0, 2.0 * direction),
                    ],
                    stroke,
                );
            }
            ToolbarIcon::New => {
                ui.painter().line_segment(
                    [center - egui::vec2(6.0, 0.0), center + egui::vec2(6.0, 0.0)],
                    stroke,
                );
                ui.painter().line_segment(
                    [center - egui::vec2(0.0, 6.0), center + egui::vec2(0.0, 6.0)],
                    stroke,
                );
            }
            ToolbarIcon::Back => {
                ui.painter()
                    .line_segment([point(6.0, 0.0), point(-6.0, 0.0)], stroke);
                ui.painter().line(
                    vec![point(-1.0, -5.0), point(-6.0, 0.0), point(-1.0, 5.0)],
                    stroke,
                );
            }
            ToolbarIcon::Refresh => {
                let arc: Vec<_> = (0..=24)
                    .map(|step| {
                        let angle = (50.0 + step as f32 * 270.0 / 24.0).to_radians();
                        point(angle.cos() * 6.0, angle.sin() * 6.0)
                    })
                    .collect();
                let end = *arc.last().unwrap();
                ui.painter().line(arc, stroke);
                ui.painter().line(
                    vec![
                        end + egui::vec2(-4.0, -0.5),
                        end,
                        end + egui::vec2(0.0, -4.0),
                    ],
                    stroke,
                );
            }
            ToolbarIcon::Expand | ToolbarIcon::Collapse => {
                let back = Rect::from_center_size(point(-2.0, 2.0), egui::vec2(12.0, 12.0));
                let front = Rect::from_center_size(point(1.0, -1.0), egui::vec2(12.0, 12.0));
                ui.painter().rect_stroke(
                    back,
                    2,
                    Stroke::new(1.0, color.gamma_multiply(0.65)),
                    egui::StrokeKind::Inside,
                );
                ui.painter().rect_filled(front, 2, palette.panel);
                ui.painter()
                    .rect_stroke(front, 2, stroke, egui::StrokeKind::Inside);
                let center = front.center();
                ui.painter().line_segment(
                    [center - egui::vec2(3.0, 0.0), center + egui::vec2(3.0, 0.0)],
                    stroke,
                );
                if matches!(icon, ToolbarIcon::Expand) {
                    ui.painter().line_segment(
                        [center - egui::vec2(0.0, 3.0), center + egui::vec2(0.0, 3.0)],
                        stroke,
                    );
                }
            }
            ToolbarIcon::Browse => paint_icon(
                ui.painter(),
                center,
                Some(&DirectoryEntryKind::Directory),
                color,
            ),
            ToolbarIcon::Sun => {
                ui.painter().circle_filled(center, 4.0, color);
                for index in 0..8 {
                    let angle = index as f32 * std::f32::consts::TAU / 8.0;
                    let direction = egui::vec2(angle.cos(), angle.sin());
                    ui.painter().line_segment(
                        [center + direction * 7.2, center + direction * 10.2],
                        Stroke::new(1.2, color),
                    );
                }
            }
            ToolbarIcon::Moon => {
                let arc: Vec<_> = (0..=24)
                    .map(|step| {
                        let angle = (60.0 + step as f32 * 240.0 / 24.0).to_radians();
                        point(angle.cos() * 7.0, angle.sin() * 7.0)
                    })
                    .chain((0..=16).map(|step| {
                        let angle = (-90.0 - step as f32 * 180.0 / 16.0).to_radians();
                        point(3.5 + angle.cos() * 5.0, angle.sin() * 6.0)
                    }))
                    .collect();
                ui.painter().add(egui::Shape::closed_line(arc, stroke));
            }
            ToolbarIcon::Cancel => {
                ui.painter()
                    .line_segment([point(-4.0, -4.0), point(4.0, 4.0)], stroke);
                ui.painter()
                    .line_segment([point(-4.0, 4.0), point(4.0, -4.0)], stroke);
            }
        }
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(label)
    })
    .inner
}

fn format_size(size: Option<u64>) -> String {
    let Some(bytes) = size else {
        return "—".into();
    };
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let units = ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let mut unit = 0;
    value /= 1024.0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", units[unit])
}

struct RowAppearance {
    depth: usize,
    expanded: bool,
    alternate: bool,
    selected: bool,
    hovered: bool,
}

fn paint_row(ui: &egui::Ui, rect: Rect, node: &TreeNode, side: usize, appearance: RowAppearance) {
    let palette = Palette::for_context(ui.ctx());
    let entry = if side == 0 { &node.left } else { &node.right };
    let kind = entry.kind.as_ref();
    let present = entry.exists;
    let state = &node.state;
    let RowAppearance {
        depth,
        expanded,
        alternate,
        selected,
        hovered,
    } = appearance;
    let painter = ui.painter().with_clip_rect(rect);
    let color = state_color(state, palette);
    let fill = if selected {
        palette.accent.gamma_multiply(0.16)
    } else if hovered {
        palette.hover
    } else if alternate {
        palette.alternate
    } else {
        palette.panel
    };
    painter.rect_filled(rect, 0, fill);
    if present && *state != DirectoryEntryState::Same {
        painter.rect_filled(
            Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
            0,
            color,
        );
    }
    let x = rect.left() + 8.0 + depth as f32 * 14.0;
    let y = rect.center().y;
    let name_right = rect.right() - 98.0;
    if !present {
        painter.text(
            egui::pos2(x + 36.0, y),
            Align2::LEFT_CENTER,
            "—",
            FontId::monospace(11.0),
            palette.muted.gamma_multiply(0.55),
        );
        return;
    }
    let name_painter = painter.with_clip_rect(Rect::from_min_max(
        rect.min,
        egui::pos2(name_right.max(rect.left()), rect.bottom()),
    ));
    if kind == Some(&DirectoryEntryKind::Directory) && node.is_expandable() {
        name_painter.text(
            egui::pos2(x, y),
            Align2::LEFT_CENTER,
            if expanded { "▾" } else { "▸" },
            FontId::monospace(15.0),
            palette.muted,
        );
    }
    paint_icon(&name_painter, egui::pos2(x + 19.0, y), kind, color);
    name_painter.text(
        egui::pos2(x + 40.0, y),
        Align2::LEFT_CENTER,
        node.name.to_string_lossy(),
        FontId::monospace(11.0),
        color,
    );
    painter.text(
        egui::pos2(rect.right() - 32.0, y),
        Align2::RIGHT_CENTER,
        format_size(entry.size),
        FontId::monospace(10.0),
        palette.muted,
    );
    if let Some(icon) = status_icon(state) {
        paint_status_icon(&painter, egui::pos2(rect.right() - 15.0, y), icon);
    }
}

fn paint_icon(
    painter: &egui::Painter,
    center: Pos2,
    kind: Option<&DirectoryEntryKind>,
    color: Color32,
) {
    let rect = Rect::from_center_size(center, egui::vec2(13.0, 11.0));
    if kind == Some(&DirectoryEntryKind::Directory) {
        painter.rect_filled(
            Rect::from_min_size(rect.min - egui::vec2(0.0, 2.0), egui::vec2(6.0, 4.0)),
            1,
            color.gamma_multiply(0.6),
        );
        painter.rect_filled(rect, 2, color.gamma_multiply(0.35));
        painter.rect_stroke(rect, 2, Stroke::new(1.0, color), egui::StrokeKind::Inside);
    } else if kind == Some(&DirectoryEntryKind::Symlink) {
        painter.text(
            center,
            Align2::CENTER_CENTER,
            "@",
            FontId::monospace(13.0),
            color,
        );
    } else {
        painter.rect_stroke(
            rect.shrink2(egui::vec2(2.0, 0.0)),
            1,
            Stroke::new(1.0, color),
            egui::StrokeKind::Inside,
        );
    }
}

fn empty_display(ui: &mut egui::Ui, height: f32, title: &str, subtitle: &str) {
    let palette = Palette::for_context(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height.max(80.0)),
        Sense::hover(),
    );
    ui.painter().line_segment(
        [rect.center_top(), rect.center_bottom()],
        Stroke::new(1.0, palette.border.gamma_multiply(0.5)),
    );
    let painter = ui.painter().with_clip_rect(rect);
    let center = rect.center();
    painter.text(
        center - egui::vec2(0.0, 24.0),
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(21.0),
        palette.text,
    );
    painter.text(
        center + egui::vec2(0.0, 8.0),
        Align2::CENTER_CENTER,
        subtitle,
        FontId::proportional(11.0),
        palette.muted,
    );
}

fn apply_theme(ctx: &egui::Context) {
    set_theme(ctx, egui::Theme::Dark);
}

fn set_theme(ctx: &egui::Context, theme: egui::Theme) {
    let palette = Palette::new(theme == egui::Theme::Dark);
    ctx.set_theme(theme);
    let mut visuals = if theme == egui::Theme::Dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.panel_fill = palette.background;
    visuals.window_fill = palette.panel;
    visuals.extreme_bg_color = palette.background;
    visuals.override_text_color = Some(palette.text);
    visuals.selection.bg_fill = palette.accent.gamma_multiply(0.3);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.border);
    ctx.set_visuals(visuals);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(6.0, 3.0);
        style.spacing.interact_size.y = 24.0;
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(11.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(11.0));
        style
            .text_styles
            .insert(egui::TextStyle::Monospace, FontId::monospace(11.0));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use versus::{DirectoryDiff, DirectoryEntry};

    fn engineering_path(relative: &str) -> String {
        // Windows needs a drive-qualified root to avoid resolving fixtures
        // relative to the CI checkout. Build components with native separators.
        let mut path = PathBuf::from(if cfg!(windows) {
            r"C:\engineering"
        } else {
            "/engineering"
        });
        path.extend(relative.split('/'));
        assert!(path.is_absolute());
        path.display().to_string()
    }

    fn fixture() -> FolderTree {
        FolderTree::from_diff(&DirectoryDiff {
            cancelled: false,
            entries: [
                ("assembly", DirectoryEntryKind::Directory),
                ("assembly/model.step", DirectoryEntryKind::File),
            ]
            .into_iter()
            .map(|(path, kind)| DirectoryEntry {
                relative_path: path.into(),
                left_size: Some(0),
                right_size: Some(0),
                left_exists: true,
                right_exists: true,
                left_kind: Some(kind.clone()),
                right_kind: Some(kind.clone()),
                kind,
                state: DirectoryEntryState::Same,
            })
            .collect(),
        })
    }

    fn loaded_app() -> VersusApp {
        let mut app = VersusApp::default();
        app.tree = Some(fixture());
        app.roots = Some(["left".into(), "right".into()]);
        app
    }

    fn render(
        app: &mut VersusApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
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
    }

    fn text_positions(output: &egui::FullOutput, value: &str) -> Vec<Pos2> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == value => Some(text.pos),
                _ => None,
            })
            .collect()
    }

    fn text_centers(output: &egui::FullOutput, value: &str) -> Vec<Pos2> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == value => {
                    Some(text.pos + text.galley.size() / 2.0)
                }
                _ => None,
            })
            .collect()
    }

    fn click(app: &mut VersusApp, ctx: &egui::Context, position: Pos2) {
        for pressed in [true, false] {
            render(
                app,
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

    #[test]
    fn clicking_either_folder_pane_updates_both_rendered_trees() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        let output = render(&mut app, &ctx, vec![]);
        let folders = text_positions(&output, "assembly");
        assert_eq!(folders.len(), 2);
        output.drop_without_applying_deltas();
        click(&mut app, &ctx, folders[0] + egui::vec2(3.0, 5.0));
        assert!(app.tree.as_ref().unwrap().is_expanded("assembly"));
        let output = render(&mut app, &ctx, vec![]);
        let children = text_positions(&output, "model.step");
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].y, children[1].y);
        let folders = text_positions(&output, "assembly");
        output.drop_without_applying_deltas();
        click(&mut app, &ctx, folders[1] + egui::vec2(3.0, 5.0));
        assert!(!app.tree.as_ref().unwrap().is_expanded("assembly"));
        let output = render(&mut app, &ctx, vec![]);
        assert!(text_positions(&output, "model.step").is_empty());
        output.drop_without_applying_deltas();
    }

    #[test]
    fn one_sided_rows_render_an_aligned_placeholder() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.tree = Some(FolderTree::from_diff(&DirectoryDiff {
            cancelled: false,
            entries: vec![DirectoryEntry {
                relative_path: "left-only.step".into(),
                left_size: Some(1024),
                right_size: None,
                left_exists: true,
                right_exists: false,
                left_kind: Some(DirectoryEntryKind::File),
                right_kind: None,
                kind: DirectoryEntryKind::File,
                state: DirectoryEntryState::LeftOnly,
            }],
        }));
        let output = render(&mut app, &ctx, vec![]);
        let names = text_positions(&output, "left-only.step");
        let placeholders = text_positions(&output, "—");
        assert_eq!(names.len(), 1);
        assert_eq!(placeholders.len(), 1);
        assert_eq!(names[0].y, placeholders[0].y);
        assert!(placeholders[0].x > 600.0);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn empty_folder_has_no_disclosure_and_click_only_selects_it() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.tree = Some(FolderTree::from_diff(&DirectoryDiff {
            cancelled: false,
            entries: vec![DirectoryEntry {
                relative_path: "empty-folder".into(),
                left_size: Some(0),
                right_size: Some(0),
                left_exists: true,
                right_exists: true,
                left_kind: Some(DirectoryEntryKind::Directory),
                right_kind: Some(DirectoryEntryKind::Directory),
                kind: DirectoryEntryKind::Directory,
                state: DirectoryEntryState::Same,
            }],
        }));
        let output = render(&mut app, &ctx, vec![]);
        assert!(text_positions(&output, "▸").is_empty());
        let folders = text_positions(&output, "empty-folder");
        output.drop_without_applying_deltas();
        click(&mut app, &ctx, folders[0] + egui::vec2(3.0, 5.0));
        assert_eq!(
            app.selected.as_deref(),
            Some(std::path::Path::new("empty-folder"))
        );
        assert!(!app.tree.as_ref().unwrap().is_expanded("empty-folder"));
    }

    #[test]
    fn minimum_window_keeps_controls_and_footer_inside_the_viewport() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 650.0))),
                ..Default::default()
            },
            |ui| app.render(ui),
        );
        output.textures_delta.clear();
        let controls: Vec<_> = ["Collapse all", "Expand all", "Refresh comparison"]
            .into_iter()
            .map(|label| ctx.read_response(egui::Id::new(label)).unwrap().rect)
            .collect();
        assert!(
            controls
                .iter()
                .all(|rect| rect.right() <= 890.0 && rect.left() > 700.0 && rect.bottom() <= 45.0)
        );
        for label in [
            "Show only differences",
            "Ignore whitespace",
            "Ignore line endings",
            "Previous difference",
            "Next difference",
            "New comparison",
        ] {
            let rect = ctx.read_response(egui::Id::new(label)).unwrap().rect;
            assert!(
                rect.left() >= 0.0 && rect.right() <= 890.0,
                "{label}: {rect:?}"
            );
            assert!(rect.top() >= 45.0 && rect.bottom() < app.pane_rects[0].top());
        }
        for text in ["Choose a folder on each side to begin."] {
            let positions = text_positions(&output, text);
            assert!(!positions.is_empty(), "Missing {text}");
            assert!(
                positions.iter().all(|position| position.x >= 0.0
                    && position.x < 900.0
                    && position.y >= 0.0
                    && position.y < 630.0),
                "{text} outside viewport: {positions:?}"
            );
        }
        output.drop_without_applying_deltas();
    }

    #[test]
    fn clicked_item_footer_is_fully_visible_at_supported_window_sizes() {
        for (width, height) in [(900.0, 650.0), (1200.0, 800.0)] {
            let ctx = egui::Context::default();
            apply_theme(&ctx);
            let mut app = loaded_app();
            app.tree.as_mut().unwrap().expand_all();
            let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(width, height));
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| app.render(ui),
            );
            output.textures_delta.clear();
            let file = text_positions(&output, "model.step")[1];
            output.drop_without_applying_deltas();
            for pressed in [true, false] {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        events: vec![
                            egui::Event::PointerMoved(file + egui::vec2(3.0, 5.0)),
                            egui::Event::PointerButton {
                                pos: file + egui::vec2(3.0, 5.0),
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            },
                        ],
                        ..Default::default()
                    },
                    |ui| app.render(ui),
                );
                output.textures_delta.clear();
                output.drop_without_applying_deltas();
            }
            assert_eq!(
                app.selected.as_deref(),
                Some(std::path::Path::new("assembly/model.step"))
            );
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| app.render(ui),
            );
            output.textures_delta.clear();
            let selected_path = PathBuf::from("assembly").join("model.step");
            let selected_text = selected_path.display().to_string();
            let (shape, text) = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == selected_text => {
                        Some((shape, text))
                    }
                    _ => None,
                })
                .expect("Clicked item should appear in footer");
            let bounds = Rect::from_min_size(text.pos, text.galley.size());
            assert!(bounds.top() > height - 55.0);
            assert!(
                screen.shrink(10.0).contains_rect(bounds),
                "Footer outside window: {bounds:?}"
            );
            assert!(
                shape.clip_rect.contains_rect(bounds),
                "Footer clipped: {bounds:?}"
            );
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn clicking_path_or_browse_opens_picker_for_the_correct_side() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.paths = [String::new(), engineering_path("release/designs")];
        let original_paths = app.paths.clone();
        let mut requests = Vec::new();
        {
            let mut render_inputs = |events| {
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            egui::vec2(1200.0, 800.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        app.folder_inputs_with_picker(ui, |side, path, _| {
                            requests.push((side, path.to_owned()));
                            None // Closing the dialog should preserve the current comparison.
                        })
                    },
                )
            };
            for (side, path_text) in [(0, "Choose a folder"), (1, original_paths[1].as_str())] {
                for path_area in [true, false] {
                    let output = render_inputs(vec![]);
                    let target = if path_area {
                        text_centers(&output, path_text)[0]
                    } else {
                        ctx.read_response(egui::Id::new(if side == 0 {
                            "Browse left folder"
                        } else {
                            "Browse right folder"
                        }))
                        .unwrap()
                        .rect
                        .center()
                    };
                    output.drop_without_applying_deltas();
                    let output = render_inputs(vec![egui::Event::PointerMoved(target)]);
                    assert_eq!(
                        output.platform_output.cursor_icon,
                        egui::CursorIcon::PointingHand
                    );
                    output.drop_without_applying_deltas();
                    for pressed in [true, false] {
                        render_inputs(vec![
                            egui::Event::PointerMoved(target),
                            egui::Event::PointerButton {
                                pos: target,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            },
                        ])
                        .drop_without_applying_deltas();
                    }
                }
            }
        }
        assert_eq!(
            requests,
            vec![
                (0, String::new()),
                (0, String::new()),
                (1, original_paths[1].clone()),
                (1, original_paths[1].clone())
            ]
        );
        assert_eq!(app.paths, original_paths);
        assert!(app.tree.is_some());
    }

    #[test]
    fn single_path_headers_center_side_labels_and_browse_buttons() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.paths = [
            engineering_path(&format!("{}release", "long-folder/".repeat(12))),
            engineering_path("release"),
        ];
        let output = render(&mut app, &ctx, vec![]);
        let mut centers = Vec::new();
        for (side, label) in ["LEFT", "RIGHT"].into_iter().enumerate() {
            let paths = text_centers(&output, &app.paths[side]);
            assert_eq!(paths.len(), 1, "Selected path must only appear once");
            let side_label = text_centers(&output, label)[0];
            let browse = ctx
                .read_response(egui::Id::new(if side == 0 {
                    "Browse left folder"
                } else {
                    "Browse right folder"
                }))
                .unwrap();
            assert_eq!(side_label.y, paths[0].y);
            assert_eq!(side_label.y, browse.rect.center().y);
            centers.push(side_label.y);
        }
        assert_eq!(centers[0], centers[1]);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn toolbar_icons_expand_collapse_and_refresh_the_comparison() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.paths = ["left".into(), "right".into()];
        render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
        let expand = ctx
            .read_response(egui::Id::new("Expand all"))
            .unwrap()
            .rect
            .center();
        click(&mut app, &ctx, expand);
        assert!(app.tree.as_ref().unwrap().is_expanded("assembly"));
        let collapse = ctx
            .read_response(egui::Id::new("Collapse all"))
            .unwrap()
            .rect
            .center();
        click(&mut app, &ctx, collapse);
        assert!(!app.tree.as_ref().unwrap().is_expanded("assembly"));
        let refresh = ctx
            .read_response(egui::Id::new("Refresh comparison"))
            .unwrap()
            .rect
            .center();
        click(&mut app, &ctx, refresh);
        assert!(app.job.is_some());
    }

    #[test]
    fn full_paths_follow_the_legend_and_precede_tree_rows() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.paths = [
            engineering_path("current/designs"),
            engineering_path("release/designs"),
        ];
        let output = render(&mut app, &ctx, vec![]);
        let legend = text_positions(&output, "Different  0")[0];
        let rows = text_positions(&output, "assembly");
        for path in &app.paths {
            let positions = text_positions(&output, path);
            assert!(!positions.is_empty());
            assert!(
                positions
                    .iter()
                    .all(|position| position.y > legend.y && position.y < rows[0].y)
            );
        }
        assert!(rows[0].y < 160.0, "Compact tree should begin near the top");
        output.drop_without_applying_deltas();
    }

    #[test]
    fn long_selected_path_wraps_without_truncation() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.paths[0] = engineering_path(&format!("{}release", "long-folder-name/".repeat(12)));
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 650.0))),
                ..Default::default()
            },
            |ui| app.render(ui),
        );
        output.textures_delta.clear();
        let wrapped = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text)
                    if text.galley.text() == app.paths[0] && text.galley.rows.len() > 1 =>
                {
                    Some(text)
                }
                _ => None,
            })
            .expect("Full selected path should wrap in the pane header");
        assert!(wrapped.pos.x + wrapped.galley.size().x < 450.0);
        assert!(text_positions(&output, "assembly")[0].y > wrapped.pos.y + wrapped.galley.size().y);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn hovering_files_and_folders_uses_the_pointer_cursor() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.tree.as_mut().unwrap().expand_all();
        let output = render(&mut app, &ctx, vec![]);
        let targets = [
            text_positions(&output, "assembly")[0],
            text_positions(&output, "model.step")[1],
        ];
        output.drop_without_applying_deltas();
        for position in targets {
            let output = render(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(position + egui::vec2(3.0, 5.0))],
            );
            assert_eq!(
                output.platform_output.cursor_icon,
                egui::CursorIcon::PointingHand
            );
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn row_shows_size_beside_status_icon_without_a_status_word() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        let output = render(&mut app, &ctx, vec![]);
        let names = text_centers(&output, "assembly");
        let sizes = text_centers(&output, "0 B");
        assert_eq!(sizes.len(), 2);
        assert!(
            sizes
                .iter()
                .zip(names)
                .all(|(size, name)| size.y == name.y && size.x > name.x)
        );
        assert!(text_positions(&output, "Identical").is_empty());
        output.drop_without_applying_deltas();
        assert_eq!(format_size(Some(1024)), "1.0 KiB");
        assert_eq!(format_size(Some(1024 * 1024)), "1.0 MiB");
        assert_eq!(format_size(None), "—");
    }

    fn test_file_row(
        left: Option<(usize, String)>,
        right: Option<(usize, String)>,
        state: DirectoryEntryState,
    ) -> versus::FileComparisonRow {
        versus::FileComparisonRow {
            left_ending: left.as_ref().map(|_| versus::DisplayLineEnding::Lf),
            right_ending: right.as_ref().map(|_| versus::DisplayLineEnding::Lf),
            left_changed: Vec::new(),
            right_changed: Vec::new(),
            left,
            right,
            state,
        }
    }

    fn loaded_file_view() -> FileView {
        FileView {
            from_folders: true,
            paths: ["/left/model.step".into(), "/right/model.step".into()],
            sources: [
                Some("/left/model.step".into()),
                Some("/right/model.step".into()),
            ],
            job: None,
            comparison: Some(FileComparison {
                rows: vec![
                    test_file_row(
                        Some((1, "unchanged".into())),
                        Some((1, "unchanged".into())),
                        DirectoryEntryState::Same,
                    ),
                    test_file_row(
                        Some((2, "old value".into())),
                        Some((2, "new value".into())),
                        DirectoryEntryState::Different,
                    ),
                    test_file_row(
                        Some((3, "removed".into())),
                        None,
                        DirectoryEntryState::LeftOnly,
                    ),
                ],
                message: None,
            }),
            error: None,
            error_icon: StatusIcon::Error,
            scroll_y: 0.0,
            content_widths: [300.0; 2],
            counts: [1, 1, 1, 0, 0, 0],
            visible_rows: vec![0, 1, 2],
            difference_rows: vec![1, 2],
            navigation_row: None,
            navigation_scroll_pending: false,
        }
    }

    #[test]
    fn double_clicking_file_on_either_side_opens_view_and_back_restores_tree() {
        for side in 0..2 {
            let ctx = egui::Context::default();
            apply_theme(&ctx);
            let mut app = loaded_app();
            app.tree.as_mut().unwrap().expand_all();
            let generation = app.scroll_generation;
            let output = render(&mut app, &ctx, vec![]);
            let target = text_positions(&output, "model.step")[side] + egui::vec2(3.0, 5.0);
            output.drop_without_applying_deltas();
            click(&mut app, &ctx, target);
            assert!(
                app.file_view.is_none(),
                "Single click should only select a file"
            );
            click(&mut app, &ctx, target);
            let view = app
                .file_view
                .as_ref()
                .expect("Double click should open file comparison");
            assert_eq!(
                view.paths,
                [
                    PathBuf::from("left/assembly/model.step"),
                    PathBuf::from("right/assembly/model.step")
                ]
            );
            let cancellation = view.job.as_ref().unwrap().cancellation.clone();
            let output = render(&mut app, &ctx, vec![]);
            let back = ctx
                .read_response(egui::Id::new("Back to folders"))
                .unwrap()
                .rect
                .center();
            assert!(text_positions(&output, "assembly").is_empty());
            output.drop_without_applying_deltas();
            click(&mut app, &ctx, back);
            assert!(app.file_view.is_none());
            assert!(cancellation.load(Ordering::Relaxed));
            assert!(app.tree.as_ref().unwrap().is_expanded("assembly"));
            assert_eq!(
                app.selected.as_deref(),
                Some(std::path::Path::new("assembly/model.step"))
            );
            assert_eq!(app.scroll_generation, generation);
            let output = render(&mut app, &ctx, vec![]);
            assert_eq!(text_positions(&output, "model.step").len(), 2);
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn file_headers_match_folder_styles_and_elide_long_paths_on_the_left() {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let ctx = egui::Context::default();
            set_theme(&ctx, theme);
            let mut app = loaded_app();
            let draw = |app: &mut VersusApp| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            egui::vec2(900.0, 650.0),
                        )),
                        ..Default::default()
                    },
                    |ui| app.render(ui),
                );
                output.textures_delta.clear();
                output
            };
            let label_style = |output: &egui::FullOutput, label: &str| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == label => Some((
                            text.fallback_color,
                            text.galley.job.sections[0].format.font_id.clone(),
                            text.pos.x,
                        )),
                        _ => None,
                    })
                    .unwrap()
            };
            let output = draw(&mut app);
            let folder_styles = [label_style(&output, "LEFT"), label_style(&output, "RIGHT")];
            output.drop_without_applying_deltas();
            let mut view = loaded_file_view();
            view.paths = [
                format!(
                    "C:\\engineering\\{}assembly\\model.step",
                    "é𐐀目录\\".repeat(60)
                )
                .into(),
                "/r/model.step".into(),
            ];
            app.file_view = Some(view);
            let output = draw(&mut app);
            for (side, label) in ["LEFT", "RIGHT"].into_iter().enumerate() {
                assert_eq!(label_style(&output, label), folder_styles[side]);
            }
            let left_path = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text().starts_with('…') => {
                        Some((shape, text))
                    }
                    _ => None,
                })
                .expect("Long path should show a leading ellipsis");
            let (shape, text) = left_path;
            assert!(text.galley.text().ends_with("assembly\\model.step"));
            assert_eq!(text.galley.rows.len(), 1);
            let bounds = Rect::from_min_size(text.pos, text.galley.size());
            assert!(bounds.right() < 450.0);
            assert!(shape.clip_rect.contains_rect(bounds));
            let path_y = bounds.center().y;
            for label in ["LEFT", "RIGHT", "/r/model.step"] {
                assert!((text_centers(&output, label)[0].y - path_y).abs() < 0.1);
            }
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn file_lines_are_aligned_and_share_difference_legend() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.file_view = Some(loaded_file_view());
        let output = render(&mut app, &ctx, vec![]);
        let identical = text_centers(&output, "unchanged");
        assert_eq!(identical.len(), 2);
        assert_eq!(identical[0].y, identical[1].y);
        assert_eq!(
            text_centers(&output, "old value")[0].y,
            text_centers(&output, "new value")[0].y
        );
        assert_eq!(
            text_centers(&output, "removed")[0].y,
            text_centers(&output, "—")[0].y
        );
        for label in [
            "Different  1",
            "Left only  1",
            "Right only  0",
            "Type mismatch  0",
            "Read error  0",
        ] {
            assert_eq!(text_positions(&output, label).len(), 1);
        }
        assert!(text_positions(&output, "Identical  1").is_empty());
        for shape in &output.shapes {
            if let egui::Shape::Text(text) = &shape.shape {
                if text.galley.text() == "unchanged" {
                    assert_eq!(text.fallback_color, Palette::new(true).muted);
                }
            }
        }
        assert!(status_icon(&DirectoryEntryState::Same).is_none());
        assert_eq!(
            state_color(&DirectoryEntryState::Same, Palette::new(true)),
            Palette::new(true).muted
        );
        output.drop_without_applying_deltas();
    }

    #[test]
    fn scrolling_either_file_pane_keeps_line_rows_aligned() {
        for side in 0..2 {
            let ctx = egui::Context::default();
            apply_theme(&ctx);
            let mut app = loaded_app();
            let mut view = loaded_file_view();
            view.comparison.as_mut().unwrap().rows = (1..=200)
                .map(|number| {
                    test_file_row(
                        Some((number, format!("line {number}"))),
                        Some((number, format!("line {number}"))),
                        DirectoryEntryState::Same,
                    )
                })
                .collect();
            update_visible_file_rows(&mut view, false);
            app.file_view = Some(view);
            render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
            let pointer = egui::pos2(if side == 0 { 300.0 } else { 900.0 }, 300.0);
            let output = render(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(pointer),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: egui::vec2(0.0, -180.0),
                        modifiers: Default::default(),
                    },
                ],
            );
            let assert_aligned = |output: &egui::FullOutput| {
                for number in 1..=200 {
                    let positions = text_positions(output, &format!("line {number}"));
                    if !positions.is_empty() {
                        assert_eq!(
                            positions.len(),
                            2,
                            "Line {number} should appear in both panes"
                        );
                        assert!(
                            (positions[0].y - positions[1].y).abs() < 0.1,
                            "Line {number} is misaligned: {positions:?}"
                        );
                    }
                }
            };
            assert_aligned(&output);
            output.drop_without_applying_deltas();
            for _ in 0..20 {
                let output = render(&mut app, &ctx, vec![]);
                assert_aligned(&output);
                output.drop_without_applying_deltas();
            }
            assert!(app.file_view.as_ref().unwrap().scroll_y > 0.0);
        }
    }

    #[test]
    fn back_button_sits_to_the_left_of_the_legend() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.file_view = Some(loaded_file_view());
        let output = render(&mut app, &ctx, vec![]);
        let back = ctx
            .read_response(egui::Id::new("Back to folders"))
            .unwrap()
            .rect
            .center();
        let legend = text_centers(&output, "Different  1")[0];
        let title = text_centers(&output, "Versus")[0];
        assert!(back.x < legend.x);
        assert!((back.y - legend.y).abs() < 1.0);
        assert!(back.y > title.y);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn theme_toggle_updates_both_views_without_losing_state() {
        for file_mode in [false, true] {
            let ctx = egui::Context::default();
            apply_theme(&ctx);
            let mut app = loaded_app();
            app.tree.as_mut().unwrap().expand_all();
            app.selected = Some("assembly/model.step".into());
            if file_mode {
                app.file_view = Some(loaded_file_view());
            }
            let generation = app.scroll_generation;
            for (action, expected_dark) in [
                ("Switch to light mode", false),
                ("Switch to dark mode", true),
            ] {
                let output = render(&mut app, &ctx, vec![]);
                let toggle = ctx.read_response(egui::Id::new(action)).unwrap().rect;
                assert!(
                    toggle.right() <= 1190.0 && toggle.left() > 1100.0 && toggle.bottom() < 45.0
                );
                assert!(
                    text_positions(&output, action).is_empty(),
                    "Theme action must use only an icon"
                );
                output.drop_without_applying_deltas();
                click(&mut app, &ctx, toggle.center());
                let output = render(&mut app, &ctx, vec![]);
                assert_eq!(ctx.theme() == egui::Theme::Dark, expected_dark);
                assert_eq!(app.logo_texture.as_ref().unwrap().0, expected_dark);
                assert_eq!(app.file_view.is_some(), file_mode);
                assert!(app.tree.as_ref().unwrap().is_expanded("assembly"));
                assert_eq!(
                    app.selected.as_deref(),
                    Some(std::path::Path::new("assembly/model.step"))
                );
                assert_eq!(app.scroll_generation, generation);
                let neutral = if file_mode { "unchanged" } else { "model.step" };
                let palette = Palette::new(expected_dark);
                for shape in &output.shapes {
                    if let egui::Shape::Text(text) = &shape.shape {
                        if text.galley.text() == neutral {
                            assert_eq!(text.fallback_color, palette.muted);
                        }
                    }
                }
                output.drop_without_applying_deltas();
            }
        }
    }

    #[test]
    fn logo_texture_respects_transparency_of_stray_and_edge_pixels() {
        let ctx = egui::Context::default();
        let mut app = loaded_app();
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            set_theme(&ctx, theme);
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                    ..Default::default()
                },
                |ui| app.render(ui),
            );
            let logo_id = app.logo_texture.as_ref().unwrap().1.id();
            let delta = &output.textures_delta.set.get(&logo_id).unwrap()[0];
            let egui::ImageData::Color(image) = &delta.image;
            let speck = image[(91, 74)];
            assert_eq!(speck.a(), 1);
            assert!(
                speck.r() <= 1 && speck.g() <= 1 && speck.b() <= 1,
                "Nearly transparent pixels must not become bright white"
            );
            for pixel in &image.pixels {
                assert!(
                    pixel.r() <= pixel.a() && pixel.g() <= pixel.a() && pixel.b() <= pixel.a(),
                    "Logo texture must use premultiplied alpha"
                );
            }
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn theme_changes_send_matching_icon_to_native_window() {
        let ctx = egui::Context::default();
        let mut app = loaded_app();
        for theme in [egui::Theme::Dark, egui::Theme::Light, egui::Theme::Dark] {
            set_theme(&ctx, theme);
            let output = render(&mut app, &ctx, vec![]);
            let expected = crate::logo::themed_icon(theme == egui::Theme::Dark);
            assert!(output.viewport_output.values().flat_map(|viewport| &viewport.commands).any(|command| {
                matches!(command,egui::ViewportCommand::Icon(Some(icon)) if icon.rgba == expected.rgba)
            }), "Native window must receive the matching theme icon");
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn file_type_mismatch_opens_explanation_without_loading_nonfile_side() {
        let mut app = loaded_app();
        app.open_file("assembly/model.step".into(), [true, true], true);
        let view = app.file_view.as_ref().unwrap();
        assert!(view.job.is_none());
        assert!(
            view.error
                .as_deref()
                .unwrap()
                .contains("Entry types differ")
        );
        assert!(matches!(view.error_icon, StatusIcon::TypeMismatch));
    }

    #[test]
    fn file_worker_errors_remain_in_file_view_with_back_available() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        let (sender, receiver) = mpsc::channel();
        let mut view = loaded_file_view();
        view.comparison = None;
        view.job = Some(FileJob {
            receiver,
            cancellation: Arc::new(AtomicBool::new(false)),
            started: Instant::now(),
            progress: Arc::new(ComparisonProgress::default()),
        });
        app.file_view = Some(view);
        sender
            .send(Err(CompareError {
                path: None,
                kind: versus::CompareErrorKind::InvalidPath,
                message: "file unavailable".into(),
            }))
            .unwrap();
        app.poll_file_comparison(&ctx);
        let output = render(&mut app, &ctx, vec![]);
        assert!(
            ctx.read_response(egui::Id::new("Back to folders"))
                .is_some()
        );
        assert!(
            app.file_view
                .as_ref()
                .unwrap()
                .error
                .as_deref()
                .unwrap()
                .contains("file unavailable")
        );
        assert!(app.tree.as_ref().is_some());
        output.drop_without_applying_deltas();
    }

    fn attach_job(app: &mut VersusApp) -> mpsc::Sender<Result<FolderTree, CompareError>> {
        let (sender, receiver) = mpsc::channel();
        app.job = Some(ComparisonJob {
            receiver,
            cancellation: Arc::new(AtomicBool::new(false)),
            roots: ["new-left".into(), "new-right".into()],
            started: Instant::now(),
            progress: Arc::new(ComparisonProgress::default()),
        });
        sender
    }

    #[test]
    fn changing_paths_cancels_scan_and_clears_prior_results() {
        let mut app = loaded_app();
        let sender = attach_job(&mut app);
        let flag = app.job.as_ref().unwrap().cancellation.clone();
        app.invalidate();
        assert!(flag.load(Ordering::Relaxed));
        assert!(app.job.is_none() && app.tree.is_none() && app.roots.is_none());
        assert!(sender.send(Ok(fixture())).is_err());
    }

    #[test]
    fn cancelled_completed_job_does_not_replace_last_comparison() {
        let mut app = loaded_app();
        let sender = attach_job(&mut app);
        sender.send(Ok(fixture())).unwrap();
        app.job
            .as_ref()
            .unwrap()
            .cancellation
            .store(true, Ordering::Relaxed);
        app.poll_comparison(&egui::Context::default());
        assert!(app.job.is_none());
        assert_eq!(
            app.roots.as_ref().unwrap(),
            &[PathBuf::from("left"), PathBuf::from("right")]
        );
        assert_eq!(app.message, "Comparison cancelled.");
    }

    #[test]
    fn failed_worker_reports_error_and_keeps_last_comparison() {
        let mut app = loaded_app();
        drop(attach_job(&mut app));
        app.poll_comparison(&egui::Context::default());
        assert!(app.error.as_ref().unwrap().contains("worker stopped"));
        assert!(app.tree.is_some());
        assert_eq!(app.roots.as_ref().unwrap()[0], PathBuf::from("left"));
    }

    struct SourceFixture(PathBuf);

    impl SourceFixture {
        fn new() -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!("versus-source-ui-{unique}"));
            std::fs::create_dir_all(root.join("left")).unwrap();
            std::fs::create_dir_all(root.join("right")).unwrap();
            std::fs::write(root.join("left/model.txt"), "common\nold\n").unwrap();
            std::fs::write(root.join("right/model.txt"), "common\nnew\n").unwrap();
            Self(root)
        }
        fn path(&self, relative: &str) -> PathBuf {
            self.0.join(relative)
        }
    }

    impl Drop for SourceFixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn settle_sources(app: &mut VersusApp, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            app.poll_sources(ctx);
            app.poll_comparison(ctx);
            app.poll_file_comparison(ctx);
            if app.source_jobs.iter().all(Option::is_none)
                && app.job.is_none()
                && app.file_view.as_ref().is_none_or(|view| view.job.is_none())
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Comparison worker did not complete"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn command_line_files_open_direct_comparison_in_supplied_order_without_writes() {
        let sources = SourceFixture::new();
        let paths = [
            sources.path("left/model.txt"),
            sources.path("right/model.txt"),
        ];
        for mode in [None, Some(ComparisonMode::File)] {
            let ctx = egui::Context::default();
            apply_theme(&ctx);
            let mut app = VersusApp::default();
            app.open_launch_request(crate::cli::LaunchRequest {
                paths: paths.clone(),
                mode,
            });
            settle_sources(&mut app, &ctx);
            assert_eq!(app.comparison_mode, ComparisonMode::File);
            let view = app.file_view.as_ref().unwrap();
            assert_eq!(view.paths, paths);
            assert_eq!(view.sources, paths.clone().map(Some));
            assert!(!view.from_folders);
            assert_eq!(view.counts[1], 1);
            let output = render(&mut app, &ctx, vec![]);
            assert_eq!(text_positions(&output, "old").len(), 1);
            assert_eq!(text_positions(&output, "new").len(), 1);
            assert!(
                ctx.read_response(egui::Id::new("Back to folders"))
                    .is_none()
            );
            output.drop_without_applying_deltas();
        }
        assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "common\nold\n");
        assert_eq!(std::fs::read_to_string(&paths[1]).unwrap(), "common\nnew\n");
    }

    #[test]
    fn command_line_folders_choose_folder_mode_and_reject_forced_file_mode() {
        let sources = SourceFixture::new();
        let ctx = egui::Context::default();
        let paths = [sources.path("left"), sources.path("right")];
        for mode in [None, Some(ComparisonMode::Folder)] {
            let mut app = VersusApp::default();
            app.open_launch_request(crate::cli::LaunchRequest {
                paths: paths.clone(),
                mode,
            });
            settle_sources(&mut app, &ctx);
            assert_eq!(app.comparison_mode, ComparisonMode::Folder);
            assert_eq!(app.roots, Some(paths.clone()));
            assert!(app.tree.is_some() && app.file_view.is_none());
        }
        for (mode, paths) in [
            (ComparisonMode::File, paths),
            (
                ComparisonMode::Folder,
                [
                    sources.path("left/model.txt"),
                    sources.path("right/model.txt"),
                ],
            ),
        ] {
            let mut app = VersusApp::default();
            app.open_launch_request(crate::cli::LaunchRequest {
                paths,
                mode: Some(mode),
            });
            settle_sources(&mut app, &ctx);
            assert!(app.error.as_ref().unwrap().contains("requires two"));
            assert!(app.tree.is_none() && app.file_view.is_none());
        }
    }

    #[test]
    fn command_line_invalid_sources_show_errors_and_new_discards_pending_launch() {
        let sources = SourceFixture::new();
        let ctx = egui::Context::default();
        for paths in [
            [sources.path("left/model.txt"), sources.path("missing")],
            [sources.path("left/model.txt"), sources.path("right")],
        ] {
            let mut app = VersusApp::default();
            app.open_launch_request(crate::cli::LaunchRequest { paths, mode: None });
            settle_sources(&mut app, &ctx);
            assert!(app.error.is_some() || app.mode() == SelectionMode::Incompatible);
            assert!(app.tree.is_none() && app.file_view.is_none());
        }
        let mut app = VersusApp::default();
        app.open_launch_request(crate::cli::LaunchRequest {
            paths: [
                sources.path("left/model.txt"),
                sources.path("right/model.txt"),
            ],
            mode: None,
        });
        app.new_comparison();
        settle_sources(&mut app, &ctx);
        assert!(app.paths.iter().all(String::is_empty));
        assert!(app.source_paths.iter().all(Option::is_none));
        assert!(app.launch_mode.is_none());
        assert!(app.tree.is_none() && app.file_view.is_none());
    }

    #[test]
    fn git_null_device_inputs_show_added_and_deleted_lines_without_opening_devices() {
        let sources = SourceFixture::new();
        let ctx = egui::Context::default();
        let mut nulls = vec![PathBuf::from("/dev/null")];
        if cfg!(windows) {
            nulls.extend([PathBuf::from("NUL"), PathBuf::from("nul")]);
        }
        for null in nulls {
            for empty_side in 0..2 {
                let mut paths = [
                    sources.path("left/model.txt"),
                    sources.path("right/model.txt"),
                ];
                paths[empty_side] = null.clone();
                let mut app = VersusApp::default();
                app.open_launch_request(crate::cli::LaunchRequest {
                    paths,
                    mode: Some(ComparisonMode::File),
                });
                settle_sources(&mut app, &ctx);
                let view = app.file_view.as_ref().unwrap();
                assert!(view.error.is_none());
                assert!(view.sources[empty_side].is_none());
                let expected = if empty_side == 0 {
                    DirectoryEntryState::RightOnly
                } else {
                    DirectoryEntryState::LeftOnly
                };
                let rows = &view.comparison.as_ref().unwrap().rows;
                assert_eq!(rows.len(), 2);
                assert!(rows.iter().all(|row| row.state == expected));
            }
        }
    }

    #[test]
    fn command_line_native_paths_survive_display_conversion() {
        #[cfg(target_os = "linux")]
        use std::os::unix::ffi::OsStringExt;
        let sources = SourceFixture::new();
        let ctx = egui::Context::default();
        // Linux permits non-UTF-8 filenames; macOS filesystems require Unicode.
        #[cfg(target_os = "linux")]
        let left_name = std::ffi::OsString::from_vec(b"left \xff.txt".to_vec());
        #[cfg(not(target_os = "linux"))]
        let left_name = std::ffi::OsString::from("left space é.txt");
        let left = sources.0.join(left_name);
        let right = sources.path("right space \u{6bd4}\u{8f03}.txt");
        std::fs::write(&left, "before\n").unwrap();
        std::fs::write(&right, "after\n").unwrap();
        let mut app = VersusApp::default();
        app.open_launch_request(crate::cli::LaunchRequest {
            paths: [left.clone(), right.clone()],
            mode: Some(ComparisonMode::File),
        });
        settle_sources(&mut app, &ctx);
        let view = app.file_view.as_ref().unwrap();
        assert_eq!(view.sources, [Some(left), Some(right)]);
        assert!(view.error.is_none());
        assert_eq!(view.counts[1], 1);
    }

    #[test]
    fn browse_results_replace_folder_state_and_open_the_file_workspace() {
        let sources = SourceFixture::new();
        for (side, path_area) in [(0, false), (1, true)] {
            let ctx = egui::Context::default();
            let mut app = loaded_app();
            app.comparison_mode = ComparisonMode::File;
            let picked = sources.path(if side == 0 {
                "left/model.txt"
            } else {
                "right/model.txt"
            });
            let mut requests = Vec::new();
            {
                let mut draw = |events| {
                    ctx.run_ui(
                        egui::RawInput {
                            events,
                            ..Default::default()
                        },
                        |ui| {
                            app.folder_inputs_with_picker(ui, |requested_side, _, _| {
                                requests.push(requested_side);
                                Some(picked.clone())
                            })
                        },
                    )
                };
                let output = draw(vec![]);
                let target = if path_area {
                    text_centers(&output, "Choose a file")[side]
                } else {
                    ctx.read_response(egui::Id::new("Browse left file"))
                        .unwrap()
                        .rect
                        .center()
                };
                output.drop_without_applying_deltas();
                for pressed in [true, false] {
                    draw(vec![
                        egui::Event::PointerMoved(target),
                        egui::Event::PointerButton {
                            pos: target,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ])
                    .drop_without_applying_deltas();
                }
            }
            assert_eq!(requests, vec![side]);
            assert_eq!(app.paths[side], picked.display().to_string());
            assert!(app.tree.is_none() && app.roots.is_none());
            settle_sources(&mut app, &ctx);
            assert_eq!(app.mode(), SelectionMode::File);
            assert!(!app.file_view.as_ref().unwrap().from_folders);
        }
    }

    #[test]
    fn selected_sources_switch_views_and_compare_only_matching_types() {
        let sources = SourceFixture::new();
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = VersusApp::default();
        app.switch_comparison_mode(ComparisonMode::File);
        // Starting on the right must work just as starting on the left does.
        app.select_path(1, sources.path("right/model.txt"));
        settle_sources(&mut app, &ctx);
        assert_eq!(app.mode(), SelectionMode::File);
        let view = app.file_view.as_ref().unwrap();
        assert!(!view.from_folders && view.comparison.is_none() && view.job.is_none());
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(text_positions(&output, "File Compare").len(), 1);
        assert!(
            ctx.read_response(egui::Id::new("Back to folders"))
                .is_none()
        );
        assert!(
            ctx.read_response(egui::Id::new("Browse left file"))
                .is_some()
        );
        assert!(
            ctx.read_response(egui::Id::new("Browse right file"))
                .is_some()
        );
        output.drop_without_applying_deltas();
        app.select_path(0, sources.path("left/model.txt"));
        settle_sources(&mut app, &ctx);
        assert!(app.file_view.as_ref().unwrap().comparison.is_some());
        assert_eq!(app.file_view.as_ref().unwrap().counts[1], 1);
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(text_positions(&output, "old").len(), 1);
        assert_eq!(text_positions(&output, "new").len(), 1);
        assert!(
            ctx.read_response(egui::Id::new("Back to folders"))
                .is_none()
        );
        output.drop_without_applying_deltas();
        // Replacing the right file with a folder rejects the mixed pair.
        app.select_path(1, sources.path("right"));
        settle_sources(&mut app, &ctx);
        assert_eq!(app.mode(), SelectionMode::Incompatible);
        assert!(app.file_view.is_none() && app.tree.is_none() && app.job.is_none());
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(
            text_positions(&output, "Incompatible source types").len(),
            1
        );
        assert!(text_positions(&output, &app.message).len() >= 1);
        assert!(
            app.message
                .contains("No comparison can be made between a file and a folder")
        );
        output.drop_without_applying_deltas();
        // Replacing the left side with a folder restores automatic comparison.
        app.select_path(0, sources.path("left"));
        settle_sources(&mut app, &ctx);
        assert_eq!(app.mode(), SelectionMode::Folder);
        assert!(app.file_view.is_none() && app.tree.is_some());
        assert_eq!(app.counts[1], 1);
    }

    #[test]
    fn first_folder_selection_opens_folder_workspace_without_comparing() {
        let sources = SourceFixture::new();
        let ctx = egui::Context::default();
        let mut app = VersusApp::default();
        app.select_path(0, sources.path("left"));
        settle_sources(&mut app, &ctx);
        assert_eq!(app.mode(), SelectionMode::Folder);
        assert!(app.tree.is_none() && app.job.is_none() && app.file_view.is_none());
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(text_positions(&output, "Folder Compare").len(), 1);
        assert_eq!(text_positions(&output, "Choose another folder").len(), 1);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn comparison_mode_buttons_stay_centered_with_icons_in_both_themes() {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            for (width, height) in [(900.0, 650.0), (1200.0, 800.0)] {
                for state in 0..4 {
                    let ctx = egui::Context::default();
                    set_theme(&ctx, theme);
                    let mut app = if state == 0 {
                        VersusApp::default()
                    } else {
                        loaded_app()
                    };
                    let _sender = (state == 2).then(|| attach_job(&mut app));
                    if state == 3 {
                        app.comparison_mode = ComparisonMode::File;
                        let mut view = loaded_file_view();
                        view.from_folders = false;
                        app.file_view = Some(view);
                    }
                    let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(width, height));
                    let output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(screen),
                            ..Default::default()
                        },
                        |ui| app.render(ui),
                    );
                    let texture = app.logo_texture.as_ref().unwrap().1.id();
                    let logo = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Rect(rect) if rect.fill_texture_id() == texture => {
                                Some(rect.rect)
                            }
                            _ => None,
                        })
                        .unwrap();
                    let folder = ctx
                        .read_response(egui::Id::new("Folder Compare"))
                        .unwrap()
                        .rect;
                    let file = ctx
                        .read_response(egui::Id::new("File Compare"))
                        .unwrap()
                        .rect;
                    assert!((folder.union(file).center().x - screen.center().x).abs() < 0.5);
                    assert!(logo.right() < folder.left());
                    let theme_button = ctx
                        .read_response(egui::Id::new(if theme == egui::Theme::Dark {
                            "Switch to light mode"
                        } else {
                            "Switch to dark mode"
                        }))
                        .unwrap()
                        .rect;
                    assert!(file.right() < theme_button.left());
                    for (label, rect) in [("Folder Compare", folder), ("File Compare", file)] {
                        assert!(screen.contains_rect(rect));
                        let text = output
                            .shapes
                            .iter()
                            .find_map(|shape| match &shape.shape {
                                egui::Shape::Text(text) if text.galley.text() == label => {
                                    Some(text)
                                }
                                _ => None,
                            })
                            .unwrap();
                        let bounds = Rect::from_min_size(text.pos, text.galley.size());
                        assert!((bounds.center().y - logo.center().y).abs() < 1.0);
                        assert!(rect.contains_rect(bounds));
                        let icon_center = egui::pos2(
                            rect.left() + ctx.style_of(theme).spacing.button_padding.x + 8.0,
                            rect.center().y,
                        );
                        let icon = Rect::from_center_size(icon_center, egui::vec2(16.0, 16.0));
                        assert!(icon.right() < bounds.left());
                        assert!(
                            output.shapes.iter().any(|shape| match &shape.shape {
                                egui::Shape::Rect(shape) if label == "Folder Compare" =>
                                    icon.contains_rect(shape.rect)
                                        && shape.rect.width() == 13.0
                                        && shape.stroke.width > 0.0,
                                egui::Shape::Path(shape) if label == "File Compare" =>
                                    shape.closed
                                        && shape.points.len() == 5
                                        && shape.points.iter().all(|point| icon.contains(*point)),
                                _ => false,
                            }),
                            "{label} must have a visible icon"
                        );
                    }
                    output.drop_without_applying_deltas();
                }
            }
        }
    }

    #[test]
    fn compare_icons_are_clickable_and_mode_buttons_support_keyboard_activation() {
        let ctx = egui::Context::default();
        let mut app = VersusApp::default();
        render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
        let file = ctx
            .read_response(egui::Id::new("File Compare"))
            .unwrap()
            .rect;
        let icon = egui::pos2(file.left() + 12.0, file.center().y);
        let output = render(&mut app, &ctx, vec![egui::Event::PointerMoved(icon)]);
        assert_eq!(
            output.platform_output.cursor_icon,
            egui::CursorIcon::PointingHand
        );
        output.drop_without_applying_deltas();
        click(&mut app, &ctx, icon);
        assert_eq!(app.comparison_mode, ComparisonMode::File);
        for (key, mode) in [
            (egui::Key::Enter, ComparisonMode::Folder),
            (egui::Key::Space, ComparisonMode::File),
        ] {
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::new(mode.label())));
            render(
                &mut app,
                &ctx,
                vec![egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
            )
            .drop_without_applying_deltas();
            assert_eq!(app.comparison_mode, mode);
        }
    }

    #[test]
    fn new_comparison_button_clears_both_sides_and_preserves_theme() {
        let ctx = egui::Context::default();
        set_theme(&ctx, egui::Theme::Light);
        let mut app = VersusApp::default();
        app.paths = ["/left/model.step".into(), "/right/model.step".into()];
        app.source_kinds = [Some(SourceKind::File); 2];
        let mut view = loaded_file_view();
        view.from_folders = false;
        app.file_view = Some(view);
        let output = render(&mut app, &ctx, vec![]);
        let new = ctx
            .read_response(egui::Id::new("New comparison"))
            .unwrap()
            .rect;
        let legend = text_centers(&output, "Different  1")[0];
        assert!(new.center().x > legend.x);
        assert!((new.center().y - legend.y).abs() < 6.0);
        output.drop_without_applying_deltas();
        click(&mut app, &ctx, new.center());
        assert_eq!(app.paths, [String::new(), String::new()]);
        assert_eq!(app.mode(), SelectionMode::Empty);
        assert!(app.file_view.is_none() && app.tree.is_none() && app.roots.is_none());
        assert_eq!(ctx.theme(), egui::Theme::Light);
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(text_positions(&output, "File Compare").len(), 1);
        assert_eq!(text_positions(&output, "Choose a folder").len(), 2);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn new_comparison_cancels_all_jobs_and_discards_late_results() {
        let mut app = loaded_app();
        let folder_sender = attach_job(&mut app);
        let folder_flag = app.job.as_ref().unwrap().cancellation.clone();
        let (file_sender, file_receiver) = mpsc::channel();
        let file_flag = Arc::new(AtomicBool::new(false));
        let mut view = loaded_file_view();
        view.job = Some(FileJob {
            receiver: file_receiver,
            cancellation: file_flag.clone(),
            started: Instant::now(),
            progress: Arc::new(ComparisonProgress::default()),
        });
        app.file_view = Some(view);
        let (source_sender, source_receiver) = mpsc::channel();
        let source_flag = Arc::new(AtomicBool::new(false));
        app.source_jobs[0] = Some(SourceJob {
            receiver: source_receiver,
            cancellation: source_flag.clone(),
        });
        app.source_errors[1] = Some("old error".into());
        app.paths = ["/old/left".into(), "/old/right".into()];
        app.source_kinds = [Some(SourceKind::Folder); 2];
        app.new_comparison();
        for flag in [folder_flag, file_flag, source_flag] {
            assert!(flag.load(Ordering::Relaxed));
        }
        assert!(folder_sender.send(Ok(fixture())).is_err());
        assert!(file_sender.send(Ok(None)).is_err());
        assert!(source_sender.send(Ok(SourceKind::File)).is_err());
        settle_sources(&mut app, &egui::Context::default());
        assert_eq!(app.mode(), SelectionMode::Empty);
        assert_eq!(app.source_kinds, [None; 2]);
        assert!(app.source_errors.iter().all(Option::is_none));
        assert!(app.error.is_none());
    }

    #[test]
    fn replacing_a_source_discards_pending_classification_and_prior_results() {
        let sources = SourceFixture::new();
        let mut app = loaded_app();
        let (sender, receiver) = mpsc::channel();
        let cancellation = Arc::new(AtomicBool::new(false));
        app.source_jobs[0] = Some(SourceJob {
            receiver,
            cancellation: cancellation.clone(),
        });
        app.select_path(0, sources.path("left/model.txt"));
        assert!(cancellation.load(Ordering::Relaxed));
        assert!(sender.send(Ok(SourceKind::Folder)).is_err());
        assert!(app.tree.is_none() && app.roots.is_none());
        settle_sources(&mut app, &egui::Context::default());
        assert_eq!(app.source_kinds[0], Some(SourceKind::File));
        assert_eq!(app.mode(), SelectionMode::File);
    }

    #[test]
    fn unavailable_selected_source_shows_error_and_keeps_browse_available() {
        let sources = SourceFixture::new();
        let ctx = egui::Context::default();
        let mut app = loaded_app();
        app.select_path(0, sources.path("missing"));
        settle_sources(&mut app, &ctx);
        assert!(app.error.as_deref().unwrap().contains("Cannot inspect"));
        assert!(app.tree.is_none() && app.file_view.is_none());
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(
            text_positions(&output, "Cannot open selected source").len(),
            1
        );
        assert!(
            ctx.read_response(egui::Id::new("Browse left folder"))
                .is_some()
        );
        output.drop_without_applying_deltas();
        app.select_path(0, sources.path("left"));
        settle_sources(&mut app, &ctx);
        assert!(app.error.is_none());
        assert_eq!(app.mode(), SelectionMode::Folder);
    }
    #[test]
    fn mode_buttons_switch_the_workspace_and_route_browse_directly() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.paths = ["/left/folder".into(), "/right/folder".into()];
        let sender = attach_job(&mut app);
        let cancellation = app.job.as_ref().unwrap().cancellation.clone();
        let output = render(&mut app, &ctx, vec![]);
        let file_button = text_centers(&output, "File Compare")[0];
        output.drop_without_applying_deltas();
        click(&mut app, &ctx, file_button);
        assert_eq!(app.comparison_mode, ComparisonMode::File);
        assert!(app.paths.iter().all(String::is_empty));
        assert!(app.tree.is_none() && app.job.is_none() && app.file_view.is_none());
        assert!(cancellation.load(Ordering::Relaxed));
        assert!(sender.send(Ok(fixture())).is_err());
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(text_positions(&output, "Choose a file").len(), 2);
        assert_eq!(text_positions(&output, "Choose files to compare").len(), 1);
        output.drop_without_applying_deltas();

        // Exercise both the path section and browse icon for both workflows.
        for mode in [ComparisonMode::File, ComparisonMode::Folder] {
            let output = render(&mut app, &ctx, vec![]);
            let button = text_centers(&output, mode.label())[0];
            output.drop_without_applying_deltas();
            click(&mut app, &ctx, button);
            assert_eq!(app.comparison_mode, mode);
            let mut requests = Vec::new();
            {
                let mut draw = |events| {
                    ctx.run_ui(
                        egui::RawInput {
                            events,
                            ..Default::default()
                        },
                        |ui| {
                            app.folder_inputs_with_picker(ui, |side, _, picker_mode| {
                                requests.push((side, picker_mode));
                                None
                            })
                        },
                    )
                };
                for (side, path_area) in [(0, true), (1, false)] {
                    let output = draw(vec![]);
                    let target = if path_area {
                        text_centers(&output, &format!("Choose a {}", mode.source_label()))[side]
                    } else {
                        ctx.read_response(egui::Id::new(format!(
                            "Browse right {}",
                            mode.source_label()
                        )))
                        .unwrap()
                        .rect
                        .center()
                    };
                    output.drop_without_applying_deltas();
                    for pressed in [true, false] {
                        draw(vec![
                            egui::Event::PointerMoved(target),
                            egui::Event::PointerButton {
                                pos: target,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            },
                        ])
                        .drop_without_applying_deltas();
                    }
                }
            }
            assert_eq!(requests, vec![(0, mode), (1, mode)]);
        }
    }

    #[test]
    fn active_mode_button_preserves_comparison_and_new_preserves_file_mode() {
        let sources = SourceFixture::new();
        let ctx = egui::Context::default();
        set_theme(&ctx, egui::Theme::Light);
        let mut app = VersusApp::default();
        app.switch_comparison_mode(ComparisonMode::File);
        app.select_path(0, sources.path("left/model.txt"));
        app.select_path(1, sources.path("right/model.txt"));
        settle_sources(&mut app, &ctx);
        let paths = app.paths.clone();
        let output = render(&mut app, &ctx, vec![]);
        let file_button = text_centers(&output, "File Compare")[0];
        output.drop_without_applying_deltas();
        click(&mut app, &ctx, file_button);
        assert_eq!(app.paths, paths);
        assert!(app.file_view.as_ref().unwrap().comparison.is_some());
        let output = render(&mut app, &ctx, vec![]);
        let new = ctx
            .read_response(egui::Id::new("New comparison"))
            .unwrap()
            .rect
            .center();
        output.drop_without_applying_deltas();
        click(&mut app, &ctx, new);
        assert_eq!(app.comparison_mode, ComparisonMode::File);
        assert!(app.paths.iter().all(String::is_empty));
        assert_eq!(ctx.theme(), egui::Theme::Light);
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(text_positions(&output, "Choose a file").len(), 2);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn folder_mode_button_returns_from_file_drilldown_without_clearing_tree() {
        let ctx = egui::Context::default();
        let mut app = loaded_app();
        app.tree.as_mut().unwrap().expand_all();
        app.selected = Some("assembly/model.step".into());
        app.file_view = Some(loaded_file_view());
        let generation = app.scroll_generation;
        let output = render(&mut app, &ctx, vec![]);
        let folder_button = text_centers(&output, "Folder Compare")[0];
        output.drop_without_applying_deltas();
        click(&mut app, &ctx, folder_button);
        assert!(app.file_view.is_none());
        assert!(app.tree.as_ref().unwrap().is_expanded("assembly"));
        assert_eq!(app.selected, Some("assembly/model.step".into()));
        assert_eq!(app.scroll_generation, generation);
    }

    fn click_action(app: &mut VersusApp, ctx: &egui::Context, label: &str) {
        render(app, ctx, vec![]).drop_without_applying_deltas();
        let response = ctx.read_response(egui::Id::new(label)).unwrap();
        assert!(response.enabled(), "{label} should be enabled");
        click(app, ctx, response.rect.center());
    }

    #[test]
    fn differences_filter_hides_equal_lines_and_preserves_original_numbers() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.file_view = Some(loaded_file_view());
        click_action(&mut app, &ctx, "Show only differences");
        let output = render(&mut app, &ctx, vec![]);
        assert!(text_positions(&output, "unchanged").is_empty());
        assert!(text_positions(&output, "1").is_empty());
        assert_eq!(text_positions(&output, "2").len(), 2);
        assert_eq!(text_positions(&output, "3").len(), 1);
        let left = text_positions(&output, "old value")[0];
        let right = text_positions(&output, "new value")[0];
        assert!((left.y - right.y).abs() < 0.1);
        output.drop_without_applying_deltas();
        click_action(&mut app, &ctx, "Show only differences");
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(text_positions(&output, "unchanged").len(), 2);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn navigation_scrolls_both_file_panes_and_advances_when_rows_fit_the_window() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        let mut view = loaded_file_view();
        view.comparison.as_mut().unwrap().rows = (1..=200)
            .map(|number| {
                let changed = [20, 80, 150].contains(&number);
                test_file_row(
                    Some((number, format!("left line {number}"))),
                    Some((number, format!("right line {number}"))),
                    if changed {
                        DirectoryEntryState::Different
                    } else {
                        DirectoryEntryState::Same
                    },
                )
            })
            .collect();
        update_visible_file_rows(&mut view, false);
        app.file_view = Some(view);
        for (action, number) in [
            ("Next difference", 20),
            ("Next difference", 80),
            ("Previous difference", 20),
        ] {
            click_action(&mut app, &ctx, action);
            let output = render(&mut app, &ctx, vec![]);
            let left = text_positions(&output, &format!("left line {number}"))[0];
            let right = text_positions(&output, &format!("right line {number}"))[0];
            assert!((left.y - right.y).abs() < 0.1);
            assert_eq!(
                app.file_view.as_ref().unwrap().navigation_row,
                Some(number - 1)
            );
            output.drop_without_applying_deltas();
        }
        app.file_view = Some(loaded_file_view());
        for (action, index) in [
            ("Next difference", 1),
            ("Next difference", 2),
            ("Previous difference", 1),
        ] {
            click_action(&mut app, &ctx, action);
            assert_eq!(app.file_view.as_ref().unwrap().navigation_row, Some(index));
        }
    }

    #[test]
    fn folder_navigation_reveals_collapsed_parents_in_both_panes() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut entries = Vec::new();
        for index in 0..100 {
            entries.push(DirectoryEntry {
                relative_path: format!("a_common_{index:03}").into(),
                left_exists: true,
                right_exists: true,
                left_kind: Some(DirectoryEntryKind::Directory),
                right_kind: Some(DirectoryEntryKind::Directory),
                left_size: Some(0),
                right_size: Some(0),
                kind: DirectoryEntryKind::Directory,
                state: DirectoryEntryState::Same,
            });
        }
        entries.push(DirectoryEntry {
            relative_path: "z_changed/deep/changed.txt".into(),
            left_exists: true,
            right_exists: true,
            left_kind: Some(DirectoryEntryKind::File),
            right_kind: Some(DirectoryEntryKind::File),
            left_size: Some(1),
            right_size: Some(1),
            kind: DirectoryEntryKind::File,
            state: DirectoryEntryState::Different,
        });
        let tree = FolderTree::from_diff(&DirectoryDiff {
            entries,
            cancelled: false,
        });
        let mut app = VersusApp::default();
        let sender = attach_job(&mut app);
        sender.send(Ok(tree)).unwrap();
        app.poll_comparison(&ctx);
        for path in ["z_changed", "z_changed/deep", "z_changed/deep/changed.txt"] {
            click_action(&mut app, &ctx, "Next difference");
            assert_eq!(app.selected.as_deref(), Some(std::path::Path::new(path)));
        }
        assert!(app.tree.as_ref().unwrap().is_expanded("z_changed"));
        assert!(app.tree.as_ref().unwrap().is_expanded("z_changed/deep"));
        let output = render(&mut app, &ctx, vec![]);
        let rows = text_positions(&output, "changed.txt");
        assert_eq!(rows.len(), 2);
        assert!((rows[0].y - rows[1].y).abs() < 0.1);
        output.drop_without_applying_deltas();
        click_action(&mut app, &ctx, "Show only differences");
        let output = render(&mut app, &ctx, vec![]);
        assert!(text_positions(&output, "a_common_000").is_empty());
        assert_eq!(text_positions(&output, "changed.txt").len(), 2);
        output.drop_without_applying_deltas();
        click_action(&mut app, &ctx, "Previous difference");
        assert_eq!(
            app.selected.as_deref(),
            Some(std::path::Path::new("z_changed/deep"))
        );
    }

    #[test]
    fn ignore_buttons_recompare_file_and_folder_results_and_show_ending_changes() {
        let sources = SourceFixture::new();
        std::fs::write(sources.path("left/model.txt"), "let value = 1;\r\n").unwrap();
        std::fs::write(sources.path("right/model.txt"), "letvalue=1;\n").unwrap();
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = VersusApp::default();
        app.open_launch_request(crate::cli::LaunchRequest {
            paths: [sources.path("left"), sources.path("right")],
            mode: None,
        });
        settle_sources(&mut app, &ctx);
        assert_eq!(app.counts[1], 1);
        app.open_file("model.txt".into(), [true, true], false);
        settle_sources(&mut app, &ctx);
        click_action(&mut app, &ctx, "Ignore whitespace");
        settle_sources(&mut app, &ctx);
        assert_eq!(app.counts[1], 0);
        assert_eq!(app.file_view.as_ref().unwrap().counts[1], 0);
        click_action(&mut app, &ctx, "Show only differences");
        let output = render(&mut app, &ctx, vec![]);
        assert!(!text_positions(&output, "No differences with the current options.").is_empty());
        output.drop_without_applying_deltas();
        click_action(&mut app, &ctx, "Ignore line endings");
        settle_sources(&mut app, &ctx);
        assert_eq!(app.counts[1], 1);
        assert_eq!(app.file_view.as_ref().unwrap().counts[1], 1);
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(text_positions(&output, "CRLF").len(), 1);
        assert_eq!(text_positions(&output, "LF").len(), 1);
        output.drop_without_applying_deltas();
        click_action(&mut app, &ctx, "Back to folders");
        assert!(app.file_view.is_none());
        assert_eq!(app.counts[1], 1);
    }

    #[test]
    fn changed_text_is_highlighted_while_common_text_stays_unhighlighted() {
        let sources = SourceFixture::new();
        std::fs::write(sources.path("left/model.txt"), "old stable old\n").unwrap();
        std::fs::write(sources.path("right/model.txt"), "new stable new\n").unwrap();
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let ctx = egui::Context::default();
            set_theme(&ctx, theme);
            let mut app = VersusApp::default();
            app.open_launch_request(crate::cli::LaunchRequest {
                paths: [
                    sources.path("left/model.txt"),
                    sources.path("right/model.txt"),
                ],
                mode: None,
            });
            settle_sources(&mut app, &ctx);
            let output = render(&mut app, &ctx, vec![]);
            for expected in ["old stable old", "new stable new"] {
                let galley = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == expected => {
                            Some(&text.galley)
                        }
                        _ => None,
                    })
                    .unwrap();
                let highlighted: Vec<_> = galley
                    .job
                    .sections
                    .iter()
                    .filter(|section| section.format.background != Color32::TRANSPARENT)
                    .map(|section| {
                        &galley.job.text[section.byte_range.start.0..section.byte_range.end.0]
                    })
                    .collect();
                assert_eq!(
                    highlighted,
                    if expected.starts_with("old") {
                        vec!["old", "old"]
                    } else {
                        vec!["new", "new"]
                    }
                );
            }
            output.drop_without_applying_deltas();
        }
    }

    #[derive(Debug)]
    struct TestDrop(PathBuf);
    impl egui::DroppedFile for TestDrop {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            panic!("Drop handlers must not read file contents on the UI thread")
        }
    }

    fn drop_sources(app: &mut VersusApp, ctx: &egui::Context, side: usize, paths: Vec<PathBuf>) {
        render(app, ctx, vec![]).drop_without_applying_deltas();
        let position = app.pane_rects[side].center();
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                events: vec![egui::Event::PointerMoved(position)],
                dropped_files: paths
                    .into_iter()
                    .map(|path| Arc::new(TestDrop(path)) as egui::DroppedFileHandle)
                    .collect(),
                ..Default::default()
            },
            |ui| app.render(ui),
        )
        .drop_without_applying_deltas();
    }

    #[test]
    fn dropped_files_and_folders_open_on_the_chosen_side_and_cancel_old_work() {
        let sources = SourceFixture::new();
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = VersusApp::default();
        drop_sources(&mut app, &ctx, 1, vec![sources.path("right/model.txt")]);
        settle_sources(&mut app, &ctx);
        assert_eq!(app.comparison_mode, ComparisonMode::File);
        assert_eq!(
            app.paths[1],
            sources.path("right/model.txt").display().to_string()
        );
        drop_sources(&mut app, &ctx, 0, vec![sources.path("left/model.txt")]);
        settle_sources(&mut app, &ctx);
        assert!(app.file_view.as_ref().unwrap().comparison.is_some());
        drop_sources(&mut app, &ctx, 0, vec![sources.path("left")]);
        settle_sources(&mut app, &ctx);
        assert_eq!(app.mode(), SelectionMode::Incompatible);
        drop_sources(&mut app, &ctx, 1, vec![sources.path("right")]);
        settle_sources(&mut app, &ctx);
        assert!(app.tree.is_some());
        let sender = attach_job(&mut app);
        let flag = app.job.as_ref().unwrap().cancellation.clone();
        drop_sources(&mut app, &ctx, 0, vec![sources.path("left/model.txt")]);
        assert!(flag.load(Ordering::Relaxed));
        assert!(sender.send(Ok(fixture())).is_err());
        settle_sources(&mut app, &ctx);
    }

    #[test]
    fn dropping_on_a_file_drilldown_retains_the_other_file_and_rejects_multiple_drops() {
        let sources = SourceFixture::new();
        let ctx = egui::Context::default();
        let mut app = VersusApp::default();
        app.open_launch_request(crate::cli::LaunchRequest {
            paths: [sources.path("left"), sources.path("right")],
            mode: None,
        });
        settle_sources(&mut app, &ctx);
        app.open_file("model.txt".into(), [true, true], false);
        settle_sources(&mut app, &ctx);
        drop_sources(&mut app, &ctx, 0, vec![sources.path("left/model.txt")]);
        settle_sources(&mut app, &ctx);
        let view = app.file_view.as_ref().unwrap();
        assert!(!view.from_folders);
        assert_eq!(view.paths[1], sources.path("right/model.txt"));
        let paths = app.paths.clone();
        drop_sources(
            &mut app,
            &ctx,
            0,
            vec![sources.path("left"), sources.path("right")],
        );
        assert_eq!(app.paths, paths);
        assert!(app.drop_message.is_some());
    }

    #[test]
    fn drop_hover_marks_the_target_and_outside_drops_preserve_selection() {
        let sources = SourceFixture::new();
        let ctx = egui::Context::default();
        let mut app = VersusApp::default();
        render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
        let position = app.pane_rects[1].center();
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                events: vec![egui::Event::PointerMoved(position)],
                hovered_files: vec![egui::HoveredFile {
                    path: Some(sources.path("right/model.txt")),
                    ..Default::default()
                }],
                ..Default::default()
            },
            |ui| app.render(ui),
        );
        assert_eq!(app.drop_hover_side, Some(1));
        assert!(!text_positions(&output, "Drop a file or folder on RIGHT").is_empty());
        output.drop_without_applying_deltas();
        let paths = app.paths.clone();
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                events: vec![egui::Event::PointerMoved(egui::pos2(600.0, 20.0))],
                dropped_files: vec![Arc::new(TestDrop(sources.path("right/model.txt")))],
                ..Default::default()
            },
            |ui| app.render(ui),
        )
        .drop_without_applying_deltas();
        assert_eq!(app.paths, paths);
        assert!(app.drop_message.is_some());
        assert!(app.source_jobs.iter().all(Option::is_none));
    }

    #[test]
    fn progress_displays_measured_stage_estimates_and_hides_unknown_estimates() {
        let ctx = egui::Context::default();
        let mut app = loaded_app();
        let _sender = attach_job(&mut app);
        let job = app.job.as_mut().unwrap();
        job.started = Instant::now() - Duration::from_secs(3);
        job.progress.begin(ProgressStage::ComparingFiles, Some(100));
        job.progress.advance(25);
        let snapshot = job.progress.snapshot();
        let now = snapshot.started + Duration::from_secs(2);
        let text = progress_text(snapshot, job.started, now);
        assert!(text.contains("25 / 100"));
        assert!(text.contains("~6s left in this stage"));
        let output = render(&mut app, &ctx, vec![]);
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().starts_with("Comparing files…"))));
        output.drop_without_applying_deltas();
        let job = app.job.as_mut().unwrap();
        job.progress.begin(ProgressStage::Scanning, None);
        let snapshot = job.progress.snapshot();
        let text = progress_text(
            snapshot,
            job.started,
            snapshot.started + Duration::from_secs(2),
        );
        assert!(text.contains("entries found"));
        assert!(!text.contains("left in this stage"));
    }
}
