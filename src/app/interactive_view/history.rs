//! History transactions and pending diff presentation after document changes.

use super::*;

impl InteractiveView {
    pub(in crate::app) fn apply_search_preview(
        &mut self,
        preview: &versus::SearchPreview,
    ) -> Result<u64, String> {
        self.publish_active();
        let id = self.next_batch + 1;
        let mut prepared = Vec::new();
        let mut unique = std::collections::BTreeSet::new();
        for change in &preview.changes {
            let base = preview
                .bases
                .get(&change.path)
                .ok_or_else(|| "Replacement is missing a file snapshot.".to_owned())?;
            let key = base.identity_key();
            let existing_key = self.key_for(&change.path);
            if self.documents.contains_key(&existing_key) && existing_key != key {
                return Err(format!(
                    "{} now resolves to a different file. Reopen the workspace before replacing it.",
                    change.path.display()
                ));
            }
            if !unique.insert(key.clone()) {
                return Err(
                    "Replacement contains duplicate file identities. Run Find again.".into(),
                );
            }
            let mut retained = self
                .documents
                .get(&key)
                .cloned()
                .unwrap_or_else(|| retained_file(base.clone()));
            if retained.document.text() != base.text
                || retained.file.fingerprint != base.fingerprint
                || retained.file.encoding != base.encoding
            {
                let mut working = retained.file.clone();
                working.text = retained.document.text().into();
                if retained.document.is_dirty() || working.is_dirty() || base.is_dirty() {
                    return Err(format!(
                        "{} changed since the search. Refresh search before replacing it.",
                        change.path.display()
                    ));
                }
                // Refresh only the staged clone. All participants must pass
                // their byte-range and snapshot checks before publication.
                // A separate history entry makes batch Undo return to fresh
                // disk text while retaining older editor history.
                if let Some(edit) = external_refresh_edit(retained.document.text(), &base.text) {
                    retained.document.apply_byte_edits(
                        HistoryTag::Local(retained.document.revision()), vec![edit],
                    ).map_err(|error| format!("{} cannot refresh content: {error}. Refresh search before replacing it.", change.path.display()))?;
                }
                retained.document.mark_saved_text(base.text.clone());
                let native_path = retained.file.path.clone();
                retained.file = base.clone();
                retained.file.path = native_path.or_else(|| Some(change.path.clone()));
            }
            if retained.document.text() != change.before {
                return Err(format!(
                    "{} changed since the search. Run Find again.",
                    change.path.display()
                ));
            }
            retained
                .document
                .apply_byte_edits(HistoryTag::FolderBatch(id), change.edits.clone())
                .map_err(|e| e.to_string())?;
            if retained.document.text() != change.after {
                return Err("Replacement does not match its selected edits.".into());
            }
            prepared.push((key, retained));
        }
        let participants = prepared.iter().map(|(key, _)| key.clone()).collect();
        for (key, retained) in prepared {
            if let Some(path) = &retained.file.path {
                self.aliases.insert(path.clone(), key.clone());
            }
            self.aliases.insert(key.clone(), key.clone());
            self.documents.insert(key, retained);
        }
        self.next_batch = id;
        self.batches.insert(id, participants);
        self.batch_undo.push(id);
        self.batch_redo.clear();
        self.refresh_active();
        self.invalidate_diff();
        Ok(id)
    }
    pub(super) fn batch_available(&self, id: u64, redo: bool) -> bool {
        self.batches.get(&id).is_some_and(|paths|paths.iter().all(|path|self.documents.get(path).is_some_and(|retained|if redo{retained.document.next_redo_tag()}else{retained.document.next_undo_tag()}==Some(HistoryTag::FolderBatch(id)))))
    }
    pub(super) fn move_batch(&mut self, id: u64, redo: bool) -> Result<(), String> {
        self.publish_active();
        if !self.batch_available(id, redo) {
            return Err(
                "Undo newer edits in the batch files before moving this replacement transaction."
                    .into(),
            );
        }
        let before = self.history_diff_snapshot();
        for path in &self.batches[&id] {
            let document = &mut self.documents.get_mut(path).unwrap().document;
            let moved = if redo {
                document.redo_expected(HistoryTag::FolderBatch(id))
            } else {
                document.undo_expected(HistoryTag::FolderBatch(id))
            };
            debug_assert!(moved);
        }
        if redo {
            self.batch_redo.retain(|current| *current != id);
            self.batch_undo.push(id);
        } else {
            self.batch_undo.retain(|current| *current != id);
            self.batch_redo.push(id);
        }
        self.refresh_active();
        self.invalidate_history_diff(before);
        Ok(())
    }
    pub(in crate::app) fn can_undo_batch(&self) -> bool {
        self.batch_undo
            .last()
            .is_some_and(|id| self.batch_available(*id, false))
    }
    pub(in crate::app) fn can_redo_batch(&self) -> bool {
        self.batch_redo
            .last()
            .is_some_and(|id| self.batch_available(*id, true))
    }
    pub(in crate::app) fn undo_batch(&mut self, redo: bool) -> Result<(), String> {
        self.publish_active();
        let id = if redo {
            self.batch_redo.last()
        } else {
            self.batch_undo.last()
        }
        .copied()
        .ok_or_else(|| "No replacement batch is available.".to_owned())?;
        self.move_batch(id, redo)
    }
    pub(super) fn move_history(&mut self, redo: bool) {
        self.publish_active();
        let active_paths: Vec<PathBuf> = self
            .paths
            .as_ref()
            .map(|paths| paths.to_vec())
            .unwrap_or_else(|| self.side_sources.iter().flatten().cloned().collect());
        let preferred_path = self
            .paths
            .as_ref()
            .map(|paths| paths[self.selected_side].clone())
            .or_else(|| self.side_sources[self.selected_side].clone());
        let Some(preferred_path) = preferred_path else {
            return;
        };
        let preferred = self.key_for(&preferred_path);
        let own = self.documents.get(&preferred).and_then(|retained| {
            if redo {
                retained.document.next_redo_tag()
            } else {
                retained.document.next_undo_tag()
            }
        });
        let fallback = self.workspace.as_ref().and_then(|w| {
            if redo {
                w.next_redo_tag()
            } else {
                w.next_undo_tag()
            }
        });
        let Some(tag) = own.or(fallback) else {
            return;
        };
        if let HistoryTag::FolderBatch(id) = tag {
            if let Err(error) = self.move_batch(id, redo) {
                self.error = Some(error);
            }
        } else {
            let key = if own.is_some() {
                preferred
            } else {
                active_paths.iter().map(|p|self.key_for(p)).find(|key|self.documents.get(key).is_some_and(|retained|if redo{retained.document.next_redo_tag()}else{retained.document.next_undo_tag()}==Some(tag))).unwrap()
            };
            let before = self.history_diff_snapshot();
            let document = &mut self.documents.get_mut(&key).unwrap().document;
            let moved = if redo {
                document.redo_expected(tag)
            } else {
                document.undo_expected(tag)
            };
            debug_assert!(moved);
            self.refresh_active();
            self.invalidate_history_diff(before);
        }
    }
    /// Editor and transfer paths call this after every mutation so an older
    /// background result cannot paint alignment for new working-copy text.
    pub(in crate::app) fn invalidate_diff(&mut self) {
        self.pending_navigation_scroll = [false; 2];
        self.publish_active();
        if !self.editing {
            if let (Some(cached), Some(diff)) = (&self.readonly_presentation, &self.diff) {
                let top = (self.scroll[0] / cached.row_height).floor() as usize;
                if let Some(row) = cached.rows.get(top).and_then(|index| diff.rows.get(*index)) {
                    self.readonly_pending_anchor = Some((
                        [
                            row.left.as_ref().map(|line| line.0),
                            row.right.as_ref().map(|line| line.0),
                        ],
                        self.scroll[0] % cached.row_height,
                    ));
                }
            }
        }
        self.readonly_presentation = None;
        if let Some(job) = self.diff_job.take() {
            job.cancel.store(true, Ordering::Relaxed);
            self.retired_diffs.push(job);
        }
        self.diff = None;
        self.presentation.clear();
        self.diff_rows.clear();
        if let Some(ctx) = &self.editor_context {
            ctx.memory_mut(|memory| {
                memory
                    .data
                    .remove::<usize>(egui::Id::new("interactive-diff-navigation"))
            });
        }
        self.diff_dirty = self.engine_enabled();
        self.diff_not_before = Some(Instant::now() + Duration::from_millis(150));
    }
    pub(super) fn merge_decoration(&self, side: usize) -> MergeDecoration {
        self.presentation
            .decoration(side, self.diff.as_ref(), self.merge_options[2])
    }
    pub(super) fn capture_merge_presentation(&self) -> Option<PendingMergePresentation> {
        (self.editing && self.run_diff && (self.diff.is_some() || self.presentation.has_pending()))
            .then(|| {
                self.presentation.capture(
                    self.diff.as_ref(),
                    self.merge_options[2],
                    self.footer_legend_counts(),
                    self.diff_rows.len(),
                )
            })
    }
    pub(super) fn history_diff_snapshot(&self) -> Option<[String; 2]> {
        (self.editing && self.run_diff && (self.diff.is_some() || self.presentation.has_pending()))
            .then(|| {
                self.workspace.as_ref().map(|workspace| {
                    std::array::from_fn(|side| workspace.text(side_of(side)).to_owned())
                })
            })
            .flatten()
    }
    pub(super) fn invalidate_history_diff(&mut self, before: Option<[String; 2]>) {
        let mut presentation = before
            .as_ref()
            .and_then(|_| self.capture_merge_presentation());
        let mut cursors = [None; 2];
        self.invalidate_diff();
        if let (Some(before), Some(presentation), Some(workspace)) =
            (before, &mut presentation, &self.workspace)
        {
            for side in 0..2 {
                let after = workspace.text(side_of(side));
                if let Some((range, replacement)) = external_refresh_edit(&before[side], after) {
                    cursors[side] = Some(range.start + replacement.len());
                    presentation.rebase(side, &before[side], range, after);
                }
            }
        }
        self.presentation.replace_pending(presentation);
        for (side, cursor) in cursors.into_iter().enumerate() {
            if let Some(cursor) = cursor {
                self.restore_editor_byte_cursor(side, cursor);
            }
        }
    }
    pub(super) fn invalidate_edited_diff(
        &mut self,
        side: usize,
        before: &str,
        range: Range<usize>,
    ) {
        let mut presentation = self.capture_merge_presentation();
        let shared_file = self.side_sources[0]
            .as_ref()
            .zip(self.side_sources[1].as_ref())
            .is_some_and(|(left, right)| self.key_for(left) == self.key_for(right));
        let peer_before = shared_file.then(|| {
            self.workspace
                .as_ref()
                .unwrap()
                .text(side_of(1 - side))
                .to_owned()
        });
        // Cancel stale workers and publish edits as usual, retaining only display
        // metadata. Actual comparison results remain invalid until the worker returns.
        self.invalidate_diff();
        if let (Some(presentation), Some(workspace)) = (&mut presentation, &self.workspace) {
            for target in 0..2 {
                let after = workspace.text(side_of(target));
                if target == side {
                    presentation.rebase(target, before, range.clone(), after);
                } else if shared_file {
                    if let Some(peer_before) = &peer_before {
                        if peer_before == before && after != peer_before {
                            presentation.rebase(target, before, range.clone(), after);
                        } else if let Some((peer_range, _)) =
                            external_refresh_edit(peer_before, after)
                        {
                            presentation.rebase(target, peer_before, peer_range, after);
                        }
                    }
                }
            }
        }
        self.presentation.replace_pending(presentation);
    }
}
