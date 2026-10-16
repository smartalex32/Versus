//! Stateful ownership for the folder Find and Replace session.
//!
//! Directory navigation deliberately does not appear here.  This type owns the
//! query, debounced worker, streamed snapshots, and result selection; callers
//! provide roots and working-copy snapshots when beginning a search.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
};

use versus::{SearchEvent, SearchPreview, SearchQuery, SearchResults, SearchScope, WorkingFile};

pub(super) struct SearchJob {
    pub(super) receiver: Receiver<Result<SearchEvent, String>>,
    pub(super) cancel: Arc<AtomicBool>,
}

impl Drop for SearchJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The state behind one Find and Replace modal.
///
/// It always starts workers with `SearchScope::Both`; the selected replacement
/// scope and displayed side are local filters over that common result set.
pub(super) struct SearchState {
    pub(super) open: bool,
    pub(super) focus_find: bool,
    pub(super) query: SearchQuery,
    pub(super) searched_query: Option<SearchQuery>,
    pub(super) replacement: String,
    pub(super) scope: SearchScope,
    pub(super) results_side: usize,
    pub(super) due_at: Option<f64>,
    pub(super) job: Option<SearchJob>,
    pub(super) results: Option<SearchResults>,
    pub(super) selected_hits: BTreeSet<u64>,
}

impl SearchState {
    pub(super) fn new(sources: &[Option<PathBuf>; 2]) -> Self {
        Self {
            open: false,
            focus_find: false,
            query: SearchQuery::plain(""),
            searched_query: None,
            replacement: String::new(),
            scope: SearchScope::Both,
            results_side: usize::from(sources[0].is_none() && sources[1].is_some()),
            due_at: None,
            job: None,
            results: None,
            selected_hits: BTreeSet::new(),
        }
    }

    pub(super) fn open(&mut self) {
        if !self.open {
            self.focus_find = true;
            self.clear_results();
            self.due_at = (!self.query.pattern.is_empty()).then_some(0.0);
        }
        self.open = true;
    }

    pub(super) fn is_open(&self) -> bool {
        self.open
    }
    pub(super) fn close(&mut self) {
        self.open = false;
        self.due_at = None;
        if self.job.is_some() {
            if let Some(results) = &mut self.results {
                results.cancelled = true;
            }
        }
        self.job = None;
    }
    pub(super) fn focus_pending(&self) -> bool {
        self.focus_find
    }
    pub(super) fn take_focus(&mut self) -> bool {
        std::mem::take(&mut self.focus_find)
    }
    pub(super) fn query(&self) -> &SearchQuery {
        &self.query
    }
    pub(super) fn query_mut(&mut self) -> &mut SearchQuery {
        &mut self.query
    }
    pub(super) fn replacement_mut(&mut self) -> &mut String {
        &mut self.replacement
    }
    pub(super) fn replacement(&self) -> &str {
        &self.replacement
    }
    pub(super) fn scope(&self) -> SearchScope {
        self.scope
    }
    pub(super) fn set_scope(&mut self, scope: SearchScope) {
        self.scope = scope;
        if scope != SearchScope::Both {
            self.results_side = if scope == SearchScope::Left { 0 } else { 1 };
        }
    }
    pub(super) fn results_side(&self) -> usize {
        self.results_side
    }
    pub(super) fn set_results_side(&mut self, side: usize) {
        self.results_side = side;
    }
    pub(super) fn is_searching(&self) -> bool {
        self.job.is_some()
    }
    pub(super) fn is_waiting(&self) -> bool {
        self.due_at.is_some()
    }
    pub(super) fn selected_count(&self) -> usize {
        self.selected_hits.len()
    }
    pub(super) fn results(&self) -> Option<&SearchResults> {
        self.results.as_ref()
    }
    pub(super) fn selected(&self) -> &BTreeSet<u64> {
        &self.selected_hits
    }
    pub(super) fn remove_selected(&mut self, id: u64) {
        self.selected_hits.remove(&id);
    }
    pub(super) fn advance_after_replacement(
        &mut self,
        plan: &SearchPreview,
        working: &BTreeMap<PathBuf, WorkingFile>,
    ) -> Result<(), String> {
        let Some(results) = &mut self.results else {
            return Ok(());
        };
        super::super::search_session::advance_results(
            results,
            &mut self.selected_hits,
            plan,
            working,
        )
    }
    pub(super) fn replacement_allowed(&self) -> bool {
        self.results_current()
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
    }
    pub(super) fn match_counts(&self) -> [usize; 2] {
        let mut counts = [0; 2];
        if let Some(results) = &self.results {
            for file in &results.files {
                counts[file.side] += file
                    .hits
                    .iter()
                    .filter(|hit| self.selected_hits.contains(&hit.id))
                    .count();
            }
        }
        counts
    }
    pub(super) fn schedule(&mut self, now: f64) {
        self.clear_results();
        self.due_at = (!self.query.pattern.is_empty()).then_some(now + 0.25);
    }
    pub(super) fn start(
        &mut self,
        roots: [Option<PathBuf>; 2],
        working: BTreeMap<PathBuf, WorkingFile>,
    ) {
        self.due_at = None;
        self.clear_results();
        if self.query.pattern.is_empty() {
            return;
        }
        let query = self.query.clone();
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
        self.job = Some(SearchJob { receiver, cancel });
        self.searched_query = Some(self.query.clone());
    }
    pub(super) fn run_due(
        &mut self,
        now: f64,
        roots: [Option<PathBuf>; 2],
        working: BTreeMap<PathBuf, WorkingFile>,
    ) -> Option<f64> {
        match self.due_at {
            Some(due) if now >= due => {
                self.start(roots, working);
                None
            }
            Some(due) => Some(due - now),
            None => None,
        }
    }
    /// Drains a bounded stream slice and returns an error for the UI owner.
    pub(super) fn poll(&mut self) -> Option<String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(4);
        for _ in 0..16 {
            let Some(job) = &self.job else {
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
                        self.results
                            .get_or_insert_with(SearchResults::default)
                            .cancelled = true;
                        self.job = None;
                        return Some("Search worker sent matches without a file snapshot".into());
                    }
                }
                Ok(Ok(SearchEvent::Skipped(path))) => self
                    .results
                    .get_or_insert_with(SearchResults::default)
                    .skipped_binary
                    .push(path),
                Ok(Ok(SearchEvent::Complete { cancelled })) => {
                    self.results
                        .get_or_insert_with(SearchResults::default)
                        .cancelled = cancelled;
                    self.job = None;
                }
                Ok(Err(error)) => {
                    self.results
                        .get_or_insert_with(SearchResults::default)
                        .cancelled = true;
                    self.job = None;
                    return Some(error);
                }
                Err(TryRecvError::Disconnected) => {
                    self.results
                        .get_or_insert_with(SearchResults::default)
                        .cancelled = true;
                    self.job = None;
                    return Some("Search worker stopped".into());
                }
                Err(TryRecvError::Empty) => break,
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
        }
        None
    }
    pub(super) fn cancel(&mut self) {
        if let Some(results) = &mut self.results {
            results.cancelled = true;
        }
        self.job = None;
    }
    pub(super) fn reset_for_sources(&mut self, sources: &[Option<PathBuf>; 2]) {
        if self.scope == SearchScope::Left {
            self.results_side = 0;
        } else if self.scope == SearchScope::Right {
            self.results_side = 1;
        } else if sources.iter().filter(|source| source.is_some()).count() == 1 {
            self.results_side = usize::from(sources[1].is_some());
        }
        self.clear_results();
    }
    pub(super) fn invalidate_after_undo(&mut self) {
        self.clear_results();
        self.due_at = None;
    }
    pub(super) fn results_current(&self) -> bool {
        self.due_at.is_none()
            && self.job.is_none()
            && self
                .results
                .as_ref()
                .is_some_and(|results| !results.cancelled)
            && self.searched_query.as_ref() == Some(&self.query)
    }
    pub(super) fn clear_results(&mut self) {
        self.job = None;
        self.due_at = None;
        self.results = None;
        self.selected_hits.clear();
        self.searched_query = None;
    }
}

pub(super) fn scope_includes(scope: SearchScope, side: usize) -> bool {
    scope == SearchScope::Both
        || (side == 0 && scope == SearchScope::Left)
        || (side == 1 && scope == SearchScope::Right)
}
