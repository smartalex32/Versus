//! Result actions and atomic text/conflict history.

use super::*;

impl GitMergeModel {
    /// Replaces a mapped region with immutable source content. A caller must
    /// supply the revision it rendered, preventing stale coordinates from
    /// overwriting unrelated Result bytes.
    pub fn choose(
        &mut self,
        id: usize,
        revision: u64,
        choice: MergeChoice,
    ) -> Result<(), GitMergeError> {
        self.require_revision(revision)?;
        if self.malformed_markers {
            return Err(GitMergeError::MalformedConflictMarkers);
        }
        if !self.mapping_available {
            return Err(GitMergeError::MappingUnavailable);
        }
        let region = self
            .regions
            .iter()
            .find(|region| region.public.id == id)
            .cloned()
            .ok_or(GitMergeError::RegionUnavailable(id))?;
        let (result_range, replacement, resolved) = match region.kind {
            RegionKind::Regular => {
                let replacement = self.choice_text(
                    &region.public,
                    choice,
                    &self.result[region.public.result.clone()],
                );
                (region.public.result, replacement, None)
            }
            RegionKind::Conflicts(conflict_ids) => {
                let replacement = self.choice_text(
                    &region.public,
                    choice,
                    &self.result[region.public.result.clone()],
                );
                (region.public.result, replacement, Some(conflict_ids))
            }
        };
        let mut next = self.result.clone();
        next.replace_range(result_range.clone(), &replacement);
        if next.len() > crate::core::DEFAULT_TEXT_SIZE_LIMIT as usize {
            return Err(GitMergeError::InputTooLarge);
        }
        let replacement_envelope = result_range.start..result_range.start + replacement.len();
        self.set_result(next);
        if let Some(conflict_ids) = resolved {
            for conflict in &mut self.conflicts {
                if conflict_ids.contains(&conflict.id) {
                    conflict.envelope = replacement_envelope.clone();
                    conflict.resolved = true;
                }
            }
        }
        self.commit();
        Ok(())
    }

    /// Explicitly accepts a manually edited conflict after its wrapper has
    /// been removed. Marker-bearing wrappers must be resolved by a source
    /// choice or direct editing first.
    pub fn mark_resolved(&mut self, id: usize, revision: u64) -> Result<(), GitMergeError> {
        self.require_revision(revision)?;
        if self.malformed_markers {
            return Err(GitMergeError::MalformedConflictMarkers);
        }
        let region = self
            .regions
            .iter()
            .find(|region| region.public.id == id)
            .ok_or(GitMergeError::UnknownRegion(id))?;
        let RegionKind::Conflicts(conflict_ids) = &region.kind else {
            return Err(GitMergeError::RegionUnavailable(id));
        };
        if conflict_ids.iter().any(|id| {
            self.conflicts
                .iter()
                .any(|conflict| conflict.id == *id && conflict.wrapper.is_some())
        }) {
            return Err(GitMergeError::ConflictStillHasMarkers(id));
        }
        for conflict in &mut self.conflicts {
            if conflict_ids.contains(&conflict.id) {
                conflict.resolved = true;
            }
        }
        self.commit();
        Ok(())
    }

    /// Applies arbitrary manual Result text. Incomplete marker edits are valid
    /// while typing; they disable finish and mapped actions until repaired.
    pub fn edit_result(&mut self, result: String) -> Result<(), GitMergeError> {
        if result.len() > crate::core::DEFAULT_TEXT_SIZE_LIMIT as usize {
            return Err(GitMergeError::InputTooLarge);
        }
        if result == self.result {
            return Ok(());
        }
        self.set_result(result);
        self.commit();
        Ok(())
    }

    /// Restores the supplied Base as one history operation and reopens every
    /// conflict known when this invocation began.
    pub fn restore_base(&mut self) -> Result<(), GitMergeError> {
        let base = self.base.clone().ok_or(GitMergeError::BaseUnavailable)?;
        self.set_result(base);
        for conflict in &mut self.conflicts {
            conflict.resolved = false;
        }
        self.commit();
        Ok(())
    }

    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.history.undo() else {
            return false;
        };
        self.restore_snapshot(snapshot);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.history.redo() else {
            return false;
        };
        self.restore_snapshot(snapshot);
        true
    }

    fn choice_text(&self, region: &MergeRegion, choice: MergeChoice, result_text: &str) -> String {
        let incoming = &self.incoming[region.incoming.clone()];
        let current = &self.current[region.current.clone()];
        choose_text(incoming, current, choice, result_text)
    }

    fn require_revision(&self, revision: u64) -> Result<(), GitMergeError> {
        (revision == self.revision)
            .then_some(())
            .ok_or(GitMergeError::StaleRegion {
                expected: revision,
                actual: self.revision,
            })
    }

    fn commit(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.regenerate_regions();
        self.history.push(Snapshot {
            result: self.result.clone(),
            conflicts: self.conflicts.clone(),
        });
    }

    fn restore_snapshot(&mut self, snapshot: Snapshot) {
        self.result = snapshot.result;
        self.conflicts = snapshot.conflicts;
        self.revision = self.revision.wrapping_add(1);
        self.regenerate_regions();
    }
}

fn choose_text(incoming: &str, current: &str, choice: MergeChoice, result: &str) -> String {
    match choice {
        MergeChoice::Incoming => incoming.to_owned(),
        MergeChoice::Current => current.to_owned(),
        MergeChoice::IncomingThenCurrent => join(incoming, current, result),
        MergeChoice::CurrentThenIncoming => join(current, incoming, result),
    }
}

fn join(first: &str, second: &str, result: &str) -> String {
    if first.is_empty()
        || second.is_empty()
        || first.ends_with('\n')
        || first.ends_with('\r')
        || second.starts_with('\n')
        || second.starts_with('\r')
    {
        return format!("{first}{second}");
    }
    let separator = if result.contains("\r\n") {
        "\r\n"
    } else if result.contains('\r') {
        "\r"
    } else {
        "\n"
    };
    format!("{first}{separator}{second}")
}
