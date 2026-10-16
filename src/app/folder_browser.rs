//! Metadata-only folder navigation and debounced file/folder find and replace.
mod search_state;
#[cfg(test)]
mod tests;

use super::*;
#[cfg(test)]
use search_state::SearchJob;
use search_state::{SearchState, scope_includes};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    hash::{Hash, Hasher},
};
use versus::{
    DirectoryEntryState, FileViewOptions, LazyDirectoryItem, SearchPreview, SearchScope,
    WorkingFile, compare_loaded_text, load_working_file,
};

struct ListingJob {
    receiver: Receiver<Result<Vec<LazyDirectoryItem>, String>>,
}
struct OverlayJob {
    receiver: Receiver<(u64, Result<BTreeMap<PathBuf, DirectoryEntryState>, String>)>,
    cancel: Arc<AtomicBool>,
    /// Input signatures covered by this worker.  They are only committed to
    /// the cache once the worker completes for the current generation.
    signatures: BTreeMap<PathBuf, u64>,
}

/// The merged directory state is independent of the visible rows.  Keeping it
/// behind an `Arc` means both folder panes and difference navigation can reuse
/// one aggregation without cloning every path on each frame.
struct MergedFolderCache {
    tree_key: Option<usize>,
    states: Arc<BTreeMap<PathBuf, DirectoryEntryState>>,
    difference_paths: Arc<Vec<PathBuf>>,
}
impl Drop for OverlayJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

enum SaveEvent {
    Saved {
        path: PathBuf,
        saved_text: String,
        result: Result<WorkingFile, String>,
    },
    Complete,
}

struct SaveJob {
    receiver: Receiver<SaveEvent>,
    total: usize,
    completed: usize,
}

pub(super) struct FolderBrowser {
    pub(super) sources: [Option<PathBuf>; 2],
    listings: [BTreeMap<PathBuf, Vec<LazyDirectoryItem>>; 2],
    jobs: [BTreeMap<PathBuf, ListingJob>; 2],
    overlays: BTreeMap<PathBuf, DirectoryEntryState>,
    merged_cache: Option<MergedFolderCache>,
    /// Every path that has had a working-copy overlay. Retaining it lets a
    /// save restore the disk-backed folder state.
    overlay_paths: BTreeSet<PathBuf>,
    overlay_cache: BTreeMap<PathBuf, u64>,
    overlay_token: Option<(u64, [bool; 3])>,
    overlay_not_before: Option<Instant>,
    overlay_job: Option<OverlayJob>,
    retired_overlay_jobs: Vec<OverlayJob>,
    overlay_generation: u64,
    expanded: [BTreeSet<PathBuf>; 2],
    pending_reveal: [Option<PathBuf>; 2],
    source_generation: [u64; 2],
    selected: [Option<PathBuf>; 2],
    error: Option<String>,
    search_state: SearchState,
    working: BTreeMap<PathBuf, WorkingFile>,
    working_revision: Option<u64>,
    save: Option<SaveJob>,
}

impl FolderBrowser {
    pub(super) fn new(roots: [PathBuf; 2]) -> Self {
        Self::new_with_sources(roots.map(Some))
    }
    pub(super) fn new_with_sources(sources: [Option<PathBuf>; 2]) -> Self {
        let mut view = Self::new_search(sources);
        for side in 0..2 {
            view.load_side(side, PathBuf::new());
        }
        view
    }
    pub(super) fn new_search(sources: [Option<PathBuf>; 2]) -> Self {
        Self {
            sources: sources.clone(),
            listings: std::array::from_fn(|_| BTreeMap::new()),
            jobs: std::array::from_fn(|_| BTreeMap::new()),
            overlays: BTreeMap::new(),
            merged_cache: None,
            overlay_paths: BTreeSet::new(),
            overlay_cache: BTreeMap::new(),
            overlay_token: None,
            overlay_not_before: None,
            overlay_job: None,
            retired_overlay_jobs: Vec::new(),
            overlay_generation: 0,
            expanded: std::array::from_fn(|_| BTreeSet::new()),
            pending_reveal: std::array::from_fn(|_| None),
            source_generation: [0, 0],
            selected: [None, None],
            error: None,
            search_state: SearchState::new(&sources),
            working: BTreeMap::new(),
            working_revision: None,
            save: None,
        }
    }
    pub(super) fn set_sources(&mut self, sources: [Option<PathBuf>; 2]) {
        self.set_sources_internal(sources, true);
    }
    /// Drops the cached projection of a comparison tree and working-copy
    /// overlays. The owner calls this when it installs or clears the base tree.
    pub(super) fn invalidate_comparison_cache(&mut self) {
        self.merged_cache = None;
    }
    pub(super) fn set_search_sources(&mut self, sources: [Option<PathBuf>; 2]) {
        self.set_sources_internal(sources, false);
    }
    fn set_sources_internal(&mut self, sources: [Option<PathBuf>; 2], enumerate: bool) {
        let changed = self.sources != sources;
        if !changed {
            return;
        }
        self.search_state.reset_for_sources(&sources);
        for side in 0..2 {
            if self.sources[side] != sources[side] {
                self.sources[side] = sources[side].clone();
                self.listings[side].clear();
                self.jobs[side].clear();
                self.expanded[side].clear();
                self.pending_reveal[side] = None;
                self.source_generation[side] = self.source_generation[side].wrapping_add(1);
                self.selected[side] = None;
                if enumerate && self.sources[side].is_some() {
                    self.load_side(side, PathBuf::new());
                }
            }
        }
        // Results are rooted in the old workspace. Drop their receivers so
        // cancellation reaches workers and stale paths cannot be applied.
        self.overlay_job = None;
        self.retired_overlay_jobs.clear();
        self.overlay_generation = self.overlay_generation.wrapping_add(1);
        self.overlay_token = None;
        self.overlay_not_before = None;
        self.overlays.clear();
        self.invalidate_comparison_cache();
        self.overlay_paths.clear();
        self.overlay_cache.clear();
        self.search_state.clear_results();
    }
    pub(super) fn source_roots(&self) -> [Option<PathBuf>; 2] {
        self.sources.clone()
    }
    pub(super) fn open_search(&mut self) {
        if !self.search_state.is_open() {
            self.error = None;
        }
        self.search_state.open();
    }
    pub(super) fn sync_working_files(&mut self, view: &InteractiveView) {
        // Editing is owned by the retained documents. Refresh their snapshots
        // when saving/undo makes them clean, rather than copying every buffer
        // on each keystroke while the dialog is closed.
        if view.any_dirty() {
            return;
        }
        let revision = view.folder_revision_signature();
        if self.working_revision != Some(revision) {
            self.working = view.export_working_files();
            self.working_revision = Some(revision);
        }
    }
    /// Refreshes only metadata for already expanded directories. Working files,
    /// selection, expansion, and scroll identities remain intact.
    pub(super) fn refresh_listing(&mut self) {
        for side in 0..2 {
            self.listings[side].clear();
            self.jobs[side].clear();
            self.load_side(side, PathBuf::new());
            let expanded: Vec<_> = self.expanded[side].iter().cloned().collect();
            for relative in expanded {
                self.load_side(side, relative);
            }
        }
    }
    pub(super) fn clear_error(&mut self) {
        self.error = None;
    }
    pub(super) fn error_details(&self) -> Vec<String> {
        self.error.iter().cloned().collect()
    }
    fn paired_roots(&self) -> Option<[PathBuf; 2]> {
        Some([self.sources[0].clone()?, self.sources[1].clone()?])
    }
    pub(super) fn any_dirty(&self) -> bool {
        self.working.values().any(WorkingFile::is_dirty)
    }
    pub(super) fn is_saving(&self) -> bool {
        self.save.is_some()
    }
    /// Reveals one relative path by loading only its ancestor directories.
    /// No file contents or unrelated tree levels are read.
    pub(super) fn reveal_difference(&mut self, relative: PathBuf) {
        self.pending_reveal = [Some(relative.clone()), Some(relative.clone())];
        self.selected = [Some(relative.clone()), Some(relative.clone())];
        let mut ancestors = Vec::new();
        let mut cursor = relative.parent();
        while let Some(parent) = cursor {
            if parent.as_os_str().is_empty() {
                break;
            }
            ancestors.push(parent.to_path_buf());
            cursor = parent.parent();
        }
        for ancestor in ancestors.into_iter().rev() {
            for side in 0..2 {
                self.expanded[side].insert(ancestor.clone());
                self.load_side(side, ancestor.clone());
            }
        }
    }
    fn load_side(&mut self, side: usize, relative: PathBuf) {
        if self.sources[side].is_none()
            || self.jobs[side].contains_key(&relative)
            || self.listings[side].contains_key(&relative)
        {
            return;
        }
        let root = self.sources[side].clone().expect("checked root");
        let rel = relative.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = versus::directory_entries_for_root(&root, &rel).map_err(|e| e.to_string());
            let _ = sender.send(result);
        });
        self.jobs[side].insert(relative, ListingJob { receiver });
    }
    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        for side in 0..2 {
            let ready: Vec<_> = self.jobs[side]
                .iter()
                .filter_map(|(path, job)| match job.receiver.try_recv() {
                    Ok(result) => Some((path.clone(), result)),
                    Err(TryRecvError::Disconnected) => {
                        Some((path.clone(), Err("Directory worker stopped".into())))
                    }
                    _ => None,
                })
                .collect();
            for (path, result) in ready {
                self.jobs[side].remove(&path);
                match result {
                    Ok(entries) => {
                        self.listings[side].insert(path, entries);
                    }
                    Err(e) => self.error = Some(e),
                }
            }
        }
        // Search-state owns bounded streaming and cancellation. Navigation
        // only receives an error suitable for the shared activity area.
        if let Some(error) = self.search_state.poll() {
            self.error = Some(error);
        }
        if self.jobs.iter().any(|jobs| !jobs.is_empty())
            || self.search_state.is_searching()
            || self.save.is_some()
            || self.overlay_job.is_some()
            || !self.retired_overlay_jobs.is_empty()
            || self.overlay_not_before.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(40));
        }
    }
    pub(super) fn poll_comparisons(
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
    fn update_overlays(&mut self, view: &InteractiveView, ctx: &egui::Context) {
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
    fn reap_retired_overlays(&mut self) {
        self.retired_overlay_jobs
            .retain_mut(|job| matches!(job.receiver.try_recv(), Err(TryRecvError::Empty)));
    }
    pub(super) fn difference_paths(
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
    pub(super) fn render_controls(&mut self, ui: &mut egui::Ui, view: &mut InteractiveView) {
        self.poll(ui.ctx());
        self.poll_save(view, ui.ctx());
        self.update_overlays(view, ui.ctx());
        self.search_window(ui.ctx(), view);
    }
    pub(super) fn render_search_controls(&mut self, ui: &mut egui::Ui, view: &mut InteractiveView) {
        self.poll(ui.ctx());
        self.poll_save(view, ui.ctx());
        self.sync_working_files(view);
        self.search_window(ui.ctx(), view);
    }
    /// Draws one status in the bottom-right status area. Controls intentionally do
    /// not duplicate this text so narrow folder views retain usable actions.
    pub(super) fn render_activity_status(&self, ui: &mut egui::Ui) -> bool {
        let palette = Palette::for_context(ui.ctx());
        if let Some(save) = &self.save {
            let percent = if save.total == 0 {
                100
            } else {
                save.completed.min(save.total) * 100 / save.total
            };
            header_status_label(ui, &format!("Saving files… {percent}%"), palette.text);
        } else if self.search_state.is_searching() {
            header_status_label(ui, "Searching folders…", palette.muted);
        } else if self.jobs.iter().any(|jobs| !jobs.is_empty()) {
            header_status_label(ui, "Loading directory metadata…", palette.muted);
        } else if self.overlay_job.is_some() || !self.retired_overlay_jobs.is_empty() {
            header_status_label(ui, "Comparing changed working copies…", palette.muted);
        } else {
            return false;
        }
        true
    }
    /// Renders one independently navigable source tree and returns a file
    /// opened from that side. Missing sources intentionally render nothing.
    pub(super) fn render_side(
        &mut self,
        ui: &mut egui::Ui,
        view: &mut InteractiveView,
        compared: Option<&FolderTree>,
        side: usize,
    ) -> Option<PathBuf> {
        let root = self.sources[side].clone()?;
        let palette = Palette::for_context(ui.ctx());
        let states = if view.run_diff_enabled() && self.paired_roots().is_some() {
            self.merged_folder_cache(compared).states.clone()
        } else {
            Arc::new(BTreeMap::new())
        };
        let mut rows = Vec::new();
        self.rows_for_side(side, std::path::Path::new(""), 0, &mut rows);
        if view.run_diff_enabled() && view.merge_options()[0] && self.paired_roots().is_some() {
            rows.retain(|(entry, _)| {
                states.get(&entry.relative_path) != Some(&DirectoryEntryState::Same)
            });
        }
        let reveal_index = self.pending_reveal[side].as_ref().and_then(|target| {
            rows.iter()
                .position(|(entry, _)| &entry.relative_path == target)
        });
        if let Some(index) = reveal_index {
            self.selected[side] = Some(rows[index].0.relative_path.clone());
            self.pending_reveal[side] = None;
        }
        let available = ui.available_height().max(80.0);
        let row_height = comparison_row_height(ui);
        let mut opened = None;
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("merge-folder-tree", side, self.source_generation[side]))
            .max_height(available - 5.0);
        if let Some(index) = reveal_index {
            scroll = scroll.vertical_scroll_offset(index.saturating_sub(1) as f32 * row_height);
        }
        scroll.show_rows(ui, row_height, rows.len(), |ui, range| {
            for index in range {
                let (entry, depth) = &rows[index];
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), row_height),
                    Sense::click(),
                );
                let directory = entry.side.kind == DirectoryEntryKind::Directory;
                let file = entry.side.kind == DirectoryEntryKind::File;
                let response = ui.interact(
                    rect,
                    ui.id().with(("merge-entry", side, &entry.relative_path)),
                    Sense::click(),
                );
                let response = if directory || file {
                    response.on_hover_cursor(egui::CursorIcon::PointingHand)
                } else {
                    response
                };
                let state = states
                    .get(&entry.relative_path)
                    .cloned()
                    .unwrap_or(DirectoryEntryState::Same);
                let selected = self.selected[side].as_ref() == Some(&entry.relative_path);
                let fill = if selected {
                    palette.accent.gamma_multiply(0.16)
                } else if response.hovered() {
                    palette.hover
                } else if index % 2 == 1 {
                    palette.alternate
                } else {
                    palette.panel
                };
                let painter = ui.painter().with_clip_rect(rect);
                painter.rect_filled(rect, 0, fill);
                let color = state_color(&state, palette);
                if state != DirectoryEntryState::Same {
                    painter.rect_filled(
                        Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
                        0,
                        state_bar_color(&state, palette, painter.ctx()),
                    );
                }
                let x = rect.left() + 8.0 + *depth as f32 * 14.0;
                let y = rect.center().y;
                let name_right = rect.right() - 98.0;
                let name_painter = painter.with_clip_rect(Rect::from_min_max(
                    rect.min,
                    egui::pos2(name_right.max(rect.left()), rect.bottom()),
                ));
                if directory {
                    name_painter.text(
                        egui::pos2(x, y),
                        Align2::LEFT_CENTER,
                        if self.expanded[side].contains(&entry.relative_path) {
                            "▾"
                        } else {
                            "▸"
                        },
                        FontId::monospace(15.0),
                        palette.muted,
                    );
                }
                paint_icon(
                    &name_painter,
                    egui::pos2(x + 19.0, y),
                    Some(&entry.side.kind),
                    color,
                );
                name_painter.text(
                    egui::pos2(x + 40.0, y),
                    Align2::LEFT_CENTER,
                    entry
                        .relative_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                    comparison_font(ui.ctx()),
                    color,
                );
                painter.text(
                    egui::pos2(rect.right() - 32.0, y),
                    Align2::RIGHT_CENTER,
                    format_size(entry.side.size),
                    FontId::monospace(10.0),
                    palette.muted,
                );
                if let Some(icon) = status_icon(&state) {
                    paint_status_icon(&painter, egui::pos2(rect.right() - 15.0, y), icon);
                }
                if response.clicked() {
                    if file {
                        self.selected[side] = Some(entry.relative_path.clone());
                    } else if directory {
                        if !self.expanded[side].insert(entry.relative_path.clone()) {
                            self.expanded[side].remove(&entry.relative_path);
                        } else {
                            self.load_side(side, entry.relative_path.clone());
                        }
                    }
                }
                if response.double_clicked() && file {
                    opened = Some(root.join(&entry.relative_path));
                }
            }
        });
        opened
    }
    fn rows_for_side(
        &self,
        side: usize,
        parent: &std::path::Path,
        depth: usize,
        output: &mut Vec<(LazyDirectoryItem, usize)>,
    ) {
        if let Some(entries) = self.listings[side].get(parent) {
            for entry in entries {
                output.push((entry.clone(), depth));
                if self.expanded[side].contains(&entry.relative_path) {
                    self.rows_for_side(side, &entry.relative_path, depth + 1, output);
                }
            }
        }
    }
    pub(super) fn render(
        &mut self,
        ui: &mut egui::Ui,
        view: &mut InteractiveView,
        compared: Option<&FolderTree>,
    ) -> Option<[PathBuf; 2]> {
        self.render_controls(ui, view);
        let mut opened = [None, None];
        ui.columns(2, |columns| {
            for side in 0..2 {
                opened[side] = self.render_side(&mut columns[side], view, compared, side);
            }
        });
        match opened {
            [Some(left), Some(right)] => Some([left, right]),
            [Some(left), None] => self.paired_roots().map(|roots| {
                let relative = left.strip_prefix(&roots[0]).unwrap_or(&left).to_path_buf();
                [left, roots[1].join(relative)]
            }),
            [None, Some(right)] => self.paired_roots().map(|roots| {
                let relative = right
                    .strip_prefix(&roots[1])
                    .unwrap_or(&right)
                    .to_path_buf();
                [roots[0].join(relative), right]
            }),
            _ => None,
        }
    }
    pub(super) fn begin_save(&mut self, view: &InteractiveView) {
        if self.save.is_some() || view.is_saving() {
            return;
        }
        // Snapshot only the copies that are dirty at the explicit save action.
        // The worker owns these values, so folder navigation and editing stay
        // responsive while disk writes and external-change checks run.
        self.working = view.export_working_files();
        // A document can be reachable through several path spellings.  Save
        // its physical identity once, using its original native save path.
        let files = pending_save_files(&self.working);
        if files.is_empty() {
            return;
        }
        let total = files.len();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            for (path, mut file, saved_text) in files {
                let result = versus::save_working_file(&mut file)
                    .map(|()| file)
                    .map_err(|error| error.to_string());
                if sender
                    .send(SaveEvent::Saved {
                        path,
                        saved_text,
                        result,
                    })
                    .is_err()
                {
                    return;
                }
            }
            let _ = sender.send(SaveEvent::Complete);
        });
        self.save = Some(SaveJob {
            receiver,
            total,
            completed: 0,
        });
        self.error = None;
    }
    pub(super) fn poll_save(&mut self, view: &mut InteractiveView, ctx: &egui::Context) {
        let mut events = Vec::new();
        let mut disconnected = false;
        let mut complete = false;
        if let Some(job) = &self.save {
            loop {
                match job.receiver.try_recv() {
                    Ok(SaveEvent::Saved {
                        path,
                        saved_text,
                        result,
                    }) => events.push((path, saved_text, result)),
                    Ok(SaveEvent::Complete) => {
                        complete = true;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if events.is_empty() && !complete && !disconnected {
            return;
        }

        // Fetch the newest editor state before applying save baselines. If the
        // user typed after the snapshot, keep that text and only advance its
        // fingerprint to the bytes that were actually written.
        self.working = view.export_working_files();
        let mut saved_files = BTreeMap::new();
        for (path, saved_text, result) in events {
            if let Some(job) = &mut self.save {
                job.completed += 1;
            }
            match result {
                Ok(saved) => {
                    saved_files.insert(path.clone(), saved.clone());
                    retain_saved_snapshot(&mut self.working, path, saved_text, saved);
                }
                Err(error) => self.error = Some(error),
            }
        }
        view.update_saved_files(&saved_files);
        if complete || disconnected {
            self.save = None;
            if disconnected {
                self.error = Some("Save worker stopped before all files were written.".into());
            }
        }
        if self.save.is_some() {
            ctx.request_repaint_after(Duration::from_millis(25));
        }
    }
    fn search_window(&mut self, ctx: &egui::Context, view: &mut InteractiveView) {
        if !self.search_state.is_open() {
            return;
        }
        if self.search_state.focus_pending() {
            self.working = view.export_working_files();
        }
        if !self.search_state.is_waiting()
            && !self.search_state.is_searching()
            && self.search_state.results().is_none()
            && !self.search_state.query().pattern.is_empty()
        {
            self.schedule_search(ctx.input(|input| input.time));
        }
        let mut close = false;
        let width = (ctx.content_rect().width() - 48.0).clamp(200.0, 576.0);
        let max_height = (ctx.content_rect().height() - 96.0).clamp(120.0, 360.0);
        let response = egui::Modal::new(egui::Id::new("folder-find-replace-modal"))
            .frame(
                egui::Frame::popup(&ctx.global_style())
                    .inner_margin(12)
                    .corner_radius(6),
            )
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Find and Replace").size(14.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close =
                            icon_button(ui, ToolbarIcon::Cancel, true, "Close Find and Replace")
                                .clicked();
                    });
                });
                ui.add_space(6.0);
                ui.columns(2, |columns| {
                    egui::ScrollArea::vertical()
                        .id_salt("folder-find-replace-content")
                        .max_height(max_height)
                        .auto_shrink([false, true])
                        .show(&mut columns[0], |ui| {
                            let editing_field = ui.memory(|memory| {
                                matches!(memory.focused(), Some(id)
                    if id == egui::Id::new("folder-find-pattern")
                        || id == egui::Id::new("folder-replacement-text"))
                            });
                            if !editing_field {
                                if ui.input_mut(|input| {
                                    input.consume_key(
                                        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                                        egui::Key::Z,
                                    ) || input.consume_key(egui::Modifiers::CTRL, egui::Key::Y)
                                }) {
                                    self.undo_batch(view, true);
                                } else if ui.input_mut(|input| {
                                    input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)
                                        || input.consume_key(egui::Modifiers::CTRL, egui::Key::Z)
                                }) {
                                    self.undo_batch(view, false);
                                }
                            }
                            let previous_query = self.search_state.query().clone();
                            let request_find_focus = self.search_state.take_focus();
                            let query = self.search_state.query_mut();
                            let find = search_text_field(
                                ui,
                                &mut query.pattern,
                                "folder-find-pattern",
                                "Find",
                                Some([
                                    &mut query.case_sensitive,
                                    &mut query.whole_word,
                                    &mut query.regex,
                                ]),
                            );
                            if request_find_focus {
                                find.request_focus();
                            }
                            let extent = control_extent(ui);
                            let gap = ui.spacing().item_spacing.x;
                            let (row, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), SEARCH_FIELD_HEIGHT),
                                Sense::hover(),
                            );
                            let field = Rect::from_min_max(
                                row.min,
                                egui::pos2(row.right() - 2.0 * (extent + gap), row.bottom()),
                            );
                            let mut replacement_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(field)
                                    .layout(egui::Layout::top_down(egui::Align::Min)),
                            );
                            search_text_field(
                                &mut replacement_ui,
                                self.search_state.replacement_mut(),
                                "folder-replacement-text",
                                "Replace",
                                None,
                            );
                            let enabled =
                                self.search_state.replacement_allowed() && !view.is_saving();
                            for (index, icon, label, next) in [
                                (0, ToolbarIcon::ReplaceNext, "Replace Next", true),
                                (1, ToolbarIcon::ReplaceAll, "Replace All", false),
                            ] {
                                let button = Rect::from_center_size(
                                    egui::pos2(
                                        field.right()
                                            + gap
                                            + extent / 2.0
                                            + index as f32 * (extent + gap),
                                        row.center().y,
                                    ),
                                    egui::vec2(extent, extent),
                                );
                                let mut button_ui = ui.new_child(
                                    egui::UiBuilder::new()
                                        .max_rect(button)
                                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                                );
                                if icon_button(&mut button_ui, icon, enabled, label).clicked() {
                                    self.replace_matches(view, next);
                                }
                            }
                            ui.add_space(6.0);
                            let mut refresh = false;
                            let mut scope_clicked = false;
                            let scope_width: f32 = ["LEFT", "RIGHT", "BOTH"]
                                .into_iter()
                                .map(|label| {
                                    ui.painter()
                                        .layout_no_wrap(
                                            label.into(),
                                            FontId::monospace(11.0),
                                            Palette::for_context(ui.ctx()).text,
                                        )
                                        .size()
                                        .x
                                        + 16.0
                                })
                                .sum();
                            let fits =
                                scope_width + 3.0 * extent + 5.0 * 3.0 <= ui.available_width();
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 3.0;
                                let palette = Palette::for_context(ui.ctx());
                                for (scope, label, color) in [
                                    (SearchScope::Left, "LEFT", palette.left_only),
                                    (SearchScope::Right, "RIGHT", palette.right_only),
                                    (SearchScope::Both, "BOTH", palette.text),
                                ] {
                                    if search_scope_button(
                                        ui,
                                        label,
                                        color,
                                        self.search_state.scope() == scope,
                                    )
                                    .clicked()
                                    {
                                        scope_clicked = true;
                                        self.search_state.set_scope(scope);
                                    }
                                }
                                if fits {
                                    refresh = self.search_action_buttons(ui, view);
                                }
                            });
                            if !fits {
                                ui.horizontal(|ui| {
                                    refresh = self.search_action_buttons(ui, view);
                                });
                            }
                            let options_changed = self.search_state.query().case_sensitive
                                != previous_query.case_sensitive
                                || self.search_state.query().whole_word
                                    != previous_query.whole_word
                                || self.search_state.query().regex != previous_query.regex;
                            if refresh
                                || options_changed
                                || (scope_clicked
                                    && (self.search_state.is_waiting()
                                        || self.search_state.query().pattern
                                            != previous_query.pattern
                                        || (!self.search_state.is_searching()
                                            && !self.search_state.results_current())))
                            {
                                self.start_search(view);
                            } else if self.search_state.query().pattern != previous_query.pattern {
                                self.schedule_search(ctx.input(|input| input.time));
                            }
                            ui.horizontal(|ui| {
                                if self.search_state.is_searching() {
                                    ui.spinner();
                                    let status = format!(
                                        "Searching… {} matches",
                                        self.search_state.selected_count()
                                    );
                                    ui.add_sized(
                                        egui::vec2(
                                            (ui.available_width()
                                                - extent
                                                - ui.spacing().item_spacing.x)
                                                .max(1.0),
                                            extent,
                                        ),
                                        egui::Label::new(&status).truncate(),
                                    )
                                    .on_hover_text(status);
                                    if icon_button(ui, ToolbarIcon::Cancel, true, "Cancel search")
                                        .clicked()
                                    {
                                        self.search_state.cancel();
                                    }
                                } else if self.search_state.is_waiting() {
                                    ui.label("Waiting for typing…");
                                } else if let Some(results) = self.search_state.results() {
                                    ui.label(if results.cancelled {
                                        "Search stopped".into()
                                    } else {
                                        format!(
                                            "Found {} matches",
                                            self.search_state.selected_count()
                                        )
                                    });
                                }
                            });
                            if let Some(error) = &self.error {
                                ui.colored_label(Palette::for_context(ctx).error, error);
                            }
                        });
                    let right = &mut columns[1];
                    let counts = self.search_match_counts();
                    right.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        let palette = Palette::for_context(ui.ctx());
                        let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
                        for (side, label) in [(0, "LEFT"), (1, "RIGHT")] {
                            let enabled = scope_includes(self.search_state.scope(), side);
                            if ui
                                .add_enabled_ui(enabled, |ui| {
                                    search_choice_button(
                                        ui,
                                        &format!("{label} ({})", counts[side]),
                                        palette.text,
                                        self.search_state.results_side() == side,
                                        egui::Id::new(("folder-search-view", side)),
                                        if side == 0 {
                                            "Show left results"
                                        } else {
                                            "Show right results"
                                        },
                                        Some(width),
                                    )
                                })
                                .inner
                                .clicked()
                            {
                                self.search_state.set_results_side(side);
                            }
                        }
                    });
                    right.add_space(3.0);
                    let results_height = (max_height - control_extent(right) - 6.0).max(60.0);
                    self.search_results_panel(right, results_height);
                });
            });
        if close || response.should_close() {
            self.search_state.close();
        } else {
            self.run_scheduled_search(ctx, view, ctx.input(|input| input.time));
        }
    }
    fn search_action_buttons(&mut self, ui: &mut egui::Ui, view: &mut InteractiveView) -> bool {
        if icon_button(
            ui,
            ToolbarIcon::Undo,
            view.can_undo_batch(),
            "Undo replacement batch",
        )
        .clicked()
        {
            self.undo_batch(view, false);
        }
        if icon_button(
            ui,
            ToolbarIcon::Redo,
            view.can_redo_batch(),
            "Redo replacement batch",
        )
        .clicked()
        {
            self.undo_batch(view, true);
        }
        return icon_button(
            ui,
            ToolbarIcon::Refresh,
            !self.search_state.query().pattern.is_empty() && !view.is_saving() && !self.is_saving(),
            "Refresh search",
        )
        .clicked();
    }

    fn search_match_counts(&self) -> [usize; 2] {
        self.search_state.match_counts()
    }

    fn search_results_panel(&mut self, ui: &mut egui::Ui, height: f32) {
        let palette = Palette::for_context(ui.ctx());
        let mut removed = Vec::new();
        egui::Frame::new()
            .fill(palette.panel)
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(6)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_height((height - 18.0).max(0.0));
                ui.interact(
                    ui.max_rect(),
                    egui::Id::new("folder-search-results"),
                    Sense::hover(),
                );
                if let Some(results) = self.search_state.results() {
                    enum Row<'a> {
                        File(&'a versus::SearchFileResults),
                        Hit(&'a versus::SearchFileResults, &'a versus::SearchHit),
                    }
                    let mut rows = Vec::new();
                    for file in results
                        .files
                        .iter()
                        .filter(|file| file.side == self.search_state.results_side())
                    {
                        let hits: Vec<_> = file
                            .hits
                            .iter()
                            .filter(|hit| self.search_state.selected().contains(&hit.id))
                            .collect();
                        if hits.is_empty() {
                            continue;
                        }
                        rows.push(Row::File(file));
                        rows.extend(hits.into_iter().map(|hit| Row::Hit(file, hit)));
                    }
                    if !rows.is_empty() {
                        let row_height = control_extent(ui);
                        egui::ScrollArea::vertical()
                            .id_salt((
                                "folder-search-results-list",
                                self.search_state.results_side(),
                            ))
                            .max_height((height - 18.0).max(30.0))
                            .auto_shrink([false, false])
                            .show_rows(ui, row_height, rows.len(), |ui, range| {
                                for index in range {
                                    match rows[index] {
                                        Row::File(file) => {
                                            let path = self.sources[file.side]
                                                .as_deref()
                                                .and_then(|root| file.path.strip_prefix(root).ok())
                                                .filter(|relative| !relative.as_os_str().is_empty())
                                                .unwrap_or(&file.path);
                                            let name = if path == file.path
                                                && self.sources[file.side].as_ref()
                                                    == Some(&file.path)
                                            {
                                                file.path
                                                    .file_name()
                                                    .unwrap_or(file.path.as_os_str())
                                                    .to_string_lossy()
                                            } else {
                                                path.as_os_str().to_string_lossy()
                                            };
                                            ui.add_sized(
                                                egui::vec2(ui.available_width(), row_height),
                                                egui::Label::new(RichText::new(name).strong())
                                                    .truncate()
                                                    .halign(egui::Align::Min),
                                            )
                                            .on_hover_text(file.path.display().to_string());
                                        }
                                        Row::Hit(file, hit) => {
                                            if search_result_rows::result_row(
                                                ui,
                                                hit,
                                                &file.working_file.text,
                                            ) {
                                                removed.push(hit.id);
                                            }
                                        }
                                    }
                                }
                            });
                        return;
                    }
                }
                let rect = ui.available_rect_before_wrap();
                let mut job = egui::text::LayoutJob::simple(
                    if self.search_state.is_searching() || self.search_state.results().is_none() {
                        "Search results will appear here"
                    } else {
                        "No results on this side"
                    }
                    .into(),
                    FontId::proportional(14.0),
                    palette.muted,
                    rect.width(),
                );
                job.halign = egui::Align::Center;
                let galley = ui.painter().layout_job(job);
                ui.painter().with_clip_rect(rect).galley(
                    rect.center() - galley.rect.center().to_vec2(),
                    galley,
                    palette.muted,
                );
            });
        for id in removed {
            self.search_state.remove_selected(id);
        }
    }

    fn schedule_search(&mut self, now: f64) {
        self.error = None;
        self.search_state.schedule(now);
    }
    fn run_scheduled_search(&mut self, ctx: &egui::Context, view: &InteractiveView, now: f64) {
        if let Some(wait) =
            self.search_state
                .run_due(now, self.sources.clone(), view.export_working_files())
        {
            ctx.request_repaint_after(Duration::from_secs_f64(wait));
        }
    }
    fn results_current(&self) -> bool {
        self.search_state.results_current()
    }
    fn start_search(&mut self, view: &InteractiveView) {
        self.error = None;
        self.working = view.export_working_files();
        self.search_state
            .start(self.sources.clone(), self.working.clone());
    }
    fn replacement_plan(&mut self, view: &InteractiveView, next: bool) -> Option<SearchPreview> {
        if !self.results_current() {
            return None;
        }
        self.working = view.export_working_files();
        let selected = self
            .search_state
            .results()
            .unwrap()
            .files
            .iter()
            .filter(|file| scope_includes(self.search_state.scope(), file.side))
            .flat_map(|file| &file.hits)
            .filter(|hit| self.search_state.selected().contains(&hit.id))
            .map(|hit| hit.id)
            .collect();
        match search_session::preview_replacements(
            self.search_state.results().unwrap(),
            &selected,
            self.search_state.query(),
            self.search_state.replacement(),
            &self.working,
            next,
        ) {
            Ok(plan) => Some(plan),
            Err(error) => {
                self.error = Some(error.to_string());
                None
            }
        }
    }
    fn replace_matches(&mut self, view: &mut InteractiveView, next: bool) {
        let Some(plan) = self.replacement_plan(view, next) else {
            return;
        };
        if plan.changes.is_empty() {
            return;
        }
        match view.apply_search_preview(&plan) {
            Ok(_) => {
                self.working = view.export_working_files();
                if let Err(error) = self
                    .search_state
                    .advance_after_replacement(&plan, &self.working)
                {
                    self.search_state.clear_results();
                    self.error = Some(error);
                }
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn undo_batch(&mut self, view: &mut InteractiveView, redo: bool) {
        match view.undo_batch(redo) {
            Ok(()) => {
                self.working = view.export_working_files();
                self.search_state.invalidate_after_undo();
                self.start_search(view);
            }
            Err(error) => self.error = Some(error),
        }
    }
}

const SEARCH_ENTRY_HEIGHT: f32 = 34.0;
// Four-point padding plus a one-point border on each side.
const SEARCH_FIELD_HEIGHT: f32 = SEARCH_ENTRY_HEIGHT + 10.0;

fn search_text_field(
    ui: &mut egui::Ui,
    text: &mut String,
    id: &str,
    placeholder: &str,
    options: Option<[&mut bool; 3]>,
) -> egui::Response {
    let palette = Palette::for_context(ui.ctx());
    let focused = ui.memory(|memory| memory.has_focus(egui::Id::new(id)));
    let frame = egui::Frame::new()
        .fill(palette.background)
        .corner_radius(3)
        .stroke(Stroke::new(
            1.0,
            if focused {
                palette.accent
            } else {
                palette.border
            },
        ))
        .inner_margin(4);
    let response = frame.show(ui, |ui| {
        let extent = control_extent(ui);
        let gap = ui.spacing().item_spacing.x;
        let reserve = if options.is_some() {
            3.0 * (extent + gap)
        } else {
            0.0
        };
        let (row, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), SEARCH_ENTRY_HEIGHT),
            Sense::hover(),
        );
        let field = Rect::from_min_max(
            row.min,
            egui::pos2((row.right() - reserve).max(row.left() + 20.0), row.bottom()),
        );
        let edit = ui
            .scope_builder(egui::UiBuilder::new().max_rect(field), |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(field));
                // TextEdit uses the scoped weak-text color for its hint atoms.
                ui.visuals_mut().weak_text_color = Some(palette.muted.gamma_multiply(0.8));
                ui.add_sized(
                    field.size(),
                    egui::TextEdit::singleline(text)
                        .id(egui::Id::new(id))
                        .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(4, 8)))
                        .font(FontId::proportional(14.0))
                        .hint_text(
                            RichText::new(placeholder)
                                .font(FontId::proportional(14.0))
                                .italics()
                                .color(palette.muted.gamma_multiply(0.8)),
                        ),
                )
            })
            .inner;
        if let Some(options) = options {
            for (index, (value, (icon, label))) in options
                .into_iter()
                .zip([
                    (ToolbarIcon::MatchCase, "Match Case"),
                    (ToolbarIcon::MatchWholeWord, "Match Whole Word"),
                    (ToolbarIcon::UseRegularExpression, "Use Regular Expression"),
                ])
                .enumerate()
            {
                let button = Rect::from_center_size(
                    egui::pos2(
                        field.right() + gap + extent / 2.0 + index as f32 * (extent + gap),
                        row.center().y,
                    ),
                    egui::vec2(extent, extent),
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(button)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        if icon_button_state(ui, icon, true, *value, label).clicked() {
                            *value = !*value;
                        }
                    },
                );
            }
        }
        edit
    });
    ui.interact(
        response.response.rect,
        egui::Id::new((id, "frame")),
        Sense::hover(),
    );
    response.inner
}

fn search_scope_button(
    ui: &mut egui::Ui,
    label: &str,
    color: Color32,
    selected: bool,
) -> egui::Response {
    search_choice_button(
        ui,
        label,
        color,
        selected,
        egui::Id::new(("folder-search-scope", label)),
        &format!("Replace on {label}"),
        None,
    )
}

fn search_choice_button(
    ui: &mut egui::Ui,
    label: &str,
    color: Color32,
    selected: bool,
    id: egui::Id,
    tooltip: &str,
    width: Option<f32>,
) -> egui::Response {
    let enabled = ui.is_enabled();
    let color = if enabled {
        color
    } else {
        Palette::for_context(ui.ctx()).muted.gamma_multiply(0.4)
    };
    let text = ui
        .painter()
        .layout_no_wrap(label.into(), FontId::monospace(11.0), color);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(width.unwrap_or(text.size().x + 16.0), control_extent(ui)),
        Sense::hover(),
    );
    let response = ui.interact(rect, id, Sense::click());
    let palette = Palette::for_context(ui.ctx());
    if selected {
        ui.painter().rect(
            rect,
            3,
            color.gamma_multiply(0.12),
            Stroke::new(1.0, color),
            egui::StrokeKind::Inside,
        );
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
    ui.painter()
        .galley(rect.center() - text.size() * 0.5, text, color);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, label)
    });
    response
        .on_hover_cursor(if enabled {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::Default
        })
        .on_hover_text(tooltip)
}

fn compare_overlay_files(
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

fn overlay_signature(
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
    fn merged_folder_cache(&mut self, tree: Option<&FolderTree>) -> &MergedFolderCache {
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

fn build_merged_folder_cache(
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

fn pending_save_files(
    working: &BTreeMap<PathBuf, WorkingFile>,
) -> Vec<(PathBuf, WorkingFile, String)> {
    working
        .iter()
        .filter(|(_, file)| file.is_dirty())
        .fold(BTreeMap::new(), |mut unique, (path, file)| {
            unique.entry(file.identity_key()).or_insert_with(|| {
                (
                    file.path.clone().unwrap_or_else(|| path.clone()),
                    file.clone(),
                    file.text.clone(),
                )
            });
            unique
        })
        .into_values()
        .collect()
}

/// Applies a completed save without discarding text changed after the worker
/// snapshot. In that case the completed bytes become the new baseline and the
/// newer text remains dirty for the next explicit save.
fn retain_saved_snapshot(
    working: &mut BTreeMap<PathBuf, WorkingFile>,
    path: PathBuf,
    saved_text: String,
    saved: WorkingFile,
) {
    let identity = saved.identity_key();
    let mut matched = false;
    for current in working
        .values_mut()
        .filter(|file| file.identity_key() == identity)
    {
        matched = true;
        if current.text == saved_text {
            *current = saved.clone();
        } else {
            current.path = saved.path.clone();
            current.identity = saved.identity.clone();
            current.encoding = saved.encoding;
            current.fingerprint = saved.fingerprint.clone();
        }
    }
    if !matched {
        working.insert(path, saved);
    }
}
