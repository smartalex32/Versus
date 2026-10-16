//! Session state, source/mode transitions and background result publication.
//!
//! Rendering lives in the parent. Workers publish only through the currently
//! owned job; replacing or dropping a job cancels/disconnects its old result.

use super::{
    FolderBrowser, InteractiveView, ROW_HEIGHT, StatusIcon, absolute_native_path, absolute_path,
    comparison_font, count_entries, empty_diff_side, file_line_anchor, file_style, find_file_line,
    is_file_difference, state_index, update_visible_file_rows,
};
use crate::recent::{RecentComparison, RecentComparisons};
use crate::selection::{self, ComparisonMode, SelectionMode, SourceKind};
use eframe::egui;
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
    CompareError, ComparisonProgress, DirectoryCompareOptions, DirectoryEntryState, FileComparison,
    FileViewOptions, FolderTree, ProgressStage, load_file_comparison_with_options,
};

pub(super) struct ComparisonJob {
    pub(super) receiver: Receiver<Result<FolderTree, CompareError>>,
    pub(super) cancellation: Arc<AtomicBool>,
    pub(super) roots: [PathBuf; 2],
    pub(super) started: Instant,
    pub(super) progress: Arc<ComparisonProgress>,
    pub(super) logged_progress: Option<(ProgressStage, u64, Option<u64>)>,
    pub(super) last_progress_log: Instant,
}

pub(super) struct FileJob {
    pub(super) receiver: Receiver<Result<Option<LoadedFile>, CompareError>>,
    pub(super) cancellation: Arc<AtomicBool>,
    pub(super) progress: Arc<ComparisonProgress>,
    pub(super) ignore_line_endings: bool,
}

pub(super) struct LoadedFile {
    pub(super) comparison: FileComparison,
    pub(super) sources: [Option<PathBuf>; 2],
}

pub(super) struct RecentStore {
    pub(super) sender: Option<mpsc::Sender<RecentComparison>>,
    pub(super) receiver: Receiver<Result<Option<RecentComparisons>, String>>,
    pub(super) worker: Option<std::thread::JoinHandle<()>>,
}

impl RecentStore {
    pub(super) fn start(ctx: &egui::Context, history_path: Option<PathBuf>) -> Self {
        let ctx = ctx.clone();
        let (sender, requests) = mpsc::channel::<RecentComparison>();
        let (results, receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let loaded = match &history_path {
                Some(path) => RecentComparisons::load_from(path),
                None => RecentComparisons::load(),
            };
            let mut recent = match loaded {
                Ok(recent) => {
                    let _ = results.send(Ok(Some(recent.clone())));
                    recent
                }
                Err(error) => {
                    let _ = results.send(Err(format!("Cannot load recent comparisons: {error}")));
                    RecentComparisons::default()
                }
            };
            ctx.request_repaint();
            for entry in requests {
                recent.record(entry.mode, entry.paths);
                let saved = match &history_path {
                    Some(path) => recent.save_to(path),
                    None => recent.save(),
                };
                let result = saved
                    .map(|()| None)
                    .map_err(|error| format!("Cannot save recent comparisons: {error}"));
                let _ = results.send(result);
                ctx.request_repaint();
            }
        });
        Self {
            sender: Some(sender),
            receiver,
            worker: Some(worker),
        }
    }
}

impl Drop for RecentStore {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(super) struct SourceJob {
    pub(super) receiver: Receiver<Result<SourceKind, String>>,
    pub(super) cancellation: Arc<AtomicBool>,
}

impl Drop for SourceJob {
    fn drop(&mut self) {
        self.cancellation.store(true, Ordering::Relaxed);
    }
}

pub(super) struct FileView {
    pub(super) row_height: f32,
    pub(super) from_folders: bool,
    pub(super) paths: [PathBuf; 2],
    pub(super) sources: [Option<PathBuf>; 2],
    pub(super) job: Option<FileJob>,
    pub(super) comparison: Option<FileComparison>,
    pub(super) error: Option<String>,
    pub(super) error_icon: StatusIcon,
    pub(super) scroll_y: f32,
    pub(super) content_widths: [f32; 2],
    pub(super) gutter_extra: f32,
    pub(super) counts: [usize; 6],
    pub(super) visible_rows: Vec<usize>,
    pub(super) difference_rows: Vec<usize>,
    pub(super) navigation_row: Option<usize>,
    pub(super) navigation_scroll_pending: bool,
    pub(super) ignore_line_endings: bool,
}

impl Drop for FileView {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancellation.store(true, Ordering::Relaxed);
        }
    }
}

pub(super) struct Session {
    pub(super) interactive: Option<InteractiveView>,
    pub(super) merge_mode: bool,
    pub(super) merge_folders: Option<FolderBrowser>,
    /// Find/replace has its own lightweight owner for direct file merge sessions.
    /// It deliberately never enumerates sources, so files keep their editor view.
    pub(super) merge_file_search: Option<FolderBrowser>,
    pub(super) merge_file_open: bool,
    pub(super) merge_file_sides: [bool; 2],
    pub(super) folder_diff_settings: Option<[bool; 3]>,
    pub(super) pending_merge_exit: Option<MergeExit>,
    pub(super) comparison_mode: ComparisonMode,
    pub(super) file_view: Option<FileView>,
    pub(super) paths: [String; 2],
    // Keep filesystem paths separate from their potentially lossy UI text.
    pub(super) source_paths: [Option<PathBuf>; 2],
    // Some(None) is a pending automatic launch; Some(Some(mode)) forces a mode.
    pub(super) launch_mode: Option<Option<ComparisonMode>>,
    pub(super) source_kinds: [Option<SourceKind>; 2],
    pub(super) source_jobs: [Option<SourceJob>; 2],
    pub(super) source_errors: [Option<String>; 2],
    pub(super) roots: Option<[PathBuf; 2]>,
    pub(super) tree: Option<FolderTree>,
    pub(super) job: Option<ComparisonJob>,
    pub(super) selected: Option<PathBuf>,
    pub(super) message: String,
    pub(super) error: Option<String>,
    pub(super) elapsed: Option<Duration>,
    pub(super) counts: [usize; 6],
    pub(super) scroll_generation: u64,
    pub(super) show_only_differences: bool,
    pub(super) ignore_whitespace: bool,
    pub(super) ignore_line_endings: bool,
    pub(super) tree_scroll_y: f32,
    pub(super) row_height: f32,
    pub(super) tree_keyboard_focus: Option<PathBuf>,
    pub(super) tree_focus_id: Option<egui::Id>,
    pub(super) file_keyboard_focus_side: Option<usize>,
    pub(super) folder_differences: Vec<PathBuf>,
    pub(super) drop_message: Option<String>,
    pub(super) recent: RecentComparisons,
    pub(super) recent_store: Option<RecentStore>,
    pub(super) recent_message: Option<String>,
}

#[derive(Clone)]
pub(super) enum MergeExit {
    Close,
    New,
    Mode(ComparisonMode),
    Recent(RecentComparison),
}

impl Default for Session {
    fn default() -> Self {
        Self {
            interactive: None,
            merge_mode: false,
            merge_folders: None,
            merge_file_search: None,
            merge_file_open: false,
            merge_file_sides: [false; 2],
            folder_diff_settings: None,
            pending_merge_exit: None,
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
            scroll_generation: 0,
            show_only_differences: true,
            ignore_whitespace: true,
            ignore_line_endings: true,
            tree_scroll_y: 0.0,
            row_height: ROW_HEIGHT,
            tree_keyboard_focus: None,
            tree_focus_id: None,
            file_keyboard_focus_side: None,
            folder_differences: Vec::new(),
            drop_message: None,
            recent: RecentComparisons::default(),
            recent_store: None,
            recent_message: None,
        }
    }
}

impl Session {
    pub(super) fn record_recent(&mut self, mode: ComparisonMode, paths: [PathBuf; 2]) {
        if paths.iter().any(|path| empty_diff_side(path)) {
            return;
        }
        self.recent.record(mode, paths.clone());
        if let Some(sender) = self
            .recent_store
            .as_ref()
            .and_then(|store| store.sender.as_ref())
        {
            if sender.send(RecentComparison { mode, paths }).is_err() {
                self.recent_message =
                    Some("Cannot save recent comparisons: the history worker stopped.".into());
            }
        }
    }

    pub(super) fn poll_recent(&mut self, ctx: &egui::Context) {
        let Some(store) = &self.recent_store else {
            return;
        };
        while let Ok(result) = store.receiver.try_recv() {
            match result {
                Ok(Some(mut saved)) => {
                    for entry in self.recent.entries().iter().rev() {
                        saved.record(entry.mode, entry.paths.clone());
                    }
                    self.recent = saved;
                    ctx.request_repaint();
                }
                Ok(None) => self.recent_message = None,
                Err(error) => self.recent_message = Some(error),
            }
        }
    }

    /// The editor is the source of truth for direct-file merge dirtiness. Keep
    /// the search owner's cached save snapshot current once ordinary editing has
    /// returned to a clean state, without cloning buffers on every keystroke.
    pub(super) fn sync_file_search_when_clean(&mut self) {
        let Some(view) = self.interactive.as_ref().filter(|view| !view.any_dirty()) else {
            return;
        };
        if let Some(browser) = &mut self.merge_file_search {
            browser.sync_working_files(view);
        }
    }

    pub(super) fn poll_merge_save_owners(&mut self, ctx: &egui::Context) {
        if let (Some(browser), Some(view)) = (&mut self.merge_folders, &mut self.interactive) {
            browser.poll_save(view, ctx);
        }
        if let (Some(browser), Some(view)) = (&mut self.merge_file_search, &mut self.interactive) {
            browser.poll_save(view, ctx);
        }
        self.sync_file_search_when_clean();
        let external_save_busy = self
            .merge_folders
            .as_ref()
            .is_some_and(FolderBrowser::is_saving)
            || self
                .merge_file_search
                .as_ref()
                .is_some_and(FolderBrowser::is_saving);
        if let Some(view) = &mut self.interactive {
            view.set_external_save_busy(external_save_busy);
        }
    }

    pub(super) fn open_recent(&mut self, entry: RecentComparison) {
        self.sync_file_search_when_clean();
        if self
            .interactive
            .as_ref()
            .is_some_and(|view| view.any_dirty() || view.is_saving())
            || self
                .merge_folders
                .as_ref()
                .is_some_and(|browser| browser.any_dirty() || browser.is_saving())
            || self
                .merge_file_search
                .as_ref()
                .is_some_and(FolderBrowser::is_saving)
        {
            self.pending_merge_exit = Some(MergeExit::Recent(entry));
            return;
        }
        self.new_comparison();
        self.comparison_mode = entry.mode;
        for (side, path) in entry.paths.into_iter().enumerate() {
            self.select_path(side, path);
        }
        self.launch_mode = Some(Some(entry.mode));
    }

    pub(super) fn open_launch_request(&mut self, launch: crate::cli::LaunchRequest) {
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

    pub(super) fn comparison_paths(&self) -> [PathBuf; 2] {
        std::array::from_fn(|side| {
            self.source_paths[side]
                .clone()
                .unwrap_or_else(|| absolute_path(&self.paths[side]))
        })
    }

    pub(super) fn invalidate(&mut self) {
        if let Some(browser) = &mut self.merge_folders {
            browser.invalidate_comparison_cache();
        }
        self.file_view = None;
        if let Some(job) = self.job.take() {
            job.cancellation.store(true, Ordering::Relaxed);
        }
        self.tree = None;
        self.roots = None;
        self.selected = None;
        self.tree_keyboard_focus = None;
        self.tree_focus_id = None;
        self.file_keyboard_focus_side = None;
        self.error = None;
        self.elapsed = None;
        self.counts = [0; 6];
        self.folder_differences.clear();
        self.tree_scroll_y = 0.0;
        self.message = "Source selection changed.".into();
        self.scroll_generation += 1;
    }

    pub(super) fn select_path(&mut self, side: usize, path: PathBuf) {
        self.invalidate();
        if self.merge_mode || (self.merge_file_open && self.interactive.is_some()) {
            self.merge_file_sides[side] = false;
            self.merge_file_open = self.merge_file_sides.iter().any(|open| *open);
            if let Some(view) = &mut self.interactive {
                view.open_side(side, None);
            }
        }
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

    pub(super) fn poll_sources(&mut self, ctx: &egui::Context) {
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

    pub(super) fn mode(&self) -> SelectionMode {
        if self.file_view.is_some() {
            SelectionMode::File
        } else if self.tree.is_some() || self.job.is_some() {
            SelectionMode::Folder
        } else {
            selection::selection_mode(self.source_kinds)
        }
    }

    pub(super) fn switch_comparison_mode(&mut self, mode: ComparisonMode) {
        self.sync_file_search_when_clean();
        if self.merge_mode
            && mode == ComparisonMode::Folder
            && self.merge_folders.is_some()
            && self.source_kinds == [Some(SourceKind::Folder); 2]
        {
            self.merge_file_sides = [false; 2];
            self.merge_file_open = false;
            if let Some(view) = &mut self.interactive {
                view.open_sources([None, None]);
            }
            self.comparison_mode = mode;
            return;
        }
        if self.comparison_mode != mode
            && (self
                .interactive
                .as_ref()
                .is_some_and(|view| view.any_dirty() || view.is_saving())
                || self
                    .merge_folders
                    .as_ref()
                    .is_some_and(|browser| browser.any_dirty() || browser.is_saving())
                || self
                    .merge_file_search
                    .as_ref()
                    .is_some_and(FolderBrowser::is_saving))
        {
            self.pending_merge_exit = Some(MergeExit::Mode(mode));
            return;
        }
        if self.merge_file_open
            && mode == ComparisonMode::Folder
            && self.merge_folders.is_some()
            && self.source_kinds == [Some(SourceKind::Folder); 2]
        {
            self.merge_file_open = false;
            self.comparison_mode = mode;
            return;
        }
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

    pub(super) fn new_comparison(&mut self) {
        self.sync_file_search_when_clean();
        if self
            .interactive
            .as_ref()
            .is_some_and(InteractiveView::is_saving)
            || self
                .merge_folders
                .as_ref()
                .is_some_and(FolderBrowser::is_saving)
            || self
                .merge_file_search
                .as_ref()
                .is_some_and(FolderBrowser::is_saving)
            || self
                .interactive
                .as_ref()
                .is_some_and(InteractiveView::any_dirty)
            || self
                .merge_folders
                .as_ref()
                .is_some_and(FolderBrowser::any_dirty)
        {
            self.pending_merge_exit = Some(MergeExit::New);
            return;
        }
        self.interactive = self.merge_mode.then(InteractiveView::default);
        self.merge_folders = None;
        self.merge_file_search = None;
        self.merge_file_open = false;
        self.merge_file_sides = [false; 2];
        self.folder_diff_settings = None;
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

    pub(super) fn start_selected_comparison(&mut self) {
        self.error = self.source_errors.iter().flatten().next().cloned();
        if self.error.is_some() && !self.merge_mode {
            self.message = "Cannot open selected source. Choose another file or folder.".into();
            return;
        }
        if self.source_jobs.iter().any(Option::is_some) && !self.merge_mode {
            return;
        }
        if let Some(requested_mode) = self
            .launch_mode
            .take_if(|_| self.source_jobs.iter().all(Option::is_none))
        {
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
                    self.message = "Cannot compare these sources.".into();
                    return;
                }
            } else {
                self.comparison_mode = match selection::selection_mode(self.source_kinds) {
                    SelectionMode::Folder => ComparisonMode::Folder,
                    _ => ComparisonMode::File,
                };
            }
        }
        let retained_file_session = self.merge_file_open
            && self.interactive.is_some()
            && self.merge_folders.is_none()
            && selection::selection_mode(self.source_kinds) == SelectionMode::File;
        if self.merge_mode || retained_file_session {
            let recent_mode = match selection::selection_mode(self.source_kinds) {
                SelectionMode::File
                    if self
                        .source_kinds
                        .iter()
                        .all(|kind| *kind == Some(SourceKind::File)) =>
                {
                    Some(ComparisonMode::File)
                }
                SelectionMode::Folder
                    if self
                        .source_kinds
                        .iter()
                        .all(|kind| *kind == Some(SourceKind::Folder)) =>
                {
                    Some(ComparisonMode::Folder)
                }
                _ => None,
            };
            if let Some(mode) = recent_mode {
                self.record_recent(mode, self.comparison_paths());
            }
            self.open_interactive_sources();
            return;
        }
        if self.merge_file_open && self.merge_folders.is_none() {
            self.merge_file_open = false;
            self.merge_file_sides = [false; 2];
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
                if ready && !paths.iter().any(|path| empty_diff_side(path)) {
                    self.record_recent(ComparisonMode::File, paths.clone());
                }
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

    pub(super) fn start_comparison(&mut self) {
        if self.paths.iter().any(|path| path.trim().is_empty()) {
            return;
        }
        if let Some(job) = self.job.take() {
            job.cancellation.store(true, Ordering::Relaxed);
        }
        let roots = self.comparison_paths();
        self.record_recent(ComparisonMode::Folder, roots.clone());
        let worker_roots = roots.clone();
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker_cancellation = cancellation.clone();
        let (sender, receiver) = mpsc::channel();
        let progress = Arc::new(ComparisonProgress::default());
        let worker_progress = progress.clone();
        let ignore_whitespace = self.ignore_whitespace;
        let ignore_line_endings = self.ignore_line_endings;
        crate::diagnostics::info(format!(
            "Folder comparison started: left={} right={} ignore_whitespace={ignore_whitespace} ignore_line_endings={ignore_line_endings}",
            roots[0].display(),
            roots[1].display()
        ));
        std::thread::spawn(move || {
            let result = crate::diagnostics::catch_worker("Folder comparison", || {
                let diff = versus::compare_directories(
                    &worker_roots[0],
                    &worker_roots[1],
                    &DirectoryCompareOptions {
                        cancellation: Some(worker_cancellation.clone()),
                        ignore_whitespace,
                        ignore_line_endings,
                        progress: Some(worker_progress.clone()),
                        ..Default::default()
                    },
                )?;
                crate::diagnostics::info(format!(
                    "Folder content comparison finished: {} entries; cancelled={}; building tree",
                    diff.entries.len(),
                    diff.cancelled
                ));
                worker_progress.begin(ProgressStage::BuildingTree, None);
                if worker_cancellation.load(Ordering::Relaxed) {
                    return Ok(FolderTree::from_diff(&versus::DirectoryDiff {
                        entries: vec![],
                        cancelled: true,
                    }));
                }
                let tree = FolderTree::from_diff(&diff);
                worker_progress.begin(ProgressStage::Finished, Some(diff.entries.len() as u64));
                worker_progress.advance(diff.entries.len() as u64);
                Ok::<_, CompareError>(tree)
            })
            .map_err(|message| CompareError {
                path: None,
                kind: versus::CompareErrorKind::Io,
                message,
            });
            let _ = sender.send(result);
        });
        self.job = Some(ComparisonJob {
            receiver,
            cancellation,
            roots,
            started: Instant::now(),
            progress,
            logged_progress: None,
            last_progress_log: Instant::now(),
        });
        self.error = None;
        self.message = "Scanning folders and comparing file contents…".into();
    }

    pub(super) fn poll_comparison(&mut self, ctx: &egui::Context) {
        let Some(job) = &mut self.job else {
            return;
        };
        let snapshot = job.progress.snapshot();
        let state = (snapshot.stage, snapshot.completed, snapshot.total);
        if job.logged_progress != Some(state)
            && (job
                .logged_progress
                .is_none_or(|last| last.0 != snapshot.stage)
                || job.last_progress_log.elapsed() >= Duration::from_secs(1))
        {
            crate::diagnostics::info(format!(
                "Folder comparison progress: {:?} {}/{}",
                snapshot.stage,
                snapshot.completed,
                snapshot
                    .total
                    .map_or_else(|| "unknown".into(), |total| total.to_string())
            ));
            job.logged_progress = Some(state);
            job.last_progress_log = Instant::now();
        }
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
                crate::diagnostics::info("Folder comparison cancelled");
                self.message = "Comparison cancelled.".into();
                return;
            }
            match result {
                Ok(mut tree) => {
                    crate::diagnostics::info(format!(
                        "Folder comparison result received after {:.3}s",
                        job.started.elapsed().as_secs_f64()
                    ));
                    let same_roots = self.roots.as_ref() == Some(&job.roots);
                    if same_roots {
                        let anchor = self.tree.as_ref().and_then(|previous| {
                            previous
                                .visible_rows()
                                .into_iter()
                                .filter(|row| {
                                    !self.show_only_differences
                                        || row.node.state != DirectoryEntryState::Same
                                })
                                .nth((self.tree_scroll_y / self.row_height).floor() as usize)
                                .map(|row| row.node.relative_path.clone())
                        });
                        if let Some(previous) = &self.tree {
                            let expanded: Vec<_> = tree
                                .all_rows()
                                .into_iter()
                                .filter(|row| previous.is_expanded(&row.node.relative_path))
                                .map(|row| row.node.relative_path.clone())
                                .collect();
                            for path in expanded {
                                tree.set_expanded(path, true);
                            }
                        }
                        if self.selected.as_ref().is_some_and(|path| {
                            !tree
                                .all_rows()
                                .iter()
                                .any(|row| &row.node.relative_path == path)
                        }) {
                            self.selected = None;
                        }
                        if let Some(top) = anchor.and_then(|path| {
                            tree.visible_rows()
                                .into_iter()
                                .filter(|row| {
                                    !self.show_only_differences
                                        || row.node.state != DirectoryEntryState::Same
                                })
                                .position(|row| row.node.relative_path == path)
                        }) {
                            self.tree_scroll_y =
                                top as f32 * self.row_height + self.tree_scroll_y % self.row_height;
                        }
                    } else {
                        self.selected = None;
                        self.tree_scroll_y = 0.0;
                        self.scroll_generation += 1;
                    }
                    self.folder_differences = tree
                        .all_rows()
                        .into_iter()
                        .filter(|row| is_file_difference(row.node))
                        .map(|row| row.node.relative_path.clone())
                        .collect();
                    self.counts = count_entries(&tree);
                    self.tree = Some(tree);
                    if let Some(browser) = &mut self.merge_folders {
                        browser.invalidate_comparison_cache();
                    }
                    self.roots = Some(job.roots);
                    self.elapsed = Some(job.started.elapsed());
                    self.message = "Comparison complete.".into();
                }
                Err(error) => {
                    crate::diagnostics::error(format!("Folder comparison failed: {error}"));
                    self.error = Some(error);
                    self.message =
                        "Comparison failed. Check the folder paths and access permissions.".into();
                }
            }
        } else {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    pub(super) fn open_file(
        &mut self,
        relative_path: PathBuf,
        present: [bool; 2],
        type_mismatch: bool,
    ) {
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

    pub(super) fn create_file_view(
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
        let worker_paths = paths.clone();
        let (sender, receiver) = mpsc::channel();
        let progress = Arc::new(ComparisonProgress::default());
        options.progress = Some(progress.clone());
        let ignore_line_endings = options.ignore_line_endings;
        if ready && !type_mismatch {
            std::thread::spawn(move || {
                let sources = if from_folders {
                    std::array::from_fn(|side| {
                        // Only a missing path represents an empty side. Keep other
                        // inspection failures visible through the guarded loader.
                        (!std::fs::symlink_metadata(&worker_paths[side])
                            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound))
                        .then(|| worker_paths[side].clone())
                    })
                } else {
                    worker_sources
                };
                let result =
                    load_file_comparison_with_options(&sources, &worker_cancellation, &options)
                        .map(|comparison| {
                            comparison.map(|comparison| LoadedFile {
                                comparison,
                                sources,
                            })
                        });
                let _ = sender.send(result);
            });
        }
        FileView {
            row_height: ROW_HEIGHT,
            from_folders,
            paths,
            sources,
            job: (ready && !type_mismatch).then_some(FileJob {
                receiver,
                cancellation,
                progress,
                ignore_line_endings,
            }),
            comparison: None,
            error: type_mismatch.then(|| "Entry types differ. Line comparison requires regular files; folders and symlink targets are not opened.".into()),
            error_icon: if type_mismatch { StatusIcon::TypeMismatch } else { StatusIcon::Error },
            scroll_y: 0.0,
            content_widths: [0.0; 2],
            gutter_extra: 0.0,
            counts: [0; 6],
            visible_rows: Vec::new(),
            difference_rows: Vec::new(),
            navigation_row: None,
            navigation_scroll_pending: false,
            ignore_line_endings,
        }
    }

    pub(super) fn poll_file_comparison(&mut self, ctx: &egui::Context) {
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
            let job = view.job.take().unwrap();
            if job.cancellation.load(Ordering::Relaxed) {
                view.error = Some("File comparison cancelled.".into());
                return;
            }
            match result {
                Ok(Some(loaded)) => {
                    let comparison = loaded.comparison;
                    view.sources = loaded.sources;
                    // Ignore rules can change row alignment. Restore the viewport
                    // and navigation by source line number, rather than row index.
                    let anchor = file_line_anchor(
                        view,
                        view.visible_rows
                            .get((view.scroll_y / view.row_height).floor() as usize)
                            .copied(),
                    );
                    let navigation = file_line_anchor(view, view.navigation_row);
                    let fraction = view.scroll_y % view.row_height;
                    view.counts = [0; 6];
                    view.content_widths = [0.0; 2];
                    let font_width = comparison_font(ctx).size;
                    let mut max_line_number = 1;
                    for row in &comparison.rows {
                        view.counts[state_index(&row.state)] += 1;
                        for (side, line) in [&row.left, &row.right].into_iter().enumerate() {
                            if let Some((number, text)) = line {
                                max_line_number = max_line_number.max(*number);
                                view.content_widths[side] = view.content_widths[side].max(
                                    (file_style::TEXT_OFFSET + 28.0)
                                        + (text.len() + text.matches('\t').count() * 3) as f32
                                            * font_width,
                                );
                            }
                        }
                    }
                    view.gutter_extra = file_style::gutter_extra(max_line_number);
                    for width in &mut view.content_widths {
                        if *width > 0.0 {
                            *width += view.gutter_extra;
                        }
                    }
                    view.comparison = Some(comparison);
                    view.ignore_line_endings = job.ignore_line_endings;
                    view.error = None;
                    update_visible_file_rows(view, self.show_only_differences);
                    let comparison = view.comparison.as_ref().unwrap();
                    if let Some(row) = find_file_line(comparison, anchor) {
                        let top = view
                            .visible_rows
                            .iter()
                            .position(|index| *index >= row)
                            .unwrap_or_else(|| view.visible_rows.len().saturating_sub(1));
                        view.scroll_y = if view.visible_rows.is_empty() {
                            0.0
                        } else {
                            top as f32 * view.row_height + fraction
                        };
                    }
                    view.navigation_row = find_file_line(comparison, navigation)
                        .filter(|index| view.difference_rows.contains(index));
                    view.navigation_scroll_pending = true;
                }
                Ok(None) => view.error = Some("File comparison cancelled.".into()),
                Err(error) => view.error = Some(error),
            }
        } else {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    pub(super) fn file_options(&self) -> FileViewOptions {
        FileViewOptions {
            ignore_whitespace: self.ignore_whitespace,
            ignore_line_endings: self.ignore_line_endings,
            progress: None,
        }
    }

    pub(super) fn recompare(&mut self) {
        if self.source_jobs.iter().any(Option::is_some) {
            return; // Pending source classification will use the latest options.
        }
        if self
            .source_kinds
            .iter()
            .all(|kind| *kind == Some(SourceKind::Folder))
        {
            self.start_comparison();
        }
        self.reload_file(false);
    }

    pub(super) fn refresh_file(&mut self) {
        self.reload_file(true);
    }

    pub(super) fn reload_file(&mut self, refresh_types: bool) {
        if self.source_jobs.iter().any(Option::is_some) {
            return;
        }
        let options = self.file_options();
        if let Some(view) = &mut self.file_view {
            if refresh_types {
                view.error_icon = StatusIcon::Error;
            }
            let ready = view.from_folders
                || self
                    .source_kinds
                    .iter()
                    .all(|kind| *kind == Some(SourceKind::File));
            let mut refreshed = Self::create_file_view(
                view.paths.clone(),
                view.sources.clone(),
                view.from_folders,
                matches!(view.error_icon, StatusIcon::TypeMismatch),
                ready,
                options,
            );
            if let Some(job) = view.job.take() {
                job.cancellation.store(true, Ordering::Relaxed);
            }
            view.job = refreshed.job.take();
            if !matches!(view.error_icon, StatusIcon::TypeMismatch) {
                view.error = None;
            }
        }
    }

    pub(super) fn jump_difference(&mut self, forward: bool) {
        if let Some(view) = &mut self.file_view {
            let top = (view.scroll_y / view.row_height).floor() as usize;
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
                    view.scroll_y = index.saturating_sub(1) as f32 * view.row_height;
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
                let top = (self.tree_scroll_y / self.row_height).floor() as usize;
                visible.get(top).and_then(|row| {
                    all.iter()
                        .position(|item| item.node.relative_path == row.node.relative_path)
                })
            });
        let selected = self.selected.is_some();
        let target = if forward {
            all.iter().enumerate().find(|(index, row)| {
                is_file_difference(row.node)
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
                is_file_difference(row.node) && anchor.is_some_and(|anchor| *index < anchor)
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
                self.tree_scroll_y = index.saturating_sub(1) as f32 * self.row_height;
            }
            self.selected = Some(path);
        }
    }

    pub(super) fn set_workspace_merge(&mut self, merge: bool) {
        if self.merge_mode == merge {
            return;
        }
        self.merge_mode = merge;
        // A folder result produced in the other mode has different comparison
        // preferences. Force its replacement when Merge is entered again.
        self.folder_diff_settings = None;
        if merge {
            if self.merge_folders.is_some() {
                self.tree = None;
                self.merge_folders
                    .as_mut()
                    .unwrap()
                    .invalidate_comparison_cache();
            }
            if let Some(job) = self.job.take() {
                job.cancellation.store(true, Ordering::Relaxed);
            }
            if let Some(view) = &mut self.file_view {
                if let Some(job) = view.job.take() {
                    job.cancellation.store(true, Ordering::Relaxed);
                }
            }
            self.interactive
                .get_or_insert_with(InteractiveView::default);
            if !self.merge_file_open && self.merge_folders.is_none() {
                if let Some(file) = &self.file_view {
                    if file.from_folders {
                        if let Some(roots) = &self.roots {
                            self.merge_folders = Some(FolderBrowser::new(roots.clone()));
                        }
                    }
                    let paths = file.paths.clone();
                    self.interactive.as_mut().unwrap().open_pair(paths);
                    self.merge_file_open = true;
                    self.merge_file_sides = [true; 2];
                } else {
                    self.open_interactive_sources();
                }
            }
        } else if !self.merge_file_open && self.merge_folders.is_some() && self.roots.is_some() {
            self.start_comparison();
        }
        let prefs = [
            self.show_only_differences,
            self.ignore_whitespace,
            self.ignore_line_endings,
        ];
        if let Some(view) = &mut self.interactive {
            view.set_editing(merge, prefs);
        }
    }

    pub(super) fn open_interactive_sources(&mut self) {
        self.sync_file_search_when_clean();
        let folder_sources: [Option<PathBuf>; 2] = std::array::from_fn(|side| {
            (self.source_kinds[side] == Some(SourceKind::Folder))
                .then(|| self.source_paths[side].clone())
                .flatten()
        });
        let previous_roots = self.merge_folders.as_ref().map(FolderBrowser::source_roots);
        let view = self
            .interactive
            .get_or_insert_with(InteractiveView::default);
        view.set_editing(
            self.merge_mode,
            [
                self.show_only_differences,
                self.ignore_whitespace,
                self.ignore_line_endings,
            ],
        );
        let current = view.current_sources();
        let files = std::array::from_fn(|side| match self.source_kinds[side] {
            Some(SourceKind::File) => self.source_paths[side].clone(),
            Some(SourceKind::Folder)
                if self.merge_file_sides[side]
                    && previous_roots
                        .as_ref()
                        .is_some_and(|roots| roots[side] == folder_sources[side]) =>
            {
                current[side].clone()
            }
            _ => None,
        });
        self.merge_file_sides = files.each_ref().map(Option::is_some);
        self.merge_file_open = self.merge_file_sides.iter().any(|open| *open);
        view.open_sources(files);
        if folder_sources.iter().any(Option::is_some) {
            if let Some(browser) = &mut self.merge_folders {
                browser.set_sources(folder_sources.clone());
            } else {
                self.merge_folders = Some(FolderBrowser::new_with_sources(folder_sources.clone()));
            }
            // A file-search save owns a receiver and must be allowed to drain
            // even if the workspace changes into a folder merge. Retire an idle
            // owner, but retain pending saves and errors for status reporting.
            if self.merge_file_search.as_ref().is_some_and(|browser| {
                !browser.is_saving() && !browser.any_dirty() && browser.error_details().is_empty()
            }) {
                self.merge_file_search = None;
            }
        } else {
            self.merge_folders = None;
            let sources = view.current_sources();
            if let Some(browser) = &mut self.merge_file_search {
                browser.set_search_sources(sources);
            }
        }
        self.roots = match folder_sources.clone() {
            [Some(left), Some(right)] => Some([left, right]),
            _ => None,
        };
        if previous_roots.as_ref() != Some(&folder_sources) {
            self.folder_diff_settings = None;
        }
    }

    pub(super) fn sync_folder_merge_diff(&mut self) {
        if !self.merge_mode || self.merge_folders.is_none() {
            return;
        }
        let Some(view) = &self.interactive else {
            return;
        };
        if !view.run_diff_enabled() || self.source_kinds != [Some(SourceKind::Folder); 2] {
            if let Some(job) = self.job.take() {
                job.cancellation.store(true, Ordering::Relaxed);
            }
            self.folder_diff_settings = None;
            if self.tree.take().is_some() {
                self.merge_folders
                    .as_mut()
                    .unwrap()
                    .invalidate_comparison_cache();
            }
            return;
        }
        let settings = view.merge_options();
        let compare_changed = self
            .folder_diff_settings
            .is_none_or(|previous| previous[1..] != settings[1..]);
        self.folder_diff_settings = Some(settings);
        if compare_changed {
            let old = [
                self.show_only_differences,
                self.ignore_whitespace,
                self.ignore_line_endings,
            ];
            self.ignore_whitespace = settings[1];
            self.ignore_line_endings = settings[2];
            self.start_comparison();
            self.show_only_differences = old[0];
            self.ignore_whitespace = old[1];
            self.ignore_line_endings = old[2];
        }
    }
    pub(super) fn sync_working_folder_diff(&mut self, ctx: &egui::Context) {
        if let (Some(browser), Some(view)) = (&mut self.merge_folders, &self.interactive) {
            browser.poll_comparisons(view, ctx, self.merge_mode);
        }
    }

    pub(super) fn set_differences_only(&mut self, enabled: bool) {
        let previous: Vec<PathBuf> = self
            .tree
            .as_ref()
            .map(|tree| {
                tree.visible_rows()
                    .into_iter()
                    .filter(|row| {
                        !self.show_only_differences || row.node.state != DirectoryEntryState::Same
                    })
                    .map(|row| row.node.relative_path.clone())
                    .collect()
            })
            .unwrap_or_default();
        let top = (self.tree_scroll_y / self.row_height).floor() as usize;
        let fraction = self.tree_scroll_y % self.row_height;
        self.show_only_differences = enabled;
        if let Some(view) = &mut self.file_view {
            update_visible_file_rows(view, enabled);
        }
        if let Some(tree) = &self.tree {
            let next: Vec<_> = tree
                .visible_rows()
                .into_iter()
                .filter(|row| !enabled || row.node.state != DirectoryEntryState::Same)
                .map(|row| &row.node.relative_path)
                .collect();
            let positions: std::collections::HashMap<_, _> = next
                .iter()
                .enumerate()
                .map(|(index, path)| (*path, index))
                .collect();
            // Keep the top entry, or its closest following/preceding survivor.
            let anchor = previous
                .iter()
                .skip(top)
                .chain(previous.iter().take(top).rev())
                .find_map(|path| positions.get(path).copied());
            self.tree_scroll_y =
                anchor.map_or(0.0, |index| index as f32 * self.row_height + fraction);
        }
    }

    pub(super) fn navigation_status(&self) -> String {
        let (current, total) = if let Some(view) = &self.file_view {
            (
                view.navigation_row.and_then(|row| {
                    view.difference_rows
                        .iter()
                        .position(|candidate| *candidate == row)
                }),
                view.difference_rows.len(),
            )
        } else {
            (
                self.selected.as_ref().and_then(|path| {
                    self.folder_differences
                        .iter()
                        .position(|candidate| candidate == path)
                }),
                self.folder_differences.len(),
            )
        };
        current.map_or_else(
            || format!("{total} changes"),
            |index| format!("Change {} of {total}", index + 1),
        )
    }

    pub(super) fn comparison_counts(&self) -> Option<[usize; 6]> {
        if let Some(view) = &self.file_view {
            view.comparison.as_ref().map(|_| view.counts)
        } else {
            self.tree.as_ref().map(|_| self.counts)
        }
    }

    /// Publish completed work in session order before the next paint.
    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        self.poll_recent(ctx);
        self.poll_sources(ctx);
        self.poll_comparison(ctx);
        self.poll_file_comparison(ctx);
        if let Some(view) = &mut self.interactive {
            view.poll(ctx);
        }
        self.sync_file_search_when_clean();
        if let Some(browser) = &mut self.merge_folders {
            browser.poll(ctx);
        }
        if let Some(browser) = &mut self.merge_file_search {
            browser.poll(ctx);
        }
        self.poll_merge_save_owners(ctx);
        self.sync_folder_merge_diff();
        self.sync_working_folder_diff(ctx);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancellation.store(true, Ordering::Relaxed);
        }
    }
}
