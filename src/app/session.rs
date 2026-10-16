//! Session state, source/mode transitions and background result publication.
//!
//! Rendering lives in the parent. Workers publish only through the currently
//! owned job; replacing or dropping a job cancels/disconnects its old result.

use super::{
    FolderBrowser, InteractiveView, absolute_native_path, absolute_path, empty_diff_side,
    file_style,
};
use crate::app::visuals::status::StatusIcon;
use crate::app::visuals::theme::ROW_HEIGHT;
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

// Capabilities are separate so an explorer can start with the transition or
// worker it needs without tracing the retained session state through one file.
mod comparison;
mod history;
mod merge_workspace;
mod navigation;
pub(super) use comparison::is_file_difference;
pub(super) use navigation::{file_line_anchor, find_file_line, update_visible_file_rows};
mod source;

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

pub(in crate::app) use history::RecentStore;

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
