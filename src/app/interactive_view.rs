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
    selected: [Range<usize>; 2],
    destination: [usize; 2],
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
    save_job: Option<SaveJob>,
    external_save_busy: bool,
    editing: bool,
    diff_options: [bool; 3],
    merge_options: [bool; 3],
    run_diff: bool,
    diff_dirty: bool,
    diff_rows: Vec<usize>,
    folder_overview: bool,
    folder_differences: Vec<PathBuf>,
    folder_difference_index: Option<usize>,
    folder_jump: Option<PathBuf>,
    expanded: [Vec<u64>; 2],
    selected: [Range<usize>; 2],
    destination: [usize; 2],
    replace: bool,
    scroll: [f32; 2],
    generation: u64,
    side_generation: [u64; 2],
    selected_side: usize,
    back: bool,
    new_requested: bool,
    search_requested: bool,
    from_folders: bool,
    editor_context: Option<egui::Context>,
    pending_auto: Option<Vec<(Range<usize>, Range<usize>)>>,
    pending_save: Option<Vec<usize>>,
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
        self.selected[side] = 0..0;
        self.destination[side] = 0;
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
        self.diff_options = diff_preferences;
        if changed {
            self.invalidate_diff();
        }
    }
    pub(super) fn set_run_diff(&mut self, enabled: bool) {
        if self.run_diff != enabled {
            self.run_diff = enabled;
            self.invalidate_diff();
        }
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
    pub(super) fn folder_ignored_ranges(&self) -> BTreeMap<PathBuf, Vec<Range<usize>>> {
        let mut ranges: BTreeMap<_, _> = self
            .documents
            .iter()
            .map(|(identity, retained)| {
                (
                    identity.clone(),
                    retained.document.ignored_ranges().to_vec(),
                )
            })
            .collect();
        if let (Some(files), Some(workspace)) = (&self.files, &self.workspace) {
            for side in 0..2 {
                ranges.insert(
                    files[side].identity_key(),
                    workspace.ignored_ranges(side_of(side)).to_vec(),
                );
            }
        }
        ranges
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
            self.destination[side] = self.destination[side].min(count);
            self.selected[side] =
                self.selected[side].start.min(count)..self.selected[side].end.min(count);
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
            self.destination[side] = self.destination[side].min(count);
            self.selected[side] =
                self.selected[side].start.min(count)..self.selected[side].end.min(count);
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
            self.selected = presentation.selected.clone();
            self.destination = presentation.destination;
            self.expanded = presentation.expanded.clone();
        } else {
            self.scroll = [0.0; 2];
            self.selected = [0..0, 0..0];
            self.destination = [0; 2];
            self.expanded = [Vec::new(), Vec::new()];
        }
        self.generation = self.generation.wrapping_add(1);
        for side in 0..2 {
            self.side_generation[side] = self.side_generation[side].wrapping_add(1);
        }
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
                self.destination[side] = self.destination[side].min(count);
                self.selected[side] =
                    self.selected[side].start.min(count)..self.selected[side].end.min(count);
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
            let key = preview
                .bases
                .get(&change.path)
                .map(WorkingFile::identity_key)
                .unwrap_or_else(|| self.key_for(&change.path));
            if !unique.insert(key.clone()) {
                return Err(
                    "Replacement preview contains duplicate file identities. Run Find again."
                        .into(),
                );
            }
            let mut retained = self
                .documents
                .get(&key)
                .cloned()
                .or_else(|| preview.bases.get(&change.path).cloned().map(retained_file))
                .ok_or_else(|| "Replacement preview is missing a file snapshot.".to_owned())?;
            if retained.document.text() != change.before {
                return Err(format!(
                    "{} changed after preview. Run Find again.",
                    change.path.display()
                ));
            }
            retained
                .document
                .apply_byte_edits(HistoryTag::FolderBatch(id), change.edits.clone())
                .map_err(|e| e.to_string())?;
            if retained.document.text() != change.after {
                return Err("Replacement preview does not match its selected edits.".into());
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
            return Err("Undo newer edits or range changes in the batch files before moving this replacement transaction.".into());
        }
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
        self.invalidate_diff();
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
            let document = &mut self.documents.get_mut(&key).unwrap().document;
            let moved = if redo {
                document.redo_expected(tag)
            } else {
                document.undo_expected(tag)
            };
            debug_assert!(moved);
            self.refresh_active();
            self.invalidate_diff();
        }
    }
    /// Editor and transfer paths call this after every mutation so an older
    /// background result cannot paint alignment for new working-copy text.
    pub(super) fn invalidate_diff(&mut self) {
        self.publish_active();
        self.pending_auto = None;
        if let Some(job) = self.diff_job.take() {
            job.cancel.store(true, Ordering::Relaxed);
            self.retired_diffs.push(job);
        }
        self.diff = None;
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
    #[cfg(test)]
    pub(super) fn validate_search_changes(
        &self,
        changes: &[versus::SearchChange],
    ) -> Result<(), String> {
        let validate = |paths: &[PathBuf; 2], workspace: &TwoWayWorkspace| {
            for change in changes {
                for side in 0..2 {
                    if paths[side] != change.path {
                        continue;
                    }
                    let segments = text_segments(&change.before);
                    for protected in workspace.ignored_ranges(side_of(side)) {
                        let start: usize = segments
                            .iter()
                            .take(protected.start)
                            .map(|(text, ending)| text.len() + ending.len())
                            .sum();
                        let end: usize = segments
                            .iter()
                            .take(protected.end)
                            .map(|(text, ending)| text.len() + ending.len())
                            .sum();
                        if change.ranges.iter().any(|range| {
                            if range.is_empty() {
                                start < range.start && range.start < end
                            } else {
                                range.start < end && start < range.end
                            }
                        }) {
                            return Err(format!(
                                "{}: replacement overlaps protected lines. Adjust the selected matches or ignored ranges.",
                                change.path.display()
                            ));
                        }
                    }
                }
            }
            Ok(())
        };
        if let (Some(paths), Some(workspace)) = (&self.paths, &self.workspace) {
            validate(paths, workspace)?;
        }
        for retained in self.documents.values() {
            if let Some(path) = &retained.file.path {
                let facade = TwoWayWorkspace::from_documents([
                    retained.document.clone(),
                    InteractiveDocument::new(""),
                ]);
                validate(&[path.clone(), PathBuf::new()], &facade)?;
            }
        }
        Ok(())
    }
    fn cache_active(&mut self) {
        if let Some(paths) = &self.paths {
            self.cache.insert(
                paths.clone(),
                CachedPair {
                    scroll: self.scroll,
                    selected: self.selected.clone(),
                    destination: self.destination,
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
            return;
        }
        if self.files.is_none() && !self.side_files.iter().all(Option::is_some) {
            self.diff_job = None;
            self.diff = None;
            self.diff_rows.clear();
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
            let ignored = [
                workspace.ignored_ranges(TwoWaySide::Left).to_vec(),
                workspace.ignored_ranges(TwoWaySide::Right).to_vec(),
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
                let result = compare_loaded_text(
                    [&text[0], &text[1]],
                    &worker_cancel,
                    &options,
                    [&ignored[0], &ignored[1]],
                )
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
                }
                Ok(None) => {}
                Err(error) => self.error = Some(error),
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
            "Select content and a destination to transfer."
        } else {
            "Inspect differences between the open files."
        }
    }
    pub(super) fn footer_height(&self, ui: &egui::Ui) -> f32 {
        workspace_footer_height(
            ui,
            self.editing,
            &self.footer_legend_icons(),
            self.footer_legend_counts(),
        )
    }
    pub(super) fn render_previews(&mut self, ctx: &egui::Context) {
        self.save_preview(ctx);
        self.auto_preview(ctx);
    }
    pub(super) fn set_folder_differences(&mut self, differences: Vec<PathBuf>) {
        if self.folder_differences != differences {
            self.folder_differences = differences;
            self.folder_difference_index = None;
        }
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
            (width / 2.0 - (5.0 * extent + 4.0 * gap) / 2.0 - gap).max(110.0)
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
        let actions_width = 5.0 * extent + 4.0 * gap;
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
                        ToolbarIcon::AutoMerge,
                        self.run_diff && self.diff.is_some(),
                        "Auto Merge Non-Conflicts",
                    )
                    .clicked()
                    {
                        self.prepare_auto_merge();
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
        let row = &diff.rows[self.diff_rows[next]];
        let font = comparison_font(ctx);
        let row_height = ctx.fonts_mut(|fonts| fonts.row_height(&font));
        for (side, line) in [&row.left, &row.right].into_iter().enumerate() {
            if let Some((number, _)) = line {
                self.scroll[side] = number.saturating_sub(1) as f32 * row_height;
            }
        }
        ctx.request_repaint();
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
        let clip = ui.clip_rect().intersect(ui.available_rect_before_wrap());
        ui.scope(|ui| {
            ui.set_clip_rect(clip);
            ui.columns(2, |columns| {
                for side in 0..2 {
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
        egui::ScrollArea::vertical()
            .id_salt(("merge-pane-controls", self.side_generation[side], side))
            .max_height(height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if !self.editing {
                    self.header(ui, side);
                    ui.separator();
                    self.editor(ui, side, height - 35.);
                } else {
                    self.editor(ui, side, height);
                }
            });
    }
    fn header(&self, ui: &mut egui::Ui, side: usize) {
        let p = Palette::for_context(ui.ctx());
        let name = if side == 0 {
            "LEFT (Incoming)"
        } else {
            "RIGHT (Current)"
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(name).monospace().color(if side == 0 {
                p.left_only
            } else {
                p.right_only
            }));
            if let Some(path) = self.side_sources[side]
                .as_ref()
                .or_else(|| self.paths.as_ref().map(|paths| &paths[side]))
            {
                ui.add(
                    egui::Label::new(
                        RichText::new(path.display().to_string())
                            .small()
                            .color(p.muted),
                    )
                    .truncate(),
                )
                .on_hover_text(path.display().to_string());
            }
            if self
                .workspace
                .as_ref()
                .is_some_and(|w| w.is_dirty(side_of(side)))
            {
                ui.label(RichText::new("Unsaved Changes").strong().color(p.changed));
            }
        });
    }
    fn editor(&mut self, ui: &mut egui::Ui, side: usize, height: f32) {
        let palette = Palette::for_context(ui.ctx());
        let source = side_of(side);
        let current = self.workspace.as_ref().unwrap().text(source).to_owned();
        let displayed = current.replace("\r\n", "\n").replace('\r', "\n");
        let mut next = displayed.clone();
        let editor_id = egui::Id::new(("two-way-edit", self.side_generation[side], side));
        let previous_cursor = egui::TextEdit::load_state(ui.ctx(), editor_id)
            .and_then(|state| state.cursor.char_range())
            .map(|cursor| {
                let range = cursor.as_sorted_char_range();
                usize::from(range.start)..usize::from(range.end)
            });
        let font = comparison_font(ui.ctx());
        let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
        let count = self.workspace.as_ref().unwrap().line_count(source);
        let chunks = self.workspace.as_ref().unwrap().chunks(source);
        let ignored = self
            .workspace
            .as_ref()
            .unwrap()
            .ignored_ranges(source)
            .to_vec();
        let changed: BTreeMap<usize, DirectoryEntryState> = self
            .diff
            .as_ref()
            .map(|diff| {
                diff.rows
                    .iter()
                    .filter_map(|row| {
                        let line = if side == 0 { &row.left } else { &row.right };
                        line.as_ref()
                            .map(|(number, _)| (number - 1, row.state.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();
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
        let text_height = (height - if merge_controls { 150.0 } else { 5.0 }).max(60.0);
        let mut pending_transfer = None;
        let mut pending_context = None;
        let output = egui::ScrollArea::vertical()
            .id_salt(("two-way-scroll", self.side_generation[side], side))
            .auto_shrink([false, false])
            .max_height(text_height)
            .vertical_scroll_offset(self.scroll[side])
            .show(ui, |ui| {
                let number_width = 45.0;
                let gutter_width = if merge_controls { 50.0 } else { 0.0 };
                let width = ui.available_width().max(180.0);
                let text_width = (width - number_width - gutter_width - 12.0).max(80.0);
                let origin = ui.cursor().min;
                let content_height = (displayed
                    .lines()
                    .count()
                    .max(count + usize::from(displayed.ends_with('\n')))
                    .max(1) as f32
                    * row_height)
                    .max(row_height);
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
                    egui::pos2(text_x, origin.y),
                    egui::vec2(text_width, content_height),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
                    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _: f32| {
                        let mut job = egui::text::LayoutJob::default();
                        job.wrap.max_width = f32::INFINITY;
                        for (index, line) in buffer.as_str().split_inclusive('\n').enumerate() {
                            let protected = ignored.iter().any(|r| r.contains(&index));
                            let state = changed.get(&index);
                            let background = if protected {
                                palette.muted.gamma_multiply(0.15)
                            } else if state.is_some_and(|s| *s != DirectoryEntryState::Same) {
                                palette.changed.gamma_multiply(0.13)
                            } else {
                                Color32::TRANSPARENT
                            };
                            let format = egui::TextFormat {
                                font_id: font.clone(),
                                color: if protected {
                                    palette.muted
                                } else {
                                    palette.text
                                },
                                background,
                                line_height: Some(row_height),
                                ..Default::default()
                            };
                            append_code_line(&mut job, line, format, syntax, palette);
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
                    let edit = egui::ScrollArea::horizontal().id_salt(("interactive-horizontal",self.side_generation[side],side)).auto_shrink([false,true]).max_width(text_width).show(ui,|ui|egui::TextEdit::multiline(&mut next)
                        .id(editor_id)
                        .font(font.clone())
                        .code_editor()
                        .interactive(self.editing)
                        .desired_width(text_width)
                        .desired_rows(1)
                        .frame(egui::Frame::NONE)
                        .layouter(&mut layouter)
                        .show(ui)).inner;
                    if edit.response.has_focus() {
                        self.selected_side = side;
                        if let Some(cursor) = edit.cursor_range {
                            let a: usize = cursor.primary.index.min(cursor.secondary.index).into();
                            let b: usize = cursor.primary.index.max(cursor.secondary.index).into();
                            let a_line = next
                                .chars()
                                .take(a)
                                .filter(|c| *c == '\n')
                                .count()
                                .min(text_segments(&next).len());
                            let b_line = next
                                .chars()
                                .take(b)
                                .filter(|c| *c == '\n')
                                .count()
                                .min(text_segments(&next).len());
                            self.destination[side] = a_line;
                            // A cursor is an insertion target; a selection is an explicit replacement target.
                            let at_line_start = b == 0 || next.chars().take(b).last() == Some('\n');
                            self.selected[side] = if a == b {
                                a_line..a_line
                            } else {
                                a_line..(b_line + usize::from(!at_line_start)).min(text_segments(&next).len())
                            };
                        }
                    }
                });
                for line in 0..count.max(1) {
                    let y = origin.y + line as f32 * row_height;
                    ui.painter().text(
                        egui::pos2(numbers_x + number_width - 5.0, y),
                        Align2::RIGHT_TOP,
                        (line + 1).to_string(),
                        font.clone(),
                        palette.muted,
                    );
                    if ignored.iter().any(|r| r.contains(&line)) {
                        ui.painter().line_segment(
                            [
                                egui::pos2(numbers_x + 2.0, y + 2.0),
                                egui::pos2(numbers_x + 2.0, y + row_height - 2.0),
                            ],
                            Stroke::new(2.0, palette.changed),
                        );
                    }
                }
                if merge_controls {
                    for chunk in &chunks {
                        let expanded = self.expanded[side].contains(&chunk.id);
                        let lines: Vec<_> = if expanded {
                            chunk.range.clone().collect()
                        } else {
                            vec![chunk.range.start]
                        };
                        for line in lines {
                            let y = origin.y + line as f32 * row_height;
                            let rect = Rect::from_min_size(
                                egui::pos2(gutter_x + 22.0, y),
                                egui::vec2(24.0, row_height),
                            );
                            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                                let tooltip=if expanded{format!("Copy line {} to the chosen target",line+1)}else{format!("Copy chunk {}–{} to the chosen target · Right-click for chunk controls",chunk.range.start+1,chunk.range.end)};
                                let response=inline_action(ui,rect,egui::Id::new(("two-way-transfer",side,chunk.id,line)),if side==0{"→"}else{"←"},&tooltip);
                                if response.clicked() {
                                    pending_transfer = Some(if expanded {
                                        line..line + 1
                                    } else {
                                        chunk.range.clone()
                                    });
                                }
                                response.context_menu(|ui| {
                                    if ui.button("Select this chunk").clicked() {
                                        self.selected[side] = chunk.range.clone();
                                        ui.close();
                                    }
                                    if ui.button("Split at selected line").clicked() {
                                        pending_context = Some((0, chunk.id));
                                        ui.close();
                                    }
                                    if ui.button("Adjust to selected range").clicked() {
                                        pending_context = Some((1, chunk.id));
                                        ui.close();
                                    }
                                    if ui.button("Combine with next chunk").clicked() {
                                        pending_context = Some((2, chunk.id));
                                        ui.close();
                                    }
                                    if self.diff.is_some()
                                        && ui.button("Suggest corresponding destination").clicked()
                                    {
                                        self.suggest_target(source, chunk.range.clone());
                                        ui.close();
                                    }
                                });
                            });
                        }
                        let toggle = Rect::from_min_size(
                            egui::pos2(gutter_x, origin.y + chunk.range.start as f32 * row_height),
                            egui::vec2(21.0, row_height),
                        );
                        ui.scope_builder(egui::UiBuilder::new().max_rect(toggle), |ui| {
                            if inline_action(ui,toggle,egui::Id::new(("two-way-granularity",side,chunk.id)),if expanded{"−"}else{"+"},if expanded{"Return to chunk controls"}else{"Show an arrow for each line"}).clicked()
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
        if next != displayed {
            let (range, replacement) = editor_byte_edit(&current, &next, previous_cursor);
            if let Err(error) =
                self.workspace
                    .as_mut()
                    .unwrap()
                    .edit_bytes(source, range, replacement)
            {
                self.error = Some(error.to_string());
            }
            // The gutter geometry was rendered for the previous text.
            pending_transfer = None;
            pending_context = None;
            self.invalidate_diff();
        }
        if let Some(range) = pending_transfer {
            self.transfer(source, range);
        }
        if let Some((action, id)) = pending_context {
            let range = self.selected[side].clone();
            let workspace = self.workspace.as_mut().unwrap();
            let result = match action {
                0 => workspace.split_chunk(source, id, range.start).map(|_| ()),
                1 => workspace.adjust_chunk(source, id, range).map(|_| ()),
                _ => chunks
                    .iter()
                    .position(|c| c.id == id)
                    .and_then(|i| chunks.get(i + 1))
                    .ok_or(versus::TwoWayMergeError::CannotCombineChunks)
                    .and_then(|next| workspace.combine_chunks(source, id, next.id).map(|_| ())),
            };
            if let Err(error) = result {
                self.error = Some(error.to_string());
            }
        }
        if merge_controls {
            self.chunk_controls(ui, side);
        }
    }
    fn chunk_controls(&mut self, ui: &mut egui::Ui, side: usize) {
        let source = side_of(side);
        let count = self.workspace.as_ref().unwrap().line_count(source);
        let mut range = self.selected[side].clone();
        ui.horizontal_wrapped(|ui| {
            let mut start = range.start + 1;
            let mut end = range.end;
            ui.label("Selected lines");
            if ui
                .add(egui::DragValue::new(&mut start).range(1..=count.max(1)))
                .changed()
            {
                range.start = start - 1;
            }
            if ui
                .add(egui::DragValue::new(&mut end).range(0..=count))
                .changed()
            {
                range.end = end;
            }
            range.start = range.start.min(range.end);
            if ui
                .add_enabled(
                    !range.is_empty(),
                    egui::Button::new(if side == 0 {
                        "Transfer →"
                    } else {
                        "← Transfer"
                    }),
                )
                .on_hover_text("Copy selected lines to the other pane's chosen destination")
                .clicked()
            {
                self.transfer(source, range.clone());
            }
        });
        self.selected[side] = range.clone();
        ui.horizontal_wrapped(|ui| {
            let mut at = self.destination[side] + 1;
            ui.label("Insert before line");
            if ui
                .add(egui::DragValue::new(&mut at).range(1..=count + 1))
                .changed()
            {
                self.destination[side] = at - 1;
            }
            ui.checkbox(&mut self.replace, "Replace selected target")
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Requires an explicit nonempty selection in the destination pane");
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!range.is_empty(), egui::Button::new("Create chunk"))
                .clicked()
            {
                match self
                    .workspace
                    .as_mut()
                    .unwrap()
                    .create_chunk(source, range.clone())
                {
                    Ok(_) => {}
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
            if ui
                .add_enabled(!range.is_empty(), egui::Button::new("Ignore range"))
                .clicked()
            {
                match self
                    .workspace
                    .as_mut()
                    .unwrap()
                    .add_ignored_range(source, range.clone())
                {
                    Ok(_) => self.invalidate_diff(),
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
            if ui.button("Manage ignored…").clicked() {
                ui.memory_mut(|m| {
                    m.data
                        .insert_temp(egui::Id::new(("ignored-manager", side)), true)
                });
            }
        });
        let id = egui::Id::new(("ignored-manager", side));
        let mut open = ui.memory(|m| m.data.get_temp::<bool>(id)).unwrap_or(false);
        if open {
            let ranges = self
                .workspace
                .as_ref()
                .unwrap()
                .ignored_ranges(source)
                .to_vec();
            let window = egui::Window::new(if side == 0 {
                "Left ignored ranges"
            } else {
                "Right ignored ranges"
            })
            .id(id)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.label("Ignored lines remain in the file and are protected from transfers.");
                for existing in ranges {
                    ui.horizontal(|ui| {
                        ui.label(format!("Lines {}–{}", existing.start + 1, existing.end));
                        if ui.button("Remove").clicked() {
                            self.workspace
                                .as_mut()
                                .unwrap()
                                .remove_ignored_range(source, existing.clone());
                            self.invalidate_diff();
                        }
                        if ui
                            .add_enabled(!range.is_empty(), egui::Button::new("Use selection"))
                            .clicked()
                        {
                            if let Err(e) = self.workspace.as_mut().unwrap().update_ignored_range(
                                source,
                                existing.clone(),
                                range.clone(),
                            ) {
                                self.error = Some(e.to_string());
                            } else {
                                self.invalidate_diff();
                            }
                        }
                    });
                }
            });
            if let Some(window) = window {
                window_button_cursor(ui.ctx(), window.response.layer_id);
            }
        }
        ui.memory_mut(|m| m.data.insert_temp(id, open));
    }
    fn footer(&mut self, ui: &mut egui::Ui) -> Rect {
        let icons = self.footer_legend_icons();
        let row = workspace_footer_layout(ui, self.editing, &icons, self.footer_legend_counts());
        ui.scope_builder(egui::UiBuilder::new().max_rect(row.info), |ui| {
            footer_guidance(ui, self.guidance_message(), self.any_dirty());
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(row.legend), |ui| {
            bottom_legend(ui, &icons, self.footer_legend_counts());
        });
        if let Some(save) = row.save {
            let enabled = self.workspace.is_some()
                && self.side_has_file(0)
                && self.side_has_file(1)
                && !self.is_saving();
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(save)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    if captioned_icon_button(ui, ToolbarIcon::Save, enabled, "Save Both").clicked()
                    {
                        self.request_save(vec![0, 1]);
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
        let diff = self.diff.as_ref()?;
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
        let target = if self.replace {
            let selected = self.selected[side].clone();
            if selected.is_empty() {
                self.error = Some("Select destination lines before replacing content.".into());
                return;
            }
            selected
        } else {
            let at = self.destination[side].min(self.workspace.as_ref().unwrap().line_count(dest));
            at..at
        };
        let placement = if self.replace {
            TransferPlacement::Replace
        } else {
            TransferPlacement::Insert
        };
        let old_count = self.workspace.as_ref().unwrap().line_count(dest);
        match self
            .workspace
            .as_mut()
            .unwrap()
            .transfer(source, range, target.clone(), placement)
        {
            Ok(()) => {
                let new_count = self.workspace.as_ref().unwrap().line_count(dest);
                let inserted = (new_count as isize - old_count as isize + target.len() as isize)
                    .max(0) as usize;
                if self.replace {
                    self.selected[side] = target.start + inserted..target.start + inserted;
                } else {
                    self.selected[side] =
                        remap_target(self.selected[side].clone(), target.clone(), inserted);
                }
                self.destination[side] = target.start + inserted;
                self.activate_editor(side);
                self.error = None;
                self.status = "Transferred into the working copy. Save when ready.".into();
                self.invalidate_diff();
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    fn suggest_target(&mut self, source: TwoWaySide, range: Range<usize>) {
        let Some(diff) = &self.diff else {
            return;
        };
        let mut target = Vec::new();
        let mut preceding = 0;
        for row in &diff.rows {
            let (from, to) = if source == TwoWaySide::Left {
                (&row.left, &row.right)
            } else {
                (&row.right, &row.left)
            };
            if let Some((number, _)) = from {
                let line = number - 1;
                if line < range.start {
                    if let Some((n, _)) = to {
                        preceding = *n;
                    }
                } else if range.contains(&line) {
                    if let Some((n, _)) = to {
                        target.push(n - 1);
                    }
                }
            }
        }
        let side = side_index(source.other());
        self.selected[side] = target
            .first()
            .zip(target.last())
            .map(|(a, b)| *a..*b + 1)
            .unwrap_or(preceding..preceding);
        self.destination[side] = self.selected[side].start;
        self.sync_editor_cursor(side);
        self.status =
            "Suggested destination selected. Choose insertion or explicit replacement.".into();
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
                self.destination[side] = count;
                self.selected[side] = count..count;
                self.activate_editor(side);
                self.invalidate_diff();
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    fn prepare_auto_merge(&mut self) {
        let Some(diff) = &self.diff else {
            return;
        };
        let mut transfers = Vec::new();
        let mut index = 0;
        let mut right_line = 0;
        while index < diff.rows.len() {
            let row = &diff.rows[index];
            if row.state != DirectoryEntryState::LeftOnly {
                if let Some((number, _)) = &row.right {
                    right_line = *number;
                }
                index += 1;
                continue;
            }
            let first = index;
            let Some((start, _)) = &row.left else {
                index += 1;
                continue;
            };
            let start = *start - 1;
            let mut end = start;
            while index < diff.rows.len()
                && diff.rows[index].state == DirectoryEntryState::LeftOnly
                && diff.rows[index]
                    .left
                    .as_ref()
                    .is_some_and(|(number, _)| *number == end + 1)
            {
                end += 1;
                index += 1;
            }
            let before = first == 0 || diff.rows[first - 1].state == DirectoryEntryState::Same;
            let after =
                index == diff.rows.len() || diff.rows[index].state == DirectoryEntryState::Same;
            if before && after {
                transfers.push((start..end, right_line..right_line));
            }
        }
        if transfers.is_empty() {
            self.status="No incoming-only sections have matching boundaries. Use the chunk arrows for manual choices.".into();
            return;
        }
        self.pending_auto = Some(transfers);
    }
    fn auto_preview(&mut self, ctx: &egui::Context) {
        let Some(transfers) = self.pending_auto.clone() else {
            return;
        };
        let Some(workspace) = &self.workspace else {
            self.pending_auto = None;
            return;
        };
        let source = text_segments(workspace.text(TwoWaySide::Left));
        let mut confirm = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("auto-merge-preview")).show(ctx,|ui|{
            ui.heading("Auto Merge Non-Conflicts");ui.label("Review incoming insertions anchored by matching content. Existing Current lines are preserved.");
            egui::ScrollArea::vertical().max_height(300.0).show(ui,|ui|{for(range,target)in &transfers{ui.label(format!("Left lines {}–{} → before Right line {}",range.start+1,range.end,target.start+1));let text=source[range.clone()].iter().map(|(text,ending)|format!("{text}{ending}")).collect::<String>();ui.label(RichText::new(text).monospace());}});
            ui.horizontal(|ui|{cancel=ui.button("Cancel").clicked();confirm=ui.button("Apply insertions").clicked();});
        });
        if cancel {
            self.pending_auto = None;
        }
        if confirm {
            self.pending_auto = None;
            let actions = transfers
                .iter()
                .map(|(from, to)| (from.clone(), to.clone(), TransferPlacement::Insert))
                .collect();
            match self
                .workspace
                .as_mut()
                .unwrap()
                .transfer_batch(TwoWaySide::Left, actions)
            {
                Ok(()) => {
                    let mut transfers = transfers;
                    transfers.sort_by_key(|(_, target)| std::cmp::Reverse(target.start));
                    for (source, target) in transfers {
                        self.selected[1] =
                            remap_target(self.selected[1].clone(), target.clone(), source.len());
                        let cursor = remap_target(
                            self.destination[1]..self.destination[1],
                            target,
                            source.len(),
                        );
                        self.destination[1] = cursor.start;
                    }
                    self.activate_editor(1);
                    self.invalidate_diff();
                    self.error = None;
                    self.status =
                        "Auto merge applied to the Right working copy. Undo reverses the batch."
                            .into();
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }
    }
    fn sync_editor_cursor(&self, side: usize) {
        let (Some(ctx), Some(workspace)) = (&self.editor_context, &self.workspace) else {
            return;
        };
        let text = workspace
            .text(side_of(side))
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        let range = &self.selected[side];
        let at = if range.is_empty() {
            self.destination[side]
        } else {
            range.start
        };
        let offset = |line: usize| {
            text.split_inclusive('\n')
                .take(line)
                .map(|line| line.chars().count())
                .sum::<usize>()
        };
        let a = offset(at);
        let b = if range.is_empty() {
            a
        } else {
            offset(range.end)
        };
        let id = egui::Id::new(("two-way-edit", self.side_generation[side], side));
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(a),
                egui::text::CCursor::new(b),
            )));
        state.clear_undoer();
        state.store(ctx, id);
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
fn retained_file(file: WorkingFile) -> RetainedFile {
    let mut document = InteractiveDocument::new(file.text.clone());
    document.mark_saved_text(file.baseline_text().unwrap_or_default());
    RetainedFile { file, document }
}
fn file_key(path: &std::path::Path) -> PathBuf {
    versus::working_file_key(path)
}
fn remap_target(range: Range<usize>, target: Range<usize>, inserted: usize) -> Range<usize> {
    let delta = inserted as isize - target.len() as isize;
    let map = |line: usize| {
        if line < target.start {
            line
        } else if line >= target.end {
            line.saturating_add_signed(delta)
        } else {
            target.start + inserted
        }
    };
    map(range.start)..map(range.end)
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
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        comparison_font(ui.ctx()),
        if response.hovered() {
            palette.accent
        } else {
            palette.text
        },
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
}

fn window_button_cursor(ctx: &egui::Context, layer: egui::LayerId) {
    // egui's built-in window close/collapse controls bypass interact_cursor.
    // Limit this fallback to click-only controls so window dragging, resizing,
    // and text editing retain their own cursors.
    let hovered =
        ctx.interaction_snapshot(|snapshot| snapshot.hovered.iter().copied().collect::<Vec<_>>());
    for id in hovered {
        if let Some(response) = ctx.read_response(id)
            && response.layer_id == layer
            && response.sense == Sense::click()
        {
            response.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
    }
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
        view.folder_differences = vec![PathBuf::from("one"), PathBuf::from("two")];
        assert_eq!(view.change_label(), "2 changes");
        view.jump_diff(true, &ctx);
        assert_eq!(view.change_label(), "Change 1 of 2");
    }
    #[test]
    fn keyboard_edit_before_repeated_protected_lines_uses_cursor_anchor() {
        let ctx = egui::Context::default();
        let mut view = view("x\r\nx\np\r", "right\n");
        view.workspace
            .as_mut()
            .unwrap()
            .add_ignored_range(TwoWaySide::Left, 1..2)
            .unwrap();
        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        view.destination[0] = 0;
        view.sync_editor_cursor(0);
        let id = egui::Id::new(("two-way-edit", view.side_generation[0], 0usize));
        ctx.memory_mut(|memory| memory.request_focus(id));
        frame(&mut view, &ctx, vec![egui::Event::Text("x\n".into())])
            .drop_without_applying_deltas();
        assert_eq!(view.working_text(0), Some("x\r\nx\r\nx\np\r"));
        assert_eq!(
            view.workspace
                .as_ref()
                .unwrap()
                .ignored_ranges(TwoWaySide::Left),
            &[2..3]
        );
        view.move_history(false);
        assert_eq!(view.working_text(0), Some("x\r\nx\np\r"));
        assert_eq!(
            view.workspace
                .as_ref()
                .unwrap()
                .ignored_ranges(TwoWaySide::Left),
            &[1..2]
        );
        view.move_history(true);
        assert_eq!(
            view.workspace
                .as_ref()
                .unwrap()
                .ignored_ranges(TwoWaySide::Left),
            &[2..3]
        );
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
        view.destination[1] = 1;
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
        view.destination[1] = 1;
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
        let position = ctx.read_response(id).unwrap().rect.center();
        for pressed in [true, false] {
            frame(
                view,
                ctx,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            )
            .drop_without_applying_deltas();
        }
    }
    #[test]
    fn merge_without_diff_keeps_preferences_and_history_across_modes() {
        let ctx = egui::Context::default();
        let mut view = view("left\n", "unrelated\n");
        view.merge_options = [false, true, false];
        view.destination[1] = 1;
        view.transfer(TwoWaySide::Left, 0..1);
        view.workspace
            .as_mut()
            .unwrap()
            .add_ignored_range(TwoWaySide::Right, 0..1)
            .unwrap();
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
        assert!(
            view.workspace
                .as_ref()
                .unwrap()
                .ignored_ranges(TwoWaySide::Right)
                .is_empty()
        );
        assert!(view.workspace.as_mut().unwrap().undo());
        assert_eq!(view.working_text(1), Some("unrelated\n"));
        assert!(view.workspace.as_mut().unwrap().redo());
        assert_eq!(view.working_text(1), Some(result.as_str()));
    }
    #[test]
    fn window_title_buttons_use_pointer_cursor() {
        let ctx = egui::Context::default();
        set_theme(&ctx, egui::Theme::Dark);
        let mut open = true;
        let mut frame = |position: Option<Pos2>| {
            ctx.run_ui(
                egui::RawInput {
                    events: position
                        .map(egui::Event::PointerMoved)
                        .into_iter()
                        .collect(),
                    ..Default::default()
                },
                |_| {
                    if let Some(window) = egui::Window::new("Ignored ranges")
                        .open(&mut open)
                        .resizable(false)
                        .movable(false)
                        .show(&ctx, |ui| {
                            ui.label("Protected content");
                        })
                    {
                        window_button_cursor(&ctx, window.response.layer_id);
                    }
                },
            )
        };
        // Windows have an initial invisible sizing pass.
        frame(None).drop_without_applying_deltas();
        frame(None).drop_without_applying_deltas();
        let buttons = ctx
            .interactive_rects_last_pass()
            .into_iter()
            .filter(|rect| rect.width() < 30.0 && rect.height() < 30.0)
            .collect::<Vec<_>>();
        assert_eq!(buttons.len(), 2, "close and collapse buttons");
        for button in buttons {
            let output = frame(Some(button.center()));
            assert_eq!(
                output.platform_output.cursor_icon,
                egui::CursorIcon::PointingHand
            );
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn actual_gutter_arrows_copy_chunks_and_lines_without_diff() {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let ctx = egui::Context::default();
            set_theme(&ctx, theme);
            let mut view = view("fn source() {\n    one();\n    two();\n}\n", "target\n");
            view.destination[1] = 1;
            frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
            let chunk = view.workspace.as_ref().unwrap().chunks(TwoWaySide::Left)[0].clone();
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
            assert_eq!(view.destination[1], 5);
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
    fn save_both_matches_toolbar_feedback_and_still_opens_a_preview() {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let ctx = egui::Context::default();
            set_theme(&ctx, theme);
            let mut view = view("left\n", "right\n");
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
            assert_eq!(view.working_text(0), Some("left\n"));
            assert_eq!(view.working_text(1), Some("right\n"));
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
                view.diff_options = [true, false, true];
                view.set_run_diff(true);
                view.workspace
                    .as_mut()
                    .unwrap()
                    .edit_bytes(TwoWaySide::Left, 0..0, "edited\n".into())
                    .unwrap();
                view.selected = [0..1, 0..1];
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
                assert_eq!(view.selected, [0..1, 0..1]);
                assert_eq!(view.merge_options, [false, true, true]);
                assert_eq!(view.diff_options, [true, false, true]);
                assert!(view.diff_job.is_none());
            }
        }
    }
    #[test]
    fn accept_all_is_atomic_and_never_replaces_protected_content() {
        let mut view = view("whole\nleft\n", "right\n");
        view.destination[1] = 1;
        view.replace = false;
        view.copy_all(TwoWaySide::Left);
        assert_eq!(view.working_text(1), view.working_text(0));
        assert!(view.workspace.as_mut().unwrap().undo());
        assert_eq!(view.working_text(1), Some("right\n"));
        view.workspace
            .as_mut()
            .unwrap()
            .add_ignored_range(TwoWaySide::Right, 0..1)
            .unwrap();
        view.copy_all(TwoWaySide::Left);
        assert_eq!(view.working_text(1), Some("right\n"));
        assert!(view.error.as_ref().unwrap().contains("protected"));
    }
    #[test]
    fn repeated_insertions_follow_the_original_target_in_source_order() {
        let mut view = view("first\nsecond\n", "keep\n");
        view.destination[1] = 0;
        view.transfer(TwoWaySide::Left, 0..1);
        view.transfer(TwoWaySide::Left, 1..2);
        assert_eq!(view.working_text(1), Some("first\nsecond\nkeep\n"));
        assert!(view.workspace.as_mut().unwrap().undo());
        assert_eq!(view.working_text(1), Some("first\nkeep\n"));
    }
    #[test]
    fn search_protection_checks_only_the_matched_ranges() {
        let mut view = view("a\nprotected\nlast\n", "right\n");
        view.workspace
            .as_mut()
            .unwrap()
            .add_ignored_range(TwoWaySide::Left, 1..2)
            .unwrap();
        let change = SearchChange {
            path: PathBuf::from("left.rs"),
            before: "a\nprotected\nlast\n".into(),
            after: "b\nprotected\nlast\n".into(),
            hit_ids: vec![1],
            ranges: vec![0..1],
            edits: vec![(0..1, "b".into())],
        };
        assert!(view.validate_search_changes(&[change.clone()]).is_ok());
        let blocked = SearchChange {
            ranges: vec![2..11],
            ..change
        };
        assert!(view.validate_search_changes(&[blocked]).is_err());
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
    fn later_protection_changes_block_atomic_batch_undo_until_their_undo() {
        let mut view = view("left\n", "right\n");
        let preview = preview_for(
            &view,
            &[("left.rs", 0..4, "batch"), ("right.rs", 0..5, "batch")],
        );
        view.apply_search_preview(&preview).unwrap();
        view.workspace
            .as_mut()
            .unwrap()
            .add_ignored_range(TwoWaySide::Right, 0..1)
            .unwrap();
        view.invalidate_diff();
        view.selected_side = 0;
        view.move_history(false);
        assert_eq!(view.working_text(0), Some("batch\n"));
        assert_eq!(view.working_text(1), Some("batch\n"));
        assert!(view.error.is_some());
        view.selected_side = 1;
        view.move_history(false);
        assert!(
            view.workspace
                .as_ref()
                .unwrap()
                .ignored_ranges(TwoWaySide::Right)
                .is_empty()
        );
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
        view.workspace
            .as_mut()
            .unwrap()
            .add_ignored_range(TwoWaySide::Right, 0..1)
            .unwrap();
        view.invalidate_diff();
        let preview = preview_for(
            &view,
            &[("left.rs", 0..4, "batch"), ("right.rs", 0..5, "batch")],
        );
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
