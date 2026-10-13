//! Metadata-only folder navigation and explicit recursive find/replace.
use super::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    hash::{Hash, Hasher},
    ops::Range,
};
use versus::{
    DirectoryEntryState, FileViewOptions, LazyDirectoryItem, SearchPreview, SearchQuery,
    SearchResults, SearchScope, WorkingFile, compare_loaded_text, load_working_file,
};

struct ListingJob {
    receiver: Receiver<Result<Vec<LazyDirectoryItem>, String>>,
}
struct SearchJob {
    receiver: Receiver<Result<SearchResults, String>>,
    cancel: Arc<AtomicBool>,
}
struct OverlayJob {
    receiver: Receiver<(u64, Result<BTreeMap<PathBuf, DirectoryEntryState>, String>)>,
    cancel: Arc<AtomicBool>,
    /// Input signatures covered by this worker.  They are only committed to
    /// the cache once the worker completes for the current generation.
    signatures: BTreeMap<PathBuf, u64>,
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
    query: SearchQuery,
    searched_query: Option<SearchQuery>,
    replacement: String,
    scope: SearchScope,
    search: Option<SearchJob>,
    results: Option<SearchResults>,
    selected_hits: BTreeSet<u64>,
    preview: Option<SearchPreview>,
    working: BTreeMap<PathBuf, WorkingFile>,
    pending_save: bool,
    save: Option<SaveJob>,
}

impl FolderBrowser {
    pub(super) fn new(roots: [PathBuf; 2]) -> Self {
        Self::new_with_sources(roots.map(Some))
    }
    pub(super) fn new_with_sources(sources: [Option<PathBuf>; 2]) -> Self {
        let mut view = Self {
            sources,
            listings: std::array::from_fn(|_| BTreeMap::new()),
            jobs: std::array::from_fn(|_| BTreeMap::new()),
            overlays: BTreeMap::new(),
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
            query: SearchQuery::plain(""),
            searched_query: None,
            replacement: String::new(),
            scope: SearchScope::Both,
            search: None,
            results: None,
            selected_hits: BTreeSet::new(),
            preview: None,
            working: BTreeMap::new(),
            pending_save: false,
            save: None,
        };
        for side in 0..2 {
            view.load_side(side, PathBuf::new());
        }
        view
    }
    pub(super) fn set_sources(&mut self, sources: [Option<PathBuf>; 2]) {
        let changed = self.sources != sources;
        if !changed {
            return;
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
                if self.sources[side].is_some() {
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
        self.overlay_paths.clear();
        self.overlay_cache.clear();
        self.search = None;
        self.results = None;
        self.preview = None;
        self.selected_hits.clear();
        self.searched_query = None;
    }
    pub(super) fn source_roots(&self) -> [Option<PathBuf>; 2] {
        self.sources.clone()
    }
    pub(super) fn open_search(&mut self) {
        self.search_open = true;
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
        if let Some(job) = &self.search {
            match job.receiver.try_recv() {
                Ok(result) => {
                    self.search = None;
                    match result {
                        Ok(results) => {
                            self.selected_hits = results
                                .files
                                .iter()
                                .flat_map(|f| f.hits.iter().map(|h| h.id))
                                .collect();
                            self.results = Some(results);
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.search = None;
                    self.error = Some("Search worker stopped".into());
                }
                _ => {}
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
            return;
        }
        // A folder diff has meaning only with two real directory sources.
        // Single-source navigation remains fully available without it.
        let Some(roots) = self.paired_roots() else {
            self.overlays.clear();
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
            let result =
                compare_overlay_files(roots, requested, loaded, ignored, options, &worker_cancel);
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
        &self,
        base: &FolderTree,
        view: &InteractiveView,
    ) -> Vec<PathBuf> {
        if !view.run_diff_enabled() {
            return Vec::new();
        }
        let states = merged_folder_states(Some(base), &self.overlays);
        base.all_rows()
            .into_iter()
            .filter(|row| !row.node.is_directory())
            .filter(|row| {
                row.node.left.kind == Some(DirectoryEntryKind::File)
                    || row.node.right.kind == Some(DirectoryEntryKind::File)
            })
            .filter_map(|row| {
                (states.get(&row.node.relative_path) != Some(&DirectoryEntryState::Same))
                    .then_some(row.node.relative_path.clone())
            })
            .collect()
    }
    pub(super) fn render_controls(&mut self, ui: &mut egui::Ui, view: &mut InteractiveView) {
        self.poll(ui.ctx());
        self.poll_save(view, ui.ctx());
        self.update_overlays(view, ui.ctx());
        self.search_window(ui.ctx(), view);
    }
    /// Draws one status in the bottom-right status area. Controls intentionally do
    /// not duplicate this text so narrow folder views retain usable actions.
    pub(super) fn render_header_status(&self, ui: &mut egui::Ui) -> bool {
        let palette = Palette::for_context(ui.ctx());
        if let Some(error) = &self.error {
            header_status_label(ui, error, palette.error);
        } else if let Some(save) = &self.save {
            let percent = if save.total == 0 {
                100
            } else {
                save.completed.min(save.total) * 100 / save.total
            };
            header_status_label(ui, &format!("Saving files… {percent}%"), palette.text);
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
            merged_folder_states(compared, &self.overlays)
        } else {
            BTreeMap::new()
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
        let mut visible = true;
        egui::Window::new("Folder Find and Replace")
            .open(&mut visible)
            .default_width(700.0)
            .show(ctx, |ui| {
                let editing_field=ui.memory(|memory|matches!(memory.focused(),Some(id)if id==egui::Id::new("folder-find-pattern")||id==egui::Id::new("folder-replacement-text")));
                if !editing_field{
                    if ui.input_mut(|input|input.consume_key(egui::Modifiers::COMMAND|egui::Modifiers::SHIFT,egui::Key::Z)||input.consume_key(egui::Modifiers::CTRL,egui::Key::Y)){self.undo_batch(view,true);}
                    else if ui.input_mut(|input|input.consume_key(egui::Modifiers::COMMAND,egui::Key::Z)||input.consume_key(egui::Modifiers::CTRL,egui::Key::Z)){self.undo_batch(view,false);}
                }
                ui.horizontal(|ui| {
                    ui.label("Find");
                    ui.add(egui::TextEdit::singleline(&mut self.query.pattern).id(egui::Id::new("folder-find-pattern")));
                });
                ui.horizontal(|ui| {
                    ui.label("Replace");
                    ui.add(egui::TextEdit::singleline(&mut self.replacement).id(egui::Id::new("folder-replacement-text")));
                });
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.scope, SearchScope::Left, "Left only");
                    ui.selectable_value(&mut self.scope, SearchScope::Right, "Right only");
                    ui.selectable_value(&mut self.scope, SearchScope::Both, "Both");
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.query.case_sensitive, "Case sensitive");
                    ui.checkbox(&mut self.query.whole_word, "Whole word");
                    ui.checkbox(&mut self.query.regex, "Regular expression");
                });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            self.search.is_none() && !self.query.pattern.is_empty(),
                            egui::Button::new("Find"),
                        )
                        .clicked()
                    {
                        self.working = view.export_working_files();
                        let roots = self.sources.clone();
                        let scope = self.scope;
                        let query = self.query.clone();
                        let working = self.working.clone();
                        let cancel = Arc::new(AtomicBool::new(false));
                        let cancellation = cancel.clone();
                        let (sender, receiver) = mpsc::channel();
                        std::thread::spawn(move || {
                            let result = versus::search_roots(
                                &roots,
                                scope,
                                &query,
                                Some(&cancellation),
                                &working,
                            )
                            .map_err(|e| e.to_string());
                            let _ = sender.send(result);
                        });
                        self.search = Some(SearchJob { receiver, cancel });
                        self.searched_query = Some(self.query.clone());
                        self.results = None;
                        self.preview = None;
                    }
                    if self.search.is_some() {
                        ui.spinner();
                        if ui.button("Cancel search").clicked() {
                            self.search = None;
                        }
                    }
                    if ui
                        .add_enabled(
                            self.results.is_some(),
                            egui::Button::new("Preview selected replacements"),
                        )
                        .clicked()
                    {
                        self.working = view.export_working_files();
                        if let (Some(results), Some(query)) = (&self.results, &self.searched_query)
                        {
                            match versus::preview_replace(
                                results,
                                &self.selected_hits,
                                query,
                                &self.replacement,
                                &self.working,
                            ) {
                                Ok(preview) => self.preview = Some(preview),
                                Err(e) => self.error = Some(e.to_string()),
                            }
                        }
                    }
                    if ui
                        .add_enabled(view.can_undo_batch(), egui::Button::new("Undo batch"))
                        .clicked()
                    {
                        self.undo_batch(view, false);
                    }
                    if ui
                        .add_enabled(view.can_redo_batch(), egui::Button::new("Redo batch"))
                        .clicked()
                    {
                        self.undo_batch(view, true);
                    }
                });
                if let Some(results) = &self.results {
                    ui.label(format!(
                        "{} files with matches · {} binary/oversized files skipped",
                        results.files.len(),
                        results.skipped_binary.len()
                    ));
                    egui::ScrollArea::vertical()
                        .max_height(220.0)
                        .show(ui, |ui| {
                            for file in &results.files {
                                let mut enabled = file
                                    .hits
                                    .iter()
                                    .any(|hit| self.selected_hits.contains(&hit.id));
                                if ui
                                    .checkbox(
                                        &mut enabled,
                                        format!(
                                            "{} · {}",
                                            if file.side == 0 { "Left" } else { "Right" },
                                            file.path.display()
                                        ),
                                    )
                                    .changed()
                                {
                                    for hit in &file.hits {
                                        if enabled {
                                            self.selected_hits.insert(hit.id);
                                        } else {
                                            self.selected_hits.remove(&hit.id);
                                        }
                                    }
                                }
                                ui.indent(&file.path, |ui| {
                                    for hit in &file.hits {
                                        let mut selected = self.selected_hits.contains(&hit.id);
                                        if ui
                                            .checkbox(
                                                &mut selected,
                                                format!("Line {}: {}", hit.line, hit.matched),
                                            )
                                            .changed()
                                        {
                                            if selected {
                                                self.selected_hits.insert(hit.id);
                                            } else {
                                                self.selected_hits.remove(&hit.id);
                                            }
                                        }
                                    }
                                });
                            }
                        });
                }
                if let Some(preview) = &self.preview {
                    ui.separator();
                    ui.label("Preview — files are changed in memory until explicitly saved");
                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .show(ui, |ui| {
                            for change in &preview.changes {
                                egui::CollapsingHeader::new(change.path.display().to_string())
                                    .show(ui, |ui| {
                                        ui.columns(2, |cols| {
                                            cols[0]
                                                .label(RichText::new(&change.before).monospace());
                                            cols[1].label(RichText::new(&change.after).monospace());
                                        });
                                    });
                            }
                        });
                }
                if ui
                    .add_enabled(
                        self.preview.as_ref().is_some_and(|p| !p.changes.is_empty()),
                        egui::Button::new("Apply preview to working copies"),
                    )
                    .clicked()
                {
                    let preview = self.preview.take().unwrap();
                    self.working = view.export_working_files();
                    match view.apply_search_preview(&preview){
                        Ok(_)=>self.working=view.export_working_files(),Err(error)=>self.error=Some(error)
                    }
                }
                let dirty = pending_save_files(&self.working).len();
                ui.label(format!("{dirty} files have unsaved replacements"));
                if ui
                    .add_enabled(
                        dirty > 0 && !self.is_saving() && !view.is_saving(),
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
                    egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                        for (path, file, _) in pending_save_files(&self.working) {
                            egui::CollapsingHeader::new(path.display().to_string()).show(ui, |ui| {
                                ui.columns(2, |columns| {
                                    columns[0].label("Loaded file");
                                    columns[0].label(RichText::new(file.baseline_text().unwrap_or_default()).monospace());
                                    columns[1].label("Pending content");
                                    columns[1].label(RichText::new(&file.text).monospace());
                                });
                            });
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
        self.search_open = visible;
    }
    fn undo_batch(&mut self, view: &mut InteractiveView, redo: bool) {
        match view.undo_batch(redo) {
            Ok(()) => self.working = view.export_working_files(),
            Err(error) => self.error = Some(error),
        }
    }
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

fn merged_folder_states(
    tree: Option<&FolderTree>,
    overlays: &BTreeMap<PathBuf, DirectoryEntryState>,
) -> BTreeMap<PathBuf, DirectoryEntryState> {
    let Some(tree) = tree else {
        return overlays.clone();
    };
    let rows = tree.all_rows();
    let mut states: BTreeMap<_, _> = rows
        .iter()
        .map(|row| (row.node.relative_path.clone(), row.node.state.clone()))
        .collect();
    states.extend(
        overlays
            .iter()
            .map(|(path, state)| (path.clone(), state.clone())),
    );

    // FolderTree owns the baseline directory aggregation. Reapply it after
    // replacing a leaf with its unsaved working-copy comparison so parent
    // badges continue to describe their visible descendants.
    let mut directories: Vec<_> = rows
        .iter()
        .filter(|row| row.node.is_directory())
        .map(|row| row.node.relative_path.clone())
        .collect();
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        let Some(state) = states.get(&directory).cloned() else {
            continue;
        };
        if matches!(
            state,
            DirectoryEntryState::Error(_)
                | DirectoryEntryState::TypeMismatch
                | DirectoryEntryState::LeftOnly
                | DirectoryEntryState::RightOnly
        ) {
            continue;
        }
        let different_child = states.iter().any(|(path, child)| {
            path.parent() == Some(directory.as_path()) && *child != DirectoryEntryState::Same
        });
        states.insert(
            directory,
            if different_child {
                DirectoryEntryState::Different
            } else {
                DirectoryEntryState::Same
            },
        );
    }
    states
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
}
