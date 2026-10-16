//! Explicit retained-working-copy save lifecycle for folder replacement edits.

use super::*;

enum SaveEvent {
    Saved {
        path: PathBuf,
        saved_text: String,
        result: Result<WorkingFile, String>,
    },
    Complete,
}

pub(super) struct SaveJob {
    receiver: Receiver<SaveEvent>,
    pub(super) total: usize,
    pub(super) completed: usize,
}

impl FolderBrowser {
    pub(in crate::app) fn begin_save(&mut self, view: &InteractiveView) {
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
    pub(in crate::app) fn poll_save(&mut self, view: &mut InteractiveView, ctx: &egui::Context) {
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
}
pub(super) fn pending_save_files(
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
pub(super) fn retain_saved_snapshot(
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
