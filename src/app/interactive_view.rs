//! Presentation for a retained, two-way interactive merge workspace.
use super::*;
mod projection;
#[cfg(test)]
use projection::remap_projection_cursor;
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
        self.presentation.clear();
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
        self.presentation
            .decoration(side, self.diff.as_ref(), self.merge_options[2])
    }
    fn capture_merge_presentation(&self) -> Option<PendingMergePresentation> {
        (self.editing && self.run_diff && (self.diff.is_some() || self.presentation.has_pending()))
            .then(|| {
                self.presentation.capture(
                    self.diff.as_ref(),
                    self.merge_options[2],
                    self.footer_legend_counts(),
                    self.diff_rows.len(),
                )
            })
    }
    fn history_diff_snapshot(&self) -> Option<[String; 2]> {
        (self.editing && self.run_diff && (self.diff.is_some() || self.presentation.has_pending()))
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
                    presentation.rebase(side, &before[side], range, after);
                }
            }
        }
        self.presentation.replace_pending(presentation);
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
                    presentation.rebase(target, before, range.clone(), after);
                } else if shared_file {
                    if let Some(peer_before) = &peer_before {
                        if peer_before == before && after != peer_before {
                            presentation.rebase(target, before, range.clone(), after);
                        } else if let Some((peer_range, _)) =
                            external_refresh_edit(peer_before, after)
                        {
                            presentation.rebase(target, peer_before, peer_range, after);
                        }
                    }
                }
            }
        }
        self.presentation.replace_pending(presentation);
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
            self.presentation.clear();
            self.readonly_presentation = None;
            return;
        }
        if self.files.is_none() && !self.side_files.iter().all(Option::is_some) {
            self.diff_job = None;
            self.diff = None;
            self.presentation.clear();
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
                    self.presentation.clear();
                    self.readonly_presentation = None;
                }
                Ok(None) => self.presentation.clear(),
                Err(error) => {
                    self.presentation.clear();
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
        if let Some(changes) = self.presentation.pending_changes() {
            return format!("{changes} changes");
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
        let (changed, ending_differences) = self.merge_decoration(side).into_parts();
        let gutter_extra = super::file_style::gutter_extra(max_line_number);
        let decorated = self.diff.is_some() || self.presentation.has_pending();
        let filtering_same = self.run_diff && self.merge_options[0] && decorated;
        let projection = filtering_same.then(|| {
            merge_projection(
                &displayed,
                &MergeDecoration::from_parts(changed.clone(), ending_differences.clone()),
            )
        });
        if projection.as_ref().is_some_and(MergeProjection::is_empty) {
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
            .map(|projection| projection.source_lines().collect())
            .unwrap_or_else(|| (0..count.max(1)).collect());
        let mut next = projection
            .as_ref()
            .map(|projection| projection.text().to_owned())
            .unwrap_or_else(|| displayed.clone());
        let editor_id = egui::Id::new(("two-way-edit", self.side_generation[side], side));
        self.presentation.update_editor_cursor(
            side,
            ui.ctx(),
            editor_id,
            EditorProjection::new(next.clone(), visible_lines.clone()),
        );
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
            .map(MergeProjection::text)
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
            return self.presentation.pending_counts();
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
        if self.diff.is_none() && !self.presentation.has_pending() {
            return false;
        }
        let decoration = self.merge_decoration(side_index(side));
        range.clone().any(|line| !decoration.is_changed(line))
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
        self.presentation.set_full_editor(side, displayed);
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
        self.presentation.set_full_editor(side, text);
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
mod tests;
