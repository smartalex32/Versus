//! Retained documents, pair activation, and side-load lifecycle.

use super::*;

impl InteractiveView {
    /// Opening files is independent from comparisons. Missing paths create empty,
    /// saveable working copies so one-sided directory entries remain mergeable.
    pub(in crate::app) fn open_pair(&mut self, paths: [PathBuf; 2]) {
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
    pub(in crate::app) fn open_sources(&mut self, sources: [Option<PathBuf>; 2]) {
        for (side, source) in sources.into_iter().enumerate() {
            self.open_side(side, source);
        }
    }
    pub(in crate::app) fn open_side(&mut self, side: usize, path: Option<PathBuf>) {
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
    pub(in crate::app) fn set_editing(&mut self, editing: bool, diff_preferences: [bool; 3]) {
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
    pub(in crate::app) fn set_run_diff(&mut self, enabled: bool) {
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
    pub(in crate::app) fn refresh_diff(&mut self) {
        self.invalidate_diff();
        self.diff_not_before = None;
    }
    pub(in crate::app) fn can_refresh_diff(&self) -> bool {
        self.has_actual_pair()
            && self.loading.is_none()
            && self.side_loading.iter().all(Option::is_none)
            && self.diff_job.is_none()
    }
    pub(super) fn engine_enabled(&self) -> bool {
        !self.editing || self.run_diff
    }
    pub(in crate::app) fn is_saving(&self) -> bool {
        self.save_job.is_some() || self.external_save_busy
    }
    pub(in crate::app) fn set_external_save_busy(&mut self, busy: bool) {
        self.external_save_busy = busy;
    }
    pub(in crate::app) fn poll(&mut self, ctx: &egui::Context) {
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
    pub(in crate::app) fn any_dirty(&self) -> bool {
        self.workspace
            .as_ref()
            .is_some_and(TwoWayWorkspace::has_unsaved_changes)
            || self
                .documents
                .values()
                .any(|retained| retained.document.is_dirty())
    }
    pub(in crate::app) fn take_back_requested(&mut self) -> bool {
        std::mem::take(&mut self.back)
    }
    pub(in crate::app) fn run_diff_enabled(&self) -> bool {
        self.run_diff
    }
    pub(in crate::app) fn merge_options(&self) -> [bool; 3] {
        self.merge_options
    }
    pub(in crate::app) fn set_from_folders(&mut self, from_folders: bool) {
        self.from_folders = from_folders;
    }
    pub(in crate::app) fn take_loaded_paths(&mut self) -> Option<[PathBuf; 2]> {
        self.loaded_paths.take()
    }
    pub(in crate::app) fn take_loaded_sources(&mut self) -> Option<[Option<PathBuf>; 2]> {
        self.loaded_sources.take()
    }
    pub(in crate::app) fn current_sources(&self) -> [Option<PathBuf>; 2] {
        self.side_sources.clone()
    }
    pub(in crate::app) fn pair_paths(&self) -> Option<&[PathBuf; 2]> {
        self.loading
            .as_ref()
            .map(|(_, paths)| paths)
            .or(self.paths.as_ref())
    }
    pub(in crate::app) fn selected_source(&self, side: usize) -> Option<&PathBuf> {
        self.side_sources[side].as_ref()
    }
    pub(in crate::app) fn take_new_requested(&mut self) -> bool {
        std::mem::take(&mut self.new_requested)
    }
    pub(in crate::app) fn take_search_requested(&mut self) -> bool {
        std::mem::take(&mut self.search_requested)
    }
    pub(in crate::app) fn is_side_dirty(&self, side: usize) -> bool {
        self.workspace
            .as_ref()
            .is_some_and(|workspace| workspace.is_dirty(side_of(side)))
    }
    pub(in crate::app) fn can_save_side(&self, side: usize) -> bool {
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
    pub(in crate::app) fn has_retained_dirty_documents(&self) -> bool {
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
    pub(in crate::app) fn take_save_all_requested(&mut self) -> bool {
        std::mem::take(&mut self.save_all_requested)
    }
    pub(in crate::app) fn request_side_save(&mut self, side: usize) {
        if self.can_save_side(side) {
            self.request_save(vec![side]);
        }
    }
    pub(in crate::app) fn clear_error(&mut self) {
        if self.error.take().is_some() {
            self.status.clear();
        }
    }
    pub(in crate::app) fn error_details(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub(super) fn key_for(&self, path: &std::path::Path) -> PathBuf {
        self.aliases
            .get(path)
            .cloned()
            .unwrap_or_else(|| file_key(path))
    }
    pub(super) fn register_file(&mut self, path: PathBuf, file: WorkingFile) {
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
    pub(in crate::app) fn export_working_files(&self) -> BTreeMap<PathBuf, WorkingFile> {
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
    pub(in crate::app) fn folder_revision_signature(&self) -> u64 {
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
    pub(in crate::app) fn folder_document_revisions(&self) -> BTreeMap<PathBuf, u64> {
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
    pub(in crate::app) fn loaded_pair_comparison(
        &self,
    ) -> Option<([PathBuf; 2], DirectoryEntryState)> {
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
    pub(super) fn publish_active(&mut self) {
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
    pub(super) fn refresh_active(&mut self) {
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
    pub(super) fn refresh_partial_active(&mut self) {
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
    pub(super) fn activate_pair(&mut self, paths: [PathBuf; 2]) {
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
    pub(super) fn activate_side_workspace(&mut self) {
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
    pub(in crate::app) fn update_saved_files(&mut self, files: &BTreeMap<PathBuf, WorkingFile>) {
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
}

pub(super) fn external_refresh_edit(before: &str, after: &str) -> Option<(Range<usize>, String)> {
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

pub(super) fn retained_file(file: WorkingFile) -> RetainedFile {
    let mut document = InteractiveDocument::new(file.text.clone());
    document.mark_saved_text(file.baseline_text().unwrap_or_default());
    RetainedFile { file, document }
}
pub(super) fn file_key(path: &std::path::Path) -> PathBuf {
    versus::working_file_key(path)
}
