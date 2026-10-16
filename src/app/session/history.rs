//! Recent-comparison persistence.
//!
//! `RecentStore` serializes history IO so the UI only publishes completed
//! snapshots during [`Session::poll`](super::Session::poll).

use super::Session;
use crate::recent::{RecentComparison, RecentComparisons};
use eframe::egui;
use std::{path::PathBuf, sync::mpsc};

pub(in crate::app) struct RecentStore {
    pub(in crate::app) sender: Option<mpsc::Sender<RecentComparison>>,
    pub(in crate::app) receiver: mpsc::Receiver<Result<Option<RecentComparisons>, String>>,
    pub(in crate::app) worker: Option<std::thread::JoinHandle<()>>,
}

impl RecentStore {
    pub(in crate::app) fn start(ctx: &egui::Context, history_path: Option<PathBuf>) -> Self {
        let ctx = ctx.clone();
        let (sender, requests) = mpsc::channel::<RecentComparison>();
        let (results, receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let loaded = match &history_path {
                Some(path) => RecentComparisons::load_from(path),
                None => RecentComparisons::load(),
            };
            let mut recent = match loaded {
                Ok(recent) => {
                    let _ = results.send(Ok(Some(recent.clone())));
                    recent
                }
                Err(error) => {
                    let _ = results.send(Err(format!("Cannot load recent comparisons: {error}")));
                    RecentComparisons::default()
                }
            };
            ctx.request_repaint();
            for entry in requests {
                recent.record(entry.mode, entry.paths);
                let saved = match &history_path {
                    Some(path) => recent.save_to(path),
                    None => recent.save(),
                };
                let result = saved
                    .map(|()| None)
                    .map_err(|error| format!("Cannot save recent comparisons: {error}"));
                let _ = results.send(result);
                ctx.request_repaint();
            }
        });
        Self {
            sender: Some(sender),
            receiver,
            worker: Some(worker),
        }
    }
}

impl Drop for RecentStore {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Session {
    pub(in crate::app) fn record_recent(
        &mut self,
        mode: crate::selection::ComparisonMode,
        paths: [PathBuf; 2],
    ) {
        if paths.iter().any(|path| super::empty_diff_side(path)) {
            return;
        }
        self.recent.record(mode, paths.clone());
        if let Some(sender) = self
            .recent_store
            .as_ref()
            .and_then(|store| store.sender.as_ref())
        {
            if sender.send(RecentComparison { mode, paths }).is_err() {
                self.recent_message =
                    Some("Cannot save recent comparisons: the history worker stopped.".into());
            }
        }
    }

    pub(in crate::app) fn poll_recent(&mut self, ctx: &egui::Context) {
        let Some(store) = &self.recent_store else {
            return;
        };
        while let Ok(result) = store.receiver.try_recv() {
            match result {
                Ok(Some(mut saved)) => {
                    for entry in self.recent.entries().iter().rev() {
                        saved.record(entry.mode, entry.paths.clone());
                    }
                    self.recent = saved;
                    ctx.request_repaint();
                }
                Ok(None) => self.recent_message = None,
                Err(error) => self.recent_message = Some(error),
            }
        }
    }
}
