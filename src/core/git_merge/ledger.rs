//! Persistent conflict identities and conservative rebasing through edits.

use super::conflicts::ParsedConflict;
use super::*;

impl GitMergeModel {
    pub(super) fn sync_conflicts(&mut self, parsed: Vec<ParsedConflict>) {
        for conflict in &mut self.conflicts {
            conflict.wrapper = None;
        }
        for parsed in parsed {
            let matches: Vec<_> = self
                .conflicts
                .iter()
                .enumerate()
                .filter(|(_, conflict)| {
                    !conflict.resolved && ranges_overlap(&conflict.envelope, &parsed.wrapper)
                })
                .map(|(index, _)| index)
                .collect();
            if !matches.is_empty() {
                // A broad edit may conservatively overlap several original
                // conflicts. Keep their identities together instead of
                // inventing new IDs every time this text is re-parsed.
                for index in matches {
                    let conflict = &mut self.conflicts[index];
                    conflict.envelope.start = conflict.envelope.start.min(parsed.wrapper.start);
                    conflict.envelope.end = conflict.envelope.end.max(parsed.wrapper.end);
                    conflict.wrapper = Some(parsed.wrapper.clone());
                }
            } else {
                let id = self.conflicts.len();
                let envelope = parsed.wrapper.clone();
                self.conflicts.push(ConflictState {
                    id,
                    envelope,
                    wrapper: Some(parsed.wrapper),
                    resolved: false,
                });
            }
        }
    }

    pub(super) fn set_result(&mut self, next: String) {
        rebase_envelopes(&self.result, &next, &mut self.conflicts);
        self.result = next;
    }
}

pub(super) fn ranges_overlap(left: &Range<usize>, right: &Range<usize>) -> bool {
    if left.is_empty() || right.is_empty() {
        left.start <= right.end && right.start <= left.end
    } else {
        left.start < right.end && right.start < left.end
    }
}

/// Rebases envelopes through an arbitrary editor replacement without running a
/// second comparison. The unchanged prefix/suffix locate one conservative edit
/// span and all byte boundaries are snapped to UTF-8 character boundaries.
fn rebase_envelopes(old: &str, new: &str, conflicts: &mut [ConflictState]) {
    let mut prefix = 0;
    while prefix < old.len()
        && prefix < new.len()
        && old.as_bytes()[prefix] == new.as_bytes()[prefix]
    {
        prefix += 1;
    }
    while prefix > 0 && (!old.is_char_boundary(prefix) || !new.is_char_boundary(prefix)) {
        prefix -= 1;
    }
    let mut old_end = old.len();
    let mut new_end = new.len();
    while old_end > prefix
        && new_end > prefix
        && old.as_bytes()[old_end - 1] == new.as_bytes()[new_end - 1]
    {
        old_end -= 1;
        new_end -= 1;
    }
    while old_end < old.len() && !old.is_char_boundary(old_end) {
        old_end += 1;
    }
    while new_end < new.len() && !new.is_char_boundary(new_end) {
        new_end += 1;
    }
    if old == new {
        return;
    }
    let delta = new_end as isize - old_end as isize;
    let insertion = prefix == old_end;
    for conflict in conflicts {
        let envelope = &mut conflict.envelope;
        let before = if insertion {
            envelope.end < prefix
        } else {
            envelope.end <= prefix && !((envelope.start == envelope.end) && envelope.end == prefix)
        };
        let after = if insertion {
            envelope.start > old_end
        } else {
            envelope.start >= old_end
                && !((envelope.start == envelope.end) && envelope.start == old_end)
        };
        if before {
            continue;
        }
        if after {
            envelope.start = envelope.start.saturating_add_signed(delta);
            envelope.end = envelope.end.saturating_add_signed(delta);
        } else {
            // Retain unchanged text after an edit *inside* the envelope.
            // Deleting an entire conflict keeps a zero-width actionable anchor.
            envelope.start = envelope.start.min(prefix);
            envelope.end = if envelope.end > old_end {
                envelope.end.saturating_add_signed(delta)
            } else {
                new_end
            };
            conflict.wrapper = None;
            conflict.resolved = false;
        }
    }
}
