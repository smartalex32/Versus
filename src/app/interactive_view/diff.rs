//! Cancellable two-way comparison lifecycle and result publication.

use super::*;

impl InteractiveView {
    pub(super) fn cache_active(&mut self) {
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
    pub(super) fn poll_diff(&mut self, ctx: &egui::Context) {
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
}
