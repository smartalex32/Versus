//! Metadata-only folder navigation and debounced file/folder find and replace.
use super::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    hash::{Hash, Hasher},
    ops::Range,
};
use versus::{
    DirectoryEntryState, FileViewOptions, LazyDirectoryItem, SearchEvent, SearchPreview,
    SearchQuery, SearchResults, SearchScope, WorkingFile, compare_loaded_text, load_working_file,
};

struct ListingJob {
    receiver: Receiver<Result<Vec<LazyDirectoryItem>, String>>,
}
struct SearchJob {
    receiver: Receiver<Result<SearchEvent, String>>,
    cancel: Arc<AtomicBool>,
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
impl Drop for SearchJob {
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
    /// Every path that has had a working-copy overlay.  Retaining it lets a
    /// save or ignored-range removal restore the disk-backed folder state.
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
    search_open: bool,
    focus_find: bool,
    query: SearchQuery,
    searched_query: Option<SearchQuery>,
    replacement: String,
    scope: SearchScope,
    results_side: usize,
    search_due_at: Option<f64>,
    search: Option<SearchJob>,
    results: Option<SearchResults>,
    selected_hits: BTreeSet<u64>,
    preview: Option<SearchPreview>,
    working: BTreeMap<PathBuf, WorkingFile>,
    working_revision: Option<u64>,
    pending_save: bool,
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
        let results_side = usize::from(sources[0].is_none() && sources[1].is_some());
        Self {
            sources,
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
            search_open: false,
            focus_find: false,
            query: SearchQuery::plain(""),
            searched_query: None,
            replacement: String::new(),
            scope: SearchScope::Both,
            results_side,
            search_due_at: None,
            search: None,
            results: None,
            selected_hits: BTreeSet::new(),
            preview: None,
            working: BTreeMap::new(),
            working_revision: None,
            pending_save: false,
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
        match self.scope {
            SearchScope::Left => self.results_side = 0,
            SearchScope::Right => self.results_side = 1,
            SearchScope::Both if sources.iter().filter(|source| source.is_some()).count() == 1 => {
                self.results_side = usize::from(sources[1].is_some());
            }
            SearchScope::Both => {}
        }
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
        self.search = None;
        self.search_due_at = None;
        self.results = None;
        self.preview = None;
        self.selected_hits.clear();
        self.searched_query = None;
    }
    pub(super) fn source_roots(&self) -> [Option<PathBuf>; 2] {
        self.sources.clone()
    }
    pub(super) fn open_search(&mut self) {
        self.focus_find = !self.search_open;
        if self.focus_find
            && self
                .results
                .as_ref()
                .is_some_and(|results| results.cancelled)
        {
            self.results = None;
            self.preview = None;
            self.selected_hits.clear();
        }
        if self.focus_find && self.results.is_none() && self.search.is_none() {
            self.searched_query = None;
        }
        self.search_open = true;
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
        // Drain a bounded amount of streamed output per frame. A slow search
        // never holds the UI hostage, and exclusions are not reset at completion.
        let deadline = Instant::now() + Duration::from_millis(4);
        for _ in 0..16 {
            let Some(job) = &self.search else {
                break;
            };
            match job.receiver.try_recv() {
                Ok(Ok(SearchEvent::File(file))) => {
                    self.selected_hits
                        .extend(file.hits.iter().map(|hit| hit.id));
                    self.results
                        .get_or_insert_with(SearchResults::default)
                        .files
                        .push(file);
                }
                Ok(Ok(SearchEvent::Hits { path, side, hits })) => {
                    if let Some(file) = self.results.as_mut().and_then(|results| {
                        results
                            .files
                            .last_mut()
                            .filter(|file| file.path == path && file.side == side)
                    }) {
                        self.selected_hits.extend(hits.iter().map(|hit| hit.id));
                        file.hits.extend(hits);
                    } else {
                        self.error =
                            Some("Search worker sent matches without a file snapshot".into());
                        self.results
                            .get_or_insert_with(SearchResults::default)
                            .cancelled = true;
                        self.search = None;
                    }
                }
                Ok(Ok(SearchEvent::Skipped(path))) => {
                    self.results
                        .get_or_insert_with(SearchResults::default)
                        .skipped_binary
                        .push(path);
                }
                Ok(Ok(SearchEvent::Complete { cancelled })) => {
                    self.results
                        .get_or_insert_with(SearchResults::default)
                        .cancelled = cancelled;
                    self.search = None;
                }
                Ok(Err(error)) => {
                    self.error = Some(error);
                    self.results
                        .get_or_insert_with(SearchResults::default)
                        .cancelled = true;
                    self.search = None;
                }
                Err(TryRecvError::Disconnected) => {
                    self.error = Some("Search worker stopped".into());
                    self.results
                        .get_or_insert_with(SearchResults::default)
                        .cancelled = true;
                    self.search = None;
                }
                Err(TryRecvError::Empty) => break,
            }
            if Instant::now() >= deadline {
                break;
            }
        }
        if self.jobs.iter().any(|jobs| !jobs.is_empty())
            || self.search.is_some()
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
        let ignored = view.folder_ignored_ranges();
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
                    if file.is_dirty()
                        || !ignored
                            .get(&file.identity_key())
                            .cloned()
                            .unwrap_or_default()
                            .is_empty()
                    {
                        modified.insert(relative.to_path_buf());
                    }
                }
            }
        }
        // Include paths that used to be dirty/protected once more.  Their
        // signature changes after save or an ignored-range edit, which puts
        // the original disk comparison back without rereading every folder.
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
                        &ignored,
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
                &ignored,
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
                compare_overlay_files(roots, requested, loaded, ignored, options, &worker_cancel)
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
        } else if self.search.is_some() {
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
            scroll = scroll.vertical_scroll_offset(index as f32 * row_height);
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
                        color,
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
    fn begin_save(&mut self, view: &InteractiveView) {
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
            self.pending_save = false;
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
        self.pending_save = false;
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
        if !self.search_open {
            return;
        }
        if self.focus_find {
            self.working = view.export_working_files();
        }
        if self.search_due_at.is_none()
            && self.search.is_none()
            && self.results.is_none()
            && self.searched_query.is_none()
            && !self.query.pattern.is_empty()
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
                            let previous_query = self.query.clone();
                            let previous_scope = self.scope;
                            let previous_replacement = self.replacement.clone();
                            let find = search_text_field(
                                ui,
                                &mut self.query.pattern,
                                "folder-find-pattern",
                                "Find",
                                Some([
                                    &mut self.query.case_sensitive,
                                    &mut self.query.whole_word,
                                    &mut self.query.regex,
                                ]),
                            );
                            if std::mem::take(&mut self.focus_find) {
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
                                &mut self.replacement,
                                "folder-replacement-text",
                                "Replace",
                                None,
                            );
                            let enabled = self.results_current()
                                && self.results.as_ref().is_some_and(|results| {
                                    results
                                        .files
                                        .iter()
                                        .filter(|file| scope_includes(self.scope, file.side))
                                        .any(|file| {
                                            file.hits
                                                .iter()
                                                .any(|hit| self.selected_hits.contains(&hit.id))
                                        })
                                })
                                && !view.is_saving();
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
                                    self.prepare_replacements(view, next);
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
                                    if search_scope_button(ui, label, color, self.scope == scope)
                                        .clicked()
                                    {
                                        scope_clicked = true;
                                        self.scope = scope;
                                        if scope != SearchScope::Both {
                                            self.results_side =
                                                if scope == SearchScope::Left { 0 } else { 1 };
                                        }
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
                            let options_changed = self.query.case_sensitive
                                != previous_query.case_sensitive
                                || self.query.whole_word != previous_query.whole_word
                                || self.query.regex != previous_query.regex;
                            if self.scope != previous_scope
                                || self.replacement != previous_replacement
                            {
                                self.preview = None;
                            }
                            if refresh
                                || options_changed
                                || (scope_clicked
                                    && (self.search_due_at.is_some()
                                        || self.query.pattern != previous_query.pattern
                                        || (self.search.is_none() && !self.results_current())))
                            {
                                self.start_search(view);
                            } else if self.query.pattern != previous_query.pattern {
                                self.schedule_search(ctx.input(|input| input.time));
                            }
                            ui.horizontal(|ui| {
                                if self.search.is_some() {
                                    ui.spinner();
                                    let status =
                                        format!("Searching… {} matches", self.selected_hits.len());
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
                                        self.search = None;
                                        if let Some(results) = &mut self.results {
                                            results.cancelled = true;
                                        }
                                    }
                                } else if self.search_due_at.is_some() {
                                    ui.label("Waiting for typing…");
                                } else if let Some(results) = &self.results {
                                    ui.label(if results.cancelled {
                                        "Search stopped".into()
                                    } else {
                                        format!("Found {} matches", self.selected_hits.len())
                                    });
                                }
                            });
                            if let Some(preview) = &self.preview {
                                ui.separator();
                                ui.label(
                                    "Preview — files are changed in memory until explicitly saved",
                                );
                                egui::ScrollArea::vertical()
                                    .max_height(120.0)
                                    .show(ui, |ui| {
                                        for change in &preview.changes {
                                            egui::CollapsingHeader::new(
                                                change.path.display().to_string(),
                                            )
                                            .show(ui, |ui| {
                                                ui.columns(2, |cols| {
                                                    cols[0].label(
                                                        RichText::new(&change.before).monospace(),
                                                    );
                                                    cols[1].label(
                                                        RichText::new(&change.after).monospace(),
                                                    );
                                                });
                                            })
                                            .header_response
                                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                                        }
                                    });
                            }
                            if self.preview.is_some()
                                && ui
                                    .add_enabled(
                                        self.preview
                                            .as_ref()
                                            .is_some_and(|p| !p.changes.is_empty()),
                                        egui::Button::new("Apply preview to working copies"),
                                    )
                                    .clicked()
                            {
                                self.apply_replacement_preview(view);
                            }
                            let dirty = pending_save_files(&self.working).len();
                            if dirty > 0 {
                                ui.label(
                                    RichText::new(format!(
                                        "{dirty} files have unsaved replacements"
                                    ))
                                    .color(Palette::for_context(ctx).changed),
                                );
                            }
                            if dirty > 0
                                && ui
                                    .add_enabled(
                                        !self.is_saving() && !view.is_saving(),
                                        egui::Button::new("Preview Save pending files"),
                                    )
                                    .clicked()
                            {
                                self.pending_save = true;
                            }
                            if let Some(save) = &self.save {
                                ui.spinner();
                                ui.label(format!(
                                    "Saving {} of {} files…",
                                    save.completed.min(save.total),
                                    save.total
                                ));
                            }
                            if self.pending_save {
                                ui.separator();
                                ui.label("Save these working copies to their original files?");
                                egui::ScrollArea::vertical()
                                    .max_height(120.0)
                                    .show(ui, |ui| {
                                        for (path, file, _) in pending_save_files(&self.working) {
                                            egui::CollapsingHeader::new(path.display().to_string())
                                                .show(ui, |ui| {
                                                    ui.columns(2, |columns| {
                                                        columns[0].label("Loaded file");
                                                        columns[0].label(
                                                            RichText::new(
                                                                file.baseline_text()
                                                                    .unwrap_or_default(),
                                                            )
                                                            .monospace(),
                                                        );
                                                        columns[1].label("Pending content");
                                                        columns[1].label(
                                                            RichText::new(&file.text).monospace(),
                                                        );
                                                    });
                                                })
                                                .header_response
                                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                                        }
                                    });
                                ui.horizontal(|ui| {
                                    if ui.button("Save pending files").clicked() {
                                        self.begin_save(view);
                                    }
                                    if ui.button("Cancel save").clicked() {
                                        self.pending_save = false;
                                    }
                                });
                            }
                            if let Some(error) = &self.error {
                                ui.colored_label(Palette::for_context(ctx).error, error);
                            }
                        });
                    let right = &mut columns[1];
                    right.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        let palette = Palette::for_context(ui.ctx());
                        let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
                        for (side, label) in [(0, "LEFT"), (1, "RIGHT")] {
                            let enabled = scope_includes(self.scope, side);
                            if ui
                                .add_enabled_ui(enabled, |ui| {
                                    search_choice_button(
                                        ui,
                                        label,
                                        palette.text,
                                        self.results_side == side,
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
                                self.results_side = side;
                            }
                        }
                    });
                    right.add_space(3.0);
                    let results_height = (max_height - control_extent(right) - 6.0).max(60.0);
                    self.search_results_panel(right, results_height);
                });
            });
        self.search_open = !(close || response.should_close());
        if !self.search_open {
            self.search_due_at = None;
            if self.search.is_some()
                && let Some(results) = &mut self.results
            {
                results.cancelled = true;
            }
            self.search = None;
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
            !self.query.pattern.is_empty() && !view.is_saving() && !self.is_saving(),
            "Refresh search",
        )
        .clicked();
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
                if let Some(results) = &self.results {
                    enum Row<'a> {
                        File(&'a versus::SearchFileResults),
                        Hit(&'a versus::SearchFileResults, &'a versus::SearchHit),
                    }
                    let mut rows = Vec::new();
                    let mut matches = 0;
                    let mut files = 0;
                    for file in results
                        .files
                        .iter()
                        .filter(|file| file.side == self.results_side)
                    {
                        let hits: Vec<_> = file
                            .hits
                            .iter()
                            .filter(|hit| self.selected_hits.contains(&hit.id))
                            .collect();
                        if hits.is_empty() {
                            continue;
                        }
                        files += 1;
                        matches += hits.len();
                        rows.push(Row::File(file));
                        rows.extend(hits.into_iter().map(|hit| Row::Hit(file, hit)));
                    }
                    ui.label(format!("{matches} matches in {files} files"));
                    if !rows.is_empty() {
                        let row_height = control_extent(ui);
                        egui::ScrollArea::vertical()
                            .id_salt(("folder-search-results-list", self.results_side))
                            .max_height((height - 48.0).max(30.0))
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
                    if self.search.is_some() || self.results.is_none() {
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
            self.selected_hits.remove(&id);
            self.preview = None;
        }
    }

    fn schedule_search(&mut self, now: f64) {
        self.search = None;
        self.results = None;
        self.preview = None;
        self.selected_hits.clear();
        self.error = None;
        self.search_due_at = (!self.query.pattern.is_empty()).then_some(now + 0.25);
    }
    fn run_scheduled_search(&mut self, ctx: &egui::Context, view: &InteractiveView, now: f64) {
        if let Some(due) = self.search_due_at {
            if now >= due {
                self.search_due_at = None;
                self.start_search(view);
            } else {
                ctx.request_repaint_after(Duration::from_secs_f64(due - now));
            }
        }
    }
    fn results_current(&self) -> bool {
        self.search_due_at.is_none()
            && self.search.is_none()
            && self
                .results
                .as_ref()
                .is_some_and(|results| !results.cancelled)
            && self.searched_query.as_ref() == Some(&self.query)
    }
    fn start_search(&mut self, view: &InteractiveView) {
        self.search_due_at = None;
        self.search = None;
        self.results = None;
        self.preview = None;
        self.selected_hits.clear();
        self.error = None;
        self.searched_query = None;
        if self.query.pattern.is_empty() {
            return;
        }
        self.working = view.export_working_files();
        let roots = self.sources.clone();
        let query = self.query.clone();
        let working = self.working.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancellation = cancel.clone();
        let (sender, receiver) = mpsc::sync_channel(4);
        std::thread::spawn(move || {
            let result = crate::diagnostics::catch_worker("Find and Replace search", || {
                versus::search_roots_streaming(
                    &roots,
                    SearchScope::Both,
                    &query,
                    Some(&cancellation),
                    &working,
                    |event| sender.send(Ok(event)).is_ok(),
                )
            });
            if let Err(error) = result {
                let _ = sender.send(Err(error));
            }
        });
        self.search = Some(SearchJob { receiver, cancel });
        self.searched_query = Some(self.query.clone());
    }
    fn prepare_replacements(&mut self, view: &InteractiveView, next: bool) {
        if !self.results_current() {
            return;
        }
        self.working = view.export_working_files();
        let selected = self
            .results
            .as_ref()
            .unwrap()
            .files
            .iter()
            .filter(|file| scope_includes(self.scope, file.side))
            .flat_map(|file| &file.hits)
            .filter(|hit| self.selected_hits.contains(&hit.id))
            .map(|hit| hit.id)
            .collect();
        match search_session::preview_replacements(
            self.results.as_ref().unwrap(),
            &selected,
            &self.query,
            &self.replacement,
            &self.working,
            next,
        ) {
            Ok(preview) => self.preview = Some(preview),
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    fn apply_replacement_preview(&mut self, view: &mut InteractiveView) {
        let Some(preview) = self.preview.take() else {
            return;
        };
        match view.apply_search_preview(&preview) {
            Ok(_) => {
                self.working = view.export_working_files();
                if let Some(results) = &mut self.results {
                    if let Err(error) = search_session::advance_results(
                        results,
                        &mut self.selected_hits,
                        &preview,
                        &self.working,
                    ) {
                        self.results = None;
                        self.selected_hits.clear();
                        self.error = Some(error);
                    }
                }
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn undo_batch(&mut self, view: &mut InteractiveView, redo: bool) {
        match view.undo_batch(redo) {
            Ok(()) => {
                self.working = view.export_working_files();
                self.search = None;
                self.search_due_at = None;
                self.searched_query = None;
                self.results = None;
                self.preview = None;
                self.selected_hits.clear();
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

fn scope_includes(scope: SearchScope, side: usize) -> bool {
    scope == SearchScope::Both
        || (side == 0 && scope == SearchScope::Left)
        || (side == 1 && scope == SearchScope::Right)
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
    ignored: BTreeMap<PathBuf, Vec<Range<usize>>>,
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
                    let left_ignored = ignored
                        .get(&left.identity_key())
                        .cloned()
                        .unwrap_or_default();
                    let right_ignored = ignored
                        .get(&right.identity_key())
                        .cloned()
                        .unwrap_or_default();
                    let comparison = compare_loaded_text(
                        [&left.text, &right.text],
                        cancellation,
                        &options,
                        [&left_ignored, &right_ignored],
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
    ignored: &BTreeMap<PathBuf, Vec<Range<usize>>>,
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
                for range in ignored.get(&file.identity_key()).into_iter().flatten() {
                    range.start.hash(&mut hasher);
                    range.end.hash(&mut hasher);
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use versus::{FileFingerprint, TextEncoding};

    fn tree_entry(
        path: &str,
        kind: DirectoryEntryKind,
        state: DirectoryEntryState,
    ) -> versus::DirectoryEntry {
        let left_exists = !matches!(state, DirectoryEntryState::RightOnly);
        let right_exists = !matches!(state, DirectoryEntryState::LeftOnly);
        versus::DirectoryEntry {
            relative_path: PathBuf::from(path),
            left_exists,
            right_exists,
            left_kind: left_exists.then(|| kind.clone()),
            right_kind: right_exists.then(|| kind.clone()),
            left_size: None,
            right_size: None,
            kind,
            state,
        }
    }

    fn folder_tree(entries: Vec<versus::DirectoryEntry>) -> FolderTree {
        FolderTree::from_diff(&versus::DirectoryDiff {
            entries,
            cancelled: false,
        })
    }

    fn dialog_frame(
        browser: &mut FolderBrowser,
        view: &mut InteractiveView,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        dialog_frame_at(browser, view, ctx, events, None)
    }
    fn dialog_frame_at(
        browser: &mut FolderBrowser,
        view: &mut InteractiveView,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        time: Option<f64>,
    ) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                events,
                time,
                ..Default::default()
            },
            |_| browser.search_window(ctx, view),
        )
    }

    #[test]
    fn merged_states_reaggregate_nested_parents_and_preserve_intrinsic_states() {
        let mut mixed_type = tree_entry(
            "file-folder",
            DirectoryEntryKind::File,
            DirectoryEntryState::TypeMismatch,
        );
        mixed_type.left_kind = Some(DirectoryEntryKind::Directory);
        mixed_type.left_size = None;
        let tree = folder_tree(vec![
            mixed_type,
            tree_entry(
                "src",
                DirectoryEntryKind::Directory,
                DirectoryEntryState::Same,
            ),
            tree_entry(
                "src/changed.rs",
                DirectoryEntryKind::File,
                DirectoryEntryState::Different,
            ),
            tree_entry(
                "src/clean.rs",
                DirectoryEntryKind::File,
                DirectoryEntryState::Same,
            ),
            tree_entry(
                "only-left",
                DirectoryEntryKind::Directory,
                DirectoryEntryState::LeftOnly,
            ),
            tree_entry(
                "mismatch",
                DirectoryEntryKind::File,
                DirectoryEntryState::TypeMismatch,
            ),
            tree_entry(
                "only-right.txt",
                DirectoryEntryKind::File,
                DirectoryEntryState::RightOnly,
            ),
            tree_entry(
                "unreadable",
                DirectoryEntryKind::Directory,
                DirectoryEntryState::Error(CompareError::io(
                    "unreadable",
                    std::io::Error::other("denied"),
                )),
            ),
        ]);
        let mut overlays = BTreeMap::new();
        overlays.insert(PathBuf::from("src/changed.rs"), DirectoryEntryState::Same);
        let cache = build_merged_folder_cache(Some(&tree), &overlays, Some(1));
        assert_eq!(
            cache.states[std::path::Path::new("src")],
            DirectoryEntryState::Same,
            "a clean overlay must restore all nested parent badges"
        );
        assert_eq!(
            cache.states[std::path::Path::new("only-left")],
            DirectoryEntryState::LeftOnly
        );
        assert_eq!(
            cache.states[std::path::Path::new("mismatch")],
            DirectoryEntryState::TypeMismatch
        );
        assert_eq!(
            cache.states[std::path::Path::new("only-right.txt")],
            DirectoryEntryState::RightOnly
        );
        assert!(matches!(
            cache.states[std::path::Path::new("unreadable")],
            DirectoryEntryState::Error(_)
        ));
        assert_eq!(
            cache.difference_paths.as_slice(),
            [
                PathBuf::from("file-folder"),
                PathBuf::from("mismatch"),
                PathBuf::from("only-right.txt")
            ]
        );
        for terminal in [
            DirectoryEntryState::TypeMismatch,
            DirectoryEntryState::LeftOnly,
            DirectoryEntryState::RightOnly,
            DirectoryEntryState::Error(CompareError::io(
                "src",
                std::io::Error::other("overlay failed"),
            )),
        ] {
            overlays.insert(PathBuf::from("src"), terminal.clone());
            let cache = build_merged_folder_cache(Some(&tree), &overlays, Some(1));
            assert_eq!(
                cache.states[std::path::Path::new("src")],
                terminal,
                "a direct terminal overlay must survive parent aggregation"
            );
        }
    }

    #[test]
    fn merged_state_cache_is_reused_for_large_identical_trees_until_invalidated() {
        let mut entries = Vec::new();
        for index in 0..8_000 {
            entries.push(tree_entry(
                &format!("dir-{index}"),
                DirectoryEntryKind::Directory,
                DirectoryEntryState::Same,
            ));
            entries.push(tree_entry(
                &format!("dir-{index}/file.txt"),
                DirectoryEntryKind::File,
                DirectoryEntryState::Same,
            ));
        }
        let tree = folder_tree(entries);
        let mut browser = FolderBrowser::new_search([None, None]);
        let first = browser.merged_folder_cache(Some(&tree)).states.clone();
        let first_paths = browser
            .merged_folder_cache(Some(&tree))
            .difference_paths
            .clone();
        let second = browser.merged_folder_cache(Some(&tree));
        assert!(Arc::ptr_eq(&first, &second.states));
        assert!(Arc::ptr_eq(&first_paths, &second.difference_paths));
        assert!(second.difference_paths.is_empty());
        browser.invalidate_comparison_cache();
        let rebuilt = browser.merged_folder_cache(Some(&tree));
        assert!(!Arc::ptr_eq(&first, &rebuilt.states));
    }

    #[test]
    fn find_modal_has_centered_half_width_layout_and_a_styled_results_placeholder() {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let ctx = egui::Context::default();
            set_theme(&ctx, theme);
            // Inspect the final colors rather than the Area's fade-in opacity.
            ctx.all_styles_mut(|style| style.animation_time = 0.0);
            let mut browser = FolderBrowser::new_with_sources([None, None]);
            let mut view = InteractiveView::default();
            browser.open_search();
            // Area sizing is invisible on its first pass; responses retain the
            // previous pass, so settle the anchored layout before inspecting it.
            dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
            dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
            let output = dialog_frame(&mut browser, &mut view, &ctx, vec![]);
            let modal = ctx
                .memory(|memory| memory.area_rect(egui::Id::new("folder-find-replace-modal")))
                .unwrap();
            assert!(modal.center().distance(ctx.content_rect().center()) < 1.0);
            assert!(
                (modal.width() - 600.0).abs() < 10.0 && modal.height() < 450.0,
                "{modal:?}"
            );
            let find = ctx
                .read_response(egui::Id::new(("folder-find-pattern", "frame")))
                .unwrap()
                .rect;
            let replacement = ctx
                .read_response(egui::Id::new(("folder-replacement-text", "frame")))
                .unwrap()
                .rect;
            assert!(find.width() > modal.width() / 2.0 - 30.0 && find.height() > 32.0);
            let results = ctx
                .read_response(egui::Id::new("folder-search-results"))
                .unwrap()
                .rect;
            assert!(results.left() >= modal.center().x && find.right() < results.left());
            assert!(modal.contains_rect(results));
            assert!(
                ctx.read_response(egui::Id::new("Find")).is_none(),
                "the Find action was removed"
            );
            assert!(replacement.height() > 32.0);
            for label in ["Match Case", "Match Whole Word", "Use Regular Expression"] {
                let control = ctx.read_response(egui::Id::new(label)).unwrap().rect;
                assert!(find.contains_rect(control));
                assert!(
                    control.left()
                        > ctx
                            .read_response(egui::Id::new("folder-find-pattern"))
                            .unwrap()
                            .rect
                            .right()
                );
            }
            let next = ctx
                .read_response(egui::Id::new("Replace Next"))
                .unwrap()
                .rect;
            let all = ctx
                .read_response(egui::Id::new("Replace All"))
                .unwrap()
                .rect;
            assert!(next.left() > replacement.right() && all.left() > next.right());
            assert!(
                (next.center().y - replacement.center().y).abs() < 1.0,
                "next={next:?}, replacement={replacement:?}"
            );
            let palette = Palette::for_context(&ctx);
            assert!(results.width() > modal.width() / 2.0 - 40.0);
            assert!(results.height() > 300.0);
            assert!(
                output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(frame) if frame.fill == palette.panel
                    && frame.rect.contains_rect(results)
                    && (frame.rect.width() - results.width() - 18.0).abs() < 1.0)),
                "the painted panel must fill the right column even without results"
            );
            let left_view = ctx
                .read_response(egui::Id::new(("folder-search-view", 0usize)))
                .unwrap()
                .rect;
            let right_view = ctx
                .read_response(egui::Id::new(("folder-search-view", 1usize)))
                .unwrap()
                .rect;
            assert!((left_view.width() - right_view.width()).abs() < 1.0);
            assert!((left_view.left() - (results.left() - 9.0)).abs() < 1.0);
            assert!((right_view.right() - (results.right() + 9.0)).abs() < 1.0);
            for label in ["LEFT", "RIGHT"] {
                assert!(
                    output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == label
                        && text.fallback_color == palette.text
                        && text.galley.rect.translate(text.pos.to_vec2()).center().y < results.top()
                        && text.pos.x > modal.center().x)),
                    "result view toggle must be neutral"
                );
            }
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text() == "Search results will appear here"
                    && text.galley.job.sections[0].format.font_id == FontId::proportional(14.0)
                    && text.fallback_color == palette.muted
                    && results.contains_rect(text.galley.rect.translate(text.pos.to_vec2()))
                    && text.galley.rect.translate(text.pos.to_vec2()).center().distance(results.center()) < 1.0)));
            for (label, color) in [("LEFT", palette.left_only), ("RIGHT", palette.right_only)] {
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == label && text.fallback_color == color)),
                    "{label} expected={color:?}, actual={:?}", output.shapes.iter().filter_map(|shape|
                        if let egui::Shape::Text(text) = &shape.shape { (text.galley.text() == label).then_some(text.fallback_color) } else { None }).collect::<Vec<_>>());
            }
            for placeholder in ["Find", "Replace"] {
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == placeholder
                        && text.galley.job.sections[0].format.font_id.size == 14.0
                        && text.galley.job.sections[0].format.italics
                        && text.galley.job.sections[0].format.color == palette.muted.gamma_multiply(0.8))));
            }
            output.drop_without_applying_deltas();
            for label in ["Match Case", "Match Whole Word", "Use Regular Expression"] {
                let pos = ctx
                    .read_response(egui::Id::new(label))
                    .unwrap()
                    .rect
                    .center();
                for pressed in [true, false] {
                    dialog_frame(
                        &mut browser,
                        &mut view,
                        &ctx,
                        vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            },
                        ],
                    )
                    .drop_without_applying_deltas();
                }
            }
            assert!(
                browser.query.case_sensitive && browser.query.whole_word && browser.query.regex
            );
            assert!(
                browser.search_open,
                "clicking controls inside the field must keep the modal open"
            );
            let outside = egui::pos2(modal.left() - 10.0, modal.center().y);
            for pressed in [true, false] {
                dialog_frame(
                    &mut browser,
                    &mut view,
                    &ctx,
                    vec![
                        egui::Event::PointerMoved(outside),
                        egui::Event::PointerButton {
                            pos: outside,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                )
                .drop_without_applying_deltas();
            }
            assert!(!browser.search_open);
        }
    }

    #[test]
    fn find_modal_columns_and_controls_fit_at_normal_and_high_zoom() {
        for zoom in [1.0, 2.0] {
            let ctx = egui::Context::default();
            set_theme(&ctx, egui::Theme::Light);
            ctx.set_zoom_factor(zoom);
            let mut browser = FolderBrowser::new_search([None, None]);
            let mut view = InteractiveView::default();
            browser.open_search();
            for _ in 0..6 {
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            egui::vec2(900.0, 800.0) / zoom,
                        )),
                        ..Default::default()
                    },
                    |_| browser.search_window(&ctx, &mut view),
                )
                .drop_without_applying_deltas();
            }
            let modal = ctx
                .memory(|memory| memory.area_rect(egui::Id::new("folder-find-replace-modal")))
                .unwrap();
            assert!(
                ctx.content_rect().contains_rect(modal),
                "zoom={zoom}: {modal:?}"
            );
            let results = ctx
                .read_response(egui::Id::new("folder-search-results"))
                .unwrap()
                .rect;
            for label in [
                "Match Case",
                "Match Whole Word",
                "Use Regular Expression",
                "Replace Next",
                "Replace All",
            ] {
                let control = ctx.read_response(egui::Id::new(label)).unwrap().rect;
                assert!(
                    modal.contains_rect(control) && control.right() < results.left(),
                    "zoom={zoom}: {label}"
                );
            }
            let replace = ctx
                .read_response(egui::Id::new(("folder-replacement-text", "frame")))
                .unwrap()
                .rect;
            let both = ctx
                .read_response(egui::Id::new(("folder-search-scope", "BOTH")))
                .unwrap()
                .rect;
            assert!(
                both.top() > replace.bottom(),
                "scope must not overlap Replace"
            );
            let actions: Vec<_> = [
                "Undo replacement batch",
                "Redo replacement batch",
                "Refresh search",
            ]
            .into_iter()
            .map(|label| ctx.read_response(egui::Id::new(label)).unwrap().rect)
            .collect();
            assert!(
                actions.iter().all(|rect| modal.contains_rect(*rect)),
                "zoom={zoom} modal={modal:?} actions={actions:?}"
            );
            assert!(
                actions
                    .windows(2)
                    .all(|pair| pair[1].left() > pair[0].right())
            );
            if zoom == 1.0 {
                assert!(actions[0].left() > both.right());
                assert!((actions[0].center().y - both.center().y).abs() < 1.0);
            }
        }
    }

    fn streamed_file(side: usize, id: u64, text: &str) -> versus::SearchFileResults {
        let path = PathBuf::from(format!("side-{side}.txt"));
        let range = text.find("match").unwrap()..text.find("match").unwrap() + 5;
        versus::SearchFileResults {
            side,
            path: path.clone(),
            working_file: WorkingFile {
                identity: None,
                path: Some(path.clone()),
                text: text.into(),
                encoding: TextEncoding::Utf8 { bom: false },
                fingerprint: FileFingerprint::Bytes(text.as_bytes().to_vec()),
            },
            hits: vec![versus::SearchHit {
                id,
                side,
                path,
                range,
                line: 1,
                matched: "match".into(),
            }],
        }
    }

    fn click_dialog(
        browser: &mut FolderBrowser,
        view: &mut InteractiveView,
        ctx: &egui::Context,
        pos: Pos2,
    ) {
        for pressed in [true, false] {
            dialog_frame(
                browser,
                view,
                ctx,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            )
            .drop_without_applying_deltas();
        }
    }

    #[test]
    fn scope_buttons_immediately_filter_cached_results_and_limit_replacement_targets() {
        let ctx = egui::Context::default();
        set_theme(&ctx, egui::Theme::Dark);
        let mut browser = FolderBrowser::new_search([None, None]);
        let mut view = InteractiveView::default();
        browser.open_search();
        browser.query = SearchQuery::plain("match");
        browser.searched_query = Some(browser.query.clone());
        browser.replacement = "changed".into();
        browser.results = Some(SearchResults {
            files: vec![
                streamed_file(0, 1, "left match"),
                streamed_file(1, 2, "right match"),
            ],
            ..Default::default()
        });
        browser.selected_hits = BTreeSet::from([1, 2]);
        for _ in 0..3 {
            dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
        }
        browser.prepare_replacements(&view, false);
        assert_eq!(browser.preview.as_ref().unwrap().changes.len(), 2);
        for (label, side, path) in [
            ("LEFT", 0usize, "side-0.txt"),
            ("RIGHT", 1usize, "side-1.txt"),
        ] {
            let button = ctx
                .read_response(egui::Id::new(("folder-search-scope", label)))
                .unwrap()
                .rect;
            click_dialog(&mut browser, &mut view, &ctx, button.center());
            assert_eq!(browser.results_side, side);
            assert!(browser.search.is_none() && browser.search_due_at.is_none());
            assert!(browser.results_current());
            assert!(
                browser.preview.is_none(),
                "scope change must invalidate a wider preview"
            );
            assert_eq!(browser.results.as_ref().unwrap().files.len(), 2);
            assert_eq!(browser.selected_hits, BTreeSet::from([1, 2]));
            dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
            let other = ctx
                .read_response(egui::Id::new(("folder-search-view", 1 - side)))
                .unwrap();
            assert!(!other.enabled());
            click_dialog(&mut browser, &mut view, &ctx, other.rect.center());
            assert_eq!(
                browser.results_side, side,
                "disabled toggle cannot change the results side"
            );
            browser.prepare_replacements(&view, false);
            assert_eq!(browser.preview.as_ref().unwrap().changes.len(), 1);
            assert_eq!(
                browser.preview.as_ref().unwrap().changes[0].path,
                PathBuf::from(path)
            );
        }
        let both = ctx
            .read_response(egui::Id::new(("folder-search-scope", "BOTH")))
            .unwrap()
            .rect;
        click_dialog(&mut browser, &mut view, &ctx, both.center());
        dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
        for side in 0..2usize {
            assert!(
                ctx.read_response(egui::Id::new(("folder-search-view", side)))
                    .unwrap()
                    .enabled()
            );
        }
        let left = ctx
            .read_response(egui::Id::new(("folder-search-view", 0usize)))
            .unwrap()
            .rect;
        click_dialog(&mut browser, &mut view, &ctx, left.center());
        assert_eq!(browser.results_side, 0);
        assert!(browser.results_current() && browser.search.is_none());
        browser.prepare_replacements(&view, false);
        assert_eq!(browser.preview.as_ref().unwrap().changes.len(), 2);
        browser.scope = SearchScope::Left;
        browser.set_search_sources([None, Some(PathBuf::from("right-only.txt"))]);
        assert_eq!(
            browser.results_side, 0,
            "source changes must not select a disabled results side"
        );
    }

    #[test]
    fn button_actions_bypass_typing_delay_and_scope_changes_keep_the_active_search() {
        let ctx = egui::Context::default();
        set_theme(&ctx, egui::Theme::Dark);
        let mut browser = FolderBrowser::new_search([None, None]);
        let mut view = InteractiveView::default();
        browser.open_search();
        for _ in 0..3 {
            dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
        }
        browser.query.pattern = "match".into();
        browser.schedule_search(100.0);
        let left = ctx
            .read_response(egui::Id::new(("folder-search-scope", "LEFT")))
            .unwrap()
            .rect;
        click_dialog(&mut browser, &mut view, &ctx, left.center());
        assert!(browser.search_due_at.is_none() && browser.search.is_some());
        assert_eq!(browser.searched_query, Some(browser.query.clone()));
        let cancellation = browser.search.as_ref().unwrap().cancel.clone();
        let right = ctx
            .read_response(egui::Id::new(("folder-search-scope", "RIGHT")))
            .unwrap()
            .rect;
        click_dialog(&mut browser, &mut view, &ctx, right.center());
        assert!(Arc::ptr_eq(
            &browser.search.as_ref().unwrap().cancel,
            &cancellation
        ));
        assert!(!cancellation.load(Ordering::Relaxed));
        for label in ["Match Case", "Match Whole Word", "Use Regular Expression"] {
            browser.schedule_search(100.0);
            let button = ctx.read_response(egui::Id::new(label)).unwrap().rect;
            click_dialog(&mut browser, &mut view, &ctx, button.center());
            assert!(
                browser.search_due_at.is_none() && browser.search.is_some(),
                "{label} should search immediately"
            );
            assert_eq!(browser.searched_query, Some(browser.query.clone()));
        }
        let cancellation = browser.search.as_ref().unwrap().cancel.clone();
        browser.query.pattern.clear();
        let button = ctx.read_response(egui::Id::new("Match Case")).unwrap().rect;
        click_dialog(&mut browser, &mut view, &ctx, button.center());
        assert!(cancellation.load(Ordering::Relaxed));
        assert!(
            browser.search.is_none()
                && browser.search_due_at.is_none()
                && browser.results.is_none()
        );
    }

    #[test]
    fn every_worker_searches_both_sides_regardless_of_replacement_scope() {
        let root =
            std::env::temp_dir().join(format!("versus-always-both-search-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let paths = [root.join("left.txt"), root.join("right.txt")];
        for path in &paths {
            std::fs::write(path, "match\n").unwrap();
        }
        let ctx = egui::Context::default();
        let view = InteractiveView::default();
        let mut browser = FolderBrowser::new_search(paths.map(Some));
        browser.query = SearchQuery::plain("match");
        for scope in [SearchScope::Left, SearchScope::Right, SearchScope::Both] {
            browser.scope = scope;
            browser.start_search(&view);
            let deadline = Instant::now() + Duration::from_secs(2);
            while browser.search.is_some() && Instant::now() < deadline {
                browser.poll(&ctx);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(browser.results_current());
            let sides: BTreeSet<_> = browser
                .results
                .as_ref()
                .unwrap()
                .files
                .iter()
                .map(|file| file.side)
                .collect();
            assert_eq!(sides, BTreeSet::from([0, 1]));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn streamed_results_are_visible_before_completion_and_exclusions_survive_view_switches() {
        let ctx = egui::Context::default();
        set_theme(&ctx, egui::Theme::Dark);
        let mut browser = FolderBrowser::new_search([None, None]);
        let mut view = InteractiveView::default();
        browser.open_search();
        browser.query = SearchQuery::plain("match");
        browser.scope = SearchScope::Both;
        browser.searched_query = Some(browser.query.clone());
        let (sender, receiver) = mpsc::channel();
        browser.search = Some(SearchJob {
            receiver,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        let left = streamed_file(0, 0, "left match\nlater match");
        let remove_id = egui::Id::new((
            "remove-search-result",
            left.side,
            left.hits[0].id,
            &left.path,
        ));
        sender.send(Ok(SearchEvent::File(left))).unwrap();
        browser.poll(&ctx);
        assert!(browser.search.is_some());
        assert!(
            !browser.results_current(),
            "partial output cannot authorize replacements"
        );
        for _ in 0..3 {
            dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
        }
        let remove = ctx
            .read_response(remove_id)
            .expect("streamed match should already be visible")
            .rect;
        click_dialog(&mut browser, &mut view, &ctx, remove.center());
        assert!(!browser.selected_hits.contains(&0));
        sender
            .send(Ok(SearchEvent::Hits {
                path: PathBuf::from("side-0.txt"),
                side: 0,
                hits: vec![versus::SearchHit {
                    id: 2,
                    side: 0,
                    path: PathBuf::from("side-0.txt"),
                    range: 17..22,
                    line: 2,
                    matched: "match".into(),
                }],
            }))
            .unwrap();
        sender
            .send(Ok(SearchEvent::File(streamed_file(1, 1, "right match"))))
            .unwrap();
        sender
            .send(Ok(SearchEvent::Complete { cancelled: false }))
            .unwrap();
        browser.poll(&ctx);
        assert!(browser.results_current());
        assert_eq!(browser.selected_hits, BTreeSet::from([1, 2]));
        assert_eq!(browser.results.as_ref().unwrap().files[0].hits.len(), 2);
        let right_toggle = ctx
            .read_response(egui::Id::new(("folder-search-view", 1usize)))
            .unwrap()
            .rect;
        click_dialog(&mut browser, &mut view, &ctx, right_toggle.center());
        assert_eq!(browser.results_side, 1);
        assert_eq!(browser.scope, SearchScope::Both);
        assert!(
            browser.results_current(),
            "display filter must not restart search"
        );
        assert_eq!(browser.results.as_ref().unwrap().files.len(), 2);
        let output = dialog_frame(&mut browser, &mut view, &ctx, vec![]);
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "1: right match")));
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "1: left match")));
        output.drop_without_applying_deltas();
    }

    #[test]
    fn refresh_repopulates_exclusions_and_rereads_changed_files() {
        let path =
            std::env::temp_dir().join(format!("versus-refresh-search-{}.txt", std::process::id()));
        std::fs::write(&path, "match old\n").unwrap();
        let ctx = egui::Context::default();
        set_theme(&ctx, egui::Theme::Dark);
        let mut browser = FolderBrowser::new_search([Some(path.clone()), None]);
        let mut view = InteractiveView::default();
        view.ingest_working_files(&BTreeMap::from([(
            path.clone(),
            load_working_file(Some(&path)).unwrap(),
        )]));
        view.open_sources([Some(path.clone()), None]);
        browser.open_search();
        browser.query = SearchQuery::plain("match");
        browser.start_search(&view);
        let wait = |browser: &mut FolderBrowser| {
            let deadline = Instant::now() + Duration::from_secs(2);
            while browser.search.is_some() && Instant::now() < deadline {
                browser.poll(&ctx);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(browser.results_current());
        };
        wait(&mut browser);
        browser.selected_hits.clear();
        std::fs::write(&path, "match new\nmatch added\n").unwrap();
        for _ in 0..3 {
            dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
        }
        let refresh = ctx
            .read_response(egui::Id::new("Refresh search"))
            .unwrap()
            .rect;
        click_dialog(&mut browser, &mut view, &ctx, refresh.center());
        wait(&mut browser);
        let file = &browser.results.as_ref().unwrap().files[0];
        assert_eq!(file.working_file.text, "match new\nmatch added\n");
        assert_eq!(browser.selected_hits.len(), 2);
        let revision = view.folder_revision_signature();
        browser.replacement = "found".into();
        for _ in 0..2 {
            browser.prepare_replacements(&view, false);
            assert!(
                browser.preview.is_some(),
                "fresh results must support replacement"
            );
            assert_eq!(
                view.folder_revision_signature(),
                revision,
                "Preview cannot mutate editor history"
            );
            assert_eq!(view.working_text(0), Some("match old\n"));
        }
        browser.apply_replacement_preview(&mut view);
        assert_eq!(view.working_text(0), Some("found new\nfound added\n"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "match new\nmatch added\n"
        );
        view.undo_batch(false).unwrap();
        assert_eq!(view.working_text(0), Some("match new\nmatch added\n"));
        view.undo_batch(true).unwrap();
        std::fs::write(&path, "changed externally again\n").unwrap();
        let mut pending = view.export_working_files()[&path].clone();
        assert!(versus::save_working_file(&mut pending).is_err());
        assert!(!view.run_diff_enabled());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn empty_state_is_clipped_to_results_and_cancelled_partial_search_restarts_on_reopen() {
        let ctx = egui::Context::default();
        set_theme(&ctx, egui::Theme::Dark);
        let mut browser = FolderBrowser::new_search([None, None]);
        let mut view = InteractiveView::default();
        browser.open_search();
        for _ in 0..3 {
            dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
        }
        let panel = ctx
            .read_response(egui::Id::new("folder-search-results"))
            .unwrap()
            .rect;
        let output = dialog_frame(&mut browser, &mut view, &ctx, vec![]);
        let hints: Vec<_> = output
            .shapes
            .iter()
            .filter(|shape| {
                matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "Search results will appear here")
            })
            .collect();
        assert_eq!(hints.len(), 1);
        assert!(panel.contains_rect(hints[0].clip_rect));
        output.drop_without_applying_deltas();
        browser.query = SearchQuery::plain("match");
        browser.searched_query = Some(browser.query.clone());
        let (sender, receiver) = mpsc::channel();
        browser.search = Some(SearchJob {
            receiver,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        sender
            .send(Ok(SearchEvent::File(streamed_file(0, 0, "match"))))
            .unwrap();
        browser.poll(&ctx);
        dialog_frame(
            &mut browser,
            &mut view,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        )
        .drop_without_applying_deltas();
        assert!(!browser.search_open);
        assert!(!browser.results_current());
        browser.open_search();
        dialog_frame_at(&mut browser, &mut view, &ctx, vec![], Some(10.0))
            .drop_without_applying_deltas();
        assert_eq!(browser.search_due_at, Some(10.25));
        assert!(browser.results.is_none());
    }

    #[test]
    fn typing_search_waits_a_quarter_second_after_the_last_edit_and_lists_results_on_the_right() {
        let path =
            std::env::temp_dir().join(format!("versus-live-search-{}.txt", std::process::id()));
        std::fs::write(&path, "alpha beta\nother content\n").unwrap();
        let ctx = egui::Context::default();
        set_theme(&ctx, egui::Theme::Dark);
        let mut browser = FolderBrowser::new_search([Some(path.clone()), None]);
        let mut view = InteractiveView::default();
        browser.open_search();
        for time in [0.0, 0.1, 0.2] {
            dialog_frame_at(&mut browser, &mut view, &ctx, vec![], Some(time))
                .drop_without_applying_deltas();
        }
        dialog_frame_at(
            &mut browser,
            &mut view,
            &ctx,
            vec![egui::Event::Text("alpha".into())],
            Some(1.0),
        )
        .drop_without_applying_deltas();
        assert_eq!(browser.query.pattern, "alpha");
        assert_eq!(browser.search_due_at, Some(1.25));
        assert!(browser.search.is_none());
        dialog_frame_at(
            &mut browser,
            &mut view,
            &ctx,
            vec![egui::Event::Text(" beta".into())],
            Some(1.2),
        )
        .drop_without_applying_deltas();
        assert_eq!(browser.query.pattern, "alpha beta");
        dialog_frame_at(&mut browser, &mut view, &ctx, vec![], Some(1.44))
            .drop_without_applying_deltas();
        assert!(browser.search.is_none(), "typing must reset the delay");
        dialog_frame_at(&mut browser, &mut view, &ctx, vec![], Some(1.45))
            .drop_without_applying_deltas();
        assert!(browser.search.is_some());
        assert_eq!(
            browser.searched_query.as_ref().unwrap().pattern,
            "alpha beta"
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while browser.search.is_some() && Instant::now() < deadline {
            browser.poll(&ctx);
            std::thread::sleep(Duration::from_millis(1));
        }
        let results = browser.results.as_ref().expect("live search must finish");
        assert_eq!(results.files.len(), 1);
        assert_eq!(results.files[0].hits.len(), 1);
        assert!(browser.results_current());
        dialog_frame_at(&mut browser, &mut view, &ctx, vec![], Some(2.0))
            .drop_without_applying_deltas();
        let output = dialog_frame_at(&mut browser, &mut view, &ctx, vec![], Some(2.1));
        let panel = ctx
            .read_response(egui::Id::new("folder-search-results"))
            .unwrap()
            .rect;
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "1: alpha beta"
                && panel.contains_rect(Rect::from_min_size(text.pos, text.galley.size())))));
        assert!(!view.run_diff_enabled());
        assert!(browser.jobs.iter().all(BTreeMap::is_empty));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "alpha beta\nother content\n"
        );
        output.drop_without_applying_deltas();
        dialog_frame_at(
            &mut browser,
            &mut view,
            &ctx,
            vec![egui::Event::Text(" more".into())],
            Some(2.2),
        )
        .drop_without_applying_deltas();
        assert!(browser.search_due_at.is_some());
        dialog_frame_at(
            &mut browser,
            &mut view,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            Some(2.3),
        )
        .drop_without_applying_deltas();
        assert!(!browser.search_open);
        assert!(browser.search_due_at.is_none() && browser.search.is_none());
        browser.open_search();
        dialog_frame_at(&mut browser, &mut view, &ctx, vec![], Some(2.5))
            .drop_without_applying_deltas();
        assert_eq!(
            browser.search_due_at,
            Some(2.75),
            "reopening resumes the pending query after a fresh delay"
        );
        assert!(browser.search.is_none());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn changing_search_cancels_stale_results_and_clearing_it_does_not_start_a_worker() {
        let ctx = egui::Context::default();
        let mut browser = FolderBrowser::new_search([None, None]);
        let view = InteractiveView::default();
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        browser.search = Some(SearchJob {
            receiver,
            cancel: cancel.clone(),
        });
        browser.results = Some(SearchResults::default());
        browser.query.pattern = "changed".into();
        browser.schedule_search(1.0);
        assert!(cancel.load(Ordering::Relaxed));
        assert!(
            sender
                .send(Ok(SearchEvent::Complete { cancelled: false }))
                .is_err(),
            "old worker cannot publish results"
        );
        assert!(browser.results.is_none() && !browser.results_current());
        browser.run_scheduled_search(&ctx, &view, 1.24);
        assert!(browser.search.is_none());
        browser.query.pattern.clear();
        browser.schedule_search(1.5);
        browser.run_scheduled_search(&ctx, &view, 2.0);
        assert!(browser.search_due_at.is_none() && browser.search.is_none());
    }

    #[test]
    fn replace_next_and_all_preview_then_advance_working_copies_without_saving() {
        let root = std::env::temp_dir().join(format!("versus-next-replace-{}", std::process::id()));
        std::fs::create_dir_all(root.join("left")).unwrap();
        std::fs::create_dir_all(root.join("right")).unwrap();
        let paths = [root.join("left/a.txt"), root.join("right/a.txt")];
        std::fs::write(&paths[0], "foo foo\n").unwrap();
        std::fs::write(&paths[1], "foo\n").unwrap();
        let working: BTreeMap<_, _> = paths
            .iter()
            .map(|path| (path.clone(), load_working_file(Some(path)).unwrap()))
            .collect();
        let mut view = InteractiveView::default();
        view.ingest_working_files(&working);
        view.open_sources(paths.clone().map(Some));
        view.set_editing(true, [true; 3]);
        let mut browser =
            FolderBrowser::new_with_sources([Some(root.join("left")), Some(root.join("right"))]);
        browser.query = SearchQuery::plain("foo");
        browser.replacement = "long value".into();
        browser.searched_query = Some(browser.query.clone());
        browser.results = Some(
            versus::search_roots(
                &browser.sources,
                SearchScope::Both,
                &browser.query,
                None,
                &working,
            )
            .unwrap(),
        );
        browser.selected_hits = browser
            .results
            .as_ref()
            .unwrap()
            .files
            .iter()
            .flat_map(|file| file.hits.iter().map(|hit| hit.id))
            .collect();
        browser.prepare_replacements(&view, true);
        assert_eq!(
            browser.preview.as_ref().unwrap().changes[0].hit_ids.len(),
            1
        );
        assert_eq!(view.working_text(0), Some("foo foo\n"));
        browser.apply_replacement_preview(&mut view);
        assert_eq!(view.working_text(0), Some("long value foo\n"));
        browser.prepare_replacements(&view, true);
        browser.apply_replacement_preview(&mut view);
        assert_eq!(view.working_text(0), Some("long value long value\n"));
        browser.prepare_replacements(&view, false);
        browser.apply_replacement_preview(&mut view);
        assert_eq!(view.working_text(1), Some("long value\n"));
        assert!(browser.results.as_ref().unwrap().files.is_empty());
        assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "foo foo\n");
        assert_eq!(std::fs::read_to_string(&paths[1]).unwrap(), "foo\n");
        browser.undo_batch(&mut view, false);
        assert_eq!(view.working_text(1), Some("foo\n"));
        assert!(
            browser.search.is_some() && browser.search_due_at.is_none(),
            "Undo must refresh immediately"
        );
        assert!(
            browser.results.is_none(),
            "undo invalidates old result snapshots"
        );
        assert!(!view.run_diff_enabled());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn save_completion_keeps_newer_edits_dirty_against_the_saved_baseline() {
        let path = PathBuf::from("changed.txt");
        let mut working = BTreeMap::new();
        working.insert(
            path.clone(),
            WorkingFile {
                identity: None,
                path: Some(path.clone()),
                text: "edited after save started\n".into(),
                encoding: TextEncoding::Utf8 { bom: false },
                fingerprint: FileFingerprint::Bytes(b"original\n".to_vec()),
            },
        );
        let saved = WorkingFile {
            identity: None,
            path: Some(path.clone()),
            text: "snapshot\n".into(),
            encoding: TextEncoding::Utf8 { bom: false },
            fingerprint: FileFingerprint::Bytes(b"snapshot\n".to_vec()),
        };

        retain_saved_snapshot(&mut working, path.clone(), "snapshot\n".into(), saved);

        let current = &working[&path];
        assert_eq!(current.text, "edited after save started\n");
        assert_eq!(
            current.fingerprint,
            FileFingerprint::Bytes(b"snapshot\n".to_vec())
        );
        assert!(current.is_dirty());
    }

    #[test]
    fn pending_save_deduplicates_path_aliases_by_physical_identity() {
        let identity = PathBuf::from("/physical/file.txt");
        let mut working = BTreeMap::new();
        for path in [
            PathBuf::from("left/file.txt"),
            PathBuf::from("right/../left/file.txt"),
        ] {
            working.insert(
                path.clone(),
                WorkingFile {
                    identity: Some(identity.clone()),
                    path: Some(path),
                    text: "edited".into(),
                    encoding: TextEncoding::Utf8 { bom: false },
                    fingerprint: FileFingerprint::Bytes(b"original".to_vec()),
                },
            );
        }
        let pending = pending_save_files(&working);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].1.identity_key(), identity);
    }

    #[test]
    fn reveal_difference_expands_only_target_ancestors_and_queues_shallow_metadata() {
        let roots = [PathBuf::from("left-root"), PathBuf::from("right-root")];
        let mut browser = FolderBrowser::new(roots);
        browser.reveal_difference(PathBuf::from("one/two/changed.txt"));
        for side in 0..2 {
            assert!(browser.expanded[side].contains(&PathBuf::from("one")));
            assert!(browser.expanded[side].contains(&PathBuf::from("one/two")));
            assert!(browser.jobs[side].contains_key(&PathBuf::from("one")));
            assert!(browser.jobs[side].contains_key(&PathBuf::from("one/two")));
            assert!(!browser.jobs[side].contains_key(&PathBuf::from("one/two/changed.txt")));
        }
        assert_eq!(
            browser.pending_reveal,
            [
                Some(PathBuf::from("one/two/changed.txt")),
                Some(PathBuf::from("one/two/changed.txt"))
            ]
        );
        assert_eq!(
            browser.selected,
            [
                Some(PathBuf::from("one/two/changed.txt")),
                Some(PathBuf::from("one/two/changed.txt"))
            ]
        );
    }

    #[test]
    fn single_source_enumerates_only_its_own_root() {
        let root =
            std::env::temp_dir().join(format!("versus-single-source-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("only-left.txt"), "content").unwrap();
        let mut browser = FolderBrowser::new_with_sources([Some(root.clone()), None]);
        for _ in 0..50 {
            browser.poll(&egui::Context::default());
            if browser.listings[0].contains_key(std::path::Path::new("")) {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            browser.listings[0]
                .get(std::path::Path::new(""))
                .is_some_and(|items| items
                    .iter()
                    .any(|item| item.relative_path == PathBuf::from("only-left.txt")))
        );
        assert!(browser.listings[1].is_empty());
        assert!(browser.jobs[1].is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn changing_one_source_preserves_the_other_tree_state() {
        let mut browser = FolderBrowser::new_with_sources([
            Some(PathBuf::from("left")),
            Some(PathBuf::from("right")),
        ]);
        browser.expanded[0].insert(PathBuf::from("kept"));
        browser.expanded[1].insert(PathBuf::from("replaced"));
        browser.set_sources([
            Some(PathBuf::from("left")),
            Some(PathBuf::from("new-right")),
        ]);
        assert!(browser.expanded[0].contains(&PathBuf::from("kept")));
        assert!(browser.expanded[1].is_empty());
        assert_eq!(browser.source_roots()[0], Some(PathBuf::from("left")));
        assert_eq!(browser.source_roots()[1], Some(PathBuf::from("new-right")));
    }

    #[test]
    fn changing_sources_discards_old_overlay_and_search_results() {
        let mut browser = FolderBrowser::new_with_sources([
            Some(PathBuf::from("left")),
            Some(PathBuf::from("right")),
        ]);
        browser
            .overlays
            .insert(PathBuf::from("old.txt"), DirectoryEntryState::Different);
        browser.overlay_paths.insert(PathBuf::from("old.txt"));
        let (sender, receiver) = mpsc::channel();
        browser.overlay_job = Some(OverlayJob {
            receiver,
            cancel: Arc::new(AtomicBool::new(false)),
            signatures: BTreeMap::new(),
        });
        browser.set_sources([
            Some(PathBuf::from("new-left")),
            Some(PathBuf::from("right")),
        ]);
        assert!(browser.overlay_job.is_none());
        assert!(browser.retired_overlay_jobs.is_empty());
        assert!(browser.overlays.is_empty());
        assert!(browser.overlay_paths.is_empty());
        drop(sender);
    }

    #[test]
    fn setting_identical_sources_preserves_overlay_state() {
        let sources = [Some(PathBuf::from("left")), Some(PathBuf::from("right"))];
        let mut browser = FolderBrowser::new_with_sources(sources.clone());
        browser
            .overlays
            .insert(PathBuf::from("kept.txt"), DirectoryEntryState::Different);
        browser.set_sources(sources);
        assert_eq!(
            browser.overlays.get(std::path::Path::new("kept.txt")),
            Some(&DirectoryEntryState::Different)
        );
    }

    #[test]
    fn search_and_error_apis_do_not_change_working_state() {
        let mut browser = FolderBrowser::new_with_sources([None, None]);
        browser.open_search();
        assert!(browser.search_open);
        browser.error = Some("metadata failure".into());
        assert_eq!(browser.error_details(), ["metadata failure"]);
        browser.clear_error();
        assert!(browser.error_details().is_empty());
        assert!(browser.working.is_empty());
    }

    #[test]
    fn overlay_compares_staged_text_without_saving_to_disk() {
        let root = std::env::temp_dir().join(format!("versus-overlay-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let left_root = root.join("left");
        let right_root = root.join("right");
        fs::create_dir_all(&left_root).unwrap();
        fs::create_dir_all(&right_root).unwrap();
        let left_path = left_root.join("item.txt");
        let right_path = right_root.join("item.txt");
        fs::write(&left_path, "left\n").unwrap();
        fs::write(&right_path, "right\n").unwrap();
        let mut left = load_working_file(Some(left_path.as_path())).unwrap();
        let right = load_working_file(Some(right_path.as_path())).unwrap();
        left.text = "right\n".into();
        let relative = PathBuf::from("item.txt");
        let results = compare_overlay_files(
            [left_root, right_root],
            [relative.clone()].into_iter().collect(),
            [(relative.clone(), [Some(left), Some(right)])]
                .into_iter()
                .collect(),
            BTreeMap::new(),
            [false, false, false],
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(results[&relative], DirectoryEntryState::Same);
        assert_eq!(fs::read_to_string(left_path).unwrap(), "left\n");
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn working_copy_overlay_keeps_a_non_file_peer_as_type_mismatch() {
        let root =
            std::env::temp_dir().join(format!("versus-overlay-mismatch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let roots = [root.join("left"), root.join("right")];
        fs::create_dir_all(&roots[0]).unwrap();
        fs::create_dir_all(roots[1].join("item")).unwrap();
        fs::write(roots[0].join("item"), "text\n").unwrap();
        let mut left = load_working_file(Some(&roots[0].join("item"))).unwrap();
        left.text = "edited\n".into();
        let relative = PathBuf::from("item");
        let states = compare_overlay_files(
            roots,
            [relative.clone()].into_iter().collect(),
            [(relative.clone(), [Some(left), None])]
                .into_iter()
                .collect(),
            BTreeMap::new(),
            [false; 3],
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(states[&relative], DirectoryEntryState::TypeMismatch);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn overlay_cache_ignores_show_only_but_rechecks_after_save() {
        let path = PathBuf::from("item.txt");
        let file = WorkingFile {
            identity: None,
            path: Some(path.clone()),
            text: "edited\n".into(),
            encoding: TextEncoding::Utf8 { bom: false },
            fingerprint: FileFingerprint::Bytes(b"saved\n".to_vec()),
        };
        let files = [Some(file), None];
        let ignored = BTreeMap::new();
        let revisions = [(files[0].as_ref().unwrap().identity_key(), 1)]
            .into_iter()
            .collect();
        let dirty = overlay_signature(
            &path,
            Some(&files),
            &ignored,
            &revisions,
            &[false, false, false],
            true,
        );
        let show_only = overlay_signature(
            &path,
            Some(&files),
            &ignored,
            &revisions,
            &[true, false, false],
            true,
        );
        let saved = overlay_signature(
            &path,
            Some(&files),
            &ignored,
            &revisions,
            &[false, false, false],
            false,
        );
        assert_eq!(dirty, show_only);
        assert_ne!(dirty, saved);
    }

    #[test]
    fn disabling_run_diff_cancels_overlay_and_clears_statuses() {
        let roots = [PathBuf::from("left-root"), PathBuf::from("right-root")];
        let mut browser = FolderBrowser::new(roots);
        browser
            .overlays
            .insert(PathBuf::from("changed.txt"), DirectoryEntryState::Different);
        browser.overlay_paths.insert(PathBuf::from("changed.txt"));
        let (sender, receiver) = mpsc::channel();
        let cancellation = Arc::new(AtomicBool::new(false));
        browser.overlay_job = Some(OverlayJob {
            receiver,
            cancel: cancellation.clone(),
            signatures: BTreeMap::new(),
        });
        let ctx = egui::Context::default();
        browser.poll_comparisons(&InteractiveView::default(), &ctx, true);
        assert!(browser.overlay_job.is_none());
        assert!(cancellation.load(Ordering::Relaxed));
        assert_eq!(browser.retired_overlay_jobs.len(), 1);
        assert!(browser.overlays.is_empty());
        assert!(browser.overlay_paths.is_empty());
        // A late result from the cancelled generation must never reappear on
        // re-enabling comparison, even when the browser was hidden meanwhile.
        sender
            .send((
                0,
                Ok(
                    [(PathBuf::from("changed.txt"), DirectoryEntryState::Different)]
                        .into_iter()
                        .collect(),
                ),
            ))
            .unwrap();
        drop(sender);
        let mut view = InteractiveView::default();
        view.set_editing(true, [false; 3]);
        view.set_run_diff(true);
        browser.poll_comparisons(&view, &ctx, true);
        browser.overlay_not_before = Some(Instant::now());
        browser.poll_comparisons(&view, &ctx, true);
        assert!(browser.overlays.is_empty());
        assert!(browser.overlay_job.is_none());
    }

    #[test]
    fn file_search_save_drains_after_a_synchronous_folder_transition() {
        let root = std::env::temp_dir().join(format!(
            "versus-file-search-save-transition-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let path = root.join("item.txt");
        let peer = root.join("peer.txt");
        fs::write(&path, "before\n").unwrap();
        fs::write(&peer, "peer\n").unwrap();
        let ctx = egui::Context::default();
        let mut view = InteractiveView::default();
        view.set_editing(true, [false; 3]);
        view.open_sources([Some(path.clone()), Some(peer.clone())]);
        let deadline = Instant::now() + Duration::from_secs(5);
        while view.working_text(0) != Some("before\n") {
            view.poll(&ctx);
            assert!(Instant::now() < deadline, "file did not load");
            std::thread::sleep(Duration::from_millis(1));
        }
        view.set_working_text_for_test(0, "saved before\n");
        assert_eq!(view.working_text(0), Some("saved before\n"));

        let mut file_search = FolderBrowser::new_search([Some(path.clone()), None]);
        file_search.begin_save(&view);
        assert!(file_search.is_saving());

        let mut app = VersusApp::default();
        app.merge_mode = true;
        app.interactive = Some(view);
        app.merge_file_search = Some(file_search);
        // This is the source transition that previously dropped the search
        // owner's receiver before it could publish the save result.
        app.source_paths[0] = Some(root.clone());
        app.source_kinds[0] = Some(SourceKind::Folder);
        app.source_paths[1] = Some(peer);
        app.source_kinds[1] = Some(SourceKind::File);
        app.open_interactive_sources();
        assert!(app.merge_folders.is_some());
        assert!(app.merge_file_search.as_ref().unwrap().is_saving());

        let deadline = Instant::now() + Duration::from_secs(5);
        while app
            .merge_file_search
            .as_ref()
            .is_some_and(FolderBrowser::is_saving)
        {
            app.poll_merge_save_owners(&ctx);
            assert!(Instant::now() < deadline, "file-search save did not drain");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "saved before\n");
        assert!(!app.interactive.as_ref().unwrap().is_saving());
        assert!(!app.interactive.as_ref().unwrap().any_dirty());
        let _ = fs::remove_dir_all(root);
    }
}
