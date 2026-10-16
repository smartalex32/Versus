//! Presentation for a retained, two-way interactive merge workspace.
use super::*;
mod projection;
use projection::{
    EditorProjection, MergeDecoration, MergePresentation, MergeProjection,
    PendingMergePresentation, editor_byte_edit, merge_projection,
    original_byte_to_normalized_chars, projected_editor_edit, visible_chunk_runs,
};
use std::{
    collections::BTreeMap,
    ops::Range,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
};
use versus::{
    ComparisonProgress, FileComparison, FileViewOptions, HistoryTag, InteractiveDocument,
    TransferPlacement, TwoWaySide, TwoWayWorkspace, WorkingFile, compare_loaded_text,
    load_working_file, save_working_file,
};

#[derive(Clone)]
struct CachedPair {
    scroll: [f32; 2],
    caret: [usize; 2],
    expanded: [Vec<u64>; 2],
}
#[derive(Clone)]
struct RetainedFile {
    file: WorkingFile,
    document: InteractiveDocument,
}
struct DiffJob {
    receiver: Receiver<Result<Option<FileComparison>, String>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<ComparisonProgress>,
}
impl Drop for DiffJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
struct SaveJob {
    receiver: Receiver<Vec<Result<WorkingFile, String>>>,
}
struct SideLoad {
    receiver: Receiver<Result<WorkingFile, String>>,
    path: PathBuf,
}

/// Derived geometry for the read-only, File Diff-style renderer.  The loaded
/// comparison is immutable while it is displayed, so calculating this once per
/// comparison/preference/font update avoids doing work proportional to every
/// file line on every frame.
struct ReadonlyPresentation {
    rows: Vec<usize>,
    content_widths: [f32; 2],
    preferences: [bool; 3],
    font: FontId,
    row_height: f32,
    gutter_extra: f32,
}

#[derive(Default)]
pub(super) struct InteractiveView {
    workspace: Option<TwoWayWorkspace>,
    files: Option<[WorkingFile; 2]>,
    paths: Option<[PathBuf; 2]>,
    cache: BTreeMap<[PathBuf; 2], CachedPair>,
    documents: BTreeMap<PathBuf, RetainedFile>,
    aliases: BTreeMap<PathBuf, PathBuf>,
    batches: BTreeMap<u64, Vec<PathBuf>>,
    next_batch: u64,
    batch_undo: Vec<u64>,
    batch_redo: Vec<u64>,
    loaded_paths: Option<[PathBuf; 2]>,
    loading: Option<(Receiver<Result<[WorkingFile; 2], String>>, [PathBuf; 2])>,
    side_sources: [Option<PathBuf>; 2],
    side_files: [Option<WorkingFile>; 2],
    side_loading: [Option<SideLoad>; 2],
    loaded_sources: Option<[Option<PathBuf>; 2]>,
    diff_job: Option<DiffJob>,
    retired_diffs: Vec<DiffJob>,
    diff_not_before: Option<Instant>,
    diff: Option<FileComparison>,
    presentation: MergePresentation,
    readonly_presentation: Option<ReadonlyPresentation>,
    readonly_pending_anchor: Option<([Option<usize>; 2], f32)>,
    save_job: Option<SaveJob>,
    external_save_busy: bool,
    editing: bool,
    diff_options: [bool; 3],
    merge_options: [bool; 3],
    // Merge starts with no comparison work.  Once the user asks to run a
    // comparison, all three helpers start enabled; later changes belong to
    // that Merge session and must survive toggling the worker off and on.
    merge_options_initialized: bool,
    run_diff: bool,
    diff_dirty: bool,
    diff_rows: Vec<usize>,
    folder_overview: bool,
    folder_differences: Arc<Vec<PathBuf>>,
    folder_difference_index: Option<usize>,
    folder_jump: Option<PathBuf>,
    expanded: [Vec<u64>; 2],
    caret: [usize; 2],
    scroll: [f32; 2],
    pending_navigation_scroll: [bool; 2],
    generation: u64,
    side_generation: [u64; 2],
    selected_side: usize,
    back: bool,
    new_requested: bool,
    search_requested: bool,
    from_folders: bool,
    editor_context: Option<egui::Context>,
    pending_save: Option<Vec<usize>>,
    save_all_requested: bool,
    error: Option<String>,
    status: String,
}

mod diff;
mod document;
mod editing_actions;
mod editor;
mod history;
mod line_rendering;
mod readonly;
mod toolbar;

use document::{external_refresh_edit, retained_file};
use line_rendering::{append_code_line, inline_action, transfer_action};

#[cfg(test)]
#[path = "../../tests/unit/app/interactive_view/mod.rs"]
mod tests;

pub(super) fn side_of(side: usize) -> TwoWaySide {
    if side == 0 {
        TwoWaySide::Left
    } else {
        TwoWaySide::Right
    }
}
pub(super) fn side_index(side: TwoWaySide) -> usize {
    if side == TwoWaySide::Left { 0 } else { 1 }
}
