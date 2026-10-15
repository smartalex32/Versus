//! Presentation for a retained, two-way interactive merge workspace.
use super::*;
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

/// Display-only source coordinates retained during an editable diff refresh.
/// These never stand in for a completed comparison or supply merge targets.
#[derive(Clone, Default)]
struct MergeDecoration {
    lines: BTreeMap<usize, (DirectoryEntryState, Vec<Range<usize>>)>,
    endings: BTreeMap<usize, versus::DisplayLineEnding>,
}

struct PendingMergePresentation {
    sides: [MergeDecoration; 2],
    changes: usize,
    legend_counts: Option<[usize; 6]>,
}

impl MergeDecoration {
    /// Translate display metadata using the actual edit anchor, without doing a
    /// comparison. Repaint touched lines provisionally until fresh results arrive.
    fn rebase(&mut self, before: &str, range: Range<usize>, after: &str) {
        let old = text_segments(before);
        let new = text_segments(after);
        let line_at = |byte: usize| {
            let mut offset = 0;
            for (line, (text, ending)) in old.iter().enumerate() {
                offset += text.len() + ending.len();
                if byte < offset || (byte == offset && ending.is_empty()) {
                    return line;
                }
            }
            old.len()
        };
        let start = line_at(range.start);
        let end = (line_at(range.end) + 1).min(old.len());
        let delta = new.len() as isize - old.len() as isize;
        let new_end = end.saturating_add_signed(delta).min(new.len());
        let previous = self.lines.get(&start).cloned();
        let ending = (end == start + 1 && new_end == end && old[start].1 == new[start].1)
            .then(|| self.endings.get(&start).copied())
            .flatten();
        let inline = if end == start + 1 && new_end == end {
            let line_start: usize = old[..start]
                .iter()
                .map(|(text, ending)| text.len() + ending.len())
                .sum();
            (range.end <= line_start + old[start].0.len()).then(|| {
                let edit = range.start - line_start..range.end - line_start;
                let inserted = new[start].0.len() + edit.len() - old[start].0.len();
                let shift = inserted as isize - edit.len() as isize;
                let mut ranges: Vec<Range<usize>> = previous
                    .as_ref()
                    .into_iter()
                    .flat_map(|(_, ranges)| ranges)
                    .map(|span| {
                        let a = if span.start <= edit.start {
                            span.start
                        } else if span.start >= edit.end {
                            span.start.saturating_add_signed(shift)
                        } else {
                            edit.start
                        };
                        let b = if span.end <= edit.start {
                            span.end
                        } else if span.end >= edit.end {
                            span.end.saturating_add_signed(shift)
                        } else {
                            edit.start + inserted
                        };
                        a..b
                    })
                    .filter(|span| !span.is_empty())
                    .collect();
                if inserted > 0 {
                    ranges.push(edit.start..edit.start + inserted);
                }
                ranges.sort_by_key(|span| span.start);
                let mut merged: Vec<Range<usize>> = Vec::new();
                for span in ranges {
                    if let Some(last) = merged.last_mut().filter(|last| span.start <= last.end) {
                        last.end = last.end.max(span.end);
                    } else {
                        merged.push(span);
                    }
                }
                merged
            })
        } else {
            None
        };
        let remap = |line: usize| {
            if line < start {
                Some(line)
            } else if line >= end {
                Some(line.saturating_add_signed(delta))
            } else {
                None
            }
        };
        self.lines = std::mem::take(&mut self.lines)
            .into_iter()
            .filter_map(|(line, value)| Some((remap(line)?, value)))
            .collect();
        self.endings = std::mem::take(&mut self.endings)
            .into_iter()
            .filter_map(|(line, value)| Some((remap(line)?, value)))
            .collect();
        if let Some(ending) = ending {
            self.endings.insert(start, ending);
        }
        for (line, (text, _)) in new.iter().enumerate().take(new_end).skip(start) {
            self.lines.insert(
                line,
                (
                    previous
                        .as_ref()
                        .map(|(state, _)| state)
                        .filter(|state| **state != DirectoryEntryState::Same)
                        .cloned()
                        .unwrap_or(DirectoryEntryState::Different),
                    if let Some(inline) = &inline {
                        inline.clone()
                    } else if text.is_empty() {
                        Vec::new()
                    } else {
                        vec![0..text.len()]
                    },
                ),
            );
        }
    }
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
    pending_merge_presentation: Option<PendingMergePresentation>,
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
    editor_projection: [Option<EditorProjection>; 2],
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
impl InteractiveView {
    /// Opening files is independent from comparisons. Missing paths create empty,
    /// saveable working copies so one-sided directory entries remain mergeable.
    pub(super) fn open_pair(&mut self, paths: [PathBuf; 2]) {
        if self.paths.as_ref() == Some(&paths) && self.loading.is_none() {
            return;
        }
        self.publish_active();
        self.cache_active();
        self.readonly_pending_anchor = None;
        self.invalidate_diff();
        self.side_sources = [Some(paths[0].clone()), Some(paths[1].clone())];
        self.loading = None;
        if paths.iter().all(|path| {
            self.aliases.contains_key(path) && self.documents.contains_key(&self.key_for(path))
        }) {
            self.activate_pair(paths);
            self.status = "Restored retained working copies.".into();
            return;
        }
        let retained = self.export_working_files();
        let requested = paths.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let retained_by_identity: BTreeMap<_, _> = retained
                .values()
                .map(|file| (file.identity_key(), file.clone()))
                .collect();
            let load = |path: &PathBuf| -> Result<WorkingFile, CompareError> {
                if let Some(file) = retained.get(path) {
                    return Ok(file.clone());
                }
                let identity = versus::resolve_working_file_identity(path)?;
                if let Some(file) = retained_by_identity.get(&identity) {
                    Ok(file.clone())
                } else {
                    load_working_file(Some(path))
                }
            };
            let result = match [load(&requested[0]), load(&requested[1])] {
                [Ok(left), Ok(right)] => Ok([left, right]),
                [Err(error), _] | [_, Err(error)] => Err(error.to_string()),
            };
            let _ = sender.send(result);
        });
        self.loading = Some((receiver, paths));
        self.status = "Loading selected working copies…".into();
        self.error = None;
    }
    /// Opens either editor independently. An absent side is deliberately not
    /// represented by a synthetic path or working file.
    pub(super) fn open_sources(&mut self, sources: [Option<PathBuf>; 2]) {
        for (side, source) in sources.into_iter().enumerate() {
            self.open_side(side, source);
        }
    }
    pub(super) fn open_side(&mut self, side: usize, path: Option<PathBuf>) {
        debug_assert!(side < 2);
        if self.side_sources[side] == path {
            return;
        }
        self.publish_active();
        self.readonly_pending_anchor = None;
        self.invalidate_diff();
        if let Some(files) = self.files.take() {
            for (index, file) in files.into_iter().enumerate() {
                if self.side_sources[index].is_some() {
                    self.side_files[index] = Some(file);
                }
            }
        }
        self.paths = None;
        self.loading = None;
        self.side_loading[side] = None;
        self.side_sources[side] = path.clone();
        self.side_files[side] = None;
        self.side_generation[side] = self.side_generation[side].wrapping_add(1);
        self.scroll[side] = 0.0;
        self.pending_navigation_scroll[side] = false;
        self.caret[side] = 0;
        self.expanded[side].clear();
        self.activate_side_workspace();
        let Some(path) = path else {
            self.loaded_sources = Some(self.side_sources.clone());
            return;
        };
        if self.aliases.contains_key(&path) && self.documents.contains_key(&self.key_for(&path)) {
            self.side_files[side] = Some(self.documents[&self.key_for(&path)].file.clone());
            self.activate_side_workspace();
            self.loaded_sources = Some(self.side_sources.clone());
            return;
        }
        let retained = self.export_working_files();
        let requested = path.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let retained_by_identity: BTreeMap<_, _> = retained
                .values()
                .map(|file| (file.identity_key(), file.clone()))
                .collect();
            let result = if let Some(file) = retained.get(&requested) {
                Ok(file.clone())
            } else {
                versus::resolve_working_file_identity(&requested).and_then(|identity| {
                    retained_by_identity
                        .get(&identity)
                        .cloned()
                        .map(Ok)
                        .unwrap_or_else(|| load_working_file(Some(&requested)))
                })
            };
            let _ = sender.send(result.map_err(|error| error.to_string()));
        });
        self.side_loading[side] = Some(SideLoad { receiver, path });
        self.status = "Loading selected file…".into();
        self.error = None;
    }
    pub(super) fn set_editing(&mut self, editing: bool, diff_preferences: [bool; 3]) {
        let changed =
            self.editing != editing || (!editing && self.diff_options != diff_preferences);
        self.editing = editing;
        if editing {
            self.readonly_pending_anchor = None;
        }
        self.diff_options = diff_preferences;
        if changed {
            self.invalidate_diff();
        }
    }
    pub(super) fn set_run_diff(&mut self, enabled: bool) {
        if self.run_diff != enabled {
            if enabled && self.editing && !self.merge_options_initialized {
                self.merge_options = [true; 3];
                self.merge_options_initialized = true;
            }
            self.run_diff = enabled;
            self.invalidate_diff();
        }
    }
    /// A refresh only recomputes the current retained working copies.  It must
    /// never reload the paths, because a Merge workspace may contain edits that
    /// have not been saved yet.
    pub(super) fn refresh_diff(&mut self) {
        self.invalidate_diff();
        self.diff_not_before = None;
    }
    pub(super) fn can_refresh_diff(&self) -> bool {
        self.has_actual_pair()
            && self.loading.is_none()
            && self.side_loading.iter().all(Option::is_none)
            && self.diff_job.is_none()
    }
    fn engine_enabled(&self) -> bool {
        !self.editing || self.run_diff
    }
    pub(super) fn is_saving(&self) -> bool {
        self.save_job.is_some() || self.external_save_busy
    }
    pub(super) fn set_external_save_busy(&mut self, busy: bool) {
        self.external_save_busy = busy;
    }
    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        self.editor_context = Some(ctx.clone());
        self.publish_active();
        let loaded = self
            .loading
            .as_ref()
            .and_then(|(receiver, _)| match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => Some(Err("Working-copy loader stopped".into())),
                Err(TryRecvError::Empty) => None,
            });
        if let Some(result) = loaded {
            let (_, paths) = self.loading.take().unwrap();
            match result {
                Ok([left, right]) => {
                    for (side, file) in [left, right].into_iter().enumerate() {
                        self.register_file(paths[side].clone(), file);
                    }
                    self.activate_pair(paths);
                    self.status.clear();
                }
                Err(error) => self.error = Some(error),
            }
        }
        for side in 0..2 {
            let result =
                self.side_loading[side]
                    .as_ref()
                    .and_then(|load| match load.receiver.try_recv() {
                        Ok(result) => Some(result),
                        Err(TryRecvError::Disconnected) => Some(Err("File loader stopped".into())),
                        Err(TryRecvError::Empty) => None,
                    });
            let Some(result) = result else { continue };
            let load = self.side_loading[side].take().unwrap();
            if self.side_sources[side].as_ref() != Some(&load.path) {
                continue;
            }
            match result {
                Ok(file) => {
                    self.register_file(load.path, file.clone());
                    self.side_files[side] = Some(file);
                    self.activate_side_workspace();
                    self.loaded_sources = Some(self.side_sources.clone());
                    self.error = None;
                    self.status.clear();
                }
                Err(error) => self.error = Some(error),
            }
        }
        if self.loading.is_some() || self.side_loading.iter().any(Option::is_some) {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
        }
        self.poll_diff(ctx);
        self.poll_save(ctx);
    }
    pub(super) fn any_dirty(&self) -> bool {
        self.workspace
            .as_ref()
            .is_some_and(TwoWayWorkspace::has_unsaved_changes)
            || self
                .documents
                .values()
                .any(|retained| retained.document.is_dirty())
    }
    pub(super) fn take_back_requested(&mut self) -> bool {
        std::mem::take(&mut self.back)
    }
    pub(super) fn run_diff_enabled(&self) -> bool {
        self.run_diff
    }
    pub(super) fn merge_options(&self) -> [bool; 3] {
        self.merge_options
    }
    pub(super) fn set_from_folders(&mut self, from_folders: bool) {
        self.from_folders = from_folders;
    }
    #[cfg(test)]
    pub(super) fn current_paths(&self) -> Option<&[PathBuf; 2]> {
        self.paths.as_ref()
    }
    pub(super) fn take_loaded_paths(&mut self) -> Option<[PathBuf; 2]> {
        self.loaded_paths.take()
    }
    pub(super) fn take_loaded_sources(&mut self) -> Option<[Option<PathBuf>; 2]> {
        self.loaded_sources.take()
    }
    pub(super) fn current_sources(&self) -> [Option<PathBuf>; 2] {
        self.side_sources.clone()
    }
    pub(super) fn pair_paths(&self) -> Option<&[PathBuf; 2]> {
        self.loading
            .as_ref()
            .map(|(_, paths)| paths)
            .or(self.paths.as_ref())
    }
    pub(super) fn selected_source(&self, side: usize) -> Option<&PathBuf> {
        self.side_sources[side].as_ref()
    }
    pub(super) fn take_new_requested(&mut self) -> bool {
        std::mem::take(&mut self.new_requested)
    }
    pub(super) fn take_search_requested(&mut self) -> bool {
        std::mem::take(&mut self.search_requested)
    }
    pub(super) fn is_side_dirty(&self, side: usize) -> bool {
        self.workspace
            .as_ref()
            .is_some_and(|workspace| workspace.is_dirty(side_of(side)))
    }
    pub(super) fn can_save_side(&self, side: usize) -> bool {
        self.editing
            && !self.is_saving()
            && self.side_has_file(side)
            && (self.is_side_dirty(side)
                || self.active_file(side).is_some_and(|file| {
                    matches!(file.fingerprint, versus::FileFingerprint::Missing)
                }))
    }
    /// Folder-wide replacement can make retained documents dirty while they
    /// are not open in either editor. The app routes this explicit Save Both
    /// request through its search owner so every retained copy is saved.
    pub(super) fn has_retained_dirty_documents(&self) -> bool {
        self.documents.iter().any(|(identity, retained)| {
            retained.document.is_dirty()
                && !self
                    .side_sources
                    .iter()
                    .flatten()
                    .chain(self.paths.iter().flat_map(|paths| paths.iter()))
                    .any(|source| self.key_for(source) == *identity)
        })
    }
    pub(super) fn take_save_all_requested(&mut self) -> bool {
        std::mem::take(&mut self.save_all_requested)
    }
    pub(super) fn request_side_save(&mut self, side: usize) {
        if self.can_save_side(side) {
            self.request_save(vec![side]);
        }
    }
    pub(super) fn clear_error(&mut self) {
        if self.error.take().is_some() {
            self.status.clear();
        }
    }
    pub(super) fn error_details(&self) -> Option<&str> {
        self.error.as_deref()
    }
    #[cfg(test)]
    pub(super) fn working_text(&self, side: usize) -> Option<&str> {
        self.workspace.as_ref().map(|w| w.text(side_of(side)))
    }
    #[cfg(test)]
    #[allow(dead_code)]
    pub(super) fn editor_id(&self, side: usize) -> egui::Id {
        egui::Id::new(("two-way-edit", self.side_generation[side], side))
    }
    #[cfg(test)]
    pub(super) fn set_working_text_for_test(&mut self, side: usize, text: &str) {
        self.workspace
            .as_mut()
            .expect("loaded test file has a workspace")
            .set_text(side_of(side), text);
    }
    fn key_for(&self, path: &std::path::Path) -> PathBuf {
        self.aliases
            .get(path)
            .cloned()
            .unwrap_or_else(|| file_key(path))
    }
    fn register_file(&mut self, path: PathBuf, file: WorkingFile) {
        let key = file.identity_key();
        self.aliases.insert(path, key.clone());
        if let Some(original) = &file.path {
            self.aliases.insert(original.clone(), key.clone());
        }
        self.aliases.insert(key.clone(), key.clone());
        self.documents
            .entry(key)
            .or_insert_with(|| retained_file(file));
    }
    pub(super) fn export_working_files(&self) -> BTreeMap<PathBuf, WorkingFile> {
        let mut output = BTreeMap::new();
        for retained in self.documents.values() {
            let mut file = retained.file.clone();
            file.text = retained.document.text().into();
            if let Some(path) = &file.path {
                output.insert(path.clone(), file);
            }
        }
        if let (Some(paths), Some(files), Some(workspace)) =
            (&self.paths, &self.files, &self.workspace)
        {
            for side in 0..2 {
                let mut file = files[side].clone();
                file.text = workspace.text(side_of(side)).into();
                output.insert(paths[side].clone(), file);
            }
        }
        output
    }
    pub(super) fn folder_revision_signature(&self) -> u64 {
        self.documents
            .values()
            .map(|retained| retained.document.revision())
            .chain(self.workspace.iter().flat_map(|workspace| {
                [
                    workspace.document(TwoWaySide::Left).revision(),
                    workspace.document(TwoWaySide::Right).revision(),
                ]
            }))
            .max()
            .unwrap_or(0)
    }
    pub(super) fn folder_document_revisions(&self) -> BTreeMap<PathBuf, u64> {
        let mut revisions: BTreeMap<_, _> = self
            .documents
            .iter()
            .map(|(identity, retained)| (identity.clone(), retained.document.revision()))
            .collect();
        if let (Some(files), Some(workspace)) = (&self.files, &self.workspace) {
            for side in 0..2 {
                revisions.insert(
                    files[side].identity_key(),
                    workspace.document(side_of(side)).revision(),
                );
            }
        }
        revisions
    }
    pub(super) fn loaded_pair_comparison(&self) -> Option<([PathBuf; 2], DirectoryEntryState)> {
        (self.engine_enabled() && self.diff.is_some()).then(|| {
            (
                self.paths.clone().unwrap(),
                if self.diff_rows.is_empty()
                    && self
                        .diff
                        .as_ref()
                        .is_some_and(|diff| diff.message.is_none())
                {
                    DirectoryEntryState::Same
                } else {
                    DirectoryEntryState::Different
                },
            )
        })
    }
    fn publish_active(&mut self) {
        if let Some(workspace) = &self.workspace {
            for side in 0..2 {
                let Some(path) = &self.side_sources[side] else {
                    continue;
                };
                let key = self.key_for(path);
                if let Some(retained) = self.documents.get_mut(&key) {
                    let document = workspace.document(side_of(side));
                    if document.revision() > retained.document.revision() {
                        retained.document = document.clone();
                    }
                }
            }
        }
        let (Some(paths), Some(files), Some(workspace)) =
            (&self.paths, &self.files, &self.workspace)
        else {
            return;
        };
        for side in 0..2 {
            let key = files[side].identity_key();
            self.aliases.insert(paths[side].clone(), key.clone());
            self.aliases.insert(key.clone(), key.clone());
            let document = workspace.document(side_of(side));
            match self.documents.get_mut(&key) {
                Some(retained) if document.revision() > retained.document.revision() => {
                    retained.document = document.clone()
                }
                None => {
                    self.documents.insert(
                        key,
                        RetainedFile {
                            file: files[side].clone(),
                            document: document.clone(),
                        },
                    );
                }
                _ => {}
            }
        }
        // Aliases use the same canonical state rather than independently saved pair snapshots.
        let stale = (0..2).any(|side| {
            self.documents[&self.key_for(&paths[side])]
                .document
                .revision()
                != workspace.document(side_of(side)).revision()
        });
        if stale {
            self.refresh_active();
        }
    }
    fn refresh_active(&mut self) {
        let Some(paths) = &self.paths else {
            self.refresh_partial_active();
            return;
        };
        if !paths.iter().all(|path| {
            self.aliases.contains_key(path) && self.documents.contains_key(&self.key_for(path))
        }) {
            return;
        }
        let documents = std::array::from_fn(|side| {
            self.documents[&self.key_for(&paths[side])].document.clone()
        });
        self.files = Some(std::array::from_fn(|side| {
            self.documents[&self.key_for(&paths[side])].file.clone()
        }));
        self.workspace = Some(TwoWayWorkspace::from_documents(documents));
        for side in 0..2 {
            let count = self.workspace.as_ref().unwrap().line_count(side_of(side));
            self.caret[side] = self.caret[side].min(count);
        }
    }
    fn refresh_partial_active(&mut self) {
        let Some(workspace) = &self.workspace else {
            return;
        };
        let documents = std::array::from_fn(|side| {
            self.side_sources[side]
                .as_ref()
                .and_then(|path| self.documents.get(&self.key_for(path)))
                .map(|retained| retained.document.clone())
                .unwrap_or_else(|| workspace.document(side_of(side)).clone())
        });
        self.workspace = Some(TwoWayWorkspace::from_documents(documents));
        for side in 0..2 {
            let count = self.workspace.as_ref().unwrap().line_count(side_of(side));
            self.caret[side] = self.caret[side].min(count);
        }
    }
    fn activate_pair(&mut self, paths: [PathBuf; 2]) {
        self.paths = Some(paths.clone());
        self.refresh_active();
        self.side_sources = [Some(paths[0].clone()), Some(paths[1].clone())];
        self.side_files = self
            .files
            .clone()
            .map(|files| [Some(files[0].clone()), Some(files[1].clone())])
            .unwrap_or([None, None]);
        if let Some(presentation) = self.cache.get(&paths) {
            self.scroll = presentation.scroll;
            self.caret = presentation.caret;
            self.expanded = presentation.expanded.clone();
        } else {
            self.scroll = [0.0; 2];
            self.caret = [0; 2];
            self.expanded = [Vec::new(), Vec::new()];
        }
        self.generation = self.generation.wrapping_add(1);
        for side in 0..2 {
            self.side_generation[side] = self.side_generation[side].wrapping_add(1);
        }
        self.pending_navigation_scroll = [false; 2];
        self.loaded_paths = Some(paths);
        self.invalidate_diff();
        self.sync_editor_cursor(0);
        self.sync_editor_cursor(1);
    }
    fn activate_side_workspace(&mut self) {
        if let ([Some(left_path), Some(right_path)], [Some(left_file), Some(right_file)]) =
            (&self.side_sources, &self.side_files)
        {
            self.paths = Some([left_path.clone(), right_path.clone()]);
            self.files = Some([left_file.clone(), right_file.clone()]);
            self.refresh_active();
            return;
        }
        let documents = std::array::from_fn(|side| {
            self.side_sources[side]
                .as_ref()
                .and_then(|path| self.documents.get(&self.key_for(path)))
                .map(|retained| retained.document.clone())
                .unwrap_or_else(|| InteractiveDocument::new(""))
        });
        if self.side_sources.iter().any(Option::is_some) {
            self.paths = None;
            self.files = None;
            self.workspace = Some(TwoWayWorkspace::from_documents(documents));
            for side in 0..2 {
                let count = self.workspace.as_ref().unwrap().line_count(side_of(side));
                self.caret[side] = self.caret[side].min(count);
            }
        } else {
            self.workspace = None;
        }
    }
    #[cfg(test)]
    pub(super) fn ingest_working_files(&mut self, files: &BTreeMap<PathBuf, WorkingFile>) {
        self.publish_active();
        for (path, file) in files {
            let key = file.identity_key();
            self.aliases.insert(path.clone(), key.clone());
            self.aliases.insert(key.clone(), key.clone());
            if let Some(retained) = self.documents.get_mut(&key) {
                if retained.document.text() != file.text {
                    let mut facade = TwoWayWorkspace::from_documents([
                        retained.document.clone(),
                        InteractiveDocument::new(""),
                    ]);
                    facade.set_text(TwoWaySide::Left, file.text.clone());
                    retained.document = facade.document(TwoWaySide::Left).clone();
                }
                retained.file = file.clone();
                if !file.is_dirty() {
                    retained.document.mark_saved_text(file.text.clone());
                }
            } else {
                self.documents.insert(key, retained_file(file.clone()));
            }
        }
        self.refresh_active();
        self.invalidate_diff();
    }
    /// A completed save advances the baseline, while later edits remain in memory.
    pub(super) fn update_saved_files(&mut self, files: &BTreeMap<PathBuf, WorkingFile>) {
        self.publish_active();
        for (path, saved) in files {
            let key = self.key_for(path);
            if let Some(retained) = self.documents.get_mut(&key) {
                retained.file.fingerprint = saved.fingerprint.clone();
                retained.file.encoding = saved.encoding;
                retained.document.mark_saved_text(saved.text.clone());
            } else {
                self.documents.insert(key, retained_file(saved.clone()));
            }
            for side in 0..2 {
                if self.side_sources[side]
                    .as_ref()
                    .is_some_and(|source| self.key_for(source) == self.key_for(path))
                {
                    self.side_files[side] = Some(saved.clone());
                }
            }
        }
        self.refresh_active();
    }
    pub(super) fn apply_search_preview(
        &mut self,
        preview: &versus::SearchPreview,
    ) -> Result<u64, String> {
        self.publish_active();
        let id = self.next_batch + 1;
        let mut prepared = Vec::new();
        let mut unique = std::collections::BTreeSet::new();
        for change in &preview.changes {
            let base = preview
                .bases
                .get(&change.path)
                .ok_or_else(|| "Replacement is missing a file snapshot.".to_owned())?;
            let key = base.identity_key();
            let existing_key = self.key_for(&change.path);
            if self.documents.contains_key(&existing_key) && existing_key != key {
                return Err(format!(
                    "{} now resolves to a different file. Reopen the workspace before replacing it.",
                    change.path.display()
                ));
            }
            if !unique.insert(key.clone()) {
                return Err(
                    "Replacement contains duplicate file identities. Run Find again.".into(),
                );
            }
            let mut retained = self
                .documents
                .get(&key)
                .cloned()
                .unwrap_or_else(|| retained_file(base.clone()));
            if retained.document.text() != base.text
                || retained.file.fingerprint != base.fingerprint
                || retained.file.encoding != base.encoding
            {
                let mut working = retained.file.clone();
                working.text = retained.document.text().into();
                if retained.document.is_dirty() || working.is_dirty() || base.is_dirty() {
                    return Err(format!(
                        "{} changed since the search. Refresh search before replacing it.",
                        change.path.display()
                    ));
                }
                // Refresh only the staged clone. All participants must pass
                // their byte-range and snapshot checks before publication.
                // A separate history entry makes batch Undo return to fresh
                // disk text while retaining older editor history.
                if let Some(edit) = external_refresh_edit(retained.document.text(), &base.text) {
                    retained.document.apply_byte_edits(
                        HistoryTag::Local(retained.document.revision()), vec![edit],
                    ).map_err(|error| format!("{} cannot refresh content: {error}. Refresh search before replacing it.", change.path.display()))?;
                }
                retained.document.mark_saved_text(base.text.clone());
                let native_path = retained.file.path.clone();
                retained.file = base.clone();
                retained.file.path = native_path.or_else(|| Some(change.path.clone()));
            }
            if retained.document.text() != change.before {
                return Err(format!(
                    "{} changed since the search. Run Find again.",
                    change.path.display()
                ));
            }
            retained
                .document
                .apply_byte_edits(HistoryTag::FolderBatch(id), change.edits.clone())
                .map_err(|e| e.to_string())?;
            if retained.document.text() != change.after {
                return Err("Replacement does not match its selected edits.".into());
            }
            prepared.push((key, retained));
        }
        let participants = prepared.iter().map(|(key, _)| key.clone()).collect();
        for (key, retained) in prepared {
            if let Some(path) = &retained.file.path {
                self.aliases.insert(path.clone(), key.clone());
            }
            self.aliases.insert(key.clone(), key.clone());
            self.documents.insert(key, retained);
        }
        self.next_batch = id;
        self.batches.insert(id, participants);
        self.batch_undo.push(id);
        self.batch_redo.clear();
        self.refresh_active();
        self.invalidate_diff();
        Ok(id)
    }
    fn batch_available(&self, id: u64, redo: bool) -> bool {
        self.batches.get(&id).is_some_and(|paths|paths.iter().all(|path|self.documents.get(path).is_some_and(|retained|if redo{retained.document.next_redo_tag()}else{retained.document.next_undo_tag()}==Some(HistoryTag::FolderBatch(id)))))
    }
    fn move_batch(&mut self, id: u64, redo: bool) -> Result<(), String> {
        self.publish_active();
        if !self.batch_available(id, redo) {
            return Err(
                "Undo newer edits in the batch files before moving this replacement transaction."
                    .into(),
            );
        }
        let before = self.history_diff_snapshot();
        for path in &self.batches[&id] {
            let document = &mut self.documents.get_mut(path).unwrap().document;
            let moved = if redo {
                document.redo_expected(HistoryTag::FolderBatch(id))
            } else {
                document.undo_expected(HistoryTag::FolderBatch(id))
            };
            debug_assert!(moved);
        }
        if redo {
            self.batch_redo.retain(|current| *current != id);
            self.batch_undo.push(id);
        } else {
            self.batch_undo.retain(|current| *current != id);
            self.batch_redo.push(id);
        }
        self.refresh_active();
        self.invalidate_history_diff(before);
        Ok(())
    }
    pub(super) fn can_undo_batch(&self) -> bool {
        self.batch_undo
            .last()
            .is_some_and(|id| self.batch_available(*id, false))
    }
    pub(super) fn can_redo_batch(&self) -> bool {
        self.batch_redo
            .last()
            .is_some_and(|id| self.batch_available(*id, true))
    }
    pub(super) fn undo_batch(&mut self, redo: bool) -> Result<(), String> {
        self.publish_active();
        let id = if redo {
            self.batch_redo.last()
        } else {
            self.batch_undo.last()
        }
        .copied()
        .ok_or_else(|| "No replacement batch is available.".to_owned())?;
        self.move_batch(id, redo)
    }
    fn move_history(&mut self, redo: bool) {
        self.publish_active();
        let active_paths: Vec<PathBuf> = self
            .paths
            .as_ref()
            .map(|paths| paths.to_vec())
            .unwrap_or_else(|| self.side_sources.iter().flatten().cloned().collect());
        let preferred_path = self
            .paths
            .as_ref()
            .map(|paths| paths[self.selected_side].clone())
            .or_else(|| self.side_sources[self.selected_side].clone());
        let Some(preferred_path) = preferred_path else {
            return;
        };
        let preferred = self.key_for(&preferred_path);
        let own = self.documents.get(&preferred).and_then(|retained| {
            if redo {
                retained.document.next_redo_tag()
            } else {
                retained.document.next_undo_tag()
            }
        });
        let fallback = self.workspace.as_ref().and_then(|w| {
            if redo {
                w.next_redo_tag()
            } else {
                w.next_undo_tag()
            }
        });
        let Some(tag) = own.or(fallback) else {
            return;
        };
        if let HistoryTag::FolderBatch(id) = tag {
            if let Err(error) = self.move_batch(id, redo) {
                self.error = Some(error);
            }
        } else {
            let key = if own.is_some() {
                preferred
            } else {
                active_paths.iter().map(|p|self.key_for(p)).find(|key|self.documents.get(key).is_some_and(|retained|if redo{retained.document.next_redo_tag()}else{retained.document.next_undo_tag()}==Some(tag))).unwrap()
            };
            let before = self.history_diff_snapshot();
            let document = &mut self.documents.get_mut(&key).unwrap().document;
            let moved = if redo {
                document.redo_expected(tag)
            } else {
                document.undo_expected(tag)
            };
            debug_assert!(moved);
            self.refresh_active();
            self.invalidate_history_diff(before);
        }
    }
    /// Editor and transfer paths call this after every mutation so an older
    /// background result cannot paint alignment for new working-copy text.
    pub(super) fn invalidate_diff(&mut self) {
        self.pending_navigation_scroll = [false; 2];
        self.publish_active();
        if !self.editing {
            if let (Some(cached), Some(diff)) = (&self.readonly_presentation, &self.diff) {
                let top = (self.scroll[0] / cached.row_height).floor() as usize;
                if let Some(row) = cached.rows.get(top).and_then(|index| diff.rows.get(*index)) {
                    self.readonly_pending_anchor = Some((
                        [
                            row.left.as_ref().map(|line| line.0),
                            row.right.as_ref().map(|line| line.0),
                        ],
                        self.scroll[0] % cached.row_height,
                    ));
                }
            }
        }
        self.readonly_presentation = None;
        if let Some(job) = self.diff_job.take() {
            job.cancel.store(true, Ordering::Relaxed);
            self.retired_diffs.push(job);
        }
        self.diff = None;
        self.pending_merge_presentation = None;
        self.diff_rows.clear();
        if let Some(ctx) = &self.editor_context {
            ctx.memory_mut(|memory| {
                memory
                    .data
                    .remove::<usize>(egui::Id::new("interactive-diff-navigation"))
            });
        }
        self.diff_dirty = self.engine_enabled();
        self.diff_not_before = Some(Instant::now() + Duration::from_millis(150));
    }
    fn merge_decoration(&self, side: usize) -> MergeDecoration {
        if let Some(diff) = &self.diff {
            let mut decoration = MergeDecoration::default();
            for row in &diff.rows {
                let (line, changed, ending) = if side == 0 {
                    (&row.left, &row.left_changed, row.left_ending)
                } else {
                    (&row.right, &row.right_changed, row.right_ending)
                };
                if let Some((number, _)) = line {
                    decoration
                        .lines
                        .insert(number - 1, (row.state.clone(), changed.clone()));
                    if !self.merge_options[2] && row.left_ending != row.right_ending {
                        if let Some(ending) = ending {
                            decoration.endings.insert(number - 1, ending);
                        }
                    }
                }
            }
            decoration
        } else {
            self.pending_merge_presentation
                .as_ref()
                .map(|pending| pending.sides[side].clone())
                .unwrap_or_default()
        }
    }
    fn capture_merge_presentation(&self) -> Option<PendingMergePresentation> {
        (self.editing
            && self.run_diff
            && (self.diff.is_some() || self.pending_merge_presentation.is_some()))
        .then(|| PendingMergePresentation {
            sides: std::array::from_fn(|side| self.merge_decoration(side)),
            legend_counts: self.footer_legend_counts(),
            changes: self
                .pending_merge_presentation
                .as_ref()
                .map_or(self.diff_rows.len(), |pending| pending.changes),
        })
    }
    fn history_diff_snapshot(&self) -> Option<[String; 2]> {
        (self.editing
            && self.run_diff
            && (self.diff.is_some() || self.pending_merge_presentation.is_some()))
        .then(|| {
            self.workspace.as_ref().map(|workspace| {
                std::array::from_fn(|side| workspace.text(side_of(side)).to_owned())
            })
        })
        .flatten()
    }
    fn invalidate_history_diff(&mut self, before: Option<[String; 2]>) {
        let mut presentation = before
            .as_ref()
            .and_then(|_| self.capture_merge_presentation());
        let mut cursors = [None; 2];
        self.invalidate_diff();
        if let (Some(before), Some(presentation), Some(workspace)) =
            (before, &mut presentation, &self.workspace)
        {
            for side in 0..2 {
                let after = workspace.text(side_of(side));
                if let Some((range, replacement)) = external_refresh_edit(&before[side], after) {
                    cursors[side] = Some(range.start + replacement.len());
                    presentation.sides[side].rebase(&before[side], range, after);
                }
            }
        }
        self.pending_merge_presentation = presentation;
        for (side, cursor) in cursors.into_iter().enumerate() {
            if let Some(cursor) = cursor {
                self.restore_editor_byte_cursor(side, cursor);
            }
        }
    }
    fn invalidate_edited_diff(&mut self, side: usize, before: &str, range: Range<usize>) {
        let mut presentation = self.capture_merge_presentation();
        let shared_file = self.side_sources[0]
            .as_ref()
            .zip(self.side_sources[1].as_ref())
            .is_some_and(|(left, right)| self.key_for(left) == self.key_for(right));
        let peer_before = shared_file.then(|| {
            self.workspace
                .as_ref()
                .unwrap()
                .text(side_of(1 - side))
                .to_owned()
        });
        // Cancel stale workers and publish edits as usual, retaining only display
        // metadata. Actual comparison results remain invalid until the worker returns.
        self.invalidate_diff();
        if let (Some(presentation), Some(workspace)) = (&mut presentation, &self.workspace) {
            for target in 0..2 {
                let after = workspace.text(side_of(target));
                if target == side {
                    presentation.sides[target].rebase(before, range.clone(), after);
                } else if shared_file {
                    if let Some(peer_before) = &peer_before {
                        if peer_before == before && after != peer_before {
                            presentation.sides[target].rebase(before, range.clone(), after);
                        } else if let Some((peer_range, _)) =
                            external_refresh_edit(peer_before, after)
                        {
                            presentation.sides[target].rebase(peer_before, peer_range, after);
                        }
                    }
                }
            }
        }
        self.pending_merge_presentation = presentation;
    }
    fn cache_active(&mut self) {
        if let Some(paths) = &self.paths {
            self.cache.insert(
                paths.clone(),
                CachedPair {
                    scroll: self.scroll,
                    caret: self.caret,
                    expanded: self.expanded.clone(),
                },
            );
        }
    }
    fn poll_diff(&mut self, ctx: &egui::Context) {
        self.retired_diffs
            .retain(|job| matches!(job.receiver.try_recv(), Err(TryRecvError::Empty)));
        if !self.retired_diffs.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(25));
        }
        if !self.engine_enabled() {
            self.diff_job = None;
            self.diff = None;
            self.pending_merge_presentation = None;
            self.readonly_presentation = None;
            return;
        }
        if self.files.is_none() && !self.side_files.iter().all(Option::is_some) {
            self.diff_job = None;
            self.diff = None;
            self.pending_merge_presentation = None;
            self.diff_rows.clear();
            self.readonly_presentation = None;
            return;
        }
        if self.diff_dirty && self.diff_job.is_none() && self.retired_diffs.is_empty() {
            if let Some(deadline) = self.diff_not_before {
                if Instant::now() < deadline {
                    ctx.request_repaint_after(deadline.saturating_duration_since(Instant::now()));
                    return;
                }
            }
            let Some(workspace) = &self.workspace else {
                return;
            };
            let text = [
                workspace.text(TwoWaySide::Left).to_owned(),
                workspace.text(TwoWaySide::Right).to_owned(),
            ];
            let preferences = if self.editing {
                self.merge_options
            } else {
                self.diff_options
            };
            let progress = Arc::new(ComparisonProgress::default());
            let options = FileViewOptions {
                ignore_whitespace: preferences[1],
                ignore_line_endings: preferences[2],
                progress: Some(progress.clone()),
            };
            let cancel = Arc::new(AtomicBool::new(false));
            let worker_cancel = cancel.clone();
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let result =
                    compare_loaded_text([&text[0], &text[1]], &worker_cancel, &options, [&[], &[]])
                        .map_err(|e| e.to_string());
                let _ = sender.send(result);
            });
            self.diff_job = Some(DiffJob {
                receiver,
                cancel,
                progress,
            });
            self.diff_dirty = false;
        }
        if let Some(result) = self
            .diff_job
            .as_ref()
            .and_then(|job| match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => Some(Err("Diff worker stopped".into())),
                Err(TryRecvError::Empty) => None,
            })
        {
            self.diff_job = None;
            match result {
                Ok(Some(comparison)) => {
                    self.diff_rows = comparison
                        .rows
                        .iter()
                        .enumerate()
                        .filter_map(|(row, row_data)| {
                            (row_data.state != versus::DirectoryEntryState::Same).then_some(row)
                        })
                        .collect();
                    self.diff = Some(comparison);
                    self.pending_merge_presentation = None;
                    self.readonly_presentation = None;
                }
                Ok(None) => self.pending_merge_presentation = None,
                Err(error) => {
                    self.pending_merge_presentation = None;
                    self.error = Some(error);
                }
            }
        }
        if self.diff_job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
        }
    }
    /// Folder mode has no open editor, but retains the same merge controls.
    pub(super) fn render_toolbar(&mut self, ui: &mut egui::Ui) {
        self.render_workspace_toolbar(ui, true);
    }
    pub(super) fn render_workspace_toolbar(&mut self, ui: &mut egui::Ui, folder_overview: bool) {
        self.folder_overview = folder_overview;
        self.poll(ui.ctx());
        self.shortcuts(ui);
        self.toolbar(ui);
    }
    pub(super) fn change_label(&self) -> String {
        if self.folder_overview {
            let count = self.folder_differences.len();
            return self
                .folder_difference_index
                .map(|index| format!("Change {} of {count}", index + 1))
                .unwrap_or_else(|| format!("{count} changes"));
        }
        if let Some(pending) = &self.pending_merge_presentation {
            return format!("{} changes", pending.changes);
        }
        let current = self.editor_context.as_ref().and_then(|ctx| {
            ctx.memory(|memory| {
                memory
                    .data
                    .get_temp::<usize>(egui::Id::new("interactive-diff-navigation"))
            })
        });
        current
            .map(|index| format!("Change {} of {}", index + 1, self.diff_rows.len()))
            .unwrap_or_else(|| format!("{} changes", self.diff_rows.len()))
    }
    pub(super) fn render_activity_status(&mut self, ui: &mut egui::Ui) -> bool {
        let palette = Palette::for_context(ui.ctx());
        if let Some(job) = &self.diff_job {
            if ui.available_width() > 2.0 * control_extent(ui) {
                ui.spinner();
            }
            let cancel_width = control_extent(ui) + ui.spacing().item_spacing.x;
            ui.allocate_ui_with_layout(
                egui::vec2(
                    (ui.available_width() - cancel_width).max(0.0),
                    control_extent(ui),
                ),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| header_progress_label(ui, job.progress.snapshot(), palette.muted),
            );
            if icon_button(ui, ToolbarIcon::Cancel, true, "Cancel file comparison").clicked() {
                self.run_diff = false;
                self.invalidate_diff();
            }
            return true;
        }
        if self.save_job.is_some() {
            ui.spinner();
            header_status_label(ui, "Saving…", palette.muted);
            return true;
        }
        if self.loading.is_some() || self.side_loading.iter().any(Option::is_some) {
            header_status_label(ui, "Loading selected files…", palette.muted);
            ui.ctx().request_repaint_after(Duration::from_millis(25));
            return true;
        }
        false
    }
    pub(super) fn render_footer(&mut self, ui: &mut egui::Ui) -> Rect {
        self.footer(ui)
    }
    fn guidance_message(&self) -> &str {
        if !self.has_actual_pair() {
            return if self.side_has_file(0) || self.side_has_file(1) {
                "Choose a file on the other side to begin."
            } else {
                "Choose a file on each side to begin."
            };
        }
        if !self.status.is_empty()
            && !self.status.starts_with("Loading")
            && !self.status.starts_with("Saving")
        {
            &self.status
        } else if self.editing {
            "Use chunk or line arrows to transfer content."
        } else {
            "Inspect differences between the open files."
        }
    }
    pub(super) fn footer_height(&self, ui: &egui::Ui) -> f32 {
        workspace_footer_height(
            ui,
            self.editing,
            self.any_dirty(),
            &self.footer_legend_icons(),
            self.footer_legend_counts(),
        )
    }
    pub(super) fn render_previews(&mut self, ctx: &egui::Context) {
        self.save_preview(ctx);
    }
    pub(super) fn set_folder_differences(&mut self, differences: impl Into<Arc<Vec<PathBuf>>>) {
        let differences = differences.into();
        if Arc::ptr_eq(&self.folder_differences, &differences) {
            return;
        }
        if self.folder_differences != differences {
            self.folder_difference_index = None;
        }
        self.folder_differences = differences;
    }
    pub(super) fn take_folder_jump(&mut self) -> Option<PathBuf> {
        self.folder_jump.take()
    }
    #[cfg(test)]
    pub(super) fn render(&mut self, ui: &mut egui::Ui) {
        let (_, status) = self.render_with_sources(ui, |_| {});
        ui.scope_builder(egui::UiBuilder::new().max_rect(status), |ui| {
            let errors: Vec<_> = self
                .error_details()
                .map(|error| error.lines().map(str::to_owned).collect())
                .unwrap_or_default();
            bottom_status_area(ui, |ui| {
                if errors.is_empty() {
                    self.render_activity_status(ui);
                } else {
                    error_summary(ui, &errors);
                }
            });
        });
    }
    pub(super) fn render_with_sources(
        &mut self,
        ui: &mut egui::Ui,
        sources: impl FnOnce(&mut egui::Ui),
    ) -> ([Rect; 2], Rect) {
        self.folder_overview = false;
        self.render_workspace_toolbar(ui, false);
        ui.add_space(5.);
        let area = ui.available_rect_before_wrap();
        let footer_height = self.footer_height(ui);
        let foot = footer_height + 8.;
        let body = Rect::from_min_max(
            area.min,
            egui::pos2(area.right(), (area.bottom() - foot).max(area.top())),
        );
        let footer = Rect::from_min_size(
            egui::pos2(area.left(), body.bottom() + 4.),
            egui::vec2(area.width(), footer_height),
        );
        let mut panes = split_rect(body);
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            comparison_panel(ui).show(ui, |ui| {
                ui.set_min_height((ui.available_height() - 2.0).max(0.0));
                sources(ui);
                panes = split_rect(ui.available_rect_before_wrap());
                self.panes(ui);
            });
        });
        let status = ui
            .scope_builder(egui::UiBuilder::new().max_rect(footer), |ui| {
                self.render_footer(ui)
            })
            .inner;
        self.render_previews(ui.ctx());
        (panes, status)
    }
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let has_pair = self.has_actual_pair();
        let extent = control_extent(ui);
        let width = ui.available_width();
        let gap = ui.spacing().item_spacing.x;
        let compact = width < 700.0;
        let height = if compact {
            3.0 * extent + 2.0 * gap
        } else {
            extent
        };
        let (center, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
        let gap = ui.spacing().item_spacing.x;
        let side_width = if compact {
            width
        } else {
            (width / 2.0 - (4.0 * extent + 3.0 * gap) / 2.0 - gap).max(110.0)
        };
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    center.min,
                    egui::pos2(center.left() + side_width, center.top() + extent),
                ))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                if !self.editing
                    && self.from_folders
                    && icon_button(ui, ToolbarIcon::Back, true, "Back to folders").clicked()
                {
                    self.back = true;
                }
                if icon_button(ui, ToolbarIcon::New, true, "New comparison").clicked() {
                    self.new_requested = true;
                }
                if self.editing
                    && icon_button(ui, ToolbarIcon::Search, true, "Find and replace").clicked()
                {
                    self.search_requested = true;
                }
                if self.engine_enabled() {
                    ui.label(
                        RichText::new(self.change_label())
                            .size(11.0)
                            .color(palette.muted),
                    );
                }
            },
        );
        let actions_width = 4.0 * extent + 3.0 * gap;
        let actions = Rect::from_center_size(
            egui::pos2(
                center.center().x,
                if compact {
                    center.top() + extent + gap + extent / 2.0
                } else {
                    center.center().y
                },
            ),
            egui::vec2(actions_width, extent),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(actions)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                if self.editing {
                    if icon_button(
                        ui,
                        ToolbarIcon::AcceptAllLeft,
                        has_pair,
                        "Accept All Left — replace Right with Left",
                    )
                    .clicked()
                    {
                        self.copy_all(TwoWaySide::Left);
                    }
                    if icon_button(
                        ui,
                        ToolbarIcon::AcceptAllRight,
                        has_pair,
                        "Accept All Right — replace Left with Right",
                    )
                    .clicked()
                    {
                        self.copy_all(TwoWaySide::Right);
                    }
                    if icon_button(
                        ui,
                        ToolbarIcon::Undo,
                        self.workspace
                            .as_ref()
                            .is_some_and(TwoWayWorkspace::can_undo),
                        "Undo",
                    )
                    .clicked()
                    {
                        self.move_history(false);
                    }
                    if icon_button(
                        ui,
                        ToolbarIcon::Redo,
                        self.workspace
                            .as_ref()
                            .is_some_and(TwoWayWorkspace::can_redo),
                        "Redo",
                    )
                    .clicked()
                    {
                        self.move_history(true);
                    }
                }
            },
        );
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
        let right_width = zoom_width
            + if self.engine_enabled() {
                5.0 * (extent + gap)
            } else {
                0.0
            };
        let right = Rect::from_min_max(
            egui::pos2(
                if compact {
                    (center.right() - right_width).max(center.left())
                } else {
                    (center.right() - right_width).max(actions.right() + gap)
                },
                if compact {
                    center.top() + 2.0 * (extent + gap)
                } else {
                    center.top()
                },
            ),
            center.max,
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(right)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                zoom_controls(ui);
                if !self.engine_enabled() {
                    return;
                }
                let previous = if self.editing {
                    self.merge_options
                } else {
                    self.diff_options
                };
                let mut prefs = previous;
                for (index, icon, label) in [
                    (0, ToolbarIcon::Differences, "Show only differences"),
                    (1, ToolbarIcon::Whitespace, "Ignore whitespace"),
                    (2, ToolbarIcon::LineEndings, "Ignore line endings"),
                ] {
                    if icon_button_state(ui, icon, true, prefs[index], label).clicked() {
                        prefs[index] = !prefs[index];
                    }
                }
                if icon_button(
                    ui,
                    ToolbarIcon::Previous,
                    self.has_differences(),
                    "Previous difference",
                )
                .clicked()
                {
                    self.jump_diff(false, ui.ctx());
                }
                if icon_button(
                    ui,
                    ToolbarIcon::Next,
                    self.has_differences(),
                    "Next difference",
                )
                .clicked()
                {
                    self.jump_diff(true, ui.ctx());
                }
                if prefs != previous {
                    if self.editing {
                        self.merge_options = prefs;
                    } else {
                        self.diff_options = prefs;
                    }
                    if prefs[1..] != previous[1..] {
                        self.invalidate_diff();
                    }
                }
            },
        );
    }
    fn jump_diff(&mut self, forward: bool, ctx: &egui::Context) {
        if self.folder_overview {
            let count = self.folder_differences.len();
            if count == 0 {
                return;
            }
            let index = match self.folder_difference_index {
                None => {
                    if forward {
                        0
                    } else {
                        count - 1
                    }
                }
                Some(index) => {
                    if forward {
                        (index + 1).min(count - 1)
                    } else {
                        index.saturating_sub(1)
                    }
                }
            };
            self.folder_difference_index = Some(index);
            self.folder_jump = Some(self.folder_differences[index].clone());
            ctx.request_repaint();
            return;
        }
        let Some(diff) = &self.diff else {
            return;
        };
        if self.diff_rows.is_empty() {
            return;
        }
        let id = egui::Id::new("interactive-diff-navigation");
        let current = ctx.memory(|m| m.data.get_temp::<usize>(id));
        let next = match current {
            None => {
                if forward {
                    0
                } else {
                    self.diff_rows.len() - 1
                }
            }
            Some(index) => {
                if forward {
                    (index + 1) % self.diff_rows.len()
                } else {
                    (index + self.diff_rows.len() - 1) % self.diff_rows.len()
                }
            }
        };
        ctx.memory_mut(|m| m.data.insert_temp(id, next));
        let row_index = self.diff_rows[next];
        let row = &diff.rows[row_index];
        let font = comparison_font(ctx);
        let row_height =
            super::ROW_HEIGHT.max(ctx.fonts_mut(|fonts| fonts.row_height(&font)).ceil() + 8.0);
        if self.editing {
            // Only move panes present on this comparison row. Matching rows
            // can have different source numbers, and the differences-only
            // projection has its own compact row positions on each side.
            for (side, line) in [&row.left, &row.right].into_iter().enumerate() {
                if let Some((number, _)) = line {
                    let display_line = if self.run_diff && self.merge_options[0] {
                        diff.rows[..row_index]
                            .iter()
                            .filter(|row| {
                                row.state != DirectoryEntryState::Same
                                    && if side == 0 {
                                        row.left.is_some()
                                    } else {
                                        row.right.is_some()
                                    }
                            })
                            .count()
                    } else {
                        number.saturating_sub(1)
                    };
                    self.scroll[side] = display_line.saturating_sub(1) as f32 * row_height;
                    self.pending_navigation_scroll[side] = true;
                }
            }
        } else if let Some(display_index) = self.readonly_display_index(row_index) {
            // Diff panes are aligned rows, including when the differences-only
            // preference filters out equal rows.
            self.scroll = [display_index.saturating_sub(1) as f32 * row_height; 2];
        }
        ctx.request_repaint();
    }
    fn navigation_lines(&self, ctx: &egui::Context) -> [Option<usize>; 2] {
        ctx.memory(|memory| {
            memory
                .data
                .get_temp::<usize>(egui::Id::new("interactive-diff-navigation"))
        })
        .and_then(|index| self.diff_rows.get(index))
        .and_then(|index| self.diff.as_ref()?.rows.get(*index))
        .map(|row| {
            [
                row.left.as_ref().map(|line| line.0.saturating_sub(1)),
                row.right.as_ref().map(|line| line.0.saturating_sub(1)),
            ]
        })
        .unwrap_or([None; 2])
    }
    fn has_differences(&self) -> bool {
        self.engine_enabled()
            && if self.folder_overview {
                !self.folder_differences.is_empty()
            } else {
                !self.diff_rows.is_empty()
            }
    }
    pub(super) fn diff_preferences(&self) -> [bool; 3] {
        self.diff_options
    }
    fn panes(&mut self, ui: &mut egui::Ui) {
        let height = ui.available_height().max(1.);
        if !self.editing && self.diff.is_some() {
            self.ensure_readonly_presentation(ui);
            let diff = self.diff.as_ref().unwrap();
            if let Some(message) = &diff.message {
                ui.label(RichText::new(message).color(Palette::for_context(ui.ctx()).muted));
                return;
            }
            if diff.rows.is_empty() {
                empty_display(
                    ui,
                    height,
                    "Both files are empty",
                    "There are no lines to compare.",
                );
                return;
            }
            if self
                .readonly_presentation
                .as_ref()
                .is_some_and(|cached| cached.rows.is_empty())
            {
                empty_display(
                    ui,
                    height,
                    "No differences",
                    "No differences with the current options.",
                );
                return;
            }
        }
        let clip = ui.clip_rect().intersect(ui.available_rect_before_wrap());
        let active_side = usize::from(
            ui.input(|input| input.pointer.hover_pos())
                .is_some_and(|position| position.x > ui.available_rect_before_wrap().center().x),
        );
        ui.scope(|ui| {
            ui.set_clip_rect(clip);
            ui.columns(2, |columns| {
                // Process the hovered pane first.  Scroll input is consumed
                // during layout, so this makes a wheel event over the right
                // pane update the shared read-only alignment in this frame.
                for side in [active_side, 1 - active_side] {
                    self.render_side(&mut columns[side], side, height);
                }
            });
        });
    }
    pub(super) fn render_side(&mut self, ui: &mut egui::Ui, side: usize, height: f32) {
        debug_assert!(side < 2);
        if self.side_sources[side].is_none() && self.paths.is_none() {
            let rect = ui.available_rect_before_wrap();
            paint_empty_drop_target(ui, rect, side, true);
            return;
        }
        if self.side_loading[side].is_some() {
            let rect = ui.available_rect_before_wrap();
            paint_empty_drop_target(ui, rect, side, true);
            return;
        }
        if self.workspace.is_none() || !self.side_has_file(side) {
            let rect = ui.available_rect_before_wrap();
            paint_empty_drop_target(ui, rect, side, true);
            return;
        }
        if !self.editing {
            self.readonly_editor(ui, side, height);
            return;
        }
        let pane = ui.available_rect_before_wrap();
        let track = self.has_actual_pair().then(|| {
            let width = 38.0;
            let rect = if side == 0 {
                Rect::from_min_max(egui::pos2(pane.right() - width, pane.top()), pane.max)
            } else {
                Rect::from_min_max(pane.min, egui::pos2(pane.left() + width, pane.bottom()))
            };
            let palette = Palette::for_context(ui.ctx());
            ui.painter().rect_filled(rect, 0.0, palette.background);
            for x in [rect.left(), rect.right()] {
                ui.painter().line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    Stroke::new(1.0, palette.border),
                );
            }
            ui.interact(
                rect,
                egui::Id::new(("merge-transfer-track", side)),
                Sense::hover(),
            );
            rect
        });
        let content = track.map_or(pane, |track| {
            if side == 0 {
                Rect::from_min_max(pane.min, egui::pos2(track.left(), pane.bottom()))
            } else {
                Rect::from_min_max(egui::pos2(track.right(), pane.top()), pane.max)
            }
        });
        let pane_clip = ui.clip_rect();
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
            ui.set_clip_rect(content.intersect(pane_clip));
            self.editor(
                ui,
                side,
                height,
                track.map(|rect| rect.intersect(pane_clip)),
            );
        });
    }
    fn ensure_readonly_presentation(&mut self, ui: &egui::Ui) {
        let font = comparison_font(ui.ctx());
        let stale = self
            .readonly_presentation
            .as_ref()
            .is_none_or(|cached| cached.preferences != self.diff_options || cached.font != font);
        if !stale {
            return;
        }
        let previous = self.readonly_presentation.take();
        let row_height = super::comparison_row_height(ui);
        let previous_fraction = self.scroll[0] % row_height;
        let previous_anchor = previous.as_ref().and_then(|cached| {
            cached
                .rows
                .get((self.scroll[0] / row_height).floor() as usize)
                .copied()
        });
        let Some(diff) = self.diff.as_ref() else {
            return;
        };
        let mut rows = Vec::with_capacity(diff.rows.len());
        // Cache only intrinsic line widths. Each pane applies its own viewport
        // minimum when it renders, so resizing cannot leave a full-body width
        // stuck on a short file.
        let mut content_widths = [0.0_f32; 2];
        let mut max_line_number = 1;
        for (index, row) in diff.rows.iter().enumerate() {
            if !self.diff_options[0] || row.state != DirectoryEntryState::Same {
                rows.push(index);
            }
            for (side, line) in [&row.left, &row.right].into_iter().enumerate() {
                if let Some((number, text)) = line {
                    max_line_number = max_line_number.max(*number);
                    // This is deliberately the same conservative width budget
                    // as native File Diff: tabs occupy four character slots.
                    let expanded =
                        text.len() + text.bytes().filter(|byte| *byte == b'\t').count() * 3;
                    content_widths[side] = content_widths[side]
                        .max(super::file_style::TEXT_OFFSET + 28.0 + expanded as f32 * font.size);
                }
            }
        }
        let gutter_extra = super::file_style::gutter_extra(max_line_number);
        for width in &mut content_widths {
            if *width > 0.0 {
                *width += gutter_extra;
            }
        }
        self.readonly_presentation = Some(ReadonlyPresentation {
            rows,
            content_widths,
            preferences: self.diff_options,
            font,
            row_height,
            gutter_extra,
        });
        if let Some((anchor, fraction)) = self.readonly_pending_anchor.take() {
            let rows = &self.readonly_presentation.as_ref().unwrap().rows;
            let top = rows
                .iter()
                .position(|index| {
                    let row = &diff.rows[*index];
                    [
                        row.left.as_ref().map(|line| line.0),
                        row.right.as_ref().map(|line| line.0),
                    ]
                    .iter()
                    .enumerate()
                    .any(|(side, number)| number.is_some() && *number == anchor[side])
                })
                .unwrap_or_else(|| rows.len().saturating_sub(1));
            self.scroll = [top as f32 * row_height + fraction; 2];
        } else if let Some(anchor) = previous_anchor {
            let rows = &self.readonly_presentation.as_ref().unwrap().rows;
            let top = rows
                .iter()
                .position(|row| *row >= anchor)
                .unwrap_or_else(|| rows.len().saturating_sub(1));
            self.scroll = [top as f32 * row_height + previous_fraction; 2];
        }
    }
    fn readonly_display_index(&self, row_index: usize) -> Option<usize> {
        self.readonly_presentation
            .as_ref()
            .and_then(|cached| cached.rows.iter().position(|index| *index == row_index))
            .or_else(|| {
                if self.diff_options[0] {
                    self.diff_rows.iter().position(|index| *index == row_index)
                } else {
                    self.diff
                        .as_ref()
                        .and_then(|diff| (row_index < diff.rows.len()).then_some(row_index))
                }
            })
    }
    /// Read-only Diff mode deliberately bypasses TextEdit.  It uses the same
    /// aligned row model and row primitives as File Diff, while retained merge
    /// documents remain untouched for a later switch back to Merge.
    fn readonly_editor(&mut self, ui: &mut egui::Ui, side: usize, height: f32) {
        if self.diff.is_none() {
            if self.diff_job.is_some() || self.diff_dirty {
                ui.centered_and_justified(|ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading file comparison…");
                    });
                });
            }
            return;
        }
        self.ensure_readonly_presentation(ui);
        let Some(diff) = self.diff.as_ref() else {
            return;
        };
        if let Some(message) = &diff.message {
            ui.label(RichText::new(message).color(Palette::for_context(ui.ctx()).muted));
            return;
        }
        let (visible_len, content_width) = self
            .readonly_presentation
            .as_ref()
            .map(|cached| (cached.rows.len(), cached.content_widths[side]))
            .unwrap_or_default();
        if visible_len == 0 {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("No differences with the current options.")
                        .color(Palette::for_context(ui.ctx()).muted),
                );
            });
            return;
        }
        let row_height = super::comparison_row_height(ui);
        let palette = Palette::for_context(ui.ctx());
        let output = egui::ScrollArea::both()
            .id_salt(("two-way-readonly", self.side_generation[side], side))
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .auto_shrink([false, false])
            .max_height(height)
            .vertical_scroll_offset(self.scroll[side])
            .show_viewport(ui, |ui, _| {
                ui.set_min_size(egui::vec2(
                    ui.available_width().max(content_width),
                    row_height * visible_len as f32,
                ));
            });
        // File Diff keeps aligned rows together.  Merge's editable renderer
        // below intentionally retains its independent per-side offsets.
        self.scroll = [output.state.offset.y; 2];
        let viewport = output.inner_rect;
        let painter = ui.painter().with_clip_rect(viewport);
        let first = (self.scroll[side] / row_height).floor() as usize;
        let last = ((self.scroll[side] + viewport.height()) / row_height).ceil() as usize;
        let navigation = ui
            .ctx()
            .memory(|memory| {
                memory
                    .data
                    .get_temp::<usize>(egui::Id::new("interactive-diff-navigation"))
            })
            .and_then(|index| self.diff_rows.get(index).copied());
        for display_index in first..last.min(visible_len) {
            let row_index = self.readonly_presentation.as_ref().unwrap().rows[display_index];
            let row = &diff.rows[row_index];
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
            let ending_diff = !self.diff_options[2] && row.left_ending != row.right_ending;
            let rect = Rect::from_min_size(
                egui::pos2(
                    viewport.left() - output.state.offset.x,
                    viewport.top() + display_index as f32 * row_height - self.scroll[side],
                ),
                egui::vec2(viewport.width().max(content_width), row_height),
            );
            super::paint_file_line(
                &painter,
                rect,
                if side == 0 {
                    row.left.as_ref()
                } else {
                    row.right.as_ref()
                },
                &row.state,
                display_index % 2 == 1,
                changed,
                ending_diff.then_some(ending).flatten(),
                navigation == Some(row_index),
                self.readonly_presentation.as_ref().unwrap().gutter_extra,
            );
            let line = if side == 0 { &row.left } else { &row.right };
            ui.interact(
                rect.intersect(viewport),
                ui.id().with(("interactive-readonly-line", side, row_index)),
                Sense::hover(),
            )
            .widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Label,
                    true,
                    super::file_line_accessible_label(side, line.as_ref(), &row.state),
                )
            });
        }
        let pane = ui.interact(
            viewport,
            egui::Id::new((
                "interactive-readonly-pane",
                self.side_generation[side],
                side,
            )),
            Sense::click(),
        );
        pane.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Panel,
                true,
                format!(
                    "{} file comparison pane. Use Up and Down Arrow to scroll lines, Page Up or Page Down for pages, Home or End for the first or last line.",
                    super::side_name(side)
                ),
            )
        });
        if pane.clicked() {
            pane.request_focus();
        }
        if pane.has_focus() {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    pane.id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        ..Default::default()
                    },
                )
            });
            let delta = ui.input(|input| {
                if input.key_pressed(egui::Key::ArrowDown) {
                    row_height
                } else if input.key_pressed(egui::Key::ArrowUp) {
                    -row_height
                } else if input.key_pressed(egui::Key::PageDown) {
                    viewport.height()
                } else if input.key_pressed(egui::Key::PageUp) {
                    -viewport.height()
                } else if input.key_pressed(egui::Key::Home) {
                    -self.scroll[side]
                } else if input.key_pressed(egui::Key::End) {
                    row_height * visible_len as f32 - self.scroll[side]
                } else {
                    0.0
                }
            });
            if delta != 0.0 {
                let next = (self.scroll[side] + delta).clamp(
                    0.0,
                    (row_height * visible_len as f32 - viewport.height()).max(0.0),
                );
                self.scroll = [next; 2];
                ui.ctx().request_repaint();
            }
            ui.painter().rect_stroke(
                viewport.shrink(1.0),
                4.0,
                Stroke::new(2.0, palette.accent),
                egui::StrokeKind::Inside,
            );
        }
    }
    fn editor(&mut self, ui: &mut egui::Ui, side: usize, height: f32, track: Option<Rect>) {
        let palette = Palette::for_context(ui.ctx());
        let source = side_of(side);
        let current = self.workspace.as_ref().unwrap().text(source).to_owned();
        let displayed = current.replace("\r\n", "\n").replace('\r', "\n");
        let displayed_lines: Vec<_> = displayed.lines().collect();
        let font = comparison_font(ui.ctx());
        let row_height = super::comparison_row_height(ui);
        let font_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
        let text_padding = ((row_height - font_height) / 2.0).max(0.0);
        let count = self.workspace.as_ref().unwrap().line_count(source);
        let chunks = self.workspace.as_ref().unwrap().chunks(source);
        let navigation_line = self.navigation_lines(ui.ctx())[side];
        let max_line_number =
            count.max(self.workspace.as_ref().unwrap().line_count(source.other()));
        let MergeDecoration {
            lines: changed,
            endings: ending_differences,
        } = self.merge_decoration(side);
        let gutter_extra = super::file_style::gutter_extra(max_line_number);
        let decorated = self.diff.is_some() || self.pending_merge_presentation.is_some();
        let filtering_same = self.run_diff && self.merge_options[0] && decorated;
        let projection = filtering_same.then(|| merge_projection(&displayed, &changed));
        if projection
            .as_ref()
            .is_some_and(|projection| projection.spans.is_empty())
        {
            self.pending_navigation_scroll[side] = false;
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("No differences with the current options.").color(palette.muted),
                );
            });
            return;
        }
        let visible_lines: Vec<usize> = projection
            .as_ref()
            .map(|projection| {
                projection
                    .spans
                    .iter()
                    .map(|span| span.source_line)
                    .collect()
            })
            .unwrap_or_else(|| (0..count.max(1)).collect());
        let mut next = projection
            .as_ref()
            .map(|projection| projection.text.clone())
            .unwrap_or_else(|| displayed.clone());
        let editor_id = egui::Id::new(("two-way-edit", self.side_generation[side], side));
        let editor_projection = EditorProjection {
            text: next.clone(),
            source_lines: visible_lines.clone(),
        };
        if let Some(previous) = self.editor_projection[side].as_ref() {
            if previous != &editor_projection {
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), editor_id) {
                    if let Some(cursor) = state.cursor.char_range() {
                        state.cursor.set_char_range(Some(remap_projection_cursor(
                            cursor,
                            previous,
                            &editor_projection,
                        )));
                    }
                    state.store(ui.ctx(), editor_id);
                }
            }
        }
        self.editor_projection[side] = Some(editor_projection);
        let previous_cursor = egui::TextEdit::load_state(ui.ctx(), editor_id)
            .and_then(|state| state.cursor.char_range())
            .map(|cursor| {
                let range = cursor.as_sorted_char_range();
                usize::from(range.start)..usize::from(range.end)
            });
        let syntax = self.side_sources[side]
            .as_ref()
            .or_else(|| self.paths.as_ref().map(|paths| &paths[side]))
            .and_then(|path| path.extension())
            .is_some_and(|ext| {
                matches!(
                    ext.to_str(),
                    Some(
                        "rs" | "py"
                            | "js"
                            | "ts"
                            | "tsx"
                            | "jsx"
                            | "c"
                            | "cpp"
                            | "h"
                            | "cs"
                            | "java"
                            | "go"
                            | "json"
                            | "toml"
                    )
                )
            });
        let merge_controls = self.editing && self.has_actual_pair();
        let text_height = (height - 5.0).max(60.0);
        let content_height = (visible_lines.len().max(1) as f32 * row_height).max(row_height);
        if std::mem::take(&mut self.pending_navigation_scroll[side]) {
            // TextEdit paints inside ScrollArea, before its final offset clamp.
            // Bound jumps using current row/viewport geometry so an EOF jump
            // cannot paint at an impossible offset and snap back next frame.
            // Extra TextEdit padding can increase the actual range; manual
            // scrolling still uses that full range without this navigation cap.
            self.scroll[side] = self.scroll[side].min((content_height - text_height).max(0.0));
        }
        let mut pending_transfer = None;
        let output = egui::ScrollArea::vertical()
            .id_salt(("two-way-scroll", self.side_generation[side], side))
            .auto_shrink([false, false])
            .max_height(text_height)
            .vertical_scroll_offset(self.scroll[side])
            .show(ui, |ui| {
                let number_width = super::file_style::TEXT_OFFSET - 4.0 + gutter_extra;
                let gutter_width = if merge_controls { 32.0 } else { 0.0 };
                let width = ui.available_width().max(180.0);
                let text_width = (width - number_width - gutter_width - 12.0).max(80.0);
                let origin = ui.cursor().min;
                let gutter_x = if side == 0 {
                    origin.x + width - gutter_width
                } else {
                    origin.x
                };
                let numbers_x = if side == 0 {
                    origin.x
                } else {
                    origin.x + gutter_width
                };
                let text_x = numbers_x + number_width + 4.0;
                let text_rect = Rect::from_min_size(
                    egui::pos2(text_x, origin.y + text_padding),
                    // Short/empty files still fill the viewport so their
                    // horizontal scrollbar sits at the bottom of the pane.
                    egui::vec2(text_width, content_height.max(text_height - text_padding)),
                );
                let mut highlight_slots = Vec::new();
                // The editable surface still owns its TextEdit/history, but
                // while comparison assistance is present its row decoration is
                // deliberately the same as File Diff.
                for (display_line, &line) in visible_lines.iter().enumerate() {
                    let state = changed
                        .get(&line)
                        .map(|(state, _)| state)
                        .unwrap_or(&DirectoryEntryState::Same);
                    let row_rect = Rect::from_min_size(
                        egui::pos2(numbers_x, origin.y + display_line as f32 * row_height),
                        egui::vec2(width - gutter_width, row_height),
                    );
                    super::file_style::paint_background(
                        ui.painter(),
                        row_rect,
                        state,
                        display_line % 2 == 1,
                        navigation_line == Some(line),
                    );
                    super::file_style::paint_gutter(
                        ui.painter(),
                        Rect::from_min_size(egui::pos2(numbers_x, row_rect.top()), row_rect.size()),
                        line + 1,
                        state,
                        gutter_extra,
                    );
                    if let Some((state, ranges)) = changed.get(&line) {
                        let slot = ui.painter().add(egui::Shape::Noop);
                        highlight_slots.push((
                            slot,
                            display_line,
                            displayed_lines
                                .get(line)
                                .copied()
                                .unwrap_or_default()
                                .to_owned(),
                            ranges.clone(),
                            state.clone(),
                        ));
                    }
                }
                let mut text_scroll_x = 0.0;
                let mut text_clip = text_rect.intersect(ui.clip_rect());
                ui.scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
                    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _: f32| {
                        let mut job = egui::text::LayoutJob::default();
                        job.wrap.max_width = f32::INFINITY;
                        for (display_line, line) in
                            buffer.as_str().split_inclusive('\n').enumerate()
                        {
                            let index = visible_lines
                                .get(display_line)
                                .copied()
                                .unwrap_or(display_line);
                            if let Some((state, ranges)) = changed.get(&index) {
                                super::file_style::append_text_without_inline_background(
                                    &mut job,
                                    line,
                                    state,
                                    ranges,
                                    palette,
                                    font.clone(),
                                    Some(row_height),
                                );
                            } else if decorated {
                                super::file_style::append_text(
                                    &mut job,
                                    line,
                                    &DirectoryEntryState::Same,
                                    &[],
                                    palette,
                                    font.clone(),
                                    Some(row_height),
                                );
                            } else {
                                append_code_line(
                                    &mut job,
                                    line,
                                    egui::TextFormat {
                                        font_id: font.clone(),
                                        color: palette.text,
                                        line_height: Some(row_height),
                                        ..Default::default()
                                    },
                                    syntax,
                                    palette,
                                );
                            }
                        }
                        if job.text.is_empty() {
                            job.append(
                                "",
                                0.0,
                                egui::TextFormat {
                                    font_id: font.clone(),
                                    ..Default::default()
                                },
                            );
                        }
                        ui.fonts_mut(|fonts| fonts.layout_job(job))
                    };
                    let horizontal = egui::ScrollArea::horizontal()
                        .id_salt(("interactive-horizontal", self.side_generation[side], side))
                        .auto_shrink([false, false])
                        .max_width(text_width)
                        .show(ui, |ui| {
                            // Keep egui's native caret margins without including
                            // the extra spacing used by comparison rows.
                            ui.visuals_mut().text_cursor.max_height = Some(font_height + 3.0);
                            egui::TextEdit::multiline(&mut next)
                                .id(editor_id)
                                .font(font.clone())
                                .code_editor()
                                .interactive(self.editing)
                                .desired_width(text_width)
                                .desired_rows(1)
                                .min_size(egui::vec2(text_width, ui.available_height()))
                                .frame(egui::Frame::NONE)
                                .layouter(&mut layouter)
                                .show(ui)
                        });
                    text_scroll_x = horizontal.state.offset.x;
                    text_clip = horizontal.inner_rect.intersect(ui.clip_rect());
                    let highlight_clip = Rect::from_min_max(
                        egui::pos2(text_clip.left(), text_clip.top() - text_padding),
                        text_clip.max,
                    )
                    .intersect(ui.clip_rect());
                    let highlight_painter = ui.painter().with_clip_rect(highlight_clip);
                    for (slot, display_line, text, ranges, state) in highlight_slots.drain(..) {
                        let shapes = super::file_style::inline_highlight_rects(
                            &highlight_painter,
                            egui::pos2(
                                text_x - text_scroll_x,
                                origin.y + display_line as f32 * row_height,
                            ),
                            &text,
                            &ranges,
                            font.clone(),
                            row_height,
                        )
                        .into_iter()
                        .map(|rect| {
                            egui::Shape::rect_filled(
                                rect,
                                0.0,
                                state_color(&state, palette).gamma_multiply(0.3),
                            )
                        })
                        .collect();
                        highlight_painter.set(slot, egui::Shape::Vec(shapes));
                    }
                    let edit = horizontal.inner;
                    if edit.response.has_focus() {
                        self.selected_side = side;
                        if let Some(cursor) = edit.cursor_range {
                            let position: usize = cursor.primary.index.into();
                            let display_line =
                                next.chars().take(position).filter(|c| *c == '\n').count();
                            self.caret[side] = visible_lines
                                .get(display_line)
                                .copied()
                                .unwrap_or_else(|| visible_lines.last().map_or(0, |line| line + 1));
                        }
                    }
                });
                for (display_line, &line) in visible_lines.iter().enumerate() {
                    let y = origin.y + display_line as f32 * row_height;
                    if let Some(ending) = ending_differences.get(&line) {
                        let text = displayed_lines.get(line).copied().unwrap_or_default();
                        let mut ending_job = egui::text::LayoutJob::default();
                        let state = changed
                            .get(&line)
                            .map(|(state, _)| state)
                            .unwrap_or(&DirectoryEntryState::Same);
                        let ranges = changed
                            .get(&line)
                            .map(|(_, ranges)| ranges.as_slice())
                            .unwrap_or(&[]);
                        super::file_style::append_text(
                            &mut ending_job,
                            text,
                            state,
                            ranges,
                            palette,
                            font.clone(),
                            Some(row_height),
                        );
                        let end_x =
                            text_x - text_scroll_x + ui.painter().layout_job(ending_job).size().x;
                        super::file_style::paint_ending(
                            &ui.painter().with_clip_rect(text_clip),
                            Rect::from_min_size(
                                egui::pos2(origin.x, y),
                                egui::vec2(width, row_height),
                            ),
                            end_x,
                            *ending,
                            state,
                        );
                    }
                }
                if merge_controls {
                    for chunk in &chunks {
                        let runs = visible_chunk_runs(&chunk.range, &visible_lines);
                        if runs.is_empty() {
                            continue;
                        }
                        let expanded = self.expanded[side].contains(&chunk.id);
                        for run in &runs {
                            let lines: Vec<_> = if expanded {
                                run.clone().collect()
                            } else {
                                vec![run.start]
                            };
                            for line in lines {
                                let display_line = visible_lines
                                    .iter()
                                    .position(|visible| *visible == line)
                                    .unwrap();
                                let y = origin.y + display_line as f32 * row_height;
                                let track = track.expect("loaded merge pair has transfer tracks");
                                let track_clip = Rect::from_min_max(
                                    egui::pos2(track.left(), track.top().max(ui.clip_rect().top())),
                                    egui::pos2(
                                        track.right(),
                                        track.bottom().min(ui.clip_rect().bottom()),
                                    ),
                                );
                                let rect = Rect::from_min_size(
                                    egui::pos2(track.center().x - 15.0, y),
                                    egui::vec2(30.0, row_height),
                                );
                                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                                    ui.set_clip_rect(track_clip);
                                    let tooltip = if expanded {
                                        format!("Copy line {} at the other pane’s cursor", line + 1)
                                    } else {
                                        format!(
                                            "Copy chunk {}–{} at the other pane’s cursor",
                                            run.start + 1,
                                            run.end
                                        )
                                    };
                                    let response = transfer_action(
                                        ui,
                                        rect,
                                        egui::Id::new(("two-way-transfer", side, chunk.id, line)),
                                        side,
                                        &tooltip,
                                    );
                                    if response.clicked() {
                                        pending_transfer = Some(if expanded {
                                            line..line + 1
                                        } else {
                                            run.clone()
                                        });
                                    }
                                });
                            }
                        }
                        let first_visible = runs[0].start;
                        let first_display = visible_lines
                            .iter()
                            .position(|line| *line == first_visible)
                            .unwrap();
                        let toggle = Rect::from_min_size(
                            egui::pos2(
                                gutter_x + 5.0,
                                origin.y + first_display as f32 * row_height,
                            ),
                            egui::vec2(21.0, row_height),
                        );
                        ui.scope_builder(egui::UiBuilder::new().max_rect(toggle), |ui| {
                            if inline_action(
                                ui,
                                toggle,
                                egui::Id::new(("two-way-granularity", side, chunk.id)),
                                if expanded { "−" } else { "+" },
                                if expanded {
                                    "Return to chunk controls"
                                } else {
                                    "Show an arrow for each line"
                                },
                            )
                            .clicked()
                            {
                                if expanded {
                                    self.expanded[side].retain(|id| *id != chunk.id);
                                } else {
                                    self.expanded[side].push(chunk.id);
                                }
                            }
                        });
                    }
                }
                ui.allocate_rect(
                    Rect::from_min_size(origin, egui::vec2(width, content_height)),
                    Sense::hover(),
                );
            });
        self.scroll[side] = output.state.offset.y;
        let base_text = projection
            .as_ref()
            .map(|projection| projection.text.as_str())
            .unwrap_or(displayed.as_str());
        if next != base_text {
            let edit = projection.as_ref().map_or_else(
                || Ok(editor_byte_edit(&current, &next, previous_cursor.clone())),
                |projection| {
                    projected_editor_edit(&current, projection, &next, previous_cursor.clone())
                },
            );
            match edit {
                Ok((range, replacement)) => {
                    let caret_byte = range.start + replacement.len();
                    if let Err(error) = self.workspace.as_mut().unwrap().edit_bytes(
                        source,
                        range.clone(),
                        replacement,
                    ) {
                        self.error = Some(error.to_string());
                    } else {
                        self.error = None;
                        // The TextEdit buffer was a compact projection. Its
                        // character offsets are no longer valid after this
                        // source mutation; restore it from the exact source
                        // byte position on the next full render.
                        self.restore_editor_byte_cursor(side, caret_byte);
                        self.invalidate_edited_diff(side, &current, range);
                    }
                }
                Err(error) => self.error = Some(error.into()),
            }
            // The gutter geometry was rendered for the previous text.
            pending_transfer = None;
        }
        if let Some(range) = pending_transfer {
            self.transfer(source, range);
        }
    }
    fn footer(&mut self, ui: &mut egui::Ui) -> Rect {
        let icons = self.footer_legend_icons();
        let dirty = self.any_dirty();
        let row =
            workspace_footer_layout(ui, self.editing, dirty, &icons, self.footer_legend_counts());
        ui.scope_builder(egui::UiBuilder::new().max_rect(row.info), |ui| {
            footer_guidance(ui, self.guidance_message());
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(row.legend), |ui| {
            bottom_legend(ui, &icons, self.footer_legend_counts());
        });
        if let Some(dirty) = row.dirty {
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(dirty)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    let response = ui.add(
                        egui::Label::new(
                            RichText::new("Unsaved Changes")
                                .strong()
                                .size(11.0)
                                .color(Palette::for_context(ui.ctx()).changed),
                        )
                        .truncate(),
                    );
                    ui.interact(
                        response.rect,
                        egui::Id::new("workspace-unsaved"),
                        Sense::hover(),
                    )
                    .on_hover_text("Changes remain in memory until saved.");
                },
            );
        }
        if let Some(save) = row.save {
            let save_sides: Vec<_> = (0..2).filter(|side| self.can_save_side(*side)).collect();
            let retained_dirty = self.has_retained_dirty_documents();
            let enabled = !self.is_saving() && (!save_sides.is_empty() || retained_dirty);
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(save)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    if captioned_icon_button(ui, ToolbarIcon::Save, enabled, "Save Both").clicked()
                    {
                        if retained_dirty {
                            self.save_all_requested = true;
                        } else {
                            self.request_save(save_sides);
                        }
                    }
                },
            );
        }
        row.status
    }
    fn footer_legend_icons(&self) -> Vec<StatusIcon> {
        let mut icons = DIFF_LEGEND_ICONS.to_vec();
        if self.editing {
            icons.push(StatusIcon::Conflict);
        }
        icons
    }
    fn footer_legend_counts(&self) -> Option<[usize; 6]> {
        let Some(diff) = self.diff.as_ref() else {
            return self.pending_merge_presentation.as_ref()?.legend_counts;
        };
        let mut counts = [0; 6];
        for row in &diff.rows {
            match row.state {
                DirectoryEntryState::Different => counts[1] += 1,
                DirectoryEntryState::LeftOnly => counts[2] += 1,
                DirectoryEntryState::RightOnly => counts[3] += 1,
                DirectoryEntryState::TypeMismatch => counts[4] += 1,
                DirectoryEntryState::Error(_) => counts[5] += 1,
                DirectoryEntryState::Same => {}
            }
        }
        Some(counts)
    }
    fn active_file(&self, side: usize) -> Option<&WorkingFile> {
        self.side_sources[side]
            .as_ref()
            .and_then(|path| self.documents.get(&self.key_for(path)))
            .map(|retained| &retained.file)
            .or_else(|| {
                self.side_files[side]
                    .as_ref()
                    .or_else(|| self.files.as_ref().map(|files| &files[side]))
            })
    }
    fn side_has_file(&self, side: usize) -> bool {
        self.active_file(side).is_some()
    }
    fn has_actual_pair(&self) -> bool {
        self.files.is_some()
            || (self.side_sources.iter().all(Option::is_some)
                && self.side_files.iter().all(Option::is_some))
    }
    fn transfer(&mut self, source: TwoWaySide, range: Range<usize>) {
        if !self.has_actual_pair() {
            return;
        }
        let dest = source.other();
        let side = side_index(dest);
        let at = self.caret[side].min(self.workspace.as_ref().unwrap().line_count(dest));
        let target = at..at;
        if self.filtered_merge_range_hidden(source, &range) {
            self.error = Some("That transfer crosses hidden unchanged lines. Turn off Show only differences to transfer the full range.".into());
            return;
        }
        let old_count = self.workspace.as_ref().unwrap().line_count(dest);
        match self.workspace.as_mut().unwrap().transfer(
            source,
            range,
            target.clone(),
            TransferPlacement::Insert,
        ) {
            Ok(()) => {
                let new_count = self.workspace.as_ref().unwrap().line_count(dest);
                let inserted = (new_count as isize - old_count as isize + target.len() as isize)
                    .max(0) as usize;
                self.caret[side] = target.start + inserted;
                self.activate_editor(side);
                self.error = None;
                self.status = "Transferred into the working copy. Save when ready.".into();
                self.invalidate_diff();
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    fn filtered_merge_range_hidden(&self, side: TwoWaySide, range: &Range<usize>) -> bool {
        if range.is_empty() || !self.editing || !self.run_diff || !self.merge_options[0] {
            return false;
        }
        if self.diff.is_none() && self.pending_merge_presentation.is_none() {
            return false;
        }
        let decoration = self.merge_decoration(side_index(side));
        range.clone().any(|line| {
            !decoration
                .lines
                .get(&line)
                .is_some_and(|(state, _)| *state != DirectoryEntryState::Same)
        })
    }

    fn copy_all(&mut self, source: TwoWaySide) {
        if !self.has_actual_pair() {
            return;
        }
        let dest = source.other();
        let source_count = self.workspace.as_ref().unwrap().line_count(source);
        let dest_count = self.workspace.as_ref().unwrap().line_count(dest);
        match self.workspace.as_mut().unwrap().transfer(
            source,
            0..source_count,
            0..dest_count,
            TransferPlacement::Replace,
        ) {
            Ok(()) => {
                let count = self.workspace.as_ref().unwrap().line_count(dest);
                let side = side_index(dest);
                self.caret[side] = count;
                self.activate_editor(side);
                self.invalidate_diff();
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    /// Restore cursor coordinates through a full source view before the next
    /// frame projects them into the filtered editor.
    fn restore_editor_byte_cursor(&mut self, side: usize, byte: usize) {
        let Some(ctx) = &self.editor_context else {
            return;
        };
        let Some(workspace) = &self.workspace else {
            return;
        };
        let updated = workspace.text(side_of(side));
        let displayed = updated.replace("\r\n", "\n").replace('\r', "\n");
        let position = original_byte_to_normalized_chars(updated, byte.min(updated.len()));
        let id = egui::Id::new(("two-way-edit", self.side_generation[side], side));
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(position),
            )));
        state.store(ctx, id);
        self.caret[side] = displayed
            .chars()
            .take(position)
            .filter(|c| *c == '\n')
            .count();
        self.editor_projection[side] = Some(EditorProjection {
            source_lines: (0..text_segments(&displayed).len()).collect(),
            text: displayed,
        });
    }
    fn sync_editor_cursor(&mut self, side: usize) {
        let (Some(ctx), Some(workspace)) = (&self.editor_context, &self.workspace) else {
            return;
        };
        let text = workspace
            .text(side_of(side))
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        let at = self.caret[side];
        let offset = text
            .split_inclusive('\n')
            .take(at)
            .map(|line| line.chars().count())
            .sum::<usize>();
        let id = egui::Id::new(("two-way-edit", self.side_generation[side], side));
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(offset),
            )));
        state.clear_undoer();
        state.store(ctx, id);
        // These explicit targets are in the full source buffer. Record that
        // coordinate space so the next filtered render maps them only once.
        self.editor_projection[side] = Some(EditorProjection {
            source_lines: (0..text_segments(&text).len()).collect(),
            text,
        });
    }
    fn activate_editor(&mut self, side: usize) {
        self.selected_side = side;
        self.sync_editor_cursor(side);
        if let Some(ctx) = &self.editor_context {
            ctx.memory_mut(|memory| {
                memory.request_focus(egui::Id::new((
                    "two-way-edit",
                    self.side_generation[side],
                    side,
                )))
            });
        }
    }
    fn save_sides(&mut self, sides: Vec<usize>) {
        if self.is_saving() {
            return;
        }
        self.publish_active();
        let Some(workspace) = &self.workspace else {
            return;
        };
        let copies: BTreeMap<_, _> = sides
            .into_iter()
            .filter_map(|side| {
                let mut file = self.active_file(side)?.clone();
                file.text = workspace.text(side_of(side)).into();
                Some((file.identity_key(), file))
            })
            .collect();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let results = copies
                .into_values()
                .map(|mut file| {
                    save_working_file(&mut file)
                        .map(|()| file)
                        .map_err(|e| e.to_string())
                })
                .collect();
            let _ = sender.send(results);
        });
        self.save_job = Some(SaveJob { receiver });
        self.status = "Saving working copies…".into();
    }
    fn poll_save(&mut self, ctx: &egui::Context) {
        let ready = self
            .save_job
            .as_ref()
            .and_then(|job| match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => Some(vec![Err("Save worker stopped".into())]),
                _ => None,
            });
        if let Some(results) = ready {
            self.save_job = None;
            let mut saved = BTreeMap::new();
            let mut errors = Vec::new();
            for result in results {
                match result {
                    Ok(file) => {
                        if let Some(path) = &file.path {
                            saved.insert(path.clone(), file);
                        }
                    }
                    Err(error) => errors.push(error),
                }
            }
            self.update_saved_files(&saved);
            self.status = format!("Saved {} working copies.", saved.len());
            self.error = (!errors.is_empty()).then(|| errors.join("\n"));
        }
        if self.save_job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(25));
        }
    }
    fn request_save(&mut self, sides: Vec<usize>) {
        if self.is_saving() {
            return;
        }
        self.pending_save = Some(sides);
    }
    fn save_preview(&mut self, ctx: &egui::Context) {
        let Some(sides) = self.pending_save.clone() else {
            return;
        };
        let mut confirm = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("interactive-save-preview")).show(ctx, |ui| {
            ui.heading("Save pending changes?");
            ui.label("These working copies will replace their original files after the external-change check.");
            for side in &sides {
                if let Some(path) = self.selected_source(*side)
                    .or_else(|| self.paths.as_ref().map(|paths| &paths[*side]))
                {
                    ui.label(path.display().to_string());
                }
            }
            if let Some(workspace)=&self.workspace{
                egui::ScrollArea::vertical().max_height(300.0).show(ui,|ui|{for side in &sides{
                    ui.label(if *side==0{"Left working copy"}else{"Right working copy"});
                    if let Some(file) = self.active_file(*side) {
                        ui.columns(2,|cols|{cols[0].label("Loaded file");cols[0].label(RichText::new(file.baseline_text().unwrap_or_default()).monospace());cols[1].label("Pending content");cols[1].label(RichText::new(workspace.text(side_of(*side))).monospace());});
                    }
                }});
            }
            ui.horizontal(|ui| { cancel = ui.button("Cancel").clicked(); confirm = ui.button("Save").clicked(); });
        });
        if cancel {
            self.pending_save = None;
        }
        if confirm {
            self.pending_save = None;
            self.save_sides(sides);
        }
    }
    fn shortcuts(&mut self, ui: &mut egui::Ui) {
        if !self.editing {
            return;
        }
        // egui permits extra Shift for less-specific shortcuts, so consume
        // Redo before Undo or Command+Shift+Z would undo instead.
        if ui.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::Z,
            ) || i.consume_key(egui::Modifiers::CTRL, egui::Key::Y)
        }) {
            self.move_history(true);
        } else if ui.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)
                || i.consume_key(egui::Modifiers::CTRL, egui::Key::Z)
        }) {
            self.move_history(false);
        }
        if ui.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)
                || i.consume_key(egui::Modifiers::CTRL, egui::Key::S)
        }) {
            self.request_save(vec![self.selected_side])
        }
    }
}
/// A bounded text splice for reconciling a clean cache with fresh disk bytes.
/// This does not run the diff engine. Conservative overlap checks are delegated
/// to the document, preserving byte boundaries and history.
fn external_refresh_edit(before: &str, after: &str) -> Option<(Range<usize>, String)> {
    if before == after {
        return None;
    }
    let mut start = before
        .bytes()
        .zip(after.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !before.is_char_boundary(start) || !after.is_char_boundary(start) {
        start -= 1;
    }
    let mut suffix = before[start..]
        .bytes()
        .rev()
        .zip(after[start..].bytes().rev())
        .take_while(|(a, b)| a == b)
        .count();
    while !before.is_char_boundary(before.len() - suffix)
        || !after.is_char_boundary(after.len() - suffix)
    {
        suffix -= 1;
    }
    Some((
        start..before.len() - suffix,
        after[start..after.len() - suffix].into(),
    ))
}

fn retained_file(file: WorkingFile) -> RetainedFile {
    let mut document = InteractiveDocument::new(file.text.clone());
    document.mark_saved_text(file.baseline_text().unwrap_or_default());
    RetainedFile { file, document }
}
fn file_key(path: &std::path::Path) -> PathBuf {
    versus::working_file_key(path)
}
fn inline_action(
    ui: &egui::Ui,
    rect: Rect,
    id: egui::Id,
    glyph: &str,
    label: &str,
) -> egui::Response {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let palette = Palette::for_context(ui.ctx());
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect.shrink(1.0), 2.0, palette.hover);
    }
    let color = if response.hovered() {
        palette.accent
    } else {
        palette.text
    };
    let stroke = Stroke::new(2.0, color);
    let center = rect.center();
    ui.painter().line_segment(
        [center - egui::vec2(6.0, 0.0), center + egui::vec2(6.0, 0.0)],
        stroke,
    );
    if glyph == "+" {
        ui.painter().line_segment(
            [center - egui::vec2(0.0, 6.0), center + egui::vec2(0.0, 6.0)],
            stroke,
        );
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
}

/// Larger, side-colored transfer arrows use the same palette slots as File
/// Diff's left-only and right-only status icons.  This stays separate from the
/// smaller expand/collapse controls kept inside the file content.
fn transfer_action(
    ui: &egui::Ui,
    rect: Rect,
    id: egui::Id,
    side: usize,
    label: &str,
) -> egui::Response {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let palette = Palette::for_context(ui.ctx());
    let color = if side == 0 {
        palette.left_only
    } else {
        palette.right_only
    };
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect.shrink(1.0), 2.0, palette.hover);
    }
    // Vector geometry avoids missing-glyph boxes in platform font fallbacks.
    let direction = if side == 0 { 1.0 } else { -1.0 };
    let point = |x: f32, y: f32| rect.center() + egui::vec2(x * direction, y);
    let stroke = Stroke::new(2.0, color);
    ui.painter()
        .line_segment([point(-5.0, 0.0), point(5.0, 0.0)], stroke);
    ui.painter().line(
        vec![point(0.0, -5.0), point(5.0, 0.0), point(0.0, 5.0)],
        stroke,
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
}

fn append_code_line(
    job: &mut egui::text::LayoutJob,
    line: &str,
    format: egui::TextFormat,
    syntax: bool,
    palette: Palette,
) {
    if !syntax {
        job.append(line, 0.0, format);
        return;
    }
    let keywords = [
        "fn", "pub", "let", "mut", "impl", "struct", "enum", "use", "if", "else", "return",
        "match", "async", "await", "class", "def", "import", "from", "const", "function", "var",
        "void", "int", "for", "while", "true", "false", "None", "null",
    ];
    let mut offset = 0;
    for token in line.split_inclusive(|c: char| !c.is_alphanumeric() && c != '_') {
        let word = token.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_');
        if keywords.contains(&word) {
            let mut colored = format.clone();
            colored.color = palette.accent;
            job.append(word, 0.0, colored);
            job.append(&token[word.len()..], 0.0, format.clone());
        } else {
            job.append(token, 0.0, format.clone());
        }
        offset += token.len();
    }
    if offset < line.len() {
        job.append(&line[offset..], 0.0, format);
    }
}
fn text_segments(text: &str) -> Vec<(&str, &str)> {
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    let mut lines = Vec::new();
    while i < bytes.len() {
        if bytes[i] == b'\n' || bytes[i] == b'\r' {
            let len = if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
            lines.push((&text[start..i], &text[i..i + len]));
            i += len;
            start = i;
        } else {
            i += 1;
        }
    }
    if start < text.len() {
        lines.push((&text[start..], ""));
    }
    lines
}
/// Convert the editor's normalized text and pre-edit selection into an exact
/// original-byte transaction. The cursor disambiguates edits in repeated text;
/// untouched CRLF/CR terminators retain their original bytes.
fn editor_byte_edit(
    original: &str,
    edited: &str,
    selection: Option<Range<usize>>,
) -> (Range<usize>, String) {
    let displayed = original.replace("\r\n", "\n").replace('\r', "\n");
    let char_byte = |index: usize| {
        displayed
            .char_indices()
            .nth(index)
            .map_or(displayed.len(), |(byte, _)| byte)
    };
    let valid = |range: &Range<usize>| -> Option<Range<usize>> {
        if range.start > range.end
            || range.end > displayed.len()
            || !displayed.is_char_boundary(range.start)
            || !displayed.is_char_boundary(range.end)
        {
            return None;
        }
        let suffix = displayed.len() - range.end;
        let end = edited.len().checked_sub(suffix)?;
        if range.start > end
            || !edited.is_char_boundary(range.start)
            || !edited.is_char_boundary(end)
        {
            return None;
        }
        (displayed[..range.start] == edited[..range.start]
            && displayed[range.end..] == edited[end..])
            .then_some(range.start..end)
    };
    let mut candidates = Vec::new();
    if let Some(selection) = selection {
        let selected = char_byte(selection.start)..char_byte(selection.end);
        candidates.push(selected.clone());
        if selected.is_empty() && displayed.len() > edited.len() {
            let deleted = displayed.len() - edited.len();
            if selected.start >= deleted {
                candidates.push(selected.start - deleted..selected.start);
            }
            candidates.push(selected.start..selected.start.saturating_add(deleted));
        }
    }
    let (old, new) = candidates
        .into_iter()
        .find_map(|range| valid(&range).map(|replacement| (range, replacement)))
        .unwrap_or_else(|| {
            let start = displayed
                .chars()
                .zip(edited.chars())
                .take_while(|(a, b)| a == b)
                .map(|(c, _)| c.len_utf8())
                .sum::<usize>();
            let suffix = displayed[start..]
                .chars()
                .rev()
                .zip(edited[start..].chars().rev())
                .take_while(|(a, b)| a == b)
                .map(|(c, _)| c.len_utf8())
                .sum::<usize>();
            (
                start..displayed.len() - suffix,
                start..edited.len() - suffix,
            )
        });
    let original_byte = |normalized: usize| {
        let mut byte = 0;
        let mut shown = 0;
        while shown < normalized && byte < original.len() {
            byte += if original.as_bytes()[byte] == b'\r'
                && original.as_bytes().get(byte + 1) == Some(&b'\n')
            {
                2
            } else {
                1
            };
            shown += 1;
        }
        byte
    };
    let range = original_byte(old.start)..original_byte(old.end);
    let lines = text_segments(original);
    let mut offset = 0;
    let local_ending = lines.iter().find_map(|(text, ending)| {
        let end = offset + text.len() + ending.len();
        let contains = range.start < end;
        offset = end;
        (contains && !ending.is_empty()).then_some(*ending)
    });
    let ending = local_ending
        .or_else(|| {
            lines
                .iter()
                .find_map(|(_, ending)| (!ending.is_empty()).then_some(*ending))
        })
        .unwrap_or("\n");
    (range, edited[new].replace('\n', ending))
}

/// An editable view of changed source lines. The backing document is never
/// filtered: each projected span remembers its original normalized byte range
/// so a visible edit can be applied to the real working copy safely.
#[derive(Clone, Debug)]
struct MergeProjection {
    text: String,
    spans: Vec<ProjectionSpan>,
}

#[derive(Clone, Debug)]
struct ProjectionSpan {
    source_line: usize,
    source: Range<usize>,
    projected: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EditorProjection {
    text: String,
    source_lines: Vec<usize>,
}

fn merge_projection(
    displayed: &str,
    changed: &BTreeMap<usize, (DirectoryEntryState, Vec<Range<usize>>)>,
) -> MergeProjection {
    let mut projection = MergeProjection {
        text: String::new(),
        spans: Vec::new(),
    };
    let mut source_start = 0;
    for (line, content) in displayed.split_inclusive('\n').enumerate() {
        let source = source_start..source_start + content.len();
        source_start = source.end;
        if !changed
            .get(&line)
            .is_some_and(|(state, _)| *state != DirectoryEntryState::Same)
        {
            continue;
        }
        let projected_start = projection.text.len();
        projection.text.push_str(content);
        projection.spans.push(ProjectionSpan {
            source_line: line,
            source,
            projected: projected_start..projection.text.len(),
        });
    }
    // `split_inclusive` does not yield an empty final line. That line has no
    // content to edit, so retaining it cannot expose a transfer or deletion.
    projection
}

fn visible_chunk_runs(range: &Range<usize>, visible_lines: &[usize]) -> Vec<Range<usize>> {
    let mut runs: Vec<Range<usize>> = Vec::new();
    for line in visible_lines
        .iter()
        .copied()
        .filter(|line| range.contains(line))
    {
        match runs.last_mut() {
            Some(last) if last.end == line => last.end += 1,
            _ => runs.push(line..line + 1),
        }
    }
    runs
}

fn remap_projection_cursor(
    cursor: egui::text::CCursorRange,
    old: &EditorProjection,
    new: &EditorProjection,
) -> egui::text::CCursorRange {
    let source_position = |projection: &EditorProjection, cursor: egui::text::CCursor| {
        let index: usize = cursor.index.into();
        let prefix: String = projection.text.chars().take(index).collect();
        let display_line = prefix
            .chars()
            .filter(|character| *character == '\n')
            .count();
        let column = prefix.rsplit_once('\n').map_or_else(
            || prefix.chars().count(),
            |(_, column)| column.chars().count(),
        );
        let source_line = projection
            .source_lines
            .get(display_line)
            .copied()
            .or_else(|| projection.source_lines.last().map(|line| line + 1))
            .unwrap_or(0);
        (source_line, column)
    };
    let projected_cursor = |(source_line, column): (usize, usize)| {
        let display_line = new
            .source_lines
            .iter()
            .position(|line| *line == source_line)
            .unwrap_or_else(|| {
                new.source_lines
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, line)| line.abs_diff(source_line))
                    .map(|(index, _)| index)
                    .unwrap_or(0)
            });
        let line_start = new
            .text
            .split_inclusive('\n')
            .take(display_line)
            .map(|line| line.chars().count())
            .sum::<usize>();
        let line = new
            .text
            .split_inclusive('\n')
            .nth(display_line)
            .unwrap_or("");
        egui::text::CCursor::new(
            line_start + column.min(line.trim_end_matches('\n').chars().count()),
        )
    };
    egui::text::CCursorRange::two(
        projected_cursor(source_position(old, cursor.primary)),
        projected_cursor(source_position(old, cursor.secondary)),
    )
}

fn normalized_byte_to_original(original: &str, normalized: usize) -> usize {
    let mut byte = 0;
    let mut shown = 0;
    while shown < normalized && byte < original.len() {
        byte += if original.as_bytes()[byte] == b'\r'
            && original.as_bytes().get(byte + 1) == Some(&b'\n')
        {
            2
        } else {
            1
        };
        shown += 1;
    }
    byte
}

fn original_byte_to_normalized_chars(original: &str, byte_offset: usize) -> usize {
    let mut byte = 0;
    let mut chars = 0;
    while byte < byte_offset && byte < original.len() {
        if original.as_bytes()[byte] == b'\r' && original.as_bytes().get(byte + 1) == Some(&b'\n') {
            byte += 2;
        } else {
            let width = original[byte..].chars().next().map_or(1, char::len_utf8);
            byte += width;
        }
        chars += 1;
    }
    chars
}

/// Translate an edit in the compact Merge projection back to the source
/// working copy. It deliberately rejects edits that bridge an omitted run: a
/// Backspace over that visual boundary must never erase hidden identical text.
fn projected_editor_edit(
    original: &str,
    projection: &MergeProjection,
    edited: &str,
    selection: Option<Range<usize>>,
) -> Result<(Range<usize>, String), &'static str> {
    let (projected_range, replacement) = editor_byte_edit(&projection.text, edited, selection);
    let first = projection
        .spans
        .iter()
        .position(|span| {
            span.projected.start <= projected_range.start
                && (projected_range.start < span.projected.end
                    || (projected_range.start == projection.text.len()
                        && span.projected.end == projection.text.len()))
        })
        .ok_or("This edit is outside the visible changed lines.")?;
    let last = if projected_range.is_empty() {
        first
    } else {
        projection
            .spans
            .iter()
            .position(|span| {
                span.projected.start <= projected_range.end
                    && projected_range.end <= span.projected.end
            })
            .ok_or("This edit crosses hidden unchanged lines. Turn off Show only differences to edit across them.")?
    };
    let start_span = &projection.spans[first];
    let end_span = &projection.spans[last];
    let normalized_original = original.replace("\r\n", "\n").replace('\r', "\n");
    if projection.spans[first..=last]
        .windows(2)
        .any(|pair| pair[1].source_line != pair[0].source_line + 1)
    {
        return Err(
            "This edit crosses hidden unchanged lines. Turn off Show only differences to edit across them.",
        );
    }
    // A projected seam can sit immediately before omitted source lines. Do
    // not allow deleting that newline, which would join a visible line to
    // hidden content in the backing file.
    if projection.spans[first..=last]
        .iter()
        .enumerate()
        .any(|(offset, span)| {
            let index = first + offset;
            let next_is_hidden = projection
                .spans
                .get(index + 1)
                .map_or(span.source.end < normalized_original.len(), |next| {
                    next.source_line != span.source_line + 1
                });
            let newline = span
                .projected
                .end
                .checked_sub(1)
                .and_then(|index| projection.text.as_bytes().get(index))
                == Some(&b'\n');
            next_is_hidden
                && newline
                && projected_range.start <= span.projected.end - 1
                && span.projected.end <= projected_range.end
        })
    {
        return Err(
            "This edit would join hidden unchanged lines. Turn off Show only differences to make that change.",
        );
    }
    let normalized = start_span.source.start + (projected_range.start - start_span.projected.start)
        ..end_span.source.start + (projected_range.end - end_span.projected.start);
    let range = normalized_byte_to_original(original, normalized.start)
        ..normalized_byte_to_original(original, normalized.end);
    let mut offset = 0;
    let ending = text_segments(original)
        .iter()
        .find_map(|(text, ending)| {
            let end = offset + text.len() + ending.len();
            let contains = range.start < end;
            offset = end;
            (contains && !ending.is_empty()).then_some(*ending)
        })
        .or_else(|| {
            text_segments(original)
                .iter()
                .find_map(|(_, ending)| (!ending.is_empty()).then_some(*ending))
        })
        .unwrap_or("\n");
    Ok((range, replacement.replace('\n', ending)))
}
#[cfg(test)]
fn restore_line_endings(original: &str, edited: &str) -> String {
    let (range, replacement) = editor_byte_edit(original, edited, None);
    let mut text = original.to_owned();
    text.replace_range(range, &replacement);
    text
}
fn side_of(side: usize) -> TwoWaySide {
    if side == 0 {
        TwoWaySide::Left
    } else {
        TwoWaySide::Right
    }
}
fn side_index(side: TwoWaySide) -> usize {
    if side == TwoWaySide::Left { 0 } else { 1 }
}
#[cfg(test)]
mod tests {
    use super::*;
    use versus::SearchChange;
    #[test]
    fn equal_folder_difference_results_adopt_the_new_cache_without_resetting_navigation() {
        let mut view = InteractiveView::default();
        let original = Arc::new(vec![PathBuf::from("changed.txt")]);
        view.set_folder_differences(original);
        view.folder_difference_index = Some(0);
        let refreshed = Arc::new(vec![PathBuf::from("changed.txt")]);
        view.set_folder_differences(refreshed.clone());
        assert!(Arc::ptr_eq(&view.folder_differences, &refreshed));
        assert_eq!(view.folder_difference_index, Some(0));
    }

    #[test]
    fn diff_starts_disabled() {
        assert!(!InteractiveView::default().run_diff_enabled())
    }
    #[test]
    fn activity_status_requires_a_running_action() {
        let ctx = egui::Context::default();
        let mut view = view("left\n", "right\n");
        view.status = "Saved 2 working copies.".into();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert!(!view.render_activity_status(ui));
        });
        output.drop_without_applying_deltas();
        assert_eq!(view.guidance_message(), "Saved 2 working copies.");
        let (_sender, receiver) = mpsc::channel();
        view.side_loading[0] = Some(SideLoad {
            receiver,
            path: "left.rs".into(),
        });
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert!(view.render_activity_status(ui));
        });
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "Loading selected files…")));
        output.drop_without_applying_deltas();
        view.side_loading[0] = None;
        view.error = Some("Load failed".into());
        ctx.run_ui(egui::RawInput::default(), |ui| {
            assert!(
                !view.render_activity_status(ui),
                "errors are shown by the separate summary"
            );
        })
        .drop_without_applying_deltas();
    }
    #[test]
    fn change_label_reports_count_before_navigation_and_position_afterward() {
        let ctx = egui::Context::default();
        let mut view = InteractiveView::default();
        view.folder_overview = true;
        view.folder_differences = Arc::new(vec![PathBuf::from("one"), PathBuf::from("two")]);
        assert_eq!(view.change_label(), "2 changes");
        view.jump_diff(true, &ctx);
        assert_eq!(view.change_label(), "Change 1 of 2");
    }
    #[test]
    fn keyboard_edit_before_repeated_lines_preserves_caret_endings_and_history() {
        let ctx = egui::Context::default();
        let mut view = view("x\r\nx\np\r", "right\n");
        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        view.caret[0] = 0;
        view.sync_editor_cursor(0);
        ctx.memory_mut(|memory| memory.request_focus(view.editor_id(0)));
        frame(&mut view, &ctx, vec![egui::Event::Text("x\n".into())])
            .drop_without_applying_deltas();
        assert_eq!(view.working_text(0), Some("x\r\nx\r\nx\np\r"));
        view.move_history(false);
        assert_eq!(view.working_text(0), Some("x\r\nx\np\r"));
        view.move_history(true);
        assert_eq!(view.working_text(0), Some("x\r\nx\r\nx\np\r"));
        assert!(view.diff_job.is_none());
    }

    #[test]
    fn byte_edit_handles_unicode_selection_backspace_and_delete() {
        for (original, edited, cursor, range, replacement) in [
            ("é\r\na\n", "é\nβ\n", 2..3, 4..5, "β"),
            ("x\r\nx\r\n", "xx\n", 2..2, 1..3, ""),
            ("x\r\nx\r\n", "x\nx", 3..3, 4..6, ""),
        ] {
            let edit = editor_byte_edit(original, edited, Some(cursor));
            assert_eq!(edit, (range, replacement.to_owned()));
        }
    }

    #[test]
    fn filtered_merge_projection_edits_only_visible_source_lines_and_guards_hidden_seams() {
        let mut changed = BTreeMap::new();
        changed.insert(1, (DirectoryEntryState::Different, Vec::new()));
        changed.insert(3, (DirectoryEntryState::Different, Vec::new()));
        let original = "same\r\nold\r\nsame\r\nlater\r\ntrailing\r\n";
        let displayed = original.replace("\r\n", "\n");
        let projection = merge_projection(&displayed, &changed);
        assert_eq!(projection.text, "old\nlater\n");
        assert_eq!(
            projection
                .spans
                .iter()
                .map(|span| span.source_line)
                .collect::<Vec<_>>(),
            [1, 3]
        );

        // Typing at the start of the second visible block maps to its source
        // line, rather than the preceding hidden seam.
        let (range, replacement) =
            projected_editor_edit(original, &projection, "old\n!later\n", None).unwrap();
        assert_eq!(&original[range.clone()], "");
        assert_eq!(replacement, "!");
        let mut edited = original.to_owned();
        edited.replace_range(range, &replacement);
        assert_eq!(edited, "same\r\nold\r\nsame\r\n!later\r\ntrailing\r\n");

        // Removing the newline before the omitted middle line must not join
        // the visible and hidden text in the backing document.
        assert!(projected_editor_edit(original, &projection, "oldlater\n", None).is_err());
        // The final visible newline is also protected when the remainder of
        // the source is hidden.
        assert!(projected_editor_edit(original, &projection, "old\nlater", None).is_err());
    }

    #[test]
    fn filtered_merge_chunk_runs_never_cross_hidden_source_lines() {
        assert_eq!(
            visible_chunk_runs(&(0..6), &[0, 2, 3, 5]),
            vec![0..1, 2..4, 5..6]
        );
    }

    #[test]
    fn merge_caret_uses_font_height_without_changing_row_spacing() {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            for zoom in [1.0, 2.0] {
                for text in ["first\nsecond\n", "first\n\nthird\n", ""] {
                    for side in 0..2usize {
                        for run_diff in [false, true] {
                            let ctx = egui::Context::default();
                            set_theme(&ctx, theme);
                            ctx.set_zoom_factor(zoom);
                            ctx.style_mut_of(theme, |style| {
                                style.visuals.text_cursor.blink = false
                            });
                            let stroke = ctx.style_of(theme).visuals.text_cursor.stroke;
                            let mut view = view(text, text);
                            view.run_diff = run_diff;
                            view.merge_options = [false; 3];
                            if run_diff {
                                view.diff = compare_loaded_text(
                                    [text, text],
                                    &AtomicBool::new(false),
                                    &FileViewOptions::default(),
                                    [&[], &[]],
                                )
                                .unwrap();
                            }
                            for _ in 0..3 {
                                frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                            }
                            view.caret[side] = usize::from(!text.is_empty());
                            view.sync_editor_cursor(side);
                            ctx.memory_mut(|memory| memory.request_focus(view.editor_id(side)));
                            let output = frame(&mut view, &ctx, vec![]);
                            assert_eq!(
                                ctx.style_of(theme).visuals.text_cursor.max_height,
                                None,
                                "merge caret styling must not leak to other text fields"
                            );
                            let font = comparison_font(&ctx);
                            let font_height = ctx.fonts_mut(|fonts| fonts.row_height(&font));
                            let cursors: Vec<_> = output
                                .shapes
                                .iter()
                                .filter_map(|shape| match &shape.shape {
                                    egui::Shape::LineSegment {
                                        points,
                                        stroke: painted,
                                    } if *painted == stroke && points[0].x == points[1].x => {
                                        Some((points[1].y - points[0].y).abs())
                                    }
                                    _ => None,
                                })
                                .collect();
                            assert_eq!(
                                cursors.len(),
                                1,
                                "one focused editor caret: {theme:?}, zoom={zoom}, side={side}"
                            );
                            assert!(
                                (cursors[0] - (font_height + 3.0)).abs() < 0.1,
                                "caret should use the font height with native caret margins, not padded rows: height={}, font={font_height}, {theme:?}, zoom={zoom}, side={side}, diff={run_diff}, text={text:?}",
                                cursors[0]
                            );
                            for shape in &output.shapes {
                                if let egui::Shape::Text(text_shape) = &shape.shape {
                                    if !text.is_empty() && text_shape.galley.text() == text {
                                        let rows = &text_shape.galley.rows;
                                        assert!(
                                            (rows[1].pos.y
                                                - rows[0].pos.y
                                                - super::ROW_HEIGHT.max(font_height.ceil() + 8.0))
                                            .abs()
                                                < 0.1
                                        );
                                    }
                                }
                            }
                            output.drop_without_applying_deltas();
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn merge_undo_redo_shortcuts_keep_the_diff_view_stable() {
        let left = "prefix\r\nold\r\nsame\r\nlater\r\nsuffix\r\n";
        let right = "prefix\nnew\nsame\nlaterR\nsuffix\n";
        for side in 0..2usize {
            for filtered in [true, false] {
                for settled in [false, true] {
                    let ctx = egui::Context::default();
                    let mut view = view(left, right);
                    view.run_diff = true;
                    view.merge_options = [filtered, true, true];
                    view.diff = compare_loaded_text(
                        [left, right],
                        &AtomicBool::new(false),
                        &FileViewOptions::default(),
                        [&[], &[]],
                    )
                    .unwrap();
                    for _ in 0..3 {
                        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                    }
                    view.caret[side] = 1;
                    view.sync_editor_cursor(side);
                    ctx.memory_mut(|memory| memory.request_focus(view.editor_id(side)));
                    frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                    frame(
                        &mut view,
                        &ctx,
                        vec![egui::Event::Text("!β\nsecond".into())],
                    )
                    .drop_without_applying_deltas();
                    view.diff_not_before = Some(Instant::now() + Duration::from_secs(60));
                    frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                    if settled {
                        view.diff_not_before = None;
                        for _ in 0..100 {
                            view.poll_diff(&ctx);
                            if view.diff.is_some() {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        assert!(view.diff.is_some() && view.pending_merge_presentation.is_none());
                        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                    }
                    let edited = view.working_text(side).unwrap().to_owned();
                    let edited_projection =
                        view.editor_projection[side].as_ref().unwrap().text.clone();
                    let original = if side == 0 { left } else { right };
                    for key in [egui::Key::Z, egui::Key::Y] {
                        let events = [true, false]
                            .into_iter()
                            .map(|pressed| egui::Event::Key {
                                key,
                                physical_key: None,
                                pressed,
                                repeat: false,
                                modifiers: egui::Modifiers::CTRL,
                            })
                            .collect();
                        frame(&mut view, &ctx, events).drop_without_applying_deltas();
                        let expected = if key == egui::Key::Z {
                            if filtered {
                                if side == 0 {
                                    "old\nlater\n".into()
                                } else {
                                    "new\nlaterR\n".into()
                                }
                            } else {
                                original.replace("\r\n", "\n")
                            }
                        } else {
                            edited_projection.clone()
                        };
                        assert_eq!(
                            view.working_text(side),
                            Some(if key == egui::Key::Z {
                                original
                            } else {
                                &edited
                            })
                        );
                        assert_eq!(
                            view.editor_projection[side].as_ref().unwrap().text,
                            expected,
                            "Ctrl+{key:?} must not expand/redecorate the file while refreshing: side={side}, filtered={filtered}"
                        );
                        assert!(view.pending_merge_presentation.is_some());
                        view.diff_not_before = Some(Instant::now() + Duration::from_secs(60));
                    }
                    frame(&mut view, &ctx, vec![egui::Event::Text("?".into())])
                        .drop_without_applying_deltas();
                    assert!(
                        view.working_text(side).unwrap().contains("second?"),
                        "typing after redo should continue at the edited position: side={side}, filtered={filtered}, text={:?}",
                        view.working_text(side)
                    );
                    assert_eq!(
                        view.working_text(1 - side),
                        Some(if side == 0 { right } else { left })
                    );
                }
            }
        }
    }

    #[test]
    fn replacement_batch_undo_redo_preserves_both_diff_panes() {
        let left = "prefix\nleft match\nsuffix\n";
        let right = "prefix\nright match\nsuffix\n";
        let ctx = egui::Context::default();
        let mut view = view(left, right);
        let preview = preview_for(
            &view,
            &[("left.rs", 12..17, "found"), ("right.rs", 13..18, "found")],
        );
        view.apply_search_preview(&preview).unwrap();
        view.run_diff = true;
        view.merge_options = [true; 3];
        view.diff = compare_loaded_text(
            [view.working_text(0).unwrap(), view.working_text(1).unwrap()],
            &AtomicBool::new(false),
            &FileViewOptions::default(),
            [&[], &[]],
        )
        .unwrap();
        for _ in 0..3 {
            frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        }
        for (key, word) in [(egui::Key::Z, "match"), (egui::Key::Y, "found")] {
            let events = [true, false]
                .into_iter()
                .map(|pressed| egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::CTRL,
                })
                .collect();
            frame(&mut view, &ctx, events).drop_without_applying_deltas();
            assert_eq!(
                view.editor_projection[0].as_ref().unwrap().text,
                format!("left {word}\n")
            );
            assert_eq!(
                view.editor_projection[1].as_ref().unwrap().text,
                format!("right {word}\n")
            );
            assert_eq!(
                view.working_text(0),
                Some(format!("prefix\nleft {word}\nsuffix\n").as_str())
            );
            assert_eq!(
                view.working_text(1),
                Some(format!("prefix\nright {word}\nsuffix\n").as_str())
            );
            assert!(view.pending_merge_presentation.is_some());
            view.diff_not_before = Some(Instant::now() + Duration::from_secs(60));
        }
    }

    #[test]
    fn failed_diff_refresh_drops_provisional_decoration_without_losing_edits() {
        for disconnected in [false, true] {
            let ctx = egui::Context::default();
            let mut view = view("old\n", "new\n");
            view.run_diff = true;
            view.diff = compare_loaded_text(
                ["old\n", "new\n"],
                &AtomicBool::new(false),
                &FileViewOptions::default(),
                [&[], &[]],
            )
            .unwrap();
            view.workspace
                .as_mut()
                .unwrap()
                .edit_bytes(TwoWaySide::Left, 0..0, "!".into())
                .unwrap();
            view.invalidate_edited_diff(0, "old\n", 0..0);
            assert!(view.pending_merge_presentation.is_some());
            let (sender, receiver) = mpsc::channel();
            if !disconnected {
                sender.send(Err("Comparison failed".into())).unwrap();
            }
            drop(sender);
            view.diff_job = Some(DiffJob {
                receiver,
                cancel: Arc::default(),
                progress: Arc::default(),
            });
            view.diff_dirty = false;
            view.poll_diff(&ctx);
            assert!(view.pending_merge_presentation.is_none() && view.diff.is_none());
            assert!(view.error.is_some());
            assert_eq!(view.working_text(0), Some("!old\n"));
            view.selected_side = 0;
            view.move_history(false);
            assert_eq!(view.working_text(0), Some("old\n"));
        }
    }

    #[test]
    fn pending_single_line_edits_keep_source_color_and_precise_inline_ranges() {
        let mut decoration = MergeDecoration::default();
        decoration
            .lines
            .insert(0, (DirectoryEntryState::LeftOnly, vec![2..4, 6..8]));
        decoration.rebase("abcdefgh\n", 0..0, "文abcdefgh\n");
        assert_eq!(
            decoration.lines[&0],
            (DirectoryEntryState::LeftOnly, vec![0..3, 5..7, 9..11])
        );
        decoration.rebase("文abcdefgh\n", 5..7, "文abefgh\n");
        assert_eq!(
            decoration.lines[&0],
            (DirectoryEntryState::LeftOnly, vec![0..3, 7..9])
        );
        decoration.rebase("文abefgh\n", 3..5, "文Xefgh\n");
        assert_eq!(
            decoration.lines[&0],
            (DirectoryEntryState::LeftOnly, vec![0..4, 6..8])
        );
    }

    #[test]
    fn pending_diff_decorations_follow_exact_byte_anchors_and_line_endings() {
        let before = "x\r\nx\nchanged\rkeep\r\n";
        let mut decoration = MergeDecoration::default();
        decoration
            .lines
            .insert(1, (DirectoryEntryState::Different, vec![0..1]));
        decoration
            .lines
            .insert(2, (DirectoryEntryState::Different, vec![0..7]));
        decoration.endings.insert(2, versus::DisplayLineEnding::Cr);
        let after = format!("x\r\n{before}");
        decoration.rebase(before, 0..0, &after);
        assert_eq!(decoration.lines[&2].1, [0..1]);
        assert_eq!(decoration.lines[&3].1, [0..7]);
        assert_eq!(decoration.endings[&3], versus::DisplayLineEnding::Cr);
        // UTF-8 text inserted into that moved line gets valid highlight ranges.
        let byte = after.find("changed").unwrap();
        let mut next = after.clone();
        next.insert_str(byte, "文");
        decoration.rebase(&after, byte..byte, &next);
        assert_eq!(decoration.lines[&3].1, [0.."文changed".len()]);
        assert_eq!(decoration.endings[&3], versus::DisplayLineEnding::Cr);
    }

    #[test]
    fn typing_with_unfiltered_diff_preserves_other_line_decorations_on_both_sides() {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            for side in 0..2usize {
                let ctx = egui::Context::default();
                set_theme(&ctx, theme);
                let mut view = view("same\r\nold\r\nother\r\n", "same\nnew\nanother\n");
                view.run_diff = true;
                view.merge_options = [false, false, true];
                view.diff = compare_loaded_text(
                    [view.working_text(0).unwrap(), view.working_text(1).unwrap()],
                    &AtomicBool::new(false),
                    &FileViewOptions::default(),
                    [&[], &[]],
                )
                .unwrap();
                for _ in 0..3 {
                    frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                }
                let original_peer = view.working_text(1 - side).unwrap().to_owned();
                let untouched = view.merge_decoration(side).lines[&2].clone();
                view.caret[side] = 1;
                view.sync_editor_cursor(side);
                ctx.memory_mut(|memory| memory.request_focus(view.editor_id(side)));
                frame(&mut view, &ctx, vec![egui::Event::Text("文".into())])
                    .drop_without_applying_deltas();
                view.diff_not_before = Some(Instant::now() + Duration::from_secs(1));
                frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                assert_eq!(view.working_text(1 - side), Some(original_peer.as_str()));
                assert!(view.working_text(side).unwrap().contains(if side == 0 {
                    "文old\r\n"
                } else {
                    "文new\n"
                }));
                assert_eq!(view.merge_decoration(side).lines[&2], untouched);
                assert_eq!(
                    view.editor_projection[side].as_ref().unwrap().source_lines,
                    [0, 1, 2]
                );
                view.set_run_diff(false);
                assert!(view.pending_merge_presentation.is_none() && view.diff_job.is_none());
            }
        }
    }

    #[test]
    fn typing_with_diff_keeps_filtered_content_and_highlights_between_results() {
        let left = "prefix\nold\nsame\nlater\nsuffix\n";
        let right = "prefix\nnew\nsame\nlaterR\nsuffix\n";
        let ctx = egui::Context::default();
        let mut view = view(left, right);
        view.run_diff = true;
        view.merge_options = [true; 3];
        view.diff = compare_loaded_text(
            [left, right],
            &AtomicBool::new(false),
            &FileViewOptions::default(),
            [&[], &[]],
        )
        .unwrap();
        for _ in 0..3 {
            frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        }
        view.diff_rows = view
            .diff
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| (row.state != DirectoryEntryState::Same).then_some(index))
            .collect();
        let counts = view.footer_legend_counts();
        let label = view.change_label();
        let stale_comparison = view.diff.as_ref().unwrap().clone();
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        view.diff_job = Some(DiffJob {
            receiver,
            cancel: cancel.clone(),
            progress: Arc::default(),
        });
        let id = view.editor_id(0);
        let mut state = egui::TextEdit::load_state(&ctx, id).unwrap();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(0),
            )));
        state.store(&ctx, id);
        ctx.memory_mut(|memory| memory.request_focus(id));
        frame(&mut view, &ctx, vec![egui::Event::Text("!".into())]).drop_without_applying_deltas();
        assert!(cancel.load(Ordering::Relaxed));
        sender.send(Ok(Some(stale_comparison))).unwrap();
        view.diff_not_before = Some(Instant::now() + Duration::from_secs(1));
        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        assert!(
            view.loaded_pair_comparison().is_none(),
            "pending display metadata is not a completed comparison"
        );
        assert_eq!(view.footer_legend_counts(), counts);
        assert_eq!(view.change_label(), label);
        assert_eq!(
            view.working_text(0),
            Some("prefix\n!old\nsame\nlater\nsuffix\n")
        );
        assert_eq!(
            view.editor_projection[0].as_ref().unwrap().text,
            "!old\nlater\n",
            "editing must not briefly expand hidden unchanged lines while diff is pending"
        );
        assert_eq!(
            view.editor_projection[1].as_ref().unwrap().text,
            "new\nlaterR\n"
        );
        assert_eq!(
            view.merge_decoration(0).lines[&1].0,
            DirectoryEntryState::Different,
            "diff decoration should remain visible while refreshing"
        );
        // Continue typing before any worker returns. Newlines must update the
        // compact projection's source coordinates without exposing hidden text.
        frame(&mut view, &ctx, vec![egui::Event::Text("β\nsecond".into())])
            .drop_without_applying_deltas();
        view.diff_not_before = Some(Instant::now() + Duration::from_secs(1));
        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        assert_eq!(
            view.editor_projection[0].as_ref().unwrap().text,
            "!β\nsecondold\nlater\n"
        );
        assert_eq!(
            view.editor_projection[0].as_ref().unwrap().source_lines,
            [1, 2, 4]
        );
        frame(&mut view, &ctx, vec![egui::Event::Text("?".into())]).drop_without_applying_deltas();
        assert_eq!(
            view.working_text(0),
            Some("prefix\n!β\nsecond?old\nsame\nlater\nsuffix\n")
        );
        // A deliberate selection joins only the two adjacent edited lines.
        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        let mut state = egui::TextEdit::load_state(&ctx, id).unwrap();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(2),
                egui::text::CCursor::new(3),
            )));
        state.store(&ctx, id);
        frame(
            &mut view,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Backspace,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        )
        .drop_without_applying_deltas();
        view.diff_not_before = Some(Instant::now() + Duration::from_secs(1));
        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        assert_eq!(
            view.editor_projection[0].as_ref().unwrap().text,
            "!βsecond?old\nlater\n"
        );
        assert_eq!(
            view.editor_projection[0].as_ref().unwrap().source_lines,
            [1, 3]
        );
        assert!(view.filtered_merge_range_hidden(TwoWaySide::Left, &(1..4)));
        // Real comparison results take over without resetting editing/history.
        view.diff_not_before = None;
        for _ in 0..100 {
            view.poll_diff(&ctx);
            if view.diff.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(view.diff.is_some() && view.pending_merge_presentation.is_none());
        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        assert_eq!(
            view.editor_projection[0].as_ref().unwrap().text,
            "!βsecond?old\nlater\n"
        );
        view.move_history(false);
        assert_eq!(
            view.working_text(0),
            Some("prefix\n!β\nsecond?old\nsame\nlater\nsuffix\n")
        );
        view.set_run_diff(false);
        assert!(view.pending_merge_presentation.is_none() && view.diff.is_none());
    }

    #[test]
    fn filtered_transfer_restores_the_destination_caret_before_further_typing() {
        let left = "prefix\nold\nsame\nlater\nsuffix\n";
        let right = "prefix\nnew\nsame\nlaterR\nsuffix\n";
        let ctx = egui::Context::default();
        set_theme(&ctx, egui::Theme::Dark);
        let mut view = view(left, right);
        view.editor_context = Some(ctx.clone());
        view.run_diff = true;
        view.merge_options = [true; 3];
        view.diff = compare_loaded_text(
            [left, right],
            &AtomicBool::new(false),
            &FileViewOptions::default(),
            [&[], &[]],
        )
        .unwrap();
        for _ in 0..3 {
            frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        }
        view.caret[1] = 3;
        view.transfer(TwoWaySide::Left, 3..4);
        frame(&mut view, &ctx, vec![egui::Event::Text("!".into())]).drop_without_applying_deltas();
        assert_eq!(
            view.working_text(1),
            Some("prefix\nnew\nsame\nlater\n!laterR\nsuffix\n")
        );
        assert_eq!(view.working_text(0), Some(left));
        view.move_history(false);
        view.move_history(false);
        assert_eq!(view.working_text(1), Some(right));
    }

    #[test]
    fn cursor_remap_keeps_the_same_source_line_when_filtering_changes() {
        let full = EditorProjection {
            text: "same\nchanged\nsame\nlater\n".into(),
            source_lines: vec![0, 1, 2, 3],
        };
        let filtered = EditorProjection {
            text: "changed\nlater\n".into(),
            source_lines: vec![1, 3],
        };
        let cursor = egui::text::CCursorRange::one(egui::text::CCursor::new(18));
        let remapped = remap_projection_cursor(cursor, &full, &filtered);
        assert_eq!(usize::from(remapped.primary.index), 8);
        let restored = remap_projection_cursor(remapped, &filtered, &full);
        assert_eq!(usize::from(restored.primary.index), 18);
    }
    #[test]
    fn folder_navigation_uses_changed_paths_without_comparing_text() {
        let ctx = egui::Context::default();
        let mut view = InteractiveView::default();
        view.editing = true;
        view.folder_overview = true;
        view.set_folder_differences(vec![PathBuf::from("nested/a"), PathBuf::from("b")]);
        assert!(!view.has_differences());
        view.run_diff = true;
        assert!(view.has_differences());
        view.jump_diff(true, &ctx);
        assert_eq!(view.take_folder_jump(), Some(PathBuf::from("nested/a")));
        view.jump_diff(true, &ctx);
        view.jump_diff(true, &ctx);
        assert_eq!(view.take_folder_jump(), Some(PathBuf::from("b")));
        view.jump_diff(false, &ctx);
        assert_eq!(view.take_folder_jump(), Some(PathBuf::from("nested/a")));
        assert!(view.diff_job.is_none());
    }
    #[test]
    fn incomplete_optional_comparison_is_never_cached_as_identical() {
        let mut view = view("left\n", "right\n");
        view.run_diff = true;
        view.diff = Some(FileComparison {
            rows: Vec::new(),
            message: Some("Comparison budget reached".into()),
        });
        assert_eq!(
            view.loaded_pair_comparison().unwrap().1,
            DirectoryEntryState::Different
        );
    }
    #[test]
    fn keyboard_redo_is_consumed_before_the_less_specific_undo_shortcut() {
        let ctx = egui::Context::default();
        let mut view = view("left\n", "right\n");
        view.caret[1] = 1;
        view.transfer(TwoWaySide::Left, 0..1);
        let key = |modifiers| egui::Event::Key {
            key: egui::Key::Z,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        frame(&mut view, &ctx, vec![key(egui::Modifiers::COMMAND)]).drop_without_applying_deltas();
        assert_eq!(view.working_text(1), Some("right\n"));
        frame(
            &mut view,
            &ctx,
            vec![key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT)],
        )
        .drop_without_applying_deltas();
        assert_eq!(view.working_text(1), Some("right\nleft\n"));
    }
    #[test]
    fn transfer_undo_targets_the_destination_after_source_edits() {
        let ctx = egui::Context::default();
        let mut view = view("left\n", "right\n");
        view.workspace
            .as_mut()
            .unwrap()
            .set_text(TwoWaySide::Left, "edited\n");
        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        view.selected_side = 0;
        view.caret[1] = 1;
        view.transfer(TwoWaySide::Left, 0..1);
        view.move_history(false);
        assert_eq!(view.working_text(0), Some("edited\n"));
        assert_eq!(view.working_text(1), Some("right\n"));
        view.move_history(true);
        assert_eq!(view.working_text(1), Some("right\nedited\n"));
    }
    fn view(left: &str, right: &str) -> InteractiveView {
        let paths = [PathBuf::from("left.rs"), PathBuf::from("right.rs")];
        let files = std::array::from_fn(|side| {
            let text = if side == 0 { left } else { right };
            WorkingFile {
                identity: None,
                path: Some(paths[side].clone()),
                text: text.into(),
                encoding: versus::TextEncoding::Utf8 { bom: false },
                fingerprint: versus::FileFingerprint::Bytes(text.as_bytes().to_vec()),
            }
        });
        InteractiveView {
            workspace: Some(TwoWayWorkspace::new(left, right)),
            files: Some(files),
            paths: Some(paths),
            editing: true,
            ..Default::default()
        }
    }
    fn readonly_comparison(lines: usize) -> FileComparison {
        FileComparison {
            rows: (0..lines)
                .map(|index| versus::FileComparisonRow {
                    left: Some((index + 1, format!("left\tline {index}"))),
                    right: Some((index + 1, format!("right line {index}"))),
                    state: if index % 3 == 0 {
                        DirectoryEntryState::Different
                    } else {
                        DirectoryEntryState::Same
                    },
                    left_changed: Vec::new(),
                    right_changed: Vec::new(),
                    left_ending: Some(versus::DisplayLineEnding::Lf),
                    right_ending: Some(versus::DisplayLineEnding::Lf),
                })
                .collect(),
            message: None,
        }
    }
    fn partial_view(path: PathBuf, text: &str) -> InteractiveView {
        let file = WorkingFile {
            identity: None,
            path: Some(path.clone()),
            text: text.into(),
            encoding: versus::TextEncoding::Utf8 { bom: false },
            fingerprint: versus::FileFingerprint::Bytes(text.as_bytes().to_vec()),
        };
        let mut view = InteractiveView {
            editing: true,
            ..Default::default()
        };
        view.register_file(path.clone(), file.clone());
        view.side_sources[0] = Some(path);
        view.side_files[0] = Some(file);
        view.activate_side_workspace();
        view
    }
    fn settle_save(view: &mut InteractiveView, ctx: &egui::Context) {
        for _ in 0..100 {
            view.poll_save(ctx);
            if !view.is_saving() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("save did not finish");
    }
    #[test]
    fn single_side_edits_undo_redo_and_repeated_saves_use_fresh_fingerprints() {
        let root = std::env::temp_dir().join(format!("versus-single-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("only.txt");
        std::fs::write(&path, "old\n").unwrap();
        let mut view = partial_view(path.clone(), "old\n");
        view.workspace
            .as_mut()
            .unwrap()
            .edit_bytes(TwoWaySide::Left, 0..0, "first ".into())
            .unwrap();
        view.selected_side = 0;
        view.move_history(false);
        assert_eq!(view.working_text(0), Some("old\n"));
        view.move_history(true);
        assert_eq!(view.working_text(0), Some("first old\n"));
        let ctx = egui::Context::default();
        view.save_sides(vec![0]);
        settle_save(&mut view, &ctx);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first old\n");
        assert!(!view.workspace.as_ref().unwrap().is_dirty(TwoWaySide::Left));
        view.workspace
            .as_mut()
            .unwrap()
            .set_text(TwoWaySide::Left, "second\n");
        view.save_sides(vec![0]);
        settle_save(&mut view, &ctx);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second\n");
        assert!(!view.workspace.as_ref().unwrap().is_dirty(TwoWaySide::Left));
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn pending_or_failed_peer_load_preserves_healthy_side_editor_and_history() {
        let mut view = partial_view(PathBuf::from("left.txt"), "left\n");
        view.workspace
            .as_mut()
            .unwrap()
            .edit_bytes(TwoWaySide::Left, 0..0, "edited ".into())
            .unwrap();
        view.publish_active();
        view.selected_side = 0;
        view.scroll[0] = 37.0;
        let editor = egui::Id::new(("two-way-edit", view.side_generation[0], 0usize));
        let (sender, receiver) = mpsc::channel();
        view.side_sources[1] = Some(PathBuf::from("missing.txt"));
        view.side_generation[1] = view.side_generation[1].wrapping_add(1);
        view.activate_side_workspace();
        view.side_loading[1] = Some(SideLoad {
            receiver,
            path: PathBuf::from("missing.txt"),
        });
        assert_eq!(view.working_text(0), Some("edited left\n"));
        assert_eq!(view.scroll[0], 37.0);
        assert_eq!(
            editor,
            egui::Id::new(("two-way-edit", view.side_generation[0], 0usize))
        );
        view.move_history(false);
        assert_eq!(view.working_text(0), Some("left\n"));
        view.move_history(true);
        assert_eq!(view.working_text(0), Some("edited left\n"));
        sender.send(Err("unavailable".into())).unwrap();
        view.poll(&egui::Context::default());
        assert!(
            view.error
                .as_deref()
                .is_some_and(|error| error.contains("unavailable"))
        );
        assert_eq!(view.working_text(0), Some("edited left\n"));
        assert!(!view.has_actual_pair());
        let ctx = egui::Context::default();
        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        assert!(ctx.read_response(editor).is_some());
        assert!(
            ctx.read_response(egui::Id::new((
                "two-way-edit",
                view.side_generation[1],
                1usize,
            )))
            .is_none()
        );
    }
    #[test]
    fn cached_side_reopen_promotes_to_a_canonical_pair() {
        let mut view = InteractiveView {
            editing: true,
            ..Default::default()
        };
        for (side, text) in ["left\n", "right\n"].into_iter().enumerate() {
            let path = PathBuf::from(if side == 0 { "left.txt" } else { "right.txt" });
            view.register_file(
                path.clone(),
                WorkingFile {
                    identity: None,
                    path: Some(path),
                    text: text.into(),
                    encoding: versus::TextEncoding::Utf8 { bom: false },
                    fingerprint: versus::FileFingerprint::Bytes(text.as_bytes().to_vec()),
                },
            );
        }
        view.open_sources([None, None]);
        view.open_sources([
            Some(PathBuf::from("left.txt")),
            Some(PathBuf::from("right.txt")),
        ]);
        assert_eq!(
            view.pair_paths().unwrap(),
            &[PathBuf::from("left.txt"), PathBuf::from("right.txt")]
        );
        assert!(view.files.is_some() && view.workspace.is_some());
        view.set_run_diff(true);
        assert!(view.loaded_pair_comparison().is_none());
    }
    fn frame(
        view: &mut InteractiveView,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(900.0 / ctx.zoom_factor(), 650.0 / ctx.zoom_factor()),
                )),
                events,
                ..Default::default()
            },
            |ui| view.render(ui),
        )
    }
    fn click(view: &mut InteractiveView, ctx: &egui::Context, id: egui::Id) {
        click_frame(view, ctx, id).drop_without_applying_deltas();
    }
    fn click_frame(
        view: &mut InteractiveView,
        ctx: &egui::Context,
        id: egui::Id,
    ) -> egui::FullOutput {
        let position = ctx.read_response(id).unwrap().rect.center();
        let pointer = |pressed| {
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        frame(view, ctx, pointer(true)).drop_without_applying_deltas();
        frame(view, ctx, pointer(false))
    }
    #[test]
    fn merge_difference_navigation_highlights_and_scrolls_only_corresponding_sides() {
        let mut comparison = readonly_comparison(240);
        for (index, row) in comparison.rows.iter_mut().enumerate() {
            let modified = row.state != DirectoryEntryState::Same;
            row.left.as_mut().unwrap().1 = if modified {
                format!("left change {index}")
            } else {
                format!("shared line {index}")
            };
            row.right.as_mut().unwrap().1 = if modified {
                format!("right change {index}")
            } else {
                format!("shared line {index}")
            };
            if index > 40 {
                row.right.as_mut().unwrap().0 -= 1;
            }
            if index > 130 {
                row.left.as_mut().unwrap().0 -= 1;
            }
        }
        comparison.rows[40].state = DirectoryEntryState::LeftOnly;
        comparison.rows[40].right = None;
        comparison.rows[100].state = DirectoryEntryState::Different;
        comparison.rows[130].state = DirectoryEntryState::RightOnly;
        comparison.rows[130].left = None;
        let texts: [String; 2] = std::array::from_fn(|side| {
            comparison
                .rows
                .iter()
                .filter_map(|row| {
                    if side == 0 {
                        row.left.as_ref()
                    } else {
                        row.right.as_ref()
                    }
                })
                .map(|line| format!("{}\n", line.1))
                .collect()
        });
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            for zoom in [1.0, 2.0] {
                for filtered in [false, true] {
                    let ctx = egui::Context::default();
                    set_theme(&ctx, theme);
                    ctx.set_zoom_factor(zoom);
                    let mut view = view(&texts[0], &texts[1]);
                    view.editor_context = Some(ctx.clone());
                    view.run_diff = true;
                    view.merge_options = [filtered, false, false];
                    view.diff_rows = comparison
                        .rows
                        .iter()
                        .enumerate()
                        .filter_map(|(index, row)| {
                            (row.state != DirectoryEntryState::Same).then_some(index)
                        })
                        .collect();
                    view.diff = Some(comparison.clone());
                    for (target, forward, source_lines, filtered_lines) in [
                        (
                            0usize,
                            true,
                            [Some(0usize), Some(0usize)],
                            [Some(0usize), Some(0usize)],
                        ),
                        (40, true, [Some(40), None], [Some(14), None]),
                        (100, true, [Some(100), Some(99)], [Some(35), Some(34)]),
                        (130, true, [None, Some(129)], [None, Some(45)]),
                        (100, false, [Some(100), Some(99)], [Some(35), Some(34)]),
                    ] {
                        view.scroll = [66.0, 88.0];
                        let target_index = view
                            .diff_rows
                            .iter()
                            .position(|index| *index == target)
                            .unwrap();
                        ctx.memory_mut(|memory| {
                            memory.data.insert_temp(
                                egui::Id::new("interactive-diff-navigation"),
                                if forward {
                                    (target_index + view.diff_rows.len() - 1) % view.diff_rows.len()
                                } else {
                                    target_index + 1
                                },
                            )
                        });
                        for _ in 0..3 {
                            frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                        }
                        let before = view.scroll;
                        click(
                            &mut view,
                            &ctx,
                            egui::Id::new(if forward {
                                "Next difference"
                            } else {
                                "Previous difference"
                            }),
                        );
                        assert_eq!(view.navigation_lines(&ctx), source_lines);
                        let font = comparison_font(&ctx);
                        let row_height = super::ROW_HEIGHT
                            .max(ctx.fonts_mut(|fonts| fonts.row_height(&font)).ceil() + 8.0);
                        for side in 0..2 {
                            let display_line = if filtered {
                                filtered_lines[side]
                            } else {
                                source_lines[side]
                            };
                            assert!(
                                (view.scroll[side]
                                    - display_line
                                        .map_or(before[side], |line| line.saturating_sub(1)
                                            as f32
                                            * row_height))
                                .abs()
                                    < 0.5
                            );
                        }
                        let output = frame(&mut view, &ctx, vec![]);
                        let highlights: Vec<_> = output
                            .shapes
                            .iter()
                            .filter_map(|shape| match &shape.shape {
                                egui::Shape::Rect(rect)
                                    if rect.fill
                                        == Palette::for_context(&ctx)
                                            .accent
                                            .gamma_multiply(0.13) =>
                                {
                                    Some((rect.rect, shape.clip_rect))
                                }
                                _ => None,
                            })
                            .collect();
                        assert_eq!(highlights.len(), source_lines.iter().flatten().count());
                        for (rect, clip) in highlights {
                            assert!(
                                rect.intersect(clip).is_positive(),
                                "navigated row must be visible"
                            );
                            let side =
                                usize::from(rect.center().x > ctx.viewport_rect().center().x);
                            if source_lines[side].is_some_and(|line| line > 0) {
                                assert!(
                                    rect.top() - clip.top() >= row_height - 1.0,
                                    "leave a line above the highlighted change"
                                );
                            }
                            assert!(
                                source_lines[side].is_some(),
                                "do not highlight an unmatched opposite line"
                            );
                        }
                        output.drop_without_applying_deltas();
                    }
                    view.set_run_diff(false);
                    let output = frame(&mut view, &ctx, vec![]);
                    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == Palette::for_context(&ctx).accent.gamma_multiply(0.13))));
                    assert_eq!(view.working_text(0), Some(texts[0].as_str()));
                    assert_eq!(view.working_text(1), Some(texts[1].as_str()));
                    assert!(!view.workspace.as_ref().unwrap().can_undo());
                    output.drop_without_applying_deltas();
                }
            }
        }
    }

    #[test]
    fn merge_navigation_near_file_end_settles_on_the_click_frame() {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            for zoom in [1.0, 2.0] {
                for count in [12, 240] {
                    for filtered in [false, true] {
                        let ctx = egui::Context::default();
                        set_theme(&ctx, theme);
                        ctx.set_zoom_factor(zoom);
                        let text: String = (0..count)
                            .map(|line| format!("fn chunk_{line}() {{}}\n"))
                            .collect();
                        let mut view = view(&text, &text);
                        view.editor_context = Some(ctx.clone());
                        view.run_diff = true;
                        view.merge_options = [filtered, false, false];
                        view.diff = Some(readonly_comparison(count));
                        view.diff_rows = vec![count - 3];
                        for _ in 0..3 {
                            frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                        }
                        let geometry = |output: &egui::FullOutput| {
                            let highlights: Vec<_> = output
                                .shapes
                                .iter()
                                .filter_map(|shape| match &shape.shape {
                                    egui::Shape::Rect(rect)
                                        if rect.fill
                                            == Palette::for_context(&ctx)
                                                .accent
                                                .gamma_multiply(0.13) =>
                                    {
                                        Some((rect.rect, shape.clip_rect))
                                    }
                                    _ => None,
                                })
                                .collect();
                            let text: Vec<_> = output
                                .shapes
                                .iter()
                                .filter_map(|shape| match &shape.shape {
                                    egui::Shape::Text(text)
                                        if text.galley.text().contains("chunk_") =>
                                    {
                                        Some((text.pos, text.galley.size(), shape.clip_rect))
                                    }
                                    _ => None,
                                })
                                .collect();
                            (highlights, text)
                        };
                        for action in ["Next difference", "Previous difference", "Next difference"]
                        {
                            let first = click_frame(&mut view, &ctx, egui::Id::new(action));
                            let first_geometry = geometry(&first);
                            first.drop_without_applying_deltas();
                            assert_eq!(first_geometry.0.len(), 2);
                            assert_eq!(first_geometry.1.len(), 2);
                            let clicked_scroll = view.scroll;
                            for _ in 0..4 {
                                let output = frame(&mut view, &ctx, vec![]);
                                let settled_geometry = geometry(&output);
                                output.drop_without_applying_deltas();
                                assert_eq!(
                                    first_geometry, settled_geometry,
                                    "{action} must settle on the first frame, including when the target cannot scroll to the top: theme={theme:?}, zoom={zoom}, count={count}, filtered={filtered}"
                                );
                                assert_eq!(view.scroll, clicked_scroll);
                            }
                        }
                        assert_eq!(view.working_text(0), Some(text.as_str()));
                        assert_eq!(view.working_text(1), Some(text.as_str()));
                        assert!(!view.workspace.as_ref().unwrap().can_undo());
                    }
                }
            }
        }
    }

    #[test]
    fn readonly_difference_navigation_leaves_a_context_row_and_clamps_at_start() {
        for filtered in [false, true] {
            let ctx = egui::Context::default();
            set_theme(&ctx, egui::Theme::Dark);
            let mut view = view("left\n", "right\n");
            view.editing = false;
            view.diff = Some(readonly_comparison(240));
            view.diff_rows = (0..240).step_by(3).collect();
            view.diff_options = [filtered, false, false];
            frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
            let font = comparison_font(&ctx);
            let row_height =
                super::ROW_HEIGHT.max(ctx.fonts_mut(|fonts| fonts.row_height(&font)).ceil() + 8.0);
            for (forward, display_line) in [
                (true, 0usize),
                (true, if filtered { 1 } else { 3 }),
                (true, if filtered { 2 } else { 6 }),
                (false, if filtered { 1 } else { 3 }),
                (false, 0),
            ] {
                view.jump_diff(forward, &ctx);
                assert_eq!(
                    view.scroll,
                    [display_line.saturating_sub(1) as f32 * row_height; 2]
                );
            }
        }
    }

    #[test]
    fn readonly_presentation_is_reused_and_rebuilds_for_filter_changes() {
        let ctx = egui::Context::default();
        let mut view = view("left\n", "right\n");
        view.editing = false;
        view.diff = Some(readonly_comparison(12));
        view.diff_rows = vec![0, 3, 6, 9];
        for _ in 0..2 {
            ctx.run_ui(egui::RawInput::default(), |ui| {
                view.ensure_readonly_presentation(ui);
            })
            .drop_without_applying_deltas();
        }
        let first_rows = view.readonly_presentation.as_ref().unwrap().rows.as_ptr();
        let first_widths = view.readonly_presentation.as_ref().unwrap().content_widths;
        ctx.run_ui(egui::RawInput::default(), |ui| {
            view.ensure_readonly_presentation(ui);
        })
        .drop_without_applying_deltas();
        assert_eq!(
            view.readonly_presentation.as_ref().unwrap().rows.as_ptr(),
            first_rows,
            "unchanged frames must reuse the cached row list"
        );
        let longest_left = (0..12)
            .map(|index| format!("left\tline {index}").len() + 3)
            .max()
            .unwrap();
        let expected_left_width = super::file_style::TEXT_OFFSET
            + 28.0
            + longest_left as f32 * view.readonly_presentation.as_ref().unwrap().font.size;
        assert_eq!(first_widths[0], expected_left_width);
        let mut row_height = 0.0;
        ctx.run_ui(egui::RawInput::default(), |ui| {
            row_height = super::comparison_row_height(ui);
        })
        .drop_without_applying_deltas();
        // The old top row is source row 5. Filtering should retain the first
        // surviving source row at or after it (row 6), not reinterpret pixel
        // offset five as display row five.
        view.scroll = [5.0 * row_height + 0.5; 2];
        view.diff_options[0] = true;
        ctx.run_ui(egui::RawInput::default(), |ui| {
            view.ensure_readonly_presentation(ui);
        })
        .drop_without_applying_deltas();
        let cached = view.readonly_presentation.as_ref().unwrap();
        assert_eq!(cached.preferences, [true, false, false]);
        assert_eq!(cached.rows, vec![0, 3, 6, 9]);
        assert!((view.scroll[0] - (2.0 * row_height + 0.5)).abs() < 0.01);
    }

    #[test]
    fn readonly_refresh_remaps_the_source_line_anchor_and_merge_keeps_its_scroll() {
        let ctx = egui::Context::default();
        let mut view = view("left\n", "right\n");
        view.editing = false;
        view.diff = Some(readonly_comparison(12));
        view.diff_rows = vec![0, 3, 6, 9];
        let mut row_height = 0.0;
        ctx.run_ui(egui::RawInput::default(), |ui| {
            row_height = super::comparison_row_height(ui);
            view.ensure_readonly_presentation(ui);
        })
        .drop_without_applying_deltas();
        // Source line 10 is at aligned display index 9 in the old result.
        view.scroll = [9.0 * row_height + 0.5; 2];
        view.invalidate_diff();
        let mut refreshed = readonly_comparison(12);
        for inserted in 0..2 {
            refreshed.rows.insert(
                inserted,
                versus::FileComparisonRow {
                    left: None,
                    right: Some((inserted + 1, format!("inserted {inserted}"))),
                    state: DirectoryEntryState::RightOnly,
                    left_changed: Vec::new(),
                    right_changed: Vec::new(),
                    left_ending: None,
                    right_ending: Some(versus::DisplayLineEnding::Lf),
                },
            );
        }
        view.diff = Some(refreshed);
        view.diff_rows = vec![0, 1, 2, 5, 8, 11];
        ctx.run_ui(egui::RawInput::default(), |ui| {
            view.ensure_readonly_presentation(ui);
        })
        .drop_without_applying_deltas();
        let cache = view.readonly_presentation.as_ref().unwrap();
        let top = (view.scroll[0] / row_height).floor() as usize;
        let row = &view.diff.as_ref().unwrap().rows[cache.rows[top]];
        assert_eq!(row.left.as_ref().map(|line| line.0), Some(10));
        assert!((view.scroll[0] % row_height - 0.5).abs() < 0.01);

        view.editing = true;
        view.scroll = [37.0, 71.0];
        view.invalidate_diff();
        assert!(view.readonly_pending_anchor.is_none());
        assert_eq!(view.scroll, [37.0, 71.0]);
    }

    #[test]
    fn merge_editor_keeps_raw_tabs_syntax_and_a_clear_right_transfer_gutter() {
        let ctx = egui::Context::default();
        let mut view = view("\tfn source() {}\n", "target\n");
        let right_chunk = view.workspace.as_ref().unwrap().chunks(TwoWaySide::Right)[0].id;
        // Run Diff is intentionally off: the editable source view still keeps
        // its syntax formatting and byte-identical TextEdit buffer.
        let output = frame(&mut view, &ctx, vec![]);
        let source = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().contains("fn source") => Some(text),
                _ => None,
            })
            .expect("source line should be rendered");
        assert!(source.galley.text().contains('\t'));
        assert!(
            source
                .galley
                .job
                .sections
                .iter()
                .any(|section| section.format.color == Palette::for_context(&ctx).accent)
        );
        let right_arrow = ctx
            .read_response(egui::Id::new((
                "two-way-transfer",
                1usize,
                right_chunk,
                0usize,
            )))
            .unwrap()
            .rect;
        let right_editor = ctx.read_response(view.editor_id(1)).unwrap().rect;
        // The text anchor is 68 points after the shifted number/status gutter:
        // it clears the right-side 58 point transfer area and its 30 point arrow.
        assert!(right_editor.left() - right_arrow.right() >= 70.0);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn right_pane_wheel_updates_linked_readonly_scroll_in_the_same_frame() {
        let ctx = egui::Context::default();
        let mut view = view("left\n", "right\n");
        view.editing = false;
        view.diff = Some(readonly_comparison(120));
        view.diff_rows = (0..120).filter(|index| index % 3 == 0).collect();
        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        let right_pane = ctx
            .read_response(egui::Id::new(("interactive-readonly-pane", 0u64, 1usize)))
            .unwrap()
            .rect
            .center();
        frame(&mut view, &ctx, vec![egui::Event::PointerMoved(right_pane)])
            .drop_without_applying_deltas();
        frame(
            &mut view,
            &ctx,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, -8.0),
                phase: egui::TouchPhase::Move,
                modifiers: Default::default(),
            }],
        )
        .drop_without_applying_deltas();
        assert!(
            view.scroll[1] > 0.0,
            "right pane should consume its wheel event; pane={right_pane:?}, scroll={:?}",
            view.scroll
        );
        assert_eq!(view.scroll[0], view.scroll[1]);
    }
    #[test]
    fn merge_content_fills_each_pane_and_horizontal_bars_stay_at_the_bottom() {
        let wide_line = "long content ".repeat(100);
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            for zoom in [1.0, 2.0] {
                for text in [
                    String::new(),
                    wide_line.clone(),
                    format!("{wide_line}\n").repeat(100),
                ] {
                    let ctx = egui::Context::default();
                    set_theme(&ctx, theme);
                    ctx.set_zoom_factor(zoom);
                    let mut view = view(&text, &text);
                    for _ in 0..3 {
                        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                    }
                    for side in 0..2usize {
                        let track = ctx
                            .read_response(egui::Id::new(("merge-transfer-track", side)))
                            .unwrap()
                            .rect;
                        let editor = ctx
                            .read_response(egui::Id::new((
                                "two-way-edit",
                                view.side_generation[side],
                                side,
                            )))
                            .unwrap()
                            .rect;
                        assert!(
                            editor.height() >= track.height() - 12.0,
                            "content must fill even an empty pane: {theme:?}, {zoom}, {editor:?}, {track:?}"
                        );
                        if !text.is_empty() {
                            let point = egui::pos2(
                                if side == 0 {
                                    track.left() - 100.0
                                } else {
                                    track.right() + 100.0
                                },
                                track.bottom() - 8.0,
                            );
                            // Long files must keep their horizontal bar visible
                            // both at the start and after independent vertical scrolling.
                            for offset in [0.0, 120.0] {
                                if offset > 0.0 && text.lines().count() == 1 {
                                    continue;
                                }
                                view.scroll[side] = offset;
                                frame(&mut view, &ctx, vec![egui::Event::PointerMoved(point)])
                                    .drop_without_applying_deltas();
                                let output =
                                    frame(&mut view, &ctx, vec![egui::Event::PointerMoved(point)]);
                                assert_eq!(
                                    output.platform_output.cursor_icon,
                                    egui::CursorIcon::PointingHand,
                                    "horizontal bar should be at pane bottom: {theme:?}, {zoom}, side={side}, lines={}, offset={offset}, point={point:?}",
                                    text.lines().count()
                                );
                                output.drop_without_applying_deltas();
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn merge_editors_fill_the_panes_without_lower_management_controls() {
        let text: String = (0..200)
            .map(|line| format!("fn item_{line}() {{}}\n"))
            .collect();
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            for zoom in [1.0, 2.0] {
                let ctx = egui::Context::default();
                set_theme(&ctx, theme);
                ctx.set_zoom_factor(zoom);
                let mut view = view(&text, &text);
                for _ in 0..3 {
                    frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                }
                let output = frame(&mut view, &ctx, vec![]);
                for label in [
                    "Selected lines",
                    "Transfer →",
                    "← Transfer",
                    "Insert before line",
                    "Replace selected target",
                    "Create chunk",
                    "Ignore range",
                    "Manage ignored…",
                ] {
                    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
                        egui::Shape::Text(text) if text.galley.text() == label)));
                }
                let tracks: [Rect; 2] = std::array::from_fn(|side| {
                    ctx.read_response(egui::Id::new(("merge-transfer-track", side)))
                        .unwrap()
                        .rect
                });
                let editors: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text().contains("fn item_0()") => {
                            Some(shape.clip_rect)
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(editors.len(), 2);
                for rect in editors {
                    let side = usize::from(rect.center().x > ctx.viewport_rect().center().x);
                    assert!(
                        rect.height() >= tracks[side].height() - 12.0,
                        "editor should reclaim the lower controls area: editor={rect:?}, track={:?}",
                        tracks[side]
                    );
                }
                output.drop_without_applying_deltas();
            }
        }
    }

    #[test]
    fn merge_without_diff_keeps_preferences_and_history_across_modes() {
        let ctx = egui::Context::default();
        let mut view = view("left\n", "unrelated\n");
        view.merge_options = [false, true, false];
        view.caret[1] = 1;
        view.transfer(TwoWaySide::Left, 0..1);
        view.invalidate_diff();
        view.poll(&ctx);
        assert!(view.diff_job.is_none() && view.diff.is_none());
        let result = view.working_text(1).unwrap().to_owned();
        view.set_editing(false, [true, true, true]);
        std::thread::sleep(Duration::from_millis(155));
        view.poll(&ctx);
        assert!(view.diff_job.is_some() || view.diff.is_some());
        view.set_editing(true, [true, true, true]);
        view.poll(&ctx);
        assert_eq!(view.merge_options, [false, true, false]);
        assert_eq!(view.diff_options, [true, true, true]);
        assert!(view.diff_job.is_none() && view.diff.is_none());
        assert_eq!(view.working_text(1), Some(result.as_str()));
        assert!(view.workspace.as_mut().unwrap().undo());
        assert_eq!(view.working_text(1), Some("unrelated\n"));
        assert!(view.workspace.as_mut().unwrap().redo());
        assert_eq!(view.working_text(1), Some(result.as_str()));
    }

    #[test]
    fn transfer_tracks_are_outside_content_and_follow_independent_source_scrolls() {
        let left: String = (0..35)
            .map(|i| format!("fn block_{i}() {{\n    left_{i}();\n}}\n"))
            .collect();
        let right: String = (0..35)
            .map(|i| format!("fn block_{i}() {{\n    right_{i}();\n}}\n"))
            .collect();
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            for zoom in [1.0, 2.0] {
                for run_diff in [false, true] {
                    let ctx = egui::Context::default();
                    set_theme(&ctx, theme);
                    ctx.set_zoom_factor(zoom);
                    let mut view = view(&left, &right);
                    if run_diff {
                        view.set_run_diff(true);
                        view.diff = compare_loaded_text(
                            [&left, &right],
                            &AtomicBool::new(false),
                            &FileViewOptions::default(),
                            [&[], &[]],
                        )
                        .unwrap();
                    }
                    let chunks = [
                        view.workspace.as_ref().unwrap().chunks(TwoWaySide::Left),
                        view.workspace.as_ref().unwrap().chunks(TwoWaySide::Right),
                    ];
                    for _ in 0..3 {
                        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                    }
                    let output = frame(&mut view, &ctx, vec![]);
                    let tracks: [Rect; 2] = std::array::from_fn(|side| {
                        ctx.read_response(egui::Id::new(("merge-transfer-track", side)))
                            .unwrap()
                            .rect
                    });
                    assert!(tracks[0].right() <= tracks[1].left());
                    for side in 0..2 {
                        assert_eq!(tracks[side].width(), 38.0);
                        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.rect == tracks[side] && rect.fill == Palette::for_context(&ctx).background)));
                        let chunk = &chunks[side][0];
                        let line = chunk.range.start + usize::from(run_diff);
                        let arrow = ctx
                            .read_response(egui::Id::new((
                                "two-way-transfer",
                                side,
                                chunk.id,
                                line,
                            )))
                            .unwrap()
                            .rect;
                        let toggle = ctx
                            .read_response(egui::Id::new(("two-way-granularity", side, chunk.id)))
                            .unwrap()
                            .rect;
                        assert!(tracks[side].contains_rect(arrow));
                        if side == 0 {
                            assert!(toggle.right() + 5.0 <= tracks[side].left());
                        } else {
                            assert!(toggle.left() >= tracks[side].right() + 5.0);
                        }
                        let color = if side == 0 {
                            Palette::for_context(&ctx).left_only
                        } else {
                            Palette::for_context(&ctx).right_only
                        };
                        let head = output
                            .shapes
                            .iter()
                            .find_map(|shape| match &shape.shape {
                                egui::Shape::Path(path)
                                    if path.stroke.color
                                        == egui::epaint::ColorMode::Solid(color)
                                        && path
                                            .points
                                            .iter()
                                            .all(|point| arrow.contains(*point)) =>
                                {
                                    Some((path, shape.clip_rect))
                                }
                                _ => None,
                            })
                            .expect("each track has a vector arrow");
                        assert!(
                            head.1.left() >= tracks[side].left()
                                && head.1.right() <= tracks[side].right()
                        );
                        assert_eq!(head.0.points[1].x > head.0.points[0].x, side == 0);
                    }
                    output.drop_without_applying_deltas();
                    let first_left = chunks[0][0].range.start + usize::from(run_diff);
                    let first_right = chunks[1][0].range.start + usize::from(run_diff);
                    let left_id =
                        egui::Id::new(("two-way-transfer", 0usize, chunks[0][0].id, first_left));
                    let right_id =
                        egui::Id::new(("two-way-transfer", 1usize, chunks[1][0].id, first_right));
                    let left_y = ctx.read_response(left_id).unwrap().rect.top();
                    let right_y = ctx.read_response(right_id).unwrap().rect.top();
                    view.scroll[0] = 50.0;
                    for _ in 0..3 {
                        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                    }
                    assert!(ctx.read_response(left_id).unwrap().rect.top() < left_y - 20.0);
                    assert!(
                        (ctx.read_response(right_id).unwrap().rect.top() - right_y).abs() < 0.1
                    );
                    view.caret[0] = 0;
                    click(&mut view, &ctx, right_id);
                    let copied = if run_diff {
                        "    right_0();\n"
                    } else {
                        "fn block_0() {\n    right_0();\n}\n"
                    };
                    assert_eq!(
                        view.working_text(0),
                        Some(format!("{copied}{left}").as_str())
                    );
                    assert_eq!(view.working_text(1), Some(right.as_str()));
                }
            }
        }
    }

    #[test]
    fn actual_gutter_arrows_copy_chunks_and_lines_without_diff() {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let ctx = egui::Context::default();
            set_theme(&ctx, theme);
            let mut view = view("fn source() {\n    one();\n    two();\n}\n", "target\n");
            view.caret[1] = 1;
            let output = frame(&mut view, &ctx, vec![]);
            let chunk = view.workspace.as_ref().unwrap().chunks(TwoWaySide::Left)[0].clone();
            let arrow = ctx
                .read_response(egui::Id::new((
                    "two-way-transfer",
                    0usize,
                    chunk.id,
                    chunk.range.start,
                )))
                .unwrap()
                .rect;
            let granularity = ctx
                .read_response(egui::Id::new(("two-way-granularity", 0usize, chunk.id)))
                .unwrap()
                .rect;
            assert!(arrow.width() >= 30.0);
            assert!(arrow.width() > granularity.width());
            let arrow_head = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Path(path)
                        if path.stroke.color
                            == egui::epaint::ColorMode::Solid(
                                Palette::for_context(&ctx).left_only,
                            )
                            && path.points.iter().all(|point| arrow.contains(*point)) =>
                    {
                        Some(path)
                    }
                    _ => None,
                })
                .expect("copy control should draw a vector arrow");
            assert_eq!(arrow_head.stroke.width, 2.0);
            assert!(arrow_head.points[1].x > arrow_head.points[0].x);
            output.drop_without_applying_deltas();
            for id in [
                egui::Id::new(("two-way-transfer", 0usize, chunk.id, chunk.range.start)),
                egui::Id::new(("two-way-granularity", 0usize, chunk.id)),
            ] {
                let position = ctx.read_response(id).unwrap().rect.center();
                let output = frame(&mut view, &ctx, vec![egui::Event::PointerMoved(position)]);
                assert_eq!(
                    output.platform_output.cursor_icon,
                    egui::CursorIcon::PointingHand
                );
                output.drop_without_applying_deltas();
            }
            click(
                &mut view,
                &ctx,
                egui::Id::new(("two-way-transfer", 0usize, chunk.id, chunk.range.start)),
            );
            assert_eq!(
                view.working_text(1),
                Some("target\nfn source() {\n    one();\n    two();\n}\n")
            );
            assert!(view.diff_job.is_none());
            assert_eq!(view.caret[1], 5);
            click(
                &mut view,
                &ctx,
                egui::Id::new(("two-way-granularity", 0usize, chunk.id)),
            );
            frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
            let first = ctx
                .read_response(egui::Id::new((
                    "two-way-transfer",
                    0usize,
                    chunk.id,
                    1usize,
                )))
                .unwrap()
                .rect;
            let second = ctx
                .read_response(egui::Id::new((
                    "two-way-transfer",
                    0usize,
                    chunk.id,
                    2usize,
                )))
                .unwrap()
                .rect;
            assert!(first.bottom() <= second.top() + 0.1, "line arrows overlap");
            click(
                &mut view,
                &ctx,
                egui::Id::new(("two-way-transfer", 0usize, chunk.id, 1usize)),
            );
            assert!(view.working_text(1).unwrap().ends_with("    one();\n"));
        }
    }
    #[test]
    fn toolbar_actions_stay_visible_and_ordered_at_normal_and_high_zoom() {
        for zoom in [1.0, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_zoom_factor(zoom);
            let mut view = view("left\n", "right\n");
            view.set_run_diff(true);
            frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
            let left = ctx
                .read_response(egui::Id::new("Accept All Left — replace Right with Left"))
                .unwrap()
                .rect;
            let undo = ctx.read_response(egui::Id::new("Undo")).unwrap().rect;
            let previous = ctx
                .read_response(egui::Id::new("Previous difference"))
                .unwrap()
                .rect;
            let next = ctx
                .read_response(egui::Id::new("Next difference"))
                .unwrap()
                .rect;
            assert!(left.right() < undo.left());
            let options = ctx
                .read_response(egui::Id::new("Ignore line endings"))
                .unwrap()
                .rect;
            assert!(options.right() < previous.left());
            assert!((options.center().y - previous.center().y).abs() < 0.5);
            assert!(previous.right() < next.left());
            assert!(
                next.right() <= ctx.viewport_rect().right() + 1.0,
                "next={next:?}, viewport={:?}",
                ctx.viewport_rect()
            );
            assert!(left.top() >= 0.0);
        }
    }
    #[test]
    fn unsaved_indicator_sits_between_activity_and_save_with_guidance_on_left() {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            for zoom in [1.0, 2.0] {
                let ctx = egui::Context::default();
                set_theme(&ctx, theme);
                ctx.set_zoom_factor(zoom);
                let mut view = view("left\n", "right\n");
                view.workspace
                    .as_mut()
                    .unwrap()
                    .set_text(TwoWaySide::Left, "changed\n");
                for _ in 0..3 {
                    frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                }
                let dirty = ctx
                    .read_response(egui::Id::new("workspace-unsaved"))
                    .unwrap()
                    .rect;
                let save = ctx.read_response(egui::Id::new("Save Both")).unwrap().rect;
                let guidance = ctx
                    .read_response(egui::Id::new("workspace-guidance"))
                    .unwrap()
                    .rect;
                let legend = ctx
                    .read_response(egui::Id::new("workspace-legend"))
                    .unwrap()
                    .rect;
                assert!(guidance.right() <= legend.left());
                let status = ctx
                    .read_response(egui::Id::new("workspace-status"))
                    .unwrap()
                    .rect;
                assert!(legend.right() <= status.left());
                assert!(status.right() <= dirty.left());
                assert!(dirty.right() < save.left());
                assert!((dirty.center().y - save.center().y).abs() < 1.0);
                let output = frame(&mut view, &ctx, vec![]);
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == "Unsaved Changes"
                        && text.galley.job.sections.iter().all(|section| section.format.color == Palette::for_context(&ctx).changed)
                        && shape.clip_rect.contains_rect(text.galley.rect.translate(text.pos.to_vec2())))));
                output.drop_without_applying_deltas();
                view.move_history(false);
                for _ in 0..3 {
                    frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                }
                assert!(
                    ctx.read_response(egui::Id::new("workspace-unsaved"))
                        .is_none()
                );
            }
        }
    }

    #[test]
    fn save_both_tracks_per_side_save_availability_and_only_previews_changed_files() {
        let ctx = egui::Context::default();
        set_theme(&ctx, egui::Theme::Dark);
        let mut view = view("left\n", "right\n");
        for side in [0usize, 1] {
            for _ in 0..2 {
                frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
            }
            assert!(!view.can_save_side(0) && !view.can_save_side(1));
            assert!(
                !ctx.read_response(egui::Id::new("Save Both"))
                    .unwrap()
                    .enabled()
            );
            view.workspace
                .as_mut()
                .unwrap()
                .set_text(side_of(side), "changed\n");
            for _ in 0..2 {
                frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
            }
            assert!(view.can_save_side(side) && !view.can_save_side(1 - side));
            assert!(
                ctx.read_response(egui::Id::new("Save Both"))
                    .unwrap()
                    .enabled()
            );
            click(&mut view, &ctx, egui::Id::new("Save Both"));
            assert_eq!(view.pending_save, Some(vec![side]));
            view.pending_save = None;
            view.move_history(false);
        }
        // A single loaded editable file still has an active Save Both action.
        let file = view.files.as_ref().unwrap()[0].clone();
        view.files = None;
        view.paths = None;
        view.side_sources = [Some("left.rs".into()), None];
        view.side_files = [Some(file), None];
        view.workspace
            .as_mut()
            .unwrap()
            .set_text(TwoWaySide::Left, "single changed\n");
        for _ in 0..2 {
            frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        }
        assert!(view.can_save_side(0) && !view.can_save_side(1));
        assert!(
            ctx.read_response(egui::Id::new("Save Both"))
                .unwrap()
                .enabled()
        );
        click(&mut view, &ctx, egui::Id::new("Save Both"));
        assert_eq!(view.pending_save, Some(vec![0]));
    }

    #[test]
    fn save_both_matches_toolbar_feedback_and_still_opens_a_preview() {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let ctx = egui::Context::default();
            set_theme(&ctx, theme);
            let mut view = view("left\n", "right\n");
            view.workspace
                .as_mut()
                .unwrap()
                .set_text(TwoWaySide::Left, "edited left\n");
            view.workspace
                .as_mut()
                .unwrap()
                .set_text(TwoWaySide::Right, "edited right\n");
            let output = frame(&mut view, &ctx, vec![]);
            let button = ctx.read_response(egui::Id::new("Save Both")).unwrap().rect;
            let caption = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Save Both" => Some(text),
                    _ => None,
                })
                .unwrap();
            assert_eq!(caption.galley.job.sections[0].format.font_id.size, 11.0);
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(rect) if button.contains_rect(rect.rect)
                    && rect.rect.width() == 14.0 && rect.rect.right() < caption.pos.x)));
            assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(rect) if rect.rect == button && rect.fill != Color32::TRANSPARENT)));
            output.drop_without_applying_deltas();
            let pos = button.center();
            let output = frame(
                &mut view,
                &ctx,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ],
            );
            assert!(
                view.pending_save.is_none(),
                "press feedback must precede activation"
            );
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(rect) if rect.rect == button
                    && rect.fill == Palette::for_context(&ctx).accent.gamma_multiply(0.35))));
            output.drop_without_applying_deltas();
            frame(
                &mut view,
                &ctx,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
            )
            .drop_without_applying_deltas();
            assert_eq!(view.pending_save, Some(vec![0, 1]));
            assert!(
                view.save_job.is_none(),
                "Save Both must preview before writing"
            );
            assert_eq!(view.working_text(0), Some("edited left\n"));
            assert_eq!(view.working_text(1), Some("edited right\n"));
            view.pending_save = None;
            view.set_external_save_busy(true);
            for _ in 0..2 {
                frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
            }
            assert!(
                !ctx.read_response(egui::Id::new("Save Both"))
                    .unwrap()
                    .enabled()
            );
            click(&mut view, &ctx, egui::Id::new("Save Both"));
            assert!(view.pending_save.is_none());
        }
    }

    #[test]
    fn merge_toolbar_uses_search_after_new_without_a_global_back_button() {
        let ctx = egui::Context::default();
        let mut view = view("left\n", "right\n");
        view.from_folders = false;
        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        let new = ctx
            .read_response(egui::Id::new("New comparison"))
            .unwrap()
            .rect;
        let search = ctx
            .read_response(egui::Id::new("Find and replace"))
            .unwrap()
            .rect;
        assert!(new.right() <= search.left());
        assert!(
            ctx.read_response(egui::Id::new("Back to folders"))
                .is_none()
        );
    }
    #[test]
    fn merge_toolbar_toggles_diff_controls_and_legends_without_resetting_edits() {
        let diff_buttons = [
            "Show only differences",
            "Ignore whitespace",
            "Ignore line endings",
            "Previous difference",
            "Next difference",
        ];
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            for zoom in [1.0, 2.0] {
                let ctx = egui::Context::default();
                set_theme(&ctx, theme);
                ctx.set_zoom_factor(zoom);
                ctx.run_ui(egui::RawInput::default(), |_| {})
                    .drop_without_applying_deltas();
                let mut view = view("left\n", "right\n");
                view.merge_options = [false, true, true];
                view.merge_options_initialized = true;
                view.diff_options = [true, false, true];
                view.set_run_diff(true);
                view.workspace
                    .as_mut()
                    .unwrap()
                    .edit_bytes(TwoWaySide::Left, 0..0, "edited\n".into())
                    .unwrap();
                view.caret = [1, 0];
                view.invalidate_diff();
                let undo = view.workspace.as_ref().unwrap().next_undo_tag();
                let mut output = frame(&mut view, &ctx, vec![]);
                output.textures_delta.clear();
                let legend_labels: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) => Some(text.galley.text()),
                        _ => None,
                    })
                    .collect();
                assert!(
                    legend_labels
                        .iter()
                        .any(|label| label.starts_with("Conflict"))
                );
                for icon in DIFF_LEGEND_ICONS {
                    assert!(
                        legend_labels
                            .iter()
                            .any(|label| label.starts_with(icon.label()))
                    );
                }
                let toolbar_undo = ctx.read_response(egui::Id::new("Undo")).unwrap().rect;
                assert!(toolbar_undo.bottom() < ctx.viewport_rect().bottom());
                for label in diff_buttons {
                    assert!(
                        ctx.read_response(egui::Id::new(label)).is_some(),
                        "{label} missing"
                    );
                }
                assert_eq!(view.working_text(0), Some("edited\nleft\n"));
                assert_eq!(view.workspace.as_ref().unwrap().next_undo_tag(), undo);
                assert_eq!(view.caret, [1, 0]);
                assert_eq!(view.merge_options, [false, true, true]);
                assert_eq!(view.diff_options, [true, false, true]);
                assert!(view.diff_job.is_none());
            }
        }
    }
    #[test]
    fn accept_all_replaces_destination_as_one_undoable_operation() {
        let mut view = view("whole\nleft\n", "right\n");
        view.copy_all(TwoWaySide::Left);
        assert_eq!(view.working_text(1), view.working_text(0));
        assert!(view.workspace.as_mut().unwrap().undo());
        assert_eq!(view.working_text(1), Some("right\n"));
        assert!(!view.workspace.as_ref().unwrap().can_undo());
        assert!(view.workspace.as_mut().unwrap().redo());
        assert_eq!(view.working_text(1), Some("whole\nleft\n"));
    }

    #[test]
    fn repeated_insertions_follow_the_original_target_in_source_order() {
        let mut view = view("first\nsecond\n", "keep\n");
        view.caret[1] = 0;
        view.transfer(TwoWaySide::Left, 0..1);
        view.transfer(TwoWaySide::Left, 1..2);
        assert_eq!(view.working_text(1), Some("first\nsecond\nkeep\n"));
        assert!(view.workspace.as_mut().unwrap().undo());
        assert_eq!(view.working_text(1), Some("first\nkeep\n"));
    }

    #[test]
    fn editing_preserves_unaffected_mixed_endings() {
        assert_eq!(
            restore_line_endings("one\r\ntwo\nthree\r", "one\nchanged\nthree\n"),
            "one\r\nchanged\nthree\r"
        );
        assert_eq!(restore_line_endings("a\r\nb", "a\nb!"), "a\r\nb!");
    }
    #[test]
    fn save_both_writes_both_snapshots_and_retains_newer_edits() {
        let root = std::env::temp_dir().join(format!("versus-ui-save-both-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let paths = [root.join("left"), root.join("right")];
        std::fs::write(&paths[0], "old-left\n").unwrap();
        std::fs::write(&paths[1], "old-right\n").unwrap();
        let mut view = view("old-left\n", "old-right\n");
        view.files = Some(std::array::from_fn(|side| {
            load_working_file(Some(&paths[side])).unwrap()
        }));
        view.paths = Some(paths.clone());
        view.workspace
            .as_mut()
            .unwrap()
            .set_text(TwoWaySide::Left, "saved-left\n");
        view.workspace
            .as_mut()
            .unwrap()
            .set_text(TwoWaySide::Right, "saved-right\n");
        view.save_sides(vec![0, 1]);
        view.workspace
            .as_mut()
            .unwrap()
            .set_text(TwoWaySide::Left, "newer\n");
        let ctx = egui::Context::default();
        for _ in 0..100 {
            view.poll_save(&ctx);
            if !view.is_saving() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!view.is_saving());
        assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "saved-left\n");
        assert_eq!(std::fs::read_to_string(&paths[1]).unwrap(), "saved-right\n");
        assert_eq!(view.working_text(0), Some("newer\n"));
        assert!(view.workspace.as_ref().unwrap().is_dirty(TwoWaySide::Left));
        assert!(!view.workspace.as_ref().unwrap().is_dirty(TwoWaySide::Right));
        view.workspace.as_mut().unwrap().undo();
        assert!(!view.workspace.as_ref().unwrap().is_dirty(TwoWaySide::Left));
        let _ = std::fs::remove_dir_all(root);
    }
    fn preview_for(
        view: &InteractiveView,
        replacements: &[(&str, std::ops::Range<usize>, &str)],
    ) -> versus::SearchPreview {
        let bases = view.export_working_files();
        let mut changes = Vec::new();
        for (path, range, replacement) in replacements {
            let file = &bases[&PathBuf::from(path)];
            let mut after = file.text.clone();
            after.replace_range(range.clone(), replacement);
            changes.push(SearchChange {
                path: PathBuf::from(path),
                before: file.text.clone(),
                after,
                hit_ids: vec![changes.len() as u64 + 1],
                ranges: vec![range.clone()],
                edits: vec![(range.clone(), (*replacement).into())],
            });
        }
        versus::SearchPreview {
            changes,
            bases,
            skipped: Vec::new(),
        }
    }
    fn refreshed_preview(
        view: &InteractiveView,
        updates: &[(&str, &str)],
    ) -> versus::SearchPreview {
        let mut bases = view.export_working_files();
        let mut changes = Vec::new();
        for (path, text) in updates {
            let path = PathBuf::from(path);
            let base = bases.get_mut(&path).unwrap();
            base.text = (*text).into();
            base.fingerprint = versus::FileFingerprint::Bytes(text.as_bytes().to_vec());
            let start = text.find("match").unwrap();
            let range = start..start + 5;
            let mut after = (*text).to_owned();
            after.replace_range(range.clone(), "found");
            changes.push(SearchChange {
                path,
                before: (*text).into(),
                after,
                hit_ids: vec![changes.len() as u64 + 1],
                ranges: vec![range.clone()],
                edits: vec![(range, "found".into())],
            });
        }
        versus::SearchPreview {
            changes,
            bases,
            skipped: Vec::new(),
        }
    }

    #[test]
    fn refreshed_replacement_preserves_earlier_edit_history() {
        let mut view = view("before match\nkept\n", "right\n");
        view.workspace
            .as_mut()
            .unwrap()
            .set_text(TwoWaySide::Left, "old match\nkept\n");
        view.invalidate_diff();
        let mut saved = view.export_working_files();
        saved
            .get_mut(&PathBuf::from("left.rs"))
            .unwrap()
            .fingerprint = versus::FileFingerprint::Bytes(b"old match\nkept\n".to_vec());
        view.update_saved_files(&saved);
        let preview = refreshed_preview(&view, &[("left.rs", "new\nold match\nkept\n")]);
        view.apply_search_preview(&preview).unwrap();
        assert_eq!(view.working_text(0), Some("new\nold found\nkept\n"));
        view.undo_batch(false).unwrap();
        assert_eq!(view.working_text(0), Some("new\nold match\nkept\n"));
        view.selected_side = 0;
        view.move_history(false);
        assert_eq!(view.working_text(0), Some("old match\nkept\n"));
        for _ in 0..3 {
            view.move_history(false);
        }
        assert_eq!(view.working_text(0), Some("before match\nkept\n"));
        assert!(!view.run_diff_enabled());
    }

    #[test]
    fn refreshed_replacement_rejects_dirty_documents_atomically() {
        let mut view = view("left match\n", "right match\n");
        let preview = refreshed_preview(
            &view,
            &[
                ("left.rs", "new left match\n"),
                ("right.rs", "new right match\n"),
            ],
        );
        view.workspace
            .as_mut()
            .unwrap()
            .set_text(TwoWaySide::Right, "unsaved match\n");
        view.invalidate_diff();
        let before = view.export_working_files();
        let revision = view.folder_revision_signature();
        assert!(view.apply_search_preview(&preview).is_err());
        assert_eq!(view.export_working_files(), before);
        assert_eq!(view.folder_revision_signature(), revision);
        assert!(!view.can_undo_batch());
    }

    #[test]
    fn external_refresh_splice_is_utf8_safe_and_preserves_exact_line_endings() {
        for (before, after) in [
            ("é match\r\n", "ê match\r\n"),
            ("a\r\n", "a\n"),
            ("", "文"),
            ("文", ""),
        ] {
            let (range, replacement) = external_refresh_edit(before, after).unwrap();
            let mut text = before.to_owned();
            text.replace_range(range, &replacement);
            assert_eq!(text, after);
        }
        assert_eq!(external_refresh_edit("same", "same"), None);
    }
    #[test]
    fn folder_batch_and_file_history_undo_and_redo_in_transaction_order() {
        let mut view = view("old-left\n", "old-right\n");
        view.workspace
            .as_mut()
            .unwrap()
            .set_text(TwoWaySide::Left, "manual-left\n");
        view.invalidate_diff();
        let preview = preview_for(
            &view,
            &[("left.rs", 0..6, "batch"), ("right.rs", 0..3, "batch")],
        );
        view.apply_search_preview(&preview).unwrap();
        assert_eq!(view.working_text(0), Some("batch-left\n"));
        assert_eq!(view.working_text(1), Some("batch-right\n"));
        view.selected_side = 0;
        view.move_history(false);
        assert_eq!(view.working_text(0), Some("manual-left\n"));
        assert_eq!(view.working_text(1), Some("old-right\n"));
        view.move_history(false);
        assert_eq!(view.working_text(0), Some("old-left\n"));
        assert!(!view.can_redo_batch());
        view.move_history(true);
        assert_eq!(view.working_text(0), Some("manual-left\n"));
        assert!(view.can_redo_batch());
        view.move_history(true);
        assert_eq!(view.working_text(0), Some("batch-left\n"));
        assert_eq!(view.working_text(1), Some("batch-right\n"));
    }
    #[test]
    fn later_manual_edits_block_atomic_batch_undo_until_their_undo() {
        let mut view = view("left\n", "right\n");
        let preview = preview_for(
            &view,
            &[("left.rs", 0..4, "batch"), ("right.rs", 0..5, "batch")],
        );
        view.apply_search_preview(&preview).unwrap();
        view.workspace
            .as_mut()
            .unwrap()
            .set_text(TwoWaySide::Right, "newer\n");
        view.invalidate_diff();
        view.selected_side = 0;
        view.move_history(false);
        assert_eq!(view.working_text(0), Some("batch\n"));
        assert_eq!(view.working_text(1), Some("newer\n"));
        assert!(view.error.is_some());
        view.selected_side = 1;
        view.move_history(false);
        assert_eq!(view.working_text(1), Some("batch\n"));
        view.move_history(false);
        assert_eq!(view.working_text(0), Some("left\n"));
        assert_eq!(view.working_text(1), Some("right\n"));
    }

    #[test]
    fn alternate_pairing_retains_canonical_file_text_and_undo_without_changing_partner() {
        let mut view = view("left\n", "right\n");
        view.workspace
            .as_mut()
            .unwrap()
            .set_text(TwoWaySide::Left, "first\n");
        view.invalidate_diff();
        let mut extra = BTreeMap::new();
        extra.insert(
            PathBuf::from("third.rs"),
            WorkingFile {
                identity: None,
                path: Some(PathBuf::from("third.rs")),
                text: "third\n".into(),
                encoding: versus::TextEncoding::Utf8 { bom: false },
                fingerprint: versus::FileFingerprint::Bytes(b"third\n".to_vec()),
            },
        );
        view.ingest_working_files(&extra);
        view.open_pair([PathBuf::from("left.rs"), PathBuf::from("third.rs")]);
        view.workspace
            .as_mut()
            .unwrap()
            .set_text(TwoWaySide::Left, "second\n");
        view.invalidate_diff();
        view.open_pair([PathBuf::from("left.rs"), PathBuf::from("right.rs")]);
        assert_eq!(view.working_text(0), Some("second\n"));
        assert_eq!(view.working_text(1), Some("right\n"));
        view.selected_side = 0;
        view.move_history(false);
        assert_eq!(view.working_text(0), Some("first\n"));
        assert_eq!(view.working_text(1), Some("right\n"));
        assert_eq!(
            view.export_working_files()[&PathBuf::from("left.rs")].text,
            "first\n"
        );
    }
    #[test]
    fn failed_multifile_batch_preflight_changes_no_text_or_history() {
        let mut view = view("left\n", "right\n");
        let mut preview = preview_for(
            &view,
            &[("left.rs", 0..4, "batch"), ("right.rs", 0..5, "batch")],
        );
        preview
            .changes
            .iter_mut()
            .find(|change| change.path == PathBuf::from("right.rs"))
            .unwrap()
            .edits = vec![(0..999, "invalid".into())];
        assert!(view.apply_search_preview(&preview).is_err());
        assert_eq!(view.working_text(0), Some("left\n"));
        assert_eq!(view.working_text(1), Some("right\n"));
        assert!(!view.can_undo_batch());
        assert!(
            view.workspace
                .as_ref()
                .unwrap()
                .document(TwoWaySide::Left)
                .next_undo_tag()
                .is_none()
        );
    }
    #[test]
    fn save_after_batch_then_undo_uses_the_saved_baseline_and_retains_batch_redo() {
        let mut view = view("left\n", "right\n");
        let preview = preview_for(
            &view,
            &[("left.rs", 0..4, "batch"), ("right.rs", 0..5, "batch")],
        );
        view.apply_search_preview(&preview).unwrap();
        let mut saved = view.export_working_files();
        for file in saved.values_mut() {
            file.fingerprint = versus::FileFingerprint::Bytes(file.text.as_bytes().to_vec());
        }
        view.update_saved_files(&saved);
        assert!(!view.any_dirty());
        view.move_history(false);
        assert!(view.any_dirty());
        assert_eq!(view.working_text(0), Some("left\n"));
        view.move_history(true);
        assert!(!view.any_dirty());
    }
}
