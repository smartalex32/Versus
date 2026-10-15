//! Keeps folder-find results usable after an explicitly applied replacement.
//!
//! This module deliberately only transforms the search result snapshots.  It
//! never writes a working copy and it never starts another search: callers
//! prepare and apply validated replacement transactions from the current search.

use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::PathBuf,
};

use versus::{CompareError, SearchChange, SearchPreview, SearchQuery, SearchResults, WorkingFile};

/// Prepares a replacement transaction for the first selected result (`next`) or all
/// selected results.  Result order is the stable file/hit order from the last
/// Find operation.
pub(super) fn preview_replacements(
    results: &SearchResults,
    selected: &BTreeSet<u64>,
    query: &SearchQuery,
    replacement: &str,
    working: &BTreeMap<PathBuf, WorkingFile>,
    next: bool,
) -> Result<SearchPreview, CompareError> {
    let selection = if next {
        results
            .files
            .iter()
            .flat_map(|file| &file.hits)
            .find(|hit| selected.contains(&hit.id))
            .map(|hit| BTreeSet::from([hit.id]))
            .unwrap_or_default()
    } else {
        selected.clone()
    };
    versus::preview_replace(results, &selection, query, replacement, working)
}

/// Advances an existing search result set after its replacement transaction has been
/// applied to `working`.  The original search remains authoritative: matches
/// created by replacement text are deliberately not introduced here.
pub(super) fn advance_results(
    results: &mut SearchResults,
    selected: &mut BTreeSet<u64>,
    applied: &SearchPreview,
    working: &BTreeMap<PathBuf, WorkingFile>,
) -> Result<(), String> {
    let changes = changes_by_identity(applied)?;
    if changes.is_empty() {
        return Ok(());
    }

    // Validate every affected alias before changing the result set.  A stale
    // session must be discarded by the caller instead of moving byte offsets
    // against unexpected content.
    let mut replacement_files = BTreeMap::new();
    for change in &applied.changes {
        let base = applied.bases.get(&change.path).ok_or_else(|| {
            format!(
                "{} is missing its replacement snapshot",
                change.path.display()
            )
        })?;
        let identity = base.identity_key();
        let updated = working_file_for_identity(working, &identity)?;
        if updated.text != change.after {
            return Err(format!(
                "{} changed after replacement; run Find again.",
                change.path.display()
            ));
        }
        replacement_files.insert(identity, updated);
    }

    let mut next_files = Vec::with_capacity(results.files.len());
    let mut consumed = BTreeSet::new();
    for mut file in results.files.clone() {
        let identity = file.working_file.identity_key();
        let Some(change) = changes.get(&identity) else {
            next_files.push(file);
            continue;
        };
        if file.working_file.text != change.before {
            return Err(format!(
                "{} changed after Find; run Find again.",
                file.path.display()
            ));
        }
        let updated = replacement_files
            .get(&identity)
            .expect("validated replacement identity")
            .clone();
        let mut hits = Vec::with_capacity(file.hits.len());
        for mut hit in file.hits {
            if overlaps_any(&hit.range, &change.edits) {
                consumed.insert(hit.id);
                continue;
            }
            let shifted =
                shift_range(&hit.range, &change.edits, &updated.text).ok_or_else(|| {
                    format!(
                        "{} changed at a search result; run Find again.",
                        file.path.display()
                    )
                })?;
            // This check makes an offset-shift bug fail closed.  It also keeps
            // an old match from targeting text newly produced by replacement.
            if updated.text.get(shifted.clone()) != Some(hit.matched.as_str()) {
                return Err(format!(
                    "{} changed at a search result; run Find again.",
                    file.path.display()
                ));
            }
            hit.range = shifted.clone();
            hit.line = line_for_offset(&updated.text, shifted.start);
            hits.push(hit);
        }
        file.hits = hits;
        file.working_file = updated;
        if !file.hits.is_empty() {
            next_files.push(file);
        }
    }
    results.files = next_files;
    selected.retain(|id| !consumed.contains(id));
    Ok(())
}

fn changes_by_identity(
    applied: &SearchPreview,
) -> Result<BTreeMap<PathBuf, &SearchChange>, String> {
    let mut changes = BTreeMap::new();
    for change in &applied.changes {
        validate_change(change)?;
        let base = applied.bases.get(&change.path).ok_or_else(|| {
            format!(
                "{} is missing its replacement snapshot",
                change.path.display()
            )
        })?;
        let identity = base.identity_key();
        if changes.insert(identity, change).is_some() {
            return Err(
                "replacement preview contains duplicate file identities; run Find again.".into(),
            );
        }
    }
    Ok(changes)
}

fn validate_change(change: &SearchChange) -> Result<(), String> {
    let mut previous_end = 0;
    for (range, _) in &change.edits {
        if range.start < previous_end
            || range.start > range.end
            || range.end > change.before.len()
            || !change.before.is_char_boundary(range.start)
            || !change.before.is_char_boundary(range.end)
        {
            return Err(format!(
                "{} has invalid replacement edits; run Find again.",
                change.path.display()
            ));
        }
        previous_end = range.end;
    }
    let mut expected = change.before.clone();
    for (range, replacement) in change.edits.iter().rev() {
        expected.replace_range(range.clone(), replacement);
    }
    if expected != change.after {
        return Err(format!(
            "{} has an invalid replacement preview; run Find again.",
            change.path.display()
        ));
    }
    Ok(())
}

fn working_file_for_identity(
    working: &BTreeMap<PathBuf, WorkingFile>,
    identity: &PathBuf,
) -> Result<WorkingFile, String> {
    let mut matches = working
        .values()
        .filter(|file| file.identity_key() == *identity);
    let Some(first) = matches.next() else {
        return Err("replacement working copy is unavailable; run Find again.".into());
    };
    if matches.any(|file| file.text != first.text) {
        return Err("replacement aliases disagree; run Find again.".into());
    }
    Ok(first.clone())
}

fn overlaps_any(range: &Range<usize>, edits: &[(Range<usize>, String)]) -> bool {
    edits.iter().any(|(edit, _)| ranges_overlap(range, edit))
}

/// Includes a zero-width range only when it has the same insertion position,
/// while a replacement spanning that position consumes it.  This avoids
/// treating insertions as length-zero no-ops when calculating later shifts.
fn ranges_overlap(left: &Range<usize>, right: &Range<usize>) -> bool {
    match (left.is_empty(), right.is_empty()) {
        (true, true) => left.start == right.start,
        (true, false) => right.start <= left.start && left.start < right.end,
        (false, true) => left.start < right.start && right.start < left.end,
        (false, false) => left.start < right.end && right.start < left.end,
    }
}

fn shift_range(
    range: &Range<usize>,
    edits: &[(Range<usize>, String)],
    text: &str,
) -> Option<Range<usize>> {
    let mut offset: isize = 0;
    for (edit, replacement) in edits {
        // Non-overlapping edits at the same offset as a non-empty hit insert
        // before that hit.  The hit must therefore move past the insertion.
        if edit.end <= range.start {
            offset += replacement.len() as isize - (edit.end - edit.start) as isize;
        }
    }
    let start = range.start.checked_add_signed(offset)?;
    let end = range.end.checked_add_signed(offset)?;
    (start <= end
        && end <= text.len()
        && text.is_char_boundary(start)
        && text.is_char_boundary(end))
    .then_some(start..end)
}

fn line_for_offset(text: &str, offset: usize) -> usize {
    text[..offset].bytes().filter(|byte| *byte == b'\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use versus::{FileFingerprint, SearchFileResults, SearchHit, TextEncoding};

    fn file(path: &str, identity: &str, text: &str) -> WorkingFile {
        WorkingFile {
            path: Some(path.into()),
            identity: Some(identity.into()),
            text: text.into(),
            encoding: TextEncoding::Utf8 { bom: false },
            fingerprint: FileFingerprint::Missing,
        }
    }

    fn results(text: &str) -> SearchResults {
        let working = file("left.txt", "/physical/left.txt", text);
        SearchResults {
            files: vec![SearchFileResults {
                path: "left.txt".into(),
                side: 0,
                hits: vec![
                    SearchHit {
                        id: 1,
                        path: "left.txt".into(),
                        side: 0,
                        range: 0..3,
                        line: 1,
                        matched: "foo".into(),
                    },
                    SearchHit {
                        id: 2,
                        path: "left.txt".into(),
                        side: 0,
                        range: 4..7,
                        line: 1,
                        matched: "foo".into(),
                    },
                ],
                working_file: working,
            }],
            ..SearchResults::default()
        }
    }

    #[test]
    fn next_previews_only_the_first_selected_hit_in_result_order() {
        let results = results("foo foo");
        let selected = BTreeSet::from([1, 2]);
        let mut working = BTreeMap::new();
        working.insert(
            "alias.txt".into(),
            file("alias.txt", "/physical/left.txt", "foo foo"),
        );
        let preview = preview_replacements(
            &results,
            &selected,
            &SearchQuery::plain("foo"),
            "bar",
            &working,
            true,
        )
        .unwrap();
        assert_eq!(preview.changes[0].after, "bar foo");
        assert_eq!(preview.changes[0].hit_ids, vec![1]);
    }

    #[test]
    fn advancing_consumes_applied_hits_and_shifts_remaining_utf8_ranges() {
        let mut results = results("foo\ncafé\nfoo");
        results.files[0].hits = vec![
            SearchHit {
                id: 1,
                path: "left.txt".into(),
                side: 0,
                range: 0..3,
                line: 1,
                matched: "foo".into(),
            },
            SearchHit {
                id: 2,
                path: "left.txt".into(),
                side: 0,
                range: 10..13,
                line: 3,
                matched: "foo".into(),
            },
        ];
        results.files[0].working_file.text = "foo\ncafé\nfoo".into();
        let selected = BTreeSet::from([1, 2]);
        let mut working = BTreeMap::new();
        working.insert(
            "left.txt".into(),
            file("left.txt", "/physical/left.txt", "foo\ncafé\nfoo"),
        );
        let preview = preview_replacements(
            &results,
            &selected,
            &SearchQuery::plain("foo"),
            "quux",
            &working,
            true,
        )
        .unwrap();
        working.get_mut(&PathBuf::from("left.txt")).unwrap().text = "quux\ncafé\nfoo".into();
        let mut selected = selected;
        advance_results(&mut results, &mut selected, &preview, &working).unwrap();
        assert_eq!(results.files[0].hits[0].id, 2);
        assert_eq!(results.files[0].hits[0].range, 11..14);
        assert_eq!(results.files[0].hits[0].line, 3);
        assert_eq!(selected, BTreeSet::from([2]));

        let next = preview_replacements(
            &results,
            &selected,
            &SearchQuery::plain("foo"),
            "quux",
            &working,
            true,
        )
        .unwrap();
        assert_eq!(next.changes[0].hit_ids, vec![2]);
        assert_eq!(next.changes[0].after, "quux\ncafé\nquux");
    }

    #[test]
    fn advancing_keeps_aliases_and_does_not_add_matches_from_replacement_text() {
        let mut results = results("foo foo");
        let alias = SearchFileResults {
            path: "alias.txt".into(),
            side: 1,
            hits: vec![SearchHit {
                id: 3,
                path: "alias.txt".into(),
                side: 1,
                range: 4..7,
                line: 1,
                matched: "foo".into(),
            }],
            working_file: file("alias.txt", "/physical/left.txt", "foo foo"),
        };
        results.files.push(alias);
        let mut selected = BTreeSet::from([1]);
        let mut working = BTreeMap::new();
        working.insert(
            "left.txt".into(),
            file("left.txt", "/physical/left.txt", "foo foo"),
        );
        let preview = preview_replacements(
            &results,
            &selected,
            &SearchQuery::plain("foo"),
            "foofoo",
            &working,
            true,
        )
        .unwrap();
        working.get_mut(&PathBuf::from("left.txt")).unwrap().text = "foofoo foo".into();
        advance_results(&mut results, &mut selected, &preview, &working).unwrap();
        assert_eq!(results.files.len(), 2);
        assert!(
            results
                .files
                .iter()
                .flat_map(|file| &file.hits)
                .all(|hit| hit.id != 1)
        );
        assert_eq!(
            results
                .files
                .iter()
                .flat_map(|file| &file.hits)
                .map(|hit| hit.id)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([2, 3])
        );
        assert!(selected.is_empty());
    }

    #[test]
    fn stale_working_copy_fails_without_changing_results() {
        let mut results = results("foo foo");
        let original = results.clone();
        let selected = BTreeSet::from([1]);
        let mut working = BTreeMap::new();
        working.insert(
            "left.txt".into(),
            file("left.txt", "/physical/left.txt", "foo foo"),
        );
        let preview = preview_replacements(
            &results,
            &selected,
            &SearchQuery::plain("foo"),
            "bar",
            &working,
            true,
        )
        .unwrap();
        working.get_mut(&PathBuf::from("left.txt")).unwrap().text = "changed".into();
        let mut selected = selected;
        assert!(advance_results(&mut results, &mut selected, &preview, &working).is_err());
        assert_eq!(results, original);
        assert_eq!(selected, BTreeSet::from([1]));
    }
}
