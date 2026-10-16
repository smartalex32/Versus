//! Search replacement planning and batch-history coordination.

use super::*;
use search_state::scope_includes;
use versus::SearchPreview;

impl FolderBrowser {
    pub(super) fn schedule_search(&mut self, now: f64) {
        self.error = None;
        self.search_state.schedule(now);
    }
    pub(super) fn run_scheduled_search(
        &mut self,
        ctx: &egui::Context,
        view: &InteractiveView,
        now: f64,
    ) {
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
    pub(super) fn start_search(&mut self, view: &InteractiveView) {
        self.error = None;
        self.working = view.export_working_files();
        self.search_state
            .start(self.sources.clone(), self.working.clone());
    }
    pub(super) fn replacement_plan(
        &mut self,
        view: &InteractiveView,
        next: bool,
    ) -> Option<SearchPreview> {
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
    pub(super) fn replace_matches(&mut self, view: &mut InteractiveView, next: bool) {
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

    pub(super) fn undo_batch(&mut self, view: &mut InteractiveView, redo: bool) {
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
