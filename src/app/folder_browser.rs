//! Folder merge orchestration and stable app-facing browser interface.
//!
//! Capability modules own navigation, staged comparisons, modal search, and saves.
use crate::app::visuals::footer::header_status_label;
use crate::app::visuals::theme::Palette;
mod navigation;
mod overlay;
mod replacement;
mod save;
mod search_modal;
mod search_state;
#[cfg(test)]
#[path = "../../tests/unit/app/folder_browser/mod.rs"]
mod tests;

use super::*;
use navigation::ListingJob;
use overlay::{MergedFolderCache, OverlayJob};
use save::SaveJob;
use search_state::SearchState;
use std::collections::{BTreeMap, BTreeSet};
use versus::{DirectoryEntryState, LazyDirectoryItem, WorkingFile};

pub(super) struct FolderBrowser {
    sources: [Option<PathBuf>; 2],
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
}

impl FolderBrowser {
    /// Coordinates frame-level capability work before either folder pane paints.
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

    /// Draws one shared activity status without duplicating it in narrow panes.
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
}
