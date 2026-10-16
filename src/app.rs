use crate::recent::{RecentComparison, RecentComparisons};
use crate::selection::{self, ComparisonMode, SelectionMode, SourceKind};
mod file_style;
mod find_icons;
mod folder_browser;
mod interactive_view;
#[cfg(test)]
mod operating_mode_tests;
mod search_result_rows;
mod search_session;
#[cfg(test)]
mod tests;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use folder_browser::FolderBrowser;
use interactive_view::InteractiveView;
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
const MIN_ZOOM: f32 = 0.2;
const MAX_ZOOM: f32 = 5.0;

struct ComparisonJob {
    receiver: Receiver<Result<FolderTree, CompareError>>,
    cancellation: Arc<AtomicBool>,
    roots: [PathBuf; 2],
    started: Instant,
    progress: Arc<ComparisonProgress>,
    logged_progress: Option<(ProgressStage, u64, Option<u64>)>,
    last_progress_log: Instant,
}

struct FileJob {
    receiver: Receiver<Result<Option<LoadedFile>, CompareError>>,
    cancellation: Arc<AtomicBool>,
    progress: Arc<ComparisonProgress>,
    ignore_line_endings: bool,
}

struct LoadedFile {
    comparison: FileComparison,
    sources: [Option<PathBuf>; 2],
}

struct RecentStore {
    sender: Option<mpsc::Sender<RecentComparison>>,
    receiver: Receiver<Result<Option<RecentComparisons>, String>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl RecentStore {
    fn start(ctx: &egui::Context, history_path: Option<PathBuf>) -> Self {
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
    row_height: f32,
    from_folders: bool,
    paths: [PathBuf; 2],
    sources: [Option<PathBuf>; 2],
    job: Option<FileJob>,
    comparison: Option<FileComparison>,
    error: Option<String>,
    error_icon: StatusIcon,
    scroll_y: f32,
    content_widths: [f32; 2],
    gutter_extra: f32,
    counts: [usize; 6],
    visible_rows: Vec<usize>,
    difference_rows: Vec<usize>,
    navigation_row: Option<usize>,
    navigation_scroll_pending: bool,
    ignore_line_endings: bool,
}

impl Drop for FileView {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancellation.store(true, Ordering::Relaxed);
        }
    }
}

struct MergePaneHeaders {
    files: [Option<PathBuf>; 2],
    back: [bool; 2],
    save: [bool; 2],
    dirty: [bool; 2],
}

pub struct VersusApp {
    interactive: Option<InteractiveView>,
    merge_mode: bool,
    merge_folders: Option<FolderBrowser>,
    /// Find/replace has its own lightweight owner for direct file merge sessions.
    /// It deliberately never enumerates sources, so files keep their editor view.
    merge_file_search: Option<FolderBrowser>,
    merge_file_open: bool,
    merge_file_sides: [bool; 2],
    folder_diff_settings: Option<[bool; 3]>,
    logged_errors: Vec<String>,
    pending_merge_exit: Option<MergeExit>,
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
    row_height: f32,
    tree_keyboard_focus: Option<PathBuf>,
    tree_focus_id: Option<egui::Id>,
    file_keyboard_focus_side: Option<usize>,
    preferences_context: Option<egui::Context>,
    folder_differences: Vec<PathBuf>,
    pane_rects: [Rect; 2],
    source_header_rects: [Rect; 2],
    native_drag_position: Option<Pos2>,
    drop_hover_side: Option<usize>,
    drop_message: Option<String>,
    recent: RecentComparisons,
    recent_store: Option<RecentStore>,
    recent_message: Option<String>,
}

#[derive(Clone)]
enum MergeExit {
    Close,
    New,
    Mode(ComparisonMode),
    Recent(RecentComparison),
}

impl Default for VersusApp {
    fn default() -> Self {
        Self {
            interactive: None,
            merge_mode: false,
            merge_folders: None,
            merge_file_search: None,
            merge_file_open: false,
            merge_file_sides: [false; 2],
            folder_diff_settings: None,
            logged_errors: Vec::new(),
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
            logo_texture: None,
            scroll_generation: 0,
            show_only_differences: true,
            ignore_whitespace: true,
            ignore_line_endings: true,
            tree_scroll_y: 0.0,
            row_height: ROW_HEIGHT,
            tree_keyboard_focus: None,
            tree_focus_id: None,
            file_keyboard_focus_side: None,
            preferences_context: None,
            folder_differences: Vec::new(),
            pane_rects: [Rect::NOTHING; 2],
            source_header_rects: [Rect::NOTHING; 2],
            native_drag_position: None,
            drop_hover_side: None,
            drop_message: None,
            recent: RecentComparisons::default(),
            recent_store: None,
            recent_message: None,
        }
    }
}

impl VersusApp {
    pub(super) fn report_startup_error(&mut self, error: String) {
        self.error = Some(error);
    }
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        launch: Option<crate::cli::LaunchRequest>,
    ) -> Self {
        apply_theme(&cc.egui_ctx);
        if let Some(storage) = cc.storage {
            restore_zoom(&cc.egui_ctx, storage);
        }
        let mut app = Self::default();
        app.preferences_context = Some(cc.egui_ctx.clone());
        app.recent_store = Some(RecentStore::start(&cc.egui_ctx, None));
        if let Some(launch) = launch {
            app.open_launch_request(launch);
            cc.egui_ctx.request_repaint();
        }
        app
    }

    fn record_recent(&mut self, mode: ComparisonMode, paths: [PathBuf; 2]) {
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

    fn poll_recent(&mut self, ctx: &egui::Context) {
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
    fn sync_file_search_when_clean(&mut self) {
        let Some(view) = self.interactive.as_ref().filter(|view| !view.any_dirty()) else {
            return;
        };
        if let Some(browser) = &mut self.merge_file_search {
            browser.sync_working_files(view);
        }
    }

    fn poll_merge_save_owners(&mut self, ctx: &egui::Context) {
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

    fn open_recent(&mut self, entry: RecentComparison) {
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

    fn select_path(&mut self, side: usize, path: PathBuf) {
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

    fn new_comparison(&mut self) {
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

    fn start_selected_comparison(&mut self) {
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

    fn start_comparison(&mut self) {
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

    fn poll_comparison(&mut self, ctx: &egui::Context) {
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
            self.start_comparison();
        }
        self.reload_file(false);
    }

    fn refresh_file(&mut self) {
        self.reload_file(true);
    }

    fn reload_file(&mut self, refresh_types: bool) {
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

    fn jump_difference(&mut self, forward: bool) {
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

    fn progress_indicator(&self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let file_job = self.file_view.as_ref().and_then(|view| view.job.as_ref());
        let job = file_job
            .map(|job| (&job.progress, &job.cancellation, "Cancel file comparison"))
            .or_else(|| {
                self.job
                    .as_ref()
                    .map(|job| (&job.progress, &job.cancellation, "Cancel folder comparison"))
            });
        if let Some((progress, cancel, tooltip)) = job {
            let snapshot = progress.snapshot();
            ui.horizontal(|ui| {
                let extent = control_extent(ui);
                let reserve = extent + ui.spacing().item_spacing.x;
                if ui.available_width() > 2.0 * extent {
                    ui.spinner();
                }
                let width = (ui.available_width() - reserve).max(0.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(width, extent),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        header_progress_label(ui, snapshot, palette.text);
                    },
                );
                if icon_button(
                    ui,
                    ToolbarIcon::Cancel,
                    !cancel.load(Ordering::Relaxed),
                    tooltip,
                )
                .clicked()
                {
                    cancel.store(true, Ordering::Relaxed);
                }
            });
            ui.ctx().request_repaint_after(Duration::from_millis(60));
        } else if self.source_jobs.iter().any(Option::is_some) {
            header_status_label(ui, "Opening selected sources…", palette.text);
            ui.ctx().request_repaint_after(Duration::from_millis(60));
        }
    }

    fn drag_and_drop(&mut self, ui: &mut egui::Ui) {
        let (hovering, position, dropped) = native_drop_frame(ui, self.native_drag_position.take());
        let side =
            position.and_then(|pos| self.pane_rects.iter().position(|rect| rect.contains(pos)));
        if hovering {
            // OLE drags can suppress ordinary mouse motion. Keep polling the
            // native cursor so the highlight follows it even without events.
            if cfg!(windows) {
                ui.ctx().request_repaint_after(Duration::from_millis(33));
            }
            if position.is_some() {
                self.drop_hover_side = side;
            }
            if let Some(side) = side.or(self.drop_hover_side) {
                paint_native_drop_highlight(
                    ui.ctx(),
                    self.pane_rects[side],
                    &format!(
                        "Drop a file or folder on {}",
                        if side == 0 { "LEFT" } else { "RIGHT" }
                    ),
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
                if self.merge_file_open && !self.merge_mode {
                    if let Some(view) = &mut self.interactive {
                        if let Some(paths) = view.pair_paths() {
                            let mut paths = paths.clone();
                            paths[side] = absolute_native_path(path);
                            view.open_pair(paths);
                            ui.ctx().request_repaint();
                            return;
                        }
                    }
                }
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
                self.drop_message = Some(if dropped.len() != 1 {
                    "Drop one file or folder at a time.".into()
                } else if dropped[0].path().as_os_str().is_empty() {
                    "This drag did not include a file or folder path. Use Browse to select it."
                        .into()
                } else {
                    "Drop into the large LEFT or RIGHT view below the path bars.".into()
                });
            }
            ui.ctx().request_repaint();
        }
        if !hovering {
            self.drop_hover_side = None;
        }
    }

    fn render(&mut self, ui: &mut egui::Ui) {
        self.dismiss_errors_on_pane_input(ui);
        self.preferences_context = Some(ui.ctx().clone());
        // egui separates modified wheel input from scrolling and smooths it.
        // Apply each delta once, including its tail across repaint frames.
        if ui.ctx().current_pass_index() == 0 {
            let delta = ui.input(|input| input.zoom_delta());
            if delta.is_finite() && delta > 0.0 && delta != 1.0 {
                ui.ctx()
                    .set_zoom_factor((ui.ctx().zoom_factor() * delta).clamp(MIN_ZOOM, MAX_ZOOM));
            }
        }
        if self.file_view.is_none()
            && self.tree_focus_id.is_some()
            && ui.memory(|memory| memory.focused()) == self.tree_focus_id
            && ui.input(|input| input.key_pressed(egui::Key::Tab))
        {
            let destination = if ui.input(|input| input.events.iter().any(|event| matches!(event,
                egui::Event::Key { key: egui::Key::Tab, pressed: true, modifiers, .. } if modifiers.shift))) {
                "Browse right folder"
            } else {
                "Folder Diff"
            };
            ui.memory_mut(|memory| {
                memory.move_focus(egui::FocusDirection::None);
                memory.request_focus(egui::Id::new(destination));
            });
            self.tree_keyboard_focus = None;
            self.tree_focus_id = None;
        }
        let row_height = comparison_row_height(ui);
        if (row_height - self.row_height).abs() > 0.1 {
            self.tree_scroll_y *= row_height / self.row_height;
            self.row_height = row_height;
        }
        if let Some(view) = &mut self.file_view {
            view.scroll_y *= row_height / view.row_height;
            view.row_height = row_height;
        }
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
                if self.interactive.is_some() && (self.merge_mode || self.merge_file_open) {
                    self.interactive_area(ui);
                    return;
                }
                self.legend(ui);
                ui.add_space(5.0);
                let available = ui.available_rect_before_wrap();
                let footer_height = workspace_footer_height(
                    ui,
                    false,
                    false,
                    &DIFF_LEGEND_ICONS,
                    self.comparison_counts(),
                );
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
                    if self.file_view.is_some() {
                        self.file_area(ui);
                    } else {
                        self.tree_area(ui, (tree_rect.height() - 2.0).max(0.0));
                    }
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(footer_rect), |ui| {
                    ui.set_clip_rect(ui.clip_rect().intersect(footer_rect));
                    self.footer(ui);
                });
            });
        self.sync_folder_merge_diff();
        self.sync_working_folder_diff(ui.ctx());
        self.drag_and_drop(ui);
        self.merge_exit_dialog(ui.ctx());
    }

    fn workspace_mode_toggle(&mut self, ui: &mut egui::Ui) {
        let mut merge = self.merge_mode;
        for (enabled, label, tooltip) in [
            (
                false,
                "Diff",
                "Inspect differences using comparison preferences",
            ),
            (
                true,
                "Merge",
                "Edit and transfer content; comparison is optional",
            ),
        ] {
            if workspace_mode_button(ui, label, merge == enabled)
                .on_hover_text(tooltip)
                .clicked()
            {
                merge = enabled;
            }
        }
        if merge != self.merge_mode {
            self.set_workspace_merge(merge);
            ui.ctx().request_repaint();
        }
    }

    fn set_workspace_merge(&mut self, merge: bool) {
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

    fn open_interactive_sources(&mut self) {
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

    fn interactive_area(&mut self, ui: &mut egui::Ui) {
        if self.merge_mode {
            self.independent_merge_area(ui);
            return;
        }
        let saving = self
            .merge_folders
            .as_ref()
            .is_some_and(FolderBrowser::is_saving)
            || self
                .merge_file_search
                .as_ref()
                .is_some_and(FolderBrowser::is_saving);
        let prefs = [
            self.show_only_differences,
            self.ignore_whitespace,
            self.ignore_line_endings,
        ];
        if let Some(view) = &mut self.interactive {
            view.set_external_save_busy(saving);
            view.set_editing(self.merge_mode, prefs);
        }
        if self.merge_file_open {
            if let Some(mut view) = self.interactive.take() {
                view.set_from_folders(self.merge_folders.is_some());
                let folder_file_paths = self.merge_folders.as_ref().map(|_| view.current_sources());
                let (panes, status) = view.render_with_sources(ui, |ui| {
                    if self.merge_folders.is_none() {
                        self.folder_inputs(ui);
                    } else if let Some([Some(left), Some(right)]) = &folder_file_paths {
                        file_path_headers(ui, &[left.clone(), right.clone()]);
                    }
                    paint_panel_divider(ui);
                });
                self.pane_rects = panes;
                ui.scope_builder(egui::UiBuilder::new().max_rect(status), |ui| {
                    self.render_bottom_status(ui, Some(&mut view));
                });
                if self.merge_folders.is_none() {
                    if let Some(paths) = view
                        .take_loaded_paths()
                        .filter(|_| self.source_jobs.iter().all(Option::is_none))
                    {
                        self.paths = paths
                            .clone()
                            .map(|path| path.to_string_lossy().into_owned());
                        self.source_paths = paths.clone().map(Some);
                        self.source_kinds = [Some(SourceKind::File); 2];
                    }
                }
                if !self.merge_mode {
                    let options = view.diff_preferences();
                    self.show_only_differences = options[0];
                    self.ignore_whitespace = options[1];
                    self.ignore_line_endings = options[2];
                }
                if view.take_back_requested() {
                    self.merge_file_open = false;
                    self.merge_file_sides = [false; 2];
                    view.open_sources([None, None]);
                    if !self.merge_mode && self.merge_folders.is_some() {
                        self.start_comparison();
                    }
                }
                self.interactive = Some(view);
            }
            if self
                .interactive
                .as_mut()
                .is_some_and(InteractiveView::take_new_requested)
            {
                self.new_comparison();
            }
        } else {
            if let Some(view) = &mut self.interactive {
                view.set_from_folders(false);
                let differences = match (&self.tree, &mut self.merge_folders) {
                    (Some(tree), Some(browser)) => browser.difference_paths(tree, view),
                    _ => Arc::new(Vec::new()),
                };
                view.set_folder_differences(differences);
                view.render_toolbar(ui);
                if let (Some(path), Some(browser)) =
                    (view.take_folder_jump(), &mut self.merge_folders)
                {
                    browser.reveal_difference(path);
                }
            }
            comparison_panel(ui).show(ui, |ui| {
                ui.set_min_height((ui.available_height() - 2.0).max(0.0));
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                self.folder_inputs(ui);
                paint_panel_divider(ui);
                self.pane_rects = split_rect(ui.available_rect_before_wrap());
                if let (Some(browser), Some(view)) =
                    (&mut self.merge_folders, &mut self.interactive)
                {
                    if let Some(paths) = browser.render(ui, view, self.tree.as_ref()) {
                        view.open_pair(paths);
                        self.merge_file_open = true;
                    }
                } else {
                    empty_display(
                        ui,
                        ui.available_height().max(60.0),
                        "Choose files or folders",
                        "Use Browse or drop a source into each pane.",
                    );
                }
            });
            if self
                .interactive
                .as_mut()
                .is_some_and(InteractiveView::take_new_requested)
            {
                self.new_comparison();
            }
        }
    }

    fn independent_merge_area(&mut self, ui: &mut egui::Ui) {
        let Some(mut view) = self.interactive.take() else {
            return;
        };
        view.set_external_save_busy(
            self.merge_folders
                .as_ref()
                .is_some_and(FolderBrowser::is_saving)
                || self
                    .merge_file_search
                    .as_ref()
                    .is_some_and(FolderBrowser::is_saving),
        );
        view.set_editing(
            true,
            [
                self.show_only_differences,
                self.ignore_whitespace,
                self.ignore_line_endings,
            ],
        );
        view.set_from_folders(self.merge_folders.is_some());
        let differences = match (&self.tree, &mut self.merge_folders) {
            (Some(tree), Some(browser)) => browser.difference_paths(tree, &view),
            _ => Arc::new(Vec::new()),
        };
        view.set_folder_differences(differences);
        view.render_workspace_toolbar(ui, !self.merge_file_sides.iter().any(|open| *open));
        if view.take_search_requested() {
            if let Some(browser) = &mut self.merge_folders {
                browser.open_search();
            } else {
                let sources = view.current_sources();
                let browser = self
                    .merge_file_search
                    .get_or_insert_with(|| FolderBrowser::new_search(sources.clone()));
                browser.set_search_sources(sources);
                browser.open_search();
            }
        }
        if self.merge_folders.is_none()
            && let Some(browser) = &mut self.merge_file_search
        {
            browser.set_search_sources(view.current_sources());
            browser.render_search_controls(ui, &mut view);
        }
        if let (Some(path), Some(browser)) = (view.take_folder_jump(), &mut self.merge_folders) {
            browser.reveal_difference(path);
        }
        if view.take_back_requested() {
            self.merge_file_sides = [false; 2];
            self.merge_file_open = false;
            view.open_sources([None, None]);
        }
        ui.add_space(5.0);
        let area = ui.available_rect_before_wrap();
        let footer_height = view.footer_height(ui) + 4.0;
        let body = Rect::from_min_max(
            area.min,
            egui::pos2(
                area.right(),
                (area.bottom() - footer_height).max(area.top()),
            ),
        );
        let footer = Rect::from_min_max(egui::pos2(area.left(), body.bottom() + 4.0), area.max);
        let mut opened: [Option<PathBuf>; 2] = [None, None];
        let mut back = [false; 2];
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            comparison_panel(ui).show(ui, |ui| {
                ui.set_min_height((ui.available_height() - 2.0).max(0.0));
                let source_spacing = ui.spacing().item_spacing.y;
                ui.spacing_mut().item_spacing = egui::vec2(
                    0.0,
                    if self.merge_file_sides.iter().any(|open| *open) {
                        source_spacing
                    } else {
                        0.0
                    },
                );
                back = self.merge_inputs(ui, &mut view);
                for side in 0..2 {
                    if self.source_jobs[side].is_some() {
                        view.open_side(side, None);
                    }
                }
                paint_panel_divider(ui);
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                if let Some(browser) = &mut self.merge_folders {
                    browser.render_controls(ui, &mut view);
                }
                let pane_area = ui.available_rect_before_wrap();
                self.pane_rects = split_rect(pane_area);
                ui.painter().line_segment(
                    [pane_area.center_top(), pane_area.center_bottom()],
                    Stroke::new(1.0, Palette::for_context(ui.ctx()).border),
                );
                ui.columns(2, |columns| {
                    for (side, pane) in columns.iter_mut().enumerate() {
                        pane.set_clip_rect(pane.clip_rect().intersect(self.pane_rects[side]));
                        if self.merge_file_sides[side] {
                            let height = pane.available_height();
                            view.render_side(pane, side, height);
                        } else if let Some(browser) = self
                            .merge_folders
                            .as_mut()
                            .filter(|browser| browser.source_roots()[side].is_some())
                        {
                            if let Some(path) =
                                browser.render_side(pane, &mut view, self.tree.as_ref(), side)
                            {
                                opened[side] = Some(path);
                            }
                        } else {
                            let (rect, _) = pane.allocate_exact_size(
                                egui::vec2(
                                    pane.available_width(),
                                    pane.available_height().max(80.0),
                                ),
                                Sense::hover(),
                            );
                            paint_empty_drop_target(pane, rect, side, true);
                        }
                    }
                });
            });
        });
        for side in 0..2 {
            if back[side] {
                self.merge_file_sides[side] = false;
                view.open_side(side, None);
            } else if let Some(path) = opened[side].take() {
                self.merge_file_sides[side] = true;
                view.open_side(side, Some(path));
            }
        }
        self.merge_file_open = self.merge_file_sides.iter().any(|open| *open);
        ui.scope_builder(egui::UiBuilder::new().max_rect(footer), |ui| {
            let status = view.render_footer(ui);
            ui.scope_builder(egui::UiBuilder::new().max_rect(status), |ui| {
                self.render_bottom_status(ui, Some(&mut view));
            });
        });
        if view.take_save_all_requested() {
            if let Some(browser) = self.merge_folders.as_mut() {
                browser.begin_save(&view);
            } else {
                let browser = self
                    .merge_file_search
                    .get_or_insert_with(|| FolderBrowser::new_search(view.current_sources()));
                browser.begin_save(&view);
            }
        }
        view.render_previews(ui.ctx());
        if self.merge_folders.is_none() {
            if let Some(paths) = view.take_loaded_sources() {
                for side in 0..2 {
                    if self.source_jobs[side].is_none() {
                        self.paths[side] = paths[side]
                            .as_ref()
                            .map(|path| path.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        self.source_paths[side] = paths[side].clone();
                        self.source_kinds[side] = paths[side].as_ref().map(|_| SourceKind::File);
                    }
                }
            }
        }
        let new = view.take_new_requested();
        self.interactive = Some(view);
        if new {
            self.new_comparison();
        }
    }

    fn sync_folder_merge_diff(&mut self) {
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
    fn sync_working_folder_diff(&mut self, ctx: &egui::Context) {
        if let (Some(browser), Some(view)) = (&mut self.merge_folders, &self.interactive) {
            browser.poll_comparisons(view, ctx, self.merge_mode);
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let modes = [ComparisonMode::Folder, ComparisonMode::File];
        let labels = modes.map(|mode| {
            egui::WidgetText::from(
                RichText::new(mode.workspace_label(self.merge_mode))
                    .size(14.0)
                    .strong(),
            )
            .into_galley(
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
        let (header, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 32.0_f32.max(control_extent(ui))),
            Sense::hover(),
        );
        let group = Rect::from_center_size(header.center(), egui::vec2(group_width, 28.0));

        let mut brand_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    header.min,
                    egui::pos2(group.left() - gap, header.bottom()),
                ))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        if header.width() >= 700.0 {
            if let Some((_, logo)) = &self.logo_texture {
                brand_ui.image((logo.id(), egui::vec2(28.0, 28.0)));
            }
            brand_ui.label(RichText::new("Versus").size(18.0).strong());
            brand_ui.add_space(16.0);
        } else {
            brand_ui.spacing_mut().item_spacing.x = 4.0;
        }
        self.workspace_mode_toggle(&mut brand_ui);

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
        if self.interactive.is_some() && (self.merge_mode || self.merge_file_open) {
            if self.merge_mode {
                let mut enabled = self.interactive.as_ref().unwrap().run_diff_enabled();
                if enabled
                    && icon_button(
                        &mut controls_ui,
                        ToolbarIcon::Refresh,
                        self.interactive.as_ref().unwrap().can_refresh_diff()
                            || self.merge_folders.is_some(),
                        "Refresh comparison",
                    )
                    .clicked()
                {
                    if let Some(browser) = &mut self.merge_folders {
                        browser.refresh_listing();
                    }
                    self.interactive.as_mut().unwrap().refresh_diff();
                    self.folder_diff_settings = None;
                }
                let view = self.interactive.as_mut().unwrap();
                if controls_ui
                    .checkbox(&mut enabled, "Run Diff")
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Compare loaded files to assist merging")
                    .changed()
                {
                    view.set_run_diff(enabled);
                }
            } else {
                let view = self.interactive.as_mut().unwrap();
                if icon_button(
                    &mut controls_ui,
                    ToolbarIcon::Refresh,
                    view.can_refresh_diff(),
                    "Refresh comparison",
                )
                .clicked()
                {
                    view.refresh_diff();
                }
            }
        } else {
            match self.mode() {
                SelectionMode::Folder => self.controls(&mut controls_ui),
                SelectionMode::File => self.file_controls(&mut controls_ui),
                _ => {}
            }
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
        pick_source: impl FnMut(usize, &str, ComparisonMode) -> Option<PathBuf>,
    ) {
        self.source_inputs(ui, pick_source, None);
    }

    fn merge_inputs(&mut self, ui: &mut egui::Ui, view: &mut InteractiveView) -> [bool; 2] {
        let files = view.current_sources();
        let roots = self
            .merge_folders
            .as_ref()
            .map(FolderBrowser::source_roots)
            .unwrap_or([None, None]);
        let header = MergePaneHeaders {
            files: files.clone(),
            back: std::array::from_fn(|side| self.merge_file_sides[side] && roots[side].is_some()),
            save: std::array::from_fn(|side| view.can_save_side(side)),
            dirty: std::array::from_fn(|side| view.is_side_dirty(side)),
        };
        let kinds = std::array::from_fn::<_, 2, _>(|side| {
            if files[side].is_some() {
                Some(SourceKind::File)
            } else {
                self.source_kinds[side]
            }
        });
        let (back, save) = self.source_inputs(
            ui,
            |side, path, mode| selection::pick_path(side, path, kinds[side], mode),
            Some(&header),
        );
        for side in 0..2 {
            if save[side] {
                view.request_side_save(side);
            }
        }
        back
    }

    fn source_inputs(
        &mut self,
        ui: &mut egui::Ui,
        mut pick_source: impl FnMut(usize, &str, ComparisonMode) -> Option<PathBuf>,
        merge: Option<&MergePaneHeaders>,
    ) -> ([bool; 2], [bool; 2]) {
        let palette = Palette::for_context(ui.ctx());
        let extent = control_extent(ui);
        let gap = ui.spacing().item_spacing.x.max(4.0);
        let width = ui.available_width();
        let paths: [String; 2] = std::array::from_fn(|side| {
            merge
                .and_then(|header| header.files[side].as_ref())
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.paths[side].clone())
        });
        let full_paths = paths.each_ref().map(|path| {
            if path.is_empty() {
                format!("Choose a {}", self.comparison_mode.source_label())
            } else {
                absolute_path(path).display().to_string()
            }
        });
        let label_widths = std::array::from_fn::<_, 2, _>(|side| {
            if merge.is_some() {
                ui.painter()
                    .layout_no_wrap(
                        if side == 0 {
                            "LEFT (Incoming)"
                        } else {
                            "RIGHT (Current)"
                        }
                        .into(),
                        FontId::monospace(10.0),
                        palette.muted,
                    )
                    .size()
                    .x
                    + gap
            } else {
                40.0
            }
        });
        let path_widths = std::array::from_fn::<_, 2, _>(|side| {
            let back = merge.is_some_and(|header| header.back[side]);
            (width / 2.0
                - 14.0
                - label_widths[side]
                - extent
                - gap
                - if merge.is_some() { extent + gap } else { 0.0 }
                - if back { extent + gap } else { 0.0 })
            .max(1.0)
        });
        let galleys = std::array::from_fn::<_, 2, _>(|side| {
            if merge.is_some() || self.source_kinds[side] == Some(SourceKind::File) {
                left_elided_path(
                    ui.painter(),
                    &full_paths[side],
                    path_widths[side],
                    palette.text,
                )
            } else {
                ui.painter().layout(
                    full_paths[side].clone(),
                    comparison_font(ui.ctx()),
                    palette.text,
                    path_widths[side],
                )
            }
        });
        let height = galleys
            .iter()
            .map(|galley| galley.size().y)
            .fold(extent, f32::max)
            + 14.0;
        let (header, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
        self.source_header_rects = split_rect(header);
        let mut back = [false; 2];
        let mut save = [false; 2];
        for (side, half) in split_rect(header).into_iter().enumerate() {
            let rect = half.shrink(7.0);
            let painter = ui.painter().with_clip_rect(rect);
            let mut label_x = rect.left();
            if merge.is_some_and(|header| header.back[side]) {
                let button = Rect::from_min_size(
                    egui::pos2(label_x, rect.center().y - extent / 2.0),
                    egui::vec2(extent, extent),
                );
                let mut button_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(button)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                back[side] = icon_button(
                    &mut button_ui,
                    ToolbarIcon::Back,
                    true,
                    &format!(
                        "Back to {} folder",
                        if side == 0 { "left" } else { "right" }
                    ),
                )
                .clicked();
                label_x += extent + gap;
            }
            if merge.is_some() {
                painter.text(
                    egui::pos2(label_x, rect.center().y),
                    Align2::LEFT_CENTER,
                    if side == 0 {
                        "LEFT (Incoming)"
                    } else {
                        "RIGHT (Current)"
                    },
                    FontId::monospace(10.0),
                    if side == 0 {
                        palette.left_only
                    } else {
                        palette.right_only
                    },
                );
                ui.interact(
                    Rect::from_min_size(
                        egui::pos2(label_x, rect.top()),
                        egui::vec2(label_widths[side], rect.height()),
                    ),
                    egui::Id::new(("merge-side-label", side)),
                    Sense::hover(),
                );
            } else {
                paint_side_label(&painter, rect, side, palette);
            }
            let position = egui::pos2(
                label_x + label_widths[side],
                rect.center().y - galleys[side].size().y / 2.0,
            );
            let path_rect = Rect::from_min_size(
                egui::pos2(position.x, rect.top()),
                egui::vec2(path_widths[side], rect.height()),
            );
            painter.galley_with_override_text_color(
                position,
                galleys[side].clone(),
                if merge.is_some_and(|header| header.dirty[side]) {
                    palette.changed
                } else {
                    palette.text
                },
            );
            let kind = if merge.is_some_and(|header| header.files[side].is_some()) {
                "file"
            } else {
                self.comparison_mode.source_label()
            };
            let browse_label =
                format!("Browse {} {kind}", if side == 0 { "left" } else { "right" });
            let path_response = ui
                .interact(
                    path_rect,
                    ui.id().with(("selected-folder", side)),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!(
                    "{}\nClick to browse for a {kind}",
                    full_paths[side]
                ));
            path_response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    format!("{browse_label} path"),
                )
            });
            let save_space = if merge.is_some() { extent + gap } else { 0.0 };
            let browse_rect = Rect::from_center_size(
                rect.right_center() - egui::vec2(extent / 2.0 + save_space, 0.0),
                egui::vec2(extent, extent),
            );
            let mut browse_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(browse_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            let browse = icon_button(&mut browse_ui, ToolbarIcon::Browse, true, &browse_label);
            if let Some(state) = merge {
                let save_rect = Rect::from_center_size(
                    rect.right_center() - egui::vec2(extent / 2.0, 0.0),
                    egui::vec2(extent, extent),
                );
                let mut save_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(save_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                save[side] = icon_button(
                    &mut save_ui,
                    ToolbarIcon::Save,
                    state.save[side],
                    if side == 0 { "Save Left" } else { "Save Right" },
                )
                .clicked();
            }
            if browse.clicked() || path_response.clicked() {
                if let Some(path) = pick_source(side, &paths[side], self.comparison_mode) {
                    self.select_path(side, path);
                    ui.ctx().request_repaint();
                }
            }
        }
        ui.painter().line_segment(
            [header.center_top(), header.center_bottom()],
            Stroke::new(1.0, palette.border),
        );
        (back, save)
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

    fn file_controls(&mut self, ui: &mut egui::Ui) {
        let ready = self.source_jobs.iter().all(Option::is_none)
            && self.source_errors.iter().all(Option::is_none)
            && self.file_view.as_ref().is_some_and(|view| {
                view.job.is_none()
                    && (view.from_folders || self.source_kinds == [Some(SourceKind::File); 2])
            });
        if icon_button(ui, ToolbarIcon::Refresh, ready, "Refresh comparison").clicked() {
            self.refresh_file();
        }
        if let Some(job) = self.file_view.as_ref().and_then(|view| view.job.as_ref()) {
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
        }
    }

    fn set_differences_only(&mut self, enabled: bool) {
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

    fn navigation_status(&self) -> String {
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

    fn legend(&mut self, ui: &mut egui::Ui) {
        let back = self
            .file_view
            .as_ref()
            .is_some_and(|view| view.from_folders);
        let palette = Palette::for_context(ui.ctx());
        let available = ui.available_width();
        let gap = ui.spacing().item_spacing.x;
        let extent = control_extent(ui);
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
        let right_width = zoom_width + 5.0 * (extent + gap);
        ui.horizontal(|ui| {
            let left_width = (available - right_width - gap).max(extent);
            ui.allocate_ui_with_layout(
                egui::vec2(left_width, extent),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(left_width);
                    ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));
                    if back && icon_button(ui, ToolbarIcon::Back, true, "Back to folders").clicked()
                    {
                        self.file_view = None;
                    }
                    if icon_button(ui, ToolbarIcon::New, true, "New comparison").clicked() {
                        self.new_comparison();
                        ui.ctx().request_repaint();
                    }
                    status_label(ui, &self.navigation_status(), palette.muted);
                },
            );
            zoom_controls(ui);
            if icon_button_state(
                ui,
                ToolbarIcon::Differences,
                true,
                self.show_only_differences,
                "Show only differences",
            )
            .clicked()
            {
                self.set_differences_only(!self.show_only_differences);
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
        });
    }

    fn file_area(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let row_height = self.row_height;
        let mut focused_file_side = self.file_keyboard_focus_side;
        let from_folders = self.file_view.as_ref().unwrap().from_folders;
        comparison_panel(ui).show(ui, |ui| {
                ui.set_min_height((ui.available_height() - 2.0).max(0.0));
                if !from_folders {
                    self.folder_inputs(ui);
                }
                let Some(view) = self.file_view.as_mut() else {
                    ui.label("Opening selected source…");
                    return;
                };
                if from_folders {
                    file_path_headers(ui, &view.paths);
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
                if let Some(error) = &view.error
                    && view.comparison.is_none()
                {
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(14.0, 14.0), Sense::hover());
                        paint_status_icon(ui.painter(), rect.center(), view.error_icon);
                        ui.colored_label(palette.error, error);
                    });
                    return;
                }
                let Some(comparison) = &view.comparison else {
                    if view.job.is_some() {
                        ui.horizontal(|ui| { ui.spinner(); ui.label("Loading file comparison…"); });
                    } else {
                        empty_display(ui, (ui.available_height() - 2.0).max(0.0), "Choose files", "Use Browse above to select each file.");
                    }
                    return;
                };
                if let Some(message) = &comparison.message {
                    ui.label(RichText::new(message).color(palette.muted));
                }
                if comparison.rows.is_empty() {
                    if comparison.message.is_none() {
                        empty_display(ui, (ui.available_height() - 2.0).max(0.0), "Both files are empty", "There are no lines to compare.");
                    }
                    return;
                }
                if view.visible_rows.is_empty() {
                    empty_display(ui, (ui.available_height() - 2.0).max(0.0), "No differences", "No differences with the current options.");
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
                                    row_height * view.visible_rows.len() as f32,
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
                    let first = (next_y / row_height).floor() as usize;
                    let last = ((next_y + viewport.height()) / row_height).ceil() as usize;
                    for index in first..last.min(view.visible_rows.len()) {
                        let original_index = view.visible_rows[index];
                        let row = &comparison.rows[original_index];
                        let line = if side == 0 { &row.left } else { &row.right };
                        let rect = Rect::from_min_size(
                            egui::pos2(
                                viewport.left() - horizontal_offsets[side],
                                viewport.top() + index as f32 * row_height - next_y,
                            ),
                            egui::vec2(
                                viewport.width().max(view.content_widths[side]),
                                row_height,
                            ),
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
                            !view.ignore_line_endings && row.left_ending != row.right_ending;
                        paint_file_line(
                            &painter,
                            rect,
                            line.as_ref(),
                            &row.state,
                            index % 2 == 1,
                            changed,
                            ending_diff.then_some(ending).flatten(),
                            view.navigation_row == Some(original_index),
                            view.gutter_extra,
                        );
                        let line_response = ui.interact(
                            rect.intersect(viewport),
                            ui.id().with(("file-line", &view.paths, side, original_index)),
                            Sense::hover(),
                        );
                        line_response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Label,
                                true,
                                file_line_accessible_label(side, line.as_ref(), &row.state),
                            )
                        });
                    }
                }
                let mut keyboard_scroll = 0.0;
                for side in 0..2 {
                    let pane_response = ui.interact(
                        viewports[side],
                        ui.id().with(("file-pane", &view.paths, side)),
                        Sense::click(),
                    );
                    pane_response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Panel,
                            true,
                            format!(
                                "{} file comparison pane: {}. Use Up and Down Arrow to scroll lines, Page Up or Page Down for pages, Home or End for the first or last line.",
                                side_name(side), view.paths[side].display()
                            ),
                        )
                    });
                    if pane_response.clicked() {
                        pane_response.request_focus();
                        focused_file_side = Some(side);
                    }
                    if pane_response.has_focus() {
                        ui.memory_mut(|memory| memory.set_focus_lock_filter(pane_response.id, egui::EventFilter {
                            horizontal_arrows: true, vertical_arrows: true, ..Default::default()
                        }));
                        focused_file_side = Some(side);
                        keyboard_scroll = ui.input(|input| {
                            if input.key_pressed(egui::Key::ArrowDown) {
                                row_height
                            } else if input.key_pressed(egui::Key::ArrowUp) {
                                -row_height
                            } else if input.key_pressed(egui::Key::PageDown) {
                                viewports[side].height()
                            } else if input.key_pressed(egui::Key::PageUp) {
                                -viewports[side].height()
                            } else if input.key_pressed(egui::Key::Home) {
                                -next_y
                            } else if input.key_pressed(egui::Key::End) {
                                (view.visible_rows.len() as f32 * row_height) - next_y
                            } else {
                                0.0
                            }
                        });
                        ui.painter().rect_stroke(
                            viewports[side].shrink(1.0),
                            4.0,
                            Stroke::new(2.0, palette.accent),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
                if keyboard_scroll != 0.0 {
                    next_y = (next_y + keyboard_scroll).clamp(
                        0.0,
                        (view.visible_rows.len() as f32 * row_height
                            - viewports[0].height())
                            .max(0.0),
                    );
                    view.navigation_row = None;
                    ui.ctx().request_repaint();
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
                self.file_keyboard_focus_side = focused_file_side;
            });
    }

    fn tree_area(&mut self, ui: &mut egui::Ui, height: f32) {
        let palette = Palette::for_context(ui.ctx());
        let row_height = self.row_height;
        let mut open = None;
        comparison_panel(ui).show(ui, |ui| {
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
                        !self.show_only_differences || row.node.state != DirectoryEntryState::Same
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
                let mut tree_keyboard_focus = self.tree_keyboard_focus.clone();
                // A target is only retained between the Arrow-key frame and
                // the frame where virtual scrolling renders that row. Keep
                // its side beside the path so focus does not jump panes.
                let pending_side_id = ui.id().with("tree-keyboard-focus-side");
                let mut pending_side =
                    ui.memory(|memory| memory.data.get_temp::<usize>(pending_side_id));
                let mut rendered_tree_focus = None;
                self.tree_focus_id = None;
                // show_rows paints before ScrollArea clamps its offset. Clamp
                // first so a jump near the end is correct on its first frame.
                self.tree_scroll_y = self
                    .tree_scroll_y
                    .clamp(0.0, (rows.len() as f32 * row_height - body_height).max(0.0));
                let output = egui::ScrollArea::vertical()
                    .id_salt(("linked-trees", self.scroll_generation))
                    .auto_shrink([false, false])
                    .max_height(body_height)
                    .vertical_scroll_offset(self.tree_scroll_y)
                    .show_rows(ui, row_height, rows.len(), |ui, range| {
                        for index in range {
                            let row = &rows[index];
                            let node = row.node;
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), row_height),
                                Sense::hover(),
                            );
                            let is_selected = self.selected.as_ref() == Some(&node.relative_path);
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
                                if tree_keyboard_focus.as_ref() == Some(&node.relative_path)
                                    && pending_side == Some(side)
                                {
                                    response.request_focus();
                                    tree_keyboard_focus = None;
                                    pending_side = None;
                                    ui.memory_mut(|memory| {
                                        memory.data.remove::<usize>(pending_side_id);
                                    });
                                }
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::WidgetType::SelectableLabel,
                                        true,
                                        is_selected,
                                        if node.is_expandable() {
                                            format!(
                                                "{}, {}",
                                                tree_row_accessible_label(node, side),
                                                if tree.is_expanded(&node.relative_path) {
                                                    "expanded"
                                                } else {
                                                    "collapsed"
                                                }
                                            )
                                        } else {
                                            tree_row_accessible_label(node, side)
                                        },
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
                                if response.has_focus() {
                                    self.tree_focus_id = Some(response.id);
                                    ui.memory_mut(|memory| {
                                        memory.set_focus_lock_filter(
                                            response.id,
                                            egui::EventFilter {
                                                horizontal_arrows: true,
                                                vertical_arrows: true,
                                                ..Default::default()
                                            },
                                        )
                                    });
                                    rendered_tree_focus = Some((node.relative_path.clone(), side));
                                    ui.painter().rect_stroke(
                                        half.shrink(1.0),
                                        3.0,
                                        Stroke::new(2.0, palette.accent),
                                        egui::StrokeKind::Inside,
                                    );
                                }
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
                                    self.tree_focus_id = Some(response.id);
                                    selected = Some(node.relative_path.clone());
                                    if expandable {
                                        toggle = Some(node.relative_path.clone());
                                    }
                                }
                                if response.has_focus() && expandable {
                                    let expanded = tree.is_expanded(&node.relative_path);
                                    if ui.input(|input| {
                                        (input.key_pressed(egui::Key::ArrowRight) && !expanded)
                                            || (input.key_pressed(egui::Key::ArrowLeft) && expanded)
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
                if let Some((current_path, side)) = rendered_tree_focus {
                    let step = ui.input(|input| {
                        if input.key_pressed(egui::Key::ArrowDown) {
                            1_i32
                        } else if input.key_pressed(egui::Key::ArrowUp) {
                            -1
                        } else {
                            0
                        }
                    });
                    if step != 0 {
                        let current = rows
                            .iter()
                            .position(|row| row.node.relative_path == current_path)
                            .unwrap_or(0);
                        let next = (current as i32 + step)
                            .clamp(0, rows.len().saturating_sub(1) as i32)
                            as usize;
                        let path = rows[next].node.relative_path.clone();
                        tree_keyboard_focus = Some(path.clone());
                        ui.memory_mut(|memory| {
                            memory.data.insert_temp(pending_side_id, side);
                        });
                        selected = Some(path);
                        let target_top = next as f32 * row_height;
                        let target_bottom = target_top + row_height;
                        let viewport_height = output.inner_rect.height();
                        self.tree_scroll_y = if target_top < self.tree_scroll_y {
                            target_top
                        } else if target_bottom > self.tree_scroll_y + viewport_height {
                            target_bottom - viewport_height
                        } else {
                            self.tree_scroll_y
                        }
                        .clamp(0.0, (rows.len() as f32 * row_height - body_height).max(0.0));
                        ui.ctx().request_repaint();
                    }
                }
                if let Some(path) = toggle {
                    tree.toggle_expanded(path);
                }
                if let Some(path) = selected {
                    self.selected = Some(path);
                }
                self.tree_keyboard_focus = tree_keyboard_focus;
            } else if self.paths.iter().all(String::is_empty)
                && (!self.recent.entries().is_empty() || self.recent_message.is_some())
            {
                self.recent_area(ui, body_height);
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
                empty_display(ui, body_height, title, "");
            }
        });
        if let Some((path, present, type_mismatch)) = open {
            self.open_file(path, present, type_mismatch);
        }
    }

    fn recent_area(&mut self, ui: &mut egui::Ui, height: f32) {
        let palette = Palette::for_context(ui.ctx());
        let mut open = None;
        let entries: Vec<_> = self
            .recent
            .entries()
            .iter()
            .filter(|entry| entry.mode == self.comparison_mode)
            .collect();
        egui::ScrollArea::vertical()
            .id_salt(("recent-comparisons", self.comparison_mode.source_label()))
            .auto_shrink([false, false])
            .max_height(height)
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Recently compared")
                            .size(14.0)
                            .color(palette.muted),
                    );
                });
                ui.add_space(6.0);
                if let Some(message) = &self.recent_message {
                    ui.colored_label(palette.error, message);
                    ui.add_space(6.0);
                }
                if entries.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.label(
                            RichText::new(format!(
                                "No recent {} comparisons.",
                                self.comparison_mode.source_label()
                            ))
                            .color(palette.muted),
                        );
                    });
                }
                ui.spacing_mut().item_spacing.y = 0.0;
                for (index, entry) in entries.into_iter().enumerate() {
                    if recent_comparison_row(ui, entry, index, palette).clicked() {
                        open = Some(entry.clone());
                    }
                }
            });
        if let Some(entry) = open {
            self.open_recent(entry);
            ui.ctx().request_repaint();
        }
    }

    fn comparison_counts(&self) -> Option<[usize; 6]> {
        if let Some(view) = &self.file_view {
            view.comparison.as_ref().map(|_| view.counts)
        } else {
            self.tree.as_ref().map(|_| self.counts)
        }
    }

    fn error_details(&self, view: Option<&InteractiveView>) -> Vec<String> {
        let mut errors = self
            .source_errors
            .iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        errors.extend(self.error.iter().cloned());
        errors.extend(self.drop_message.iter().cloned());
        errors.extend(self.file_view.as_ref().and_then(|file| file.error.clone()));
        if let Some(error) = view.and_then(InteractiveView::error_details) {
            errors.extend(error.lines().map(str::to_owned));
        }
        if let Some(browser) = &self.merge_folders {
            errors.extend(browser.error_details());
        }
        if let Some(browser) = &self.merge_file_search {
            errors.extend(browser.error_details());
        }
        errors.sort();
        errors.dedup();
        errors
    }

    fn dismiss_errors_on_pane_input(&mut self, ui: &egui::Ui) {
        let moved = ui.input(|input| {
            input.events.iter().any(|event| {
                let position = match event {
                    egui::Event::PointerMoved(position) => Some(*position),
                    egui::Event::PointerButton {
                        pos, pressed: true, ..
                    } => Some(*pos),
                    _ => None,
                };
                position.is_some_and(|position| {
                    self.pane_rects
                        .iter()
                        .chain(self.source_header_rects.iter())
                        .any(|rect| rect.contains(position))
                })
            })
        });
        if moved {
            if !self.error_details(self.interactive.as_ref()).is_empty() {
                self.message.clear();
            }
            self.error = None;
            self.drop_message = None;
            self.source_errors = [None, None];
            if let Some(file) = &mut self.file_view {
                file.error = None;
            }
            if let Some(view) = &mut self.interactive {
                view.clear_error();
            }
            if let Some(browser) = &mut self.merge_folders {
                browser.clear_error();
            }
            if let Some(browser) = &mut self.merge_file_search {
                browser.clear_error();
            }
        }
    }

    fn render_bottom_status(&mut self, ui: &mut egui::Ui, view: Option<&mut InteractiveView>) {
        let errors = self.error_details(view.as_deref());
        for error in &errors {
            if !self.logged_errors.contains(error) {
                crate::diagnostics::error(format!("Status error: {error}"));
            }
        }
        self.logged_errors = errors.clone();
        bottom_status_area(ui, |ui| {
            if !errors.is_empty() {
                error_summary(ui, &errors);
            } else if self.drop_message.is_some()
                || self.source_jobs.iter().any(Option::is_some)
                || self.job.is_some()
            {
                self.progress_indicator(ui);
            } else if self.merge_mode
                && self
                    .merge_folders
                    .as_ref()
                    .is_some_and(|browser| browser.render_activity_status(ui))
            {
            } else if self.merge_mode
                && self
                    .merge_file_search
                    .as_ref()
                    .is_some_and(|browser| browser.render_activity_status(ui))
            {
            } else if view.is_none_or(|view| !view.render_activity_status(ui)) {
                self.progress_indicator(ui);
            }
        });
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        let row = workspace_footer_layout(
            ui,
            false,
            false,
            &DIFF_LEGEND_ICONS,
            self.comparison_counts(),
        );
        let guidance = self
            .selected
            .as_ref()
            .filter(|_| self.tree.is_some() && self.file_view.is_none())
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| self.message.clone());
        ui.scope_builder(egui::UiBuilder::new().max_rect(row.info), |ui| {
            footer_guidance(ui, &guidance);
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(row.legend), |ui| {
            bottom_legend(ui, &DIFF_LEGEND_ICONS, self.comparison_counts());
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(row.status), |ui| {
            self.render_bottom_status(ui, None);
        });
    }

    fn merge_exit_dialog(&mut self, ctx: &egui::Context) {
        self.sync_file_search_when_clean();
        if ctx.input(|input| input.viewport().close_requested())
            && (self
                .interactive
                .as_ref()
                .is_some_and(InteractiveView::any_dirty)
                || self
                    .merge_folders
                    .as_ref()
                    .is_some_and(FolderBrowser::any_dirty)
                || self
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
                    .is_some_and(FolderBrowser::is_saving))
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending_merge_exit = Some(MergeExit::Close);
        }
        let Some(destination) = self.pending_merge_exit.clone() else {
            return;
        };
        let saving = self
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
            self.pending_merge_exit = None;
        } else if discard {
            self.pending_merge_exit = None;
            self.interactive = None;
            self.merge_folders = None;
            self.merge_file_search = None;
            self.merge_file_open = false;
            self.merge_file_sides = [false; 2];
            match destination {
                MergeExit::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                MergeExit::New => self.new_comparison(),
                MergeExit::Mode(mode) => self.switch_comparison_mode(mode),
                MergeExit::Recent(entry) => self.open_recent(entry),
            }
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
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(ctx) = &self.preferences_context {
            storage.set_string("versus.zoom", ctx.zoom_factor().to_string());
        }
    }

    fn persist_egui_memory(&self) -> bool {
        // Only the UI scale is retained; compared paths/content remain session-local.
        false
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        #[cfg(windows)]
        if ui.input(|input| {
            !input.raw.hovered_files.is_empty() || !input.raw.dropped_files.is_empty()
        }) {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            self.native_drag_position = _frame.window_handle().ok().and_then(|handle| {
                if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                    crate::native_drop::cursor_position(
                        handle.hwnd.get(),
                        ui.ctx().pixels_per_point(),
                    )
                } else {
                    None
                }
            });
        }
        self.render(ui);
    }
}

fn restore_zoom(ctx: &egui::Context, storage: &dyn eframe::Storage) {
    if let Some(zoom) = storage
        .get_string("versus.zoom")
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|zoom| zoom.is_finite() && (MIN_ZOOM..=MAX_ZOOM).contains(zoom))
    {
        ctx.set_zoom_factor(zoom);
    }
}

fn status_label(ui: &mut egui::Ui, text: &str, color: Color32) {
    ui.add(egui::Label::new(RichText::new(text).size(11.0).color(color)).truncate())
        .on_hover_text(text);
}

fn zoom_controls(ui: &mut egui::Ui) {
    let zoom_factor = ui.ctx().zoom_factor();
    let zoom = format!("Zoom {:.0}%", zoom_factor * 100.0);
    let shortcuts = [
        egui::gui_zoom::kb_shortcuts::ZOOM_IN,
        egui::gui_zoom::kb_shortcuts::ZOOM_OUT,
        egui::gui_zoom::kb_shortcuts::ZOOM_RESET,
    ]
    .map(|shortcut| ui.ctx().format_shortcut(&shortcut));
    if ui.add(egui::Button::new(RichText::new(zoom).color(Palette::for_context(ui.ctx()).muted)).frame(false))
        .on_hover_text(format!("{} to enlarge · {} to reduce · {} to reset\nCtrl + mouse wheel to zoom. Click to reset. Your zoom is saved for next time.", shortcuts[0], shortcuts[1], shortcuts[2]))
        .clicked() { ui.ctx().set_zoom_factor(1.0); }
    if icon_button(ui, ToolbarIcon::ZoomOut, zoom_factor > MIN_ZOOM, "Zoom out").clicked() {
        egui::gui_zoom::zoom_out(ui.ctx());
    }
    if icon_button(ui, ToolbarIcon::ZoomIn, zoom_factor < MAX_ZOOM, "Zoom in").clicked() {
        egui::gui_zoom::zoom_in(ui.ctx());
    }
}

fn comparison_row_height(ui: &egui::Ui) -> f32 {
    ROW_HEIGHT.max(ui.text_style_height(&egui::TextStyle::Monospace).ceil() + 8.0)
}

fn control_extent(ui: &egui::Ui) -> f32 {
    24.0_f32.max(ui.text_style_height(&egui::TextStyle::Button).ceil() + 10.0)
}

fn comparison_font(ctx: &egui::Context) -> FontId {
    egui::TextStyle::Monospace.resolve(&ctx.style_of(ctx.theme()))
}

fn recent_comparison_row(
    ui: &mut egui::Ui,
    entry: &RecentComparison,
    index: usize,
    palette: Palette,
) -> egui::Response {
    let paths = entry
        .paths
        .each_ref()
        .map(|path| path.display().to_string());
    let font = egui::TextStyle::Body.resolve(ui.style());
    let separator_font = FontId::new(font.size + 2.0, font.family.clone());
    let separator = ui
        .painter()
        .layout_no_wrap("vs".into(), separator_font, palette.text);
    let padding = 8.0;
    let gap = 10.0;
    let available =
        (ui.available_width() - padding * 2.0 - gap * 2.0 - separator.size().x).max(1.0);
    let mut labels = paths.each_ref().map(|path| {
        ui.painter()
            .layout_no_wrap(path.clone(), font.clone(), palette.text)
    });
    if labels.iter().map(|label| label.size().x).sum::<f32>() > available {
        // Keep both paths readable at narrow widths instead of hiding one side.
        labels = paths.each_ref().map(|path| {
            ui.painter()
                .layout(path.clone(), font.clone(), palette.text, available / 2.0)
        });
    }
    let height = labels
        .iter()
        .map(|label| label.size().y)
        .fold(separator.size().y, f32::max)
        + padding * 2.0;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(
            ui.available_width(),
            height.max(ui.spacing().interact_size.y),
        ),
        Sense::hover(),
    );
    let response = ui.interact(
        rect,
        egui::Id::new(("recent-comparison", entry.mode.source_label(), index)),
        Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            format!("{} vs {}", paths[0], paths[1]),
        )
    });
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(
        rect,
        0,
        if response.hovered() || response.is_pointer_button_down_on() {
            palette.hover
        } else if index % 2 == 0 {
            palette.panel
        } else {
            palette.alternate
        },
    );
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            0,
            Stroke::new(1.0, palette.accent),
            egui::StrokeKind::Inside,
        );
    }
    let left = egui::pos2(
        rect.left() + padding,
        rect.center().y - labels[0].size().y / 2.0,
    );
    let versus = egui::pos2(
        left.x + labels[0].size().x + gap,
        rect.center().y - separator.size().y / 2.0,
    );
    let right = egui::pos2(
        versus.x + separator.size().x + gap,
        rect.center().y - labels[1].size().y / 2.0,
    );
    painter.galley(left, labels[0].clone(), palette.text);
    // Egui's default proportional font has no bold face. A small overprint gives
    // just the separator a heavier weight without adding a font dependency.
    painter.galley(versus, separator.clone(), palette.text);
    painter.galley(versus + egui::vec2(0.5, 0.0), separator, palette.text);
    painter.galley(right, labels[1].clone(), palette.text);
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!(
            "{} vs {}\nRecompare these sources",
            paths[0], paths[1]
        ))
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
            label.text(),
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

fn workspace_mode_button(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.into(),
        FontId::proportional(11.0),
        Palette::for_context(ui.ctx()).text,
    );
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(galley.size().x + 12.0, 28.0), Sense::hover());
    let response = ui
        .interact(
            rect,
            egui::Id::new(("workspace-mode", label)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, label)
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
    ui.painter().galley_with_override_text_color(
        rect.center() - galley.size() * 0.5,
        galley,
        visuals.fg_stroke.color,
    );
    response
}

fn comparison_panel(ui: &egui::Ui) -> egui::Frame {
    let palette = Palette::for_context(ui.ctx());
    egui::Frame::default()
        .fill(palette.panel)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(6)
}
fn paint_panel_divider(ui: &mut egui::Ui) {
    let (divider, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment(
        [divider.left_center(), divider.right_center()],
        Stroke::new(1.0, Palette::for_context(ui.ctx()).border),
    );
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

fn file_path_headers(ui: &mut egui::Ui, paths: &[PathBuf; 2]) {
    let palette = Palette::for_context(ui.ctx());
    let (header, _) = ui.allocate_exact_size(
        egui::vec2(
            ui.available_width(),
            38.0_f32.max(comparison_row_height(ui) + 14.0),
        ),
        Sense::hover(),
    );
    for (side, half) in split_rect(header).into_iter().enumerate() {
        let rect = half.shrink(7.0);
        let painter = ui.painter().with_clip_rect(rect);
        paint_side_label(&painter, rect, side, palette);
        let path = paths[side].display().to_string();
        let path_rect = Rect::from_min_max(rect.min + egui::vec2(40.0, 0.0), rect.max);
        let galley = left_elided_path(&painter, &path, path_rect.width(), palette.text);
        let position = egui::pos2(path_rect.left(), rect.center().y - galley.size().y / 2.0);
        painter.galley(position, galley, palette.text);
        ui.interact(path_rect, ui.id().with(("file-path", side)), Sense::hover())
            .on_hover_text(path);
    }
    ui.painter().line_segment(
        [header.center_top(), header.center_bottom()],
        Stroke::new(1.0, palette.border),
    );
}

fn left_elided_path(
    painter: &egui::Painter,
    path: &str,
    width: f32,
    color: Color32,
) -> Arc<egui::Galley> {
    let layout = |text: String| painter.layout_no_wrap(text, comparison_font(painter.ctx()), color);
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
    gutter_extra: f32,
) {
    let palette = Palette::for_context(painter.ctx());
    let painter = painter.with_clip_rect(rect);
    let color = state_color(state, palette);
    file_style::paint_background(&painter, rect, state, alternate, selected);
    let Some((number, text)) = line else {
        painter.text(
            rect.left_center() + egui::vec2(file_style::TEXT_OFFSET - 2.0 + gutter_extra, 0.0),
            Align2::LEFT_CENTER,
            "—",
            comparison_font(painter.ctx()),
            palette.muted,
        );
        return;
    };
    file_style::paint_gutter(&painter, rect, *number, state, gutter_extra);
    let mut job = egui::text::LayoutJob::default();
    file_style::append_text(
        &mut job,
        text,
        state,
        changed,
        palette,
        comparison_font(painter.ctx()),
        None,
    );
    let galley = painter.layout_job(job);
    let position = egui::pos2(
        rect.left() + file_style::TEXT_OFFSET + gutter_extra,
        rect.center().y - galley.size().y / 2.0,
    );
    let end_x = position.x + galley.size().x;
    painter.galley(position, galley, color);
    if let Some(ending) = ending {
        file_style::paint_ending(&painter, rect, end_x, ending, state);
    }
}

fn file_line_anchor(view: &FileView, index: Option<usize>) -> [Option<usize>; 2] {
    index
        .and_then(|index| view.comparison.as_ref()?.rows.get(index))
        .map(|row| {
            [
                row.left.as_ref().map(|line| line.0),
                row.right.as_ref().map(|line| line.0),
            ]
        })
        .unwrap_or([None; 2])
}

fn find_file_line(comparison: &FileComparison, anchor: [Option<usize>; 2]) -> Option<usize> {
    let side = if anchor[0].is_some() { 0 } else { 1 };
    let number = anchor[side]?;
    comparison.rows.iter().position(|row| {
        let line = if side == 0 { &row.left } else { &row.right };
        line.as_ref().is_some_and(|line| line.0 == number)
    })
}

fn update_visible_file_rows(view: &mut FileView, differences_only: bool) {
    let fraction = view.scroll_y % view.row_height;
    let anchor = view
        .visible_rows
        .get((view.scroll_y / view.row_height).floor() as usize)
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
        .unwrap_or_else(|| view.visible_rows.len().saturating_sub(1));
    view.scroll_y = if view.visible_rows.is_empty() {
        0.0
    } else {
        top as f32 * view.row_height + fraction
    };
}

fn progress_text(snapshot: versus::ProgressSnapshot) -> String {
    let stage = match snapshot.stage {
        ProgressStage::Scanning => "Scanning folders",
        ProgressStage::Reading => "Reading files",
        ProgressStage::ComparingFiles => "Comparing files",
        ProgressStage::ComparingLines => "Comparing lines",
        ProgressStage::Highlighting => "Highlighting differences",
        ProgressStage::BuildingTree => "Finalizing folder tree",
        ProgressStage::Finished => "Preparing comparison",
    };
    if let Some(fraction) = snapshot.fraction() {
        format!("{stage}… {:.0}% complete", fraction * 100.0)
    } else if snapshot.stage == ProgressStage::Scanning {
        format!("{stage}… {} entries found", snapshot.completed)
    } else {
        format!("{stage}…")
    }
}

fn is_file_difference(node: &TreeNode) -> bool {
    node.state != DirectoryEntryState::Same
        && (node.left.kind == Some(DirectoryEntryKind::File)
            || node.right.kind == Some(DirectoryEntryKind::File))
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusIcon {
    Different,
    LeftOnly,
    RightOnly,
    TypeMismatch,
    Error,
    Conflict,
}

impl StatusIcon {
    fn label(self) -> &'static str {
        match self {
            Self::Different => "Different",
            Self::LeftOnly => "Left only",
            Self::RightOnly => "Right only",
            Self::TypeMismatch => "Type mismatch",
            Self::Error => "Read error",
            Self::Conflict => "Conflict",
        }
    }
    fn color(self, palette: Palette) -> Color32 {
        match self {
            Self::Different => palette.changed,
            Self::LeftOnly => palette.left_only,
            Self::RightOnly => palette.right_only,
            Self::TypeMismatch | Self::Error | Self::Conflict => palette.error,
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

fn side_name(side: usize) -> &'static str {
    if side == 0 { "Left" } else { "Right" }
}

fn tree_row_accessible_label(node: &TreeNode, side: usize) -> String {
    let entry = if side == 0 { &node.left } else { &node.right };
    if !entry.exists {
        return format!(
            "{} side, {}: not present on this side; {}",
            side_name(side),
            node.relative_path.display(),
            state_label(&node.state)
        );
    }
    let kind = match entry.kind {
        Some(DirectoryEntryKind::Directory) => "folder",
        Some(DirectoryEntryKind::File) => "file",
        Some(DirectoryEntryKind::Symlink) => "symbolic link",
        Some(DirectoryEntryKind::Other) => "other item",
        None => "item",
    };
    format!(
        "{} {}: {}, {}, {}",
        side_name(side),
        kind,
        node.relative_path.display(),
        format_size(entry.size),
        state_label(&node.state),
    )
}

fn file_line_accessible_label(
    side: usize,
    line: Option<&(usize, String)>,
    state: &DirectoryEntryState,
) -> String {
    match line {
        Some((number, text)) => {
            const MAX_BYTES: usize = 4096;
            let mut end = text.len().min(MAX_BYTES);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            let excerpt = if text.is_empty() {
                "blank"
            } else {
                &text[..end]
            };
            let omitted = if end < text.len() {
                format!(" … ({} bytes omitted)", text.len() - end)
            } else {
                String::new()
            };
            format!(
                "{} line {number}: {}, {excerpt}{omitted}",
                side_name(side),
                state_label(state)
            )
        }
        None => format!(
            "{}: no corresponding line, {}",
            side_name(side),
            state_label(state)
        ),
    }
}

fn state_color(state: &DirectoryEntryState, palette: Palette) -> Color32 {
    status_icon(state).map_or(palette.muted, |icon| icon.color(palette))
}

fn state_bar_color(state: &DirectoryEntryState, palette: Palette, ctx: &egui::Context) -> Color32 {
    let color = state_color(state, palette);
    if ctx.theme() != egui::Theme::Light {
        return color;
    }
    // Thin bars need saturated shades against the light background.
    match state {
        DirectoryEntryState::Different => Color32::from_rgb(236, 150, 0),
        DirectoryEntryState::LeftOnly => Color32::from_rgb(0, 164, 187),
        DirectoryEntryState::RightOnly => Color32::from_rgb(148, 76, 232),
        DirectoryEntryState::TypeMismatch | DirectoryEntryState::Error(_) => {
            Color32::from_rgb(235, 55, 82)
        }
        DirectoryEntryState::Same => color,
    }
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
        StatusIcon::Conflict => {
            painter.rect_stroke(
                Rect::from_center_size(center, egui::vec2(12.0, 12.0)),
                2,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.line_segment([point(0.0, -3.0), point(0.0, 1.0)], stroke);
            painter.circle_filled(point(0.0, 3.0), 0.7, icon.color(palette));
        }
    }
}

const DIFF_LEGEND_ICONS: [StatusIcon; 5] = [
    StatusIcon::Different,
    StatusIcon::LeftOnly,
    StatusIcon::RightOnly,
    StatusIcon::TypeMismatch,
    StatusIcon::Error,
];

fn legend_label(icon: StatusIcon, counts: Option<[usize; 6]>) -> String {
    let index = DIFF_LEGEND_ICONS
        .iter()
        .position(|candidate| *candidate == icon);
    match (counts, index) {
        (Some(counts), Some(index)) => format!("{}  {}", icon.label(), counts[index + 1]),
        _ => icon.label().into(),
    }
}

fn legend_rows(
    ui: &egui::Ui,
    icons: &[StatusIcon],
    counts: Option<[usize; 6]>,
) -> Vec<Vec<(StatusIcon, String, f32)>> {
    legend_rows_in_width(ui, icons, counts, ui.available_width())
}

fn legend_rows_in_width(
    ui: &egui::Ui,
    icons: &[StatusIcon],
    counts: Option<[usize; 6]>,
    available: f32,
) -> Vec<Vec<(StatusIcon, String, f32)>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut width = 0.0;
    let gap = ui.spacing().item_spacing.x;
    let available = available.max(1.0);
    for icon in icons {
        let label = legend_label(*icon, counts);
        let text_width = ui
            .painter()
            .layout_no_wrap(
                label.clone(),
                FontId::proportional(10.0),
                Palette::for_context(ui.ctx()).text,
            )
            .size()
            .x;
        let item_width = (14.0 + gap + text_width + 5.0).min(available);
        if !row.is_empty() && width + gap + item_width > available {
            rows.push(std::mem::take(&mut row));
            width = 0.0;
        }
        if !row.is_empty() {
            width += gap;
        }
        width += item_width;
        row.push((*icon, label, item_width));
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

fn bottom_legend_height(ui: &egui::Ui, icons: &[StatusIcon], counts: Option<[usize; 6]>) -> f32 {
    let rows = legend_rows(ui, icons, counts).len();
    rows as f32 * control_extent(ui) + rows.saturating_sub(1) as f32 * ui.spacing().item_spacing.y
}

fn bottom_legend(ui: &mut egui::Ui, icons: &[StatusIcon], counts: Option<[usize; 6]>) -> f32 {
    let rows = legend_rows(ui, icons, counts);
    let height = bottom_legend_height(ui, icons, counts);
    let (area, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let palette = Palette::for_context(ui.ctx());
    for (index, row) in rows.into_iter().enumerate() {
        let gap = ui.spacing().item_spacing.x;
        let width =
            row.iter().map(|item| item.2).sum::<f32>() + row.len().saturating_sub(1) as f32 * gap;
        let mut x = area.center().x - width / 2.0;
        let y = area.top() + index as f32 * (control_extent(ui) + ui.spacing().item_spacing.y);
        for (icon, label, width) in row {
            let rect = Rect::from_min_size(egui::pos2(x, y), egui::vec2(width, control_extent(ui)));
            let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
            paint_status_icon(
                &painter,
                egui::pos2(rect.left() + 7.0, rect.center().y),
                icon,
            );
            painter.text(
                egui::pos2(rect.left() + 14.0 + gap, rect.center().y),
                Align2::LEFT_CENTER,
                &label,
                FontId::proportional(10.0),
                icon.color(palette),
            );
            ui.interact(
                rect,
                egui::Id::new(("legend", icon.label())),
                Sense::hover(),
            )
            .on_hover_text(&label)
            .widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label.clone())
            });
            x += width + gap;
        }
    }
    height
}

fn error_summary(ui: &mut egui::Ui, errors: &[String]) {
    let label = format!(
        "{} error{}",
        errors.len(),
        if errors.len() == 1 { "" } else { "s" }
    );
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let response = ui.add(
            egui::Label::new(
                RichText::new(&label)
                    .size(11.0)
                    .color(Palette::for_context(ui.ctx()).error),
            )
            .truncate(),
        );
        ui.interact(
            response.rect,
            egui::Id::new("workspace-errors"),
            Sense::hover(),
        )
        .on_hover_text(errors.join("\n\n"));
    });
}

struct WorkspaceFooterLayout {
    info: Rect,
    legend: Rect,
    status: Rect,
    dirty: Option<Rect>,
    save: Option<Rect>,
}

fn footer_side_width(ui: &egui::Ui, width: f32, with_save: bool, with_dirty: bool) -> f32 {
    let dirty_width = ui
        .painter()
        .layout_no_wrap(
            "Unsaved Changes".into(),
            FontId::proportional(11.0),
            Palette::for_context(ui.ctx()).changed,
        )
        .size()
        .x;
    let save_and_dirty = with_dirty.then(|| {
        dirty_width
            + 40.0
            + with_save
                .then(|| captioned_icon_width(ui, "Save Both") + ui.spacing().item_spacing.x)
                .unwrap_or_default()
    });
    (width * 0.20)
        .max(save_and_dirty.unwrap_or(if with_save { 160.0 } else { 128.0 }))
        .min(260.0)
        .min(width / 2.0)
}

fn workspace_footer_height(
    ui: &egui::Ui,
    with_save: bool,
    with_dirty: bool,
    icons: &[StatusIcon],
    counts: Option<[usize; 6]>,
) -> f32 {
    let width = ui.available_width();
    let legend_width = width - 2.0 * footer_side_width(ui, width, with_save, with_dirty);
    let rows = legend_rows_in_width(ui, icons, counts, legend_width)
        .len()
        .max(1);
    rows as f32 * control_extent(ui) + rows.saturating_sub(1) as f32 * ui.spacing().item_spacing.y
}

fn workspace_footer_layout(
    ui: &mut egui::Ui,
    with_save: bool,
    with_dirty: bool,
    icons: &[StatusIcon],
    counts: Option<[usize; 6]>,
) -> WorkspaceFooterLayout {
    let height = workspace_footer_height(ui, with_save, with_dirty, icons, counts);
    let extent = control_extent(ui);
    let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    // Symmetric side areas keep the legend centered. At high zoom its entries
    // wrap inside this same footer, with guidance, activity and Save alongside.
    let side = footer_side_width(ui, row.width(), with_save, with_dirty);
    let line_top = row.center().y - extent / 2.0;
    let line_bottom = line_top + extent;
    let info = Rect::from_min_max(
        egui::pos2(row.left(), line_top),
        egui::pos2(row.left() + side, line_bottom),
    );
    let legend = Rect::from_min_max(
        egui::pos2(info.right(), row.top()),
        egui::pos2(row.right() - side, row.bottom()),
    );
    ui.interact(legend, egui::Id::new("workspace-legend"), Sense::hover());
    let save = with_save.then(|| {
        let width = captioned_icon_width(ui, "Save Both");
        Rect::from_min_max(
            egui::pos2(row.right() - width.min(side), line_top),
            egui::pos2(row.right(), line_bottom),
        )
    });
    let dirty = with_dirty.then(|| {
        let right = save.map_or(row.right(), |save| {
            save.left() - ui.spacing().item_spacing.x
        });
        let width = ui
            .painter()
            .layout_no_wrap(
                "Unsaved Changes".into(),
                FontId::proportional(11.0),
                Palette::for_context(ui.ctx()).changed,
            )
            .size()
            .x
            .min((right - legend.right() - ui.spacing().item_spacing.x).max(0.0));
        Rect::from_min_max(
            egui::pos2(right - width, line_top),
            egui::pos2(right, line_bottom),
        )
    });
    let status = Rect::from_min_max(
        egui::pos2(legend.right(), line_top),
        egui::pos2(
            dirty.or(save).map_or(row.right(), |dirty| {
                dirty.left() - ui.spacing().item_spacing.x
            }),
            line_bottom,
        ),
    );
    WorkspaceFooterLayout {
        info,
        legend,
        status,
        dirty,
        save,
    }
}

fn footer_guidance(ui: &mut egui::Ui, text: &str) {
    let palette = Palette::for_context(ui.ctx());
    ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));
    ui.horizontal(|ui| {
        let available = ui.available_rect_before_wrap();
        ui.interact(
            available,
            egui::Id::new("workspace-guidance"),
            Sense::hover(),
        );
        ui.add(
            egui::Label::new(RichText::new(text).size(11.0).color(palette.muted))
                .truncate()
                .halign(egui::Align::LEFT),
        )
        .on_hover_text(text);
    });
}

fn bottom_status_area(ui: &mut egui::Ui, draw: impl FnOnce(&mut egui::Ui)) {
    let (row, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), control_extent(ui)),
        Sense::hover(),
    );
    let width = row.width().min(420.0);
    let rect = Rect::from_min_max(egui::pos2(row.right() - width, row.top()), row.max);
    ui.interact(rect, egui::Id::new("workspace-status"), Sense::hover());
    let mut status_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    status_ui.set_clip_rect(status_ui.clip_rect().intersect(rect));
    draw(&mut status_ui);
}

fn header_progress_label(ui: &mut egui::Ui, snapshot: versus::ProgressSnapshot, color: Color32) {
    let full = progress_text(snapshot);
    let width = ui
        .painter()
        .layout_no_wrap(full.clone(), FontId::proportional(11.0), color)
        .size()
        .x;
    let text = if width > ui.available_width() {
        snapshot
            .fraction()
            .map(|fraction| format!("{:.0}% complete", fraction * 100.0))
            .unwrap_or_else(|| full.clone())
    } else {
        full.clone()
    };
    ui.add_sized(
        ui.available_size(),
        egui::Label::new(RichText::new(text).size(11.0).color(color))
            .truncate()
            .halign(egui::Align::Center),
    )
    .on_hover_text(full);
}

fn header_status_label(ui: &mut egui::Ui, text: &str, color: Color32) {
    ui.add_sized(
        ui.available_size(),
        egui::Label::new(RichText::new(text).size(11.0).color(color))
            .truncate()
            .halign(egui::Align::Center),
    )
    .on_hover_text(text);
}

#[derive(Clone, Copy)]
enum ToolbarIcon {
    AcceptAllLeft,
    AcceptAllRight,
    Undo,
    Redo,
    ZoomIn,
    ZoomOut,
    Search,
    MatchCase,
    MatchWholeWord,
    UseRegularExpression,
    ReplaceNext,
    ReplaceAll,
    Save,
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
    icon_button_content(ui, icon, enabled, selected, label, None)
}

fn captioned_icon_width(ui: &egui::Ui, label: &str) -> f32 {
    let text =
        ui.painter()
            .layout_no_wrap(label.into(), FontId::proportional(11.0), Color32::WHITE);
    control_extent(ui) + ui.spacing().item_spacing.x + text.size().x + ui.spacing().button_padding.x
}

fn captioned_icon_button(
    ui: &mut egui::Ui,
    icon: ToolbarIcon,
    enabled: bool,
    label: &str,
) -> egui::Response {
    icon_button_content(ui, icon, enabled, false, label, Some(label))
}

fn icon_button_content(
    ui: &mut egui::Ui,
    icon: ToolbarIcon,
    enabled: bool,
    selected: bool,
    label: &str,
    caption: Option<&str>,
) -> egui::Response {
    let palette = Palette::for_context(ui.ctx());
    ui.add_enabled_ui(enabled, |ui| {
        let extent = control_extent(ui);
        let width = caption.map_or(extent, |text| captioned_icon_width(ui, text));
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, extent), Sense::hover());
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
        if response.is_pointer_button_down_on() {
            ui.painter()
                .rect_filled(rect, 3, palette.accent.gamma_multiply(0.35));
        } else if response.hovered() || response.has_focus() {
            ui.painter().rect_filled(rect, 3, palette.border);
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                3,
                Stroke::new(2.0, palette.accent),
                egui::StrokeKind::Inside,
            );
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
        let center = egui::pos2(rect.left() + extent / 2.0, rect.center().y);
        let point = |x, y| center + egui::vec2(x, y);
        let stroke = Stroke::new(1.4, color);
        match icon {
            ToolbarIcon::AcceptAllLeft | ToolbarIcon::AcceptAllRight => {
                let direction = if matches!(icon, ToolbarIcon::AcceptAllLeft) {
                    1.0
                } else {
                    -1.0
                };
                for y in [-3.0, 3.0] {
                    ui.painter().line_segment(
                        [point(-6.0 * direction, y), point(5.0 * direction, y)],
                        stroke,
                    );
                    ui.painter().line(
                        vec![
                            point(direction, y - 3.0),
                            point(5.0 * direction, y),
                            point(direction, y + 3.0),
                        ],
                        stroke,
                    );
                }
            }
            ToolbarIcon::Redo => {
                ui.painter().line(
                    vec![
                        point(6.0, -3.0),
                        point(-1.0, -3.0),
                        point(-5.0, 0.0),
                        point(-5.0, 4.0),
                        point(0.0, 6.0),
                    ],
                    stroke,
                );
                ui.painter().line(
                    vec![point(2.0, -7.0), point(6.0, -3.0), point(2.0, 1.0)],
                    stroke,
                );
            }
            ToolbarIcon::Undo => {
                ui.painter().line(
                    vec![
                        point(-6.0, -3.0),
                        point(1.0, -3.0),
                        point(5.0, 0.0),
                        point(5.0, 4.0),
                        point(0.0, 6.0),
                    ],
                    stroke,
                );
                ui.painter().line(
                    vec![point(-2.0, -7.0), point(-6.0, -3.0), point(-2.0, 1.0)],
                    stroke,
                );
            }
            ToolbarIcon::MatchCase
            | ToolbarIcon::MatchWholeWord
            | ToolbarIcon::UseRegularExpression
            | ToolbarIcon::ReplaceNext
            | ToolbarIcon::ReplaceAll => {
                let glyph = match icon {
                    ToolbarIcon::MatchCase => find_icons::FindIcon::MatchCase,
                    ToolbarIcon::MatchWholeWord => find_icons::FindIcon::MatchWholeWord,
                    ToolbarIcon::UseRegularExpression => find_icons::FindIcon::UseRegularExpression,
                    ToolbarIcon::ReplaceNext => find_icons::FindIcon::ReplaceNext,
                    ToolbarIcon::ReplaceAll => find_icons::FindIcon::ReplaceAll,
                    _ => unreachable!(),
                };
                find_icons::paint(ui, center, glyph, color);
            }
            ToolbarIcon::Save => {
                let rect = Rect::from_center_size(center, egui::vec2(14.0, 14.0));
                ui.painter()
                    .rect_stroke(rect, 1.0, stroke, egui::StrokeKind::Inside);
                ui.painter().rect_stroke(
                    Rect::from_min_max(
                        rect.min + egui::vec2(3.0, 0.0),
                        rect.min + egui::vec2(10.0, 5.0),
                    ),
                    0.0,
                    stroke,
                    egui::StrokeKind::Inside,
                );
                ui.painter().rect_stroke(
                    Rect::from_min_max(
                        rect.min + egui::vec2(3.0, 8.0),
                        rect.max - egui::vec2(3.0, 0.0),
                    ),
                    0.0,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
            ToolbarIcon::ZoomIn | ToolbarIcon::ZoomOut | ToolbarIcon::Search => {
                let lens = point(-2.0, -2.0);
                ui.painter().circle_stroke(lens, 5.5, stroke);
                ui.painter()
                    .line_segment([point(2.0, 2.0), point(7.0, 7.0)], stroke);
                if !matches!(icon, ToolbarIcon::Search) {
                    ui.painter()
                        .line_segment([point(-5.0, -2.0), point(1.0, -2.0)], stroke);
                }
                if matches!(icon, ToolbarIcon::ZoomIn) {
                    ui.painter()
                        .line_segment([point(-2.0, -5.0), point(-2.0, 1.0)], stroke);
                }
            }
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
        if let Some(caption) = caption {
            let text =
                ui.painter()
                    .layout_no_wrap(caption.into(), FontId::proportional(11.0), color);
            let position = egui::pos2(
                rect.left() + extent + ui.spacing().item_spacing.x,
                rect.center().y - text.size().y / 2.0,
            );
            ui.painter().galley(position, text, color);
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
            state_bar_color(state, palette, painter.ctx()),
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
            comparison_font(ui.ctx()),
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
        comparison_font(ui.ctx()),
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

// Shared by compare and merge so native targeting and feedback stay consistent.
fn native_drop_frame(
    ui: &egui::Ui,
    native_position: Option<Pos2>,
) -> (bool, Option<Pos2>, Vec<egui::DroppedFileHandle>) {
    let (hovering, frame_position, retained_position) = ui.input(|input| {
        (
            !input.raw.hovered_files.is_empty(),
            input.raw.events.iter().rev().find_map(|event| match event {
                egui::Event::PointerMoved(position) => Some(*position),
                _ => None,
            }),
            input.pointer.interact_pos(),
        )
    });
    let position = frame_position.or(native_position).or(retained_position);
    let dropped = ui
        .ctx()
        .input_mut(|input| std::mem::take(&mut input.raw.dropped_files));
    (hovering, position, dropped)
}

fn paint_drop_outline(painter: &egui::Painter, target: Rect, color: Color32) {
    let corners = [
        target.left_top(),
        target.right_top(),
        target.right_bottom(),
        target.left_bottom(),
        target.left_top(),
    ];
    painter.extend(egui::Shape::dashed_line(
        &corners,
        Stroke::new(1.0, color),
        6.0,
        4.0,
    ));
}

fn paint_native_drop_highlight(ctx: &egui::Context, pane: Rect, message: &str) {
    let rect = pane.shrink(3.0);
    let palette = Palette::for_context(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("native-drop-highlight"),
    ));
    painter.rect_filled(rect, 4, palette.accent.gamma_multiply(0.1));
    painter.rect_stroke(
        rect,
        4,
        Stroke::new(3.0, palette.accent),
        egui::StrokeKind::Inside,
    );
    let label =
        painter.layout_no_wrap(message.to_owned(), FontId::proportional(16.0), palette.text);
    let label_rect = Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + 64.0),
        label.size() + egui::vec2(24.0, 16.0),
    );
    painter.rect_filled(label_rect, 4, palette.panel);
    painter.galley(
        label_rect.center() - label.size() * 0.5,
        label,
        palette.text,
    );
}

fn paint_empty_drop_target(ui: &mut egui::Ui, rect: Rect, side: usize, chooser: bool) {
    let palette = Palette::for_context(ui.ctx());
    let painter = ui.painter().with_clip_rect(rect);
    let target = rect.shrink(8.0);
    paint_drop_outline(&painter, target, palette.border);
    ui.interact(
        target,
        ui.id().with(("empty-drop-target", side)),
        Sense::hover(),
    )
    .widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Label,
            true,
            format!(
                "{}: Drop a file or folder here, or use Browse above.",
                side_name(side)
            ),
        )
    });
    painter.text(
        target.center() - egui::vec2(0.0, if chooser { 12.0 } else { -24.0 }),
        Align2::CENTER_CENTER,
        "Drop a file or folder here",
        FontId::proportional(14.0),
        palette.muted,
    );
    painter.text(
        target.center() + egui::vec2(0.0, if chooser { 12.0 } else { 48.0 }),
        Align2::CENTER_CENTER,
        "or use Browse above",
        FontId::proportional(11.0),
        palette.muted,
    );
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
    {
        let chooser = title.starts_with("Choose ");
        for (side, half) in split_rect(rect).into_iter().enumerate() {
            paint_empty_drop_target(ui, half, side, chooser);
        }
        if chooser {
            return;
        }
    }
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
    // Support literal Ctrl on every platform as well as the platform command key.
    ctx.options_mut(|options| {
        options.input_options.zoom_modifier = egui::Modifiers::CTRL | egui::Modifiers::COMMAND
    });
    set_theme(ctx, egui::Theme::Dark);
}

fn set_theme(ctx: &egui::Context, theme: egui::Theme) {
    let installed = egui::Id::new("versus-codicons-installed");
    if !ctx.data(|data| data.get_temp::<bool>(installed).unwrap_or(false)) {
        let mut fonts = egui::FontDefinitions::default();
        find_icons::register(&mut fonts);
        ctx.set_fonts(fonts);
        ctx.data_mut(|data| data.insert_temp(installed, true));
    }
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
    visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
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
