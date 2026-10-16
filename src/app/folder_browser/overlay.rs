//! Working-copy folder comparison, signatures, and cached merged tree state.

use super::*;
use std::{
    fs,
    hash::{Hash, Hasher},
};
use versus::{FileViewOptions, compare_loaded_text, load_working_file};

pub(super) struct OverlayJob {
    pub(super) receiver: Receiver<(u64, Result<BTreeMap<PathBuf, DirectoryEntryState>, String>)>,
    pub(super) cancel: Arc<AtomicBool>,
    /// Input signatures are committed only when this generation completes.
    pub(super) signatures: BTreeMap<PathBuf, u64>,
}

pub(super) struct MergedFolderCache {
    pub(super) tree_key: Option<usize>,
    pub(super) states: Arc<BTreeMap<PathBuf, DirectoryEntryState>>,
    pub(super) difference_paths: Arc<Vec<PathBuf>>,
}

impl Drop for OverlayJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl FolderBrowser {
    pub(in crate::app) fn poll_comparisons(
        &mut self,
        view: &InteractiveView,
        ctx: &egui::Context,
        merge_mode: bool,
    ) {
        if merge_mode {
            self.update_overlays(view, ctx);
        } else {
            if let Some(job) = self.overlay_job.take() {
                job.cancel.store(true, Ordering::Relaxed);
                self.retired_overlay_jobs.push(job);
            }
            self.reap_retired_overlays();
            self.overlay_not_before = None;
            self.overlay_token = None;
        }
    }
    pub(super) fn update_overlays(&mut self, view: &InteractiveView, ctx: &egui::Context) {
        self.reap_retired_overlays();
        if !view.run_diff_enabled() {
            if let Some(job) = self.overlay_job.take() {
                job.cancel.store(true, Ordering::Relaxed);
                self.retired_overlay_jobs.push(job);
            }
            self.overlay_not_before = None;
            self.overlay_token = None;
            self.overlay_paths.clear();
            self.overlay_cache.clear();
            self.overlays.clear();
            self.invalidate_comparison_cache();
            return;
        }
        // A folder diff has meaning only with two real directory sources.
        // Single-source navigation remains fully available without it.
        let Some(roots) = self.paired_roots() else {
            self.overlays.clear();
            self.invalidate_comparison_cache();
            self.overlay_paths.clear();
            self.overlay_cache.clear();
            return;
        };
        // Show-only is a presentation filter, not a comparison input.
        let preferences = view.merge_options();
        let token = (
            view.folder_revision_signature(),
            [false, preferences[1], preferences[2]],
        );
        if self.overlay_token != Some(token) {
            self.overlay_token = Some(token);
            self.overlay_not_before = Some(Instant::now() + Duration::from_millis(150));
            if let Some(job) = self.overlay_job.take() {
                job.cancel.store(true, Ordering::Relaxed);
                self.retired_overlay_jobs.push(job);
            }
            self.overlay_generation = self.overlay_generation.wrapping_add(1);
        }
        let completed = self
            .overlay_job
            .as_ref()
            .and_then(|job| match job.receiver.try_recv() {
                Ok(result) => Some(Ok(result)),
                Err(TryRecvError::Disconnected) => {
                    Some(Err("Merge folder diff worker stopped".into()))
                }
                Err(TryRecvError::Empty) => None,
            });
        if let Some(completed) = completed {
            let job = self.overlay_job.take().expect("completed overlay job");
            match completed {
                Ok((generation, Ok(results))) if generation == self.overlay_generation => {
                    // A job only owns its requested paths, so preserve a
                    // freshly reused active-editor comparison beside it.
                    self.overlays.extend(results);
                    self.overlay_cache.extend(job.signatures.clone());
                    self.invalidate_comparison_cache();
                }
                Ok((_, Err(error))) => self.error = Some(error),
                Ok(_) => {}
                Err(error) => self.error = Some(error),
            }
        }
        if self.overlay_job.is_some() {
            return;
        }
        let Some(deadline) = self.overlay_not_before else {
            return;
        };
        if Instant::now() < deadline {
            ctx.request_repaint_after(deadline.saturating_duration_since(Instant::now()));
            return;
        }

        // Do not pile up workers while a cancelled comparison is still
        // winding down; compare_loaded_text cooperates with cancellation.
        if !self.retired_overlay_jobs.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(20));
            return;
        }
        self.overlay_not_before = None;
        let working = view.export_working_files();
        let revisions = view.folder_document_revisions();
        let mut loaded: BTreeMap<PathBuf, [Option<WorkingFile>; 2]> = BTreeMap::new();
        let mut modified = BTreeSet::new();
        for file in working.values() {
            let Some(path) = file.path.as_ref() else {
                continue;
            };
            for side in 0..2 {
                if let Ok(relative) = path.strip_prefix(&roots[side]) {
                    let entry = loaded
                        .entry(relative.to_path_buf())
                        .or_insert_with(|| [None, None]);
                    entry[side] = Some(file.clone());
                    if file.is_dirty() {
                        modified.insert(relative.to_path_buf());
                    }
                }
            }
        }
        // Include paths that used to be dirty once more. Their signature
        // changes after save, which puts the original disk comparison back
        // without rereading every folder.
        let mut affected = self.overlay_paths.clone();
        affected.extend(modified.iter().cloned());
        self.overlay_paths = affected.clone();
        if let Some((paths, state)) = view.loaded_pair_comparison() {
            if let (Ok(left), Ok(right)) = (
                paths[0].strip_prefix(&roots[0]),
                paths[1].strip_prefix(&roots[1]),
            ) {
                if left == right && affected.remove(left) {
                    self.overlays.insert(left.to_path_buf(), state);
                    let signature = overlay_signature(
                        left,
                        loaded.get(left),
                        &revisions,
                        &preferences,
                        modified.contains(left),
                    );
                    self.overlay_cache.insert(left.to_path_buf(), signature);
                    self.invalidate_comparison_cache();
                }
            }
        }
        let mut requested = BTreeSet::new();
        let mut signatures = BTreeMap::new();
        for relative in affected {
            let signature = overlay_signature(
                &relative,
                loaded.get(&relative),
                &revisions,
                &preferences,
                modified.contains(&relative),
            );
            if self.overlay_cache.get(&relative) != Some(&signature) {
                signatures.insert(relative.clone(), signature);
                requested.insert(relative);
            }
        }
        if requested.is_empty() {
            return;
        }
        let options = view.merge_options();
        let generation = self.overlay_generation;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = crate::diagnostics::catch_worker("Merge working-copy comparison", || {
                compare_overlay_files(roots, requested, loaded, options, &worker_cancel)
            });
            let _ = sender.send((generation, result));
        });
        self.overlay_job = Some(OverlayJob {
            receiver,
            cancel,
            signatures,
        });
    }
    pub(super) fn reap_retired_overlays(&mut self) {
        self.retired_overlay_jobs
            .retain_mut(|job| matches!(job.receiver.try_recv(), Err(TryRecvError::Empty)));
    }
    pub(in crate::app) fn difference_paths(
        &mut self,
        base: &FolderTree,
        view: &InteractiveView,
    ) -> Arc<Vec<PathBuf>> {
        if !view.run_diff_enabled() {
            return Arc::new(Vec::new());
        }
        self.merged_folder_cache(Some(base))
            .difference_paths
            .clone()
    }
}
pub(super) fn compare_overlay_files(
    roots: [PathBuf; 2],
    affected: BTreeSet<PathBuf>,
    loaded: BTreeMap<PathBuf, [Option<WorkingFile>; 2]>,
    preferences: [bool; 3],
    cancellation: &AtomicBool,
) -> Result<BTreeMap<PathBuf, DirectoryEntryState>, String> {
    let options = FileViewOptions {
        ignore_whitespace: preferences[1],
        ignore_line_endings: preferences[2],
        progress: None,
    };
    let mut states = BTreeMap::new();
    for relative in affected {
        if cancellation.load(Ordering::Relaxed) {
            return Ok(states);
        }
        let cached = loaded
            .get(&relative)
            .cloned()
            .unwrap_or_else(|| [None, None]);
        let mut files: [Option<WorkingFile>; 2] = cached;
        let mut non_file = [false; 2];
        for side in 0..2 {
            if files[side].is_none() {
                let path = roots[side].join(&relative);
                match fs::symlink_metadata(&path) {
                    Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                        files[side] = Some(
                            load_working_file(Some(path.as_path()))
                                .map_err(|error| error.to_string())?,
                        );
                    }
                    Ok(_) => non_file[side] = true,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(format!("{}: {error}", path.display())),
                }
            }
        }
        let state = if non_file.iter().any(|is_non_file| *is_non_file) {
            DirectoryEntryState::TypeMismatch
        } else {
            match (&files[0], &files[1]) {
                (Some(_), None) => DirectoryEntryState::LeftOnly,
                (None, Some(_)) => DirectoryEntryState::RightOnly,
                (None, None) => continue,
                (Some(left), Some(right)) => {
                    let comparison = compare_loaded_text(
                        [&left.text, &right.text],
                        cancellation,
                        &options,
                        [&[], &[]],
                    )
                    .map_err(|error| error.to_string())?;
                    let Some(comparison) = comparison else {
                        return Ok(states);
                    };
                    if comparison.message.is_some()
                        || comparison
                            .rows
                            .iter()
                            .any(|row| row.state != DirectoryEntryState::Same)
                    {
                        DirectoryEntryState::Different
                    } else {
                        DirectoryEntryState::Same
                    }
                }
            }
        };
        states.insert(relative, state);
    }
    Ok(states)
}

pub(super) fn overlay_signature(
    relative: &std::path::Path,
    files: Option<&[Option<WorkingFile>; 2]>,
    revisions: &BTreeMap<PathBuf, u64>,
    preferences: &[bool; 3],
    modified: bool,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    relative.hash(&mut hasher);
    preferences[1].hash(&mut hasher);
    preferences[2].hash(&mut hasher);
    modified.hash(&mut hasher);
    for file in files.into_iter().flat_map(|files| files.iter()) {
        match file {
            Some(file) => {
                file.identity_key().hash(&mut hasher);
                revisions.get(&file.identity_key()).hash(&mut hasher);
            }
            None => 0_u8.hash(&mut hasher),
        }
    }
    hasher.finish()
}

impl FolderBrowser {
    /// Returns a projection that includes working-copy comparisons.  It is
    /// rebuilt only when a comparison tree or overlay changes, never while the
    /// user merely scrolls, expands, or paints either pane.
    pub(super) fn merged_folder_cache(&mut self, tree: Option<&FolderTree>) -> &MergedFolderCache {
        let tree_key = tree.map(|tree| std::ptr::from_ref(tree.root()) as usize);
        let stale = self
            .merged_cache
            .as_ref()
            .is_none_or(|cache| cache.tree_key != tree_key);
        if stale {
            let started = Instant::now();
            let cache = build_merged_folder_cache(tree, &self.overlays, tree_key);
            crate::diagnostics::info(format!(
                "Merge folder state cache rebuilt: {} entries, {} differences in {:.3}s",
                cache.states.len(),
                cache.difference_paths.len(),
                started.elapsed().as_secs_f64()
            ));
            self.merged_cache = Some(cache);
        }
        self.merged_cache
            .as_ref()
            .expect("cache was just populated")
    }
}

pub(super) fn build_merged_folder_cache(
    tree: Option<&FolderTree>,
    overlays: &BTreeMap<PathBuf, DirectoryEntryState>,
    tree_key: Option<usize>,
) -> MergedFolderCache {
    let mut states = BTreeMap::new();
    let mut differences = Vec::new();
    if let Some(tree) = tree {
        merge_node_states(tree.root(), overlays, &mut states, &mut differences);
        // Overlay paths are normally leaves from this tree. Keep any that are
        // temporarily absent from a refreshed tree so callers retain the old
        // behavior until the next comparison tree is installed.
        for (path, state) in overlays {
            states.entry(path.clone()).or_insert_with(|| state.clone());
        }
    } else {
        states.extend(
            overlays
                .iter()
                .map(|(path, state)| (path.clone(), state.clone())),
        );
    }
    MergedFolderCache {
        tree_key,
        states: Arc::new(states),
        difference_paths: Arc::new(differences),
    }
}

/// Aggregates from direct children rather than repeatedly scanning the complete
/// state map for every directory. This is linear in the tree size after the
/// underlying ordered-map inserts, and lets overlays restore a previously
/// changed parent to `Same` when all of its visible children are clean.
fn merge_node_states(
    node: &TreeNode,
    overlays: &BTreeMap<PathBuf, DirectoryEntryState>,
    states: &mut BTreeMap<PathBuf, DirectoryEntryState>,
    differences: &mut Vec<PathBuf>,
) -> DirectoryEntryState {
    let mut child_is_different = false;
    for child in &node.children {
        let state = merge_node_states(child, overlays, states, differences);
        child_is_different |= state != DirectoryEntryState::Same;
    }
    let direct = overlays
        .get(&node.relative_path)
        .cloned()
        .unwrap_or_else(|| node.state.clone());
    let state = if node.is_directory()
        && !matches!(
            direct,
            DirectoryEntryState::Error(_)
                | DirectoryEntryState::TypeMismatch
                | DirectoryEntryState::LeftOnly
                | DirectoryEntryState::RightOnly
        ) {
        if child_is_different {
            DirectoryEntryState::Different
        } else {
            DirectoryEntryState::Same
        }
    } else {
        direct
    };
    if !node.relative_path.as_os_str().is_empty() {
        if (node.left.kind == Some(DirectoryEntryKind::File)
            || node.right.kind == Some(DirectoryEntryKind::File))
            && state != DirectoryEntryState::Same
        {
            differences.push(node.relative_path.clone());
        }
        states.insert(node.relative_path.clone(), state.clone());
    }
    state
}
