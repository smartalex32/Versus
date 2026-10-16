use super::{CompareError, DEFAULT_TEXT_SIZE_LIMIT, read_text};
use similar::{ChangeTag, TextDiff as SimilarTextDiff};
use std::{ops::Range, path::Path};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MergeHunkKind {
    LeftOnly,
    RightOnly,
    Identical,
    Conflict,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeHunk {
    pub base_range: Range<usize>,
    pub kind: MergeHunkKind,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreeWayDiff {
    pub hunks: Vec<MergeHunk>,
    pub merged: String,
}
#[derive(Clone, Debug)]
struct Edit {
    range: Range<usize>,
    replacement: Vec<String>,
}

/// Reads bounded UTF-8 inputs and performs a conventional CLI-style three-way
/// merge. Conflict markers use `OURS` and `THEIRS`.
pub fn merge_three_way(
    base: impl AsRef<Path>,
    ours: impl AsRef<Path>,
    theirs: impl AsRef<Path>,
) -> Result<ThreeWayDiff, CompareError> {
    merge_three_way_text(
        &read_text(base.as_ref(), DEFAULT_TEXT_SIZE_LIMIT)?,
        &read_text(ours.as_ref(), DEFAULT_TEXT_SIZE_LIMIT)?,
        &read_text(theirs.as_ref(), DEFAULT_TEXT_SIZE_LIMIT)?,
    )
}
/// Performs a conventional three-way merge with `OURS` / `THEIRS` conflict labels.
///
/// This is intentionally independent of the desktop editing workflow.  In
/// particular, it combines edits relative to the base instead of making either
/// side the preferred result.
pub fn merge_three_way_text(
    base: &str,
    ours: &str,
    theirs: &str,
) -> Result<ThreeWayDiff, CompareError> {
    let base_lines = lines(base);
    let left_lines = lines(ours);
    let right_lines = lines(theirs);
    let left_edits = edits(&base_lines, &left_lines);
    let right_edits = edits(&base_lines, &right_lines);
    let mut output = Vec::new();
    let mut hunks = Vec::new();
    let mut index = 0;
    let mut l = 0;
    let mut r = 0;
    while l < left_edits.len() || r < right_edits.len() {
        let next_l = left_edits.get(l);
        let next_r = right_edits.get(r);
        let start = next_l
            .map(|e| e.range.start)
            .into_iter()
            .chain(next_r.map(|e| e.range.start))
            .min()
            .unwrap();
        output.extend_from_slice(&base_lines[index..start]);
        let left_active = next_l.is_some_and(|edit| edit.range.start == start);
        let (seed, other) = if left_active {
            (next_l.unwrap(), next_r)
        } else {
            (next_r.unwrap(), next_l)
        };
        let concurrent = other.is_some_and(|edit| edits_intersect_for_merge(seed, edit));

        if concurrent {
            // An overlap can span more than one edit on either side.  Close over
            // all cross-side overlaps before rendering the region; consuming only
            // the first pair loses changes after an overlapping deletion.
            let (left_count, right_count) =
                overlapping_edit_counts(&left_edits[l..], &right_edits[r..], left_active);
            let left_group = &left_edits[l..l + left_count];
            let right_group = &right_edits[r..r + right_count];
            let start = left_group
                .iter()
                .chain(right_group)
                .map(|edit| edit.range.start)
                .min()
                .expect("overlapping merge group has an edit");
            let end = left_group
                .iter()
                .chain(right_group)
                .map(|edit| edit.range.end)
                .max()
                .expect("overlapping merge group has an edit");
            let left_replacement = render_edits(&base_lines, start, end, left_group);
            let right_replacement = render_edits(&base_lines, start, end, right_group);
            let kind = if left_replacement == right_replacement {
                MergeHunkKind::Identical
            } else {
                MergeHunkKind::Conflict
            };
            if kind == MergeHunkKind::Conflict {
                output.push("<<<<<<< OURS".into());
                output.extend(left_replacement.clone());
                output.push("=======".into());
                output.extend(right_replacement.clone());
                output.push(">>>>>>> THEIRS".into());
            } else {
                output.extend(left_replacement.clone());
            }
            hunks.push(MergeHunk {
                base_range: start..end,
                kind,
            });
            index = end;
            l += left_count;
            r += right_count;
        } else if left_active {
            let e = next_l.unwrap();
            output.extend(e.replacement.clone());
            hunks.push(MergeHunk {
                base_range: e.range.clone(),
                kind: MergeHunkKind::LeftOnly,
            });
            index = e.range.end;
            l += 1;
        } else {
            let e = next_r.unwrap();
            output.extend(e.replacement.clone());
            hunks.push(MergeHunk {
                base_range: e.range.clone(),
                kind: MergeHunkKind::RightOnly,
            });
            index = e.range.end;
            r += 1;
        }
    }
    output.extend_from_slice(&base_lines[index..]);
    Ok(ThreeWayDiff {
        hunks,
        merged: output.join("\n"),
    })
}

fn edits_intersect_for_merge(left: &Edit, right: &Edit) -> bool {
    match (left.range.is_empty(), right.range.is_empty()) {
        (true, true) => left.range.start == right.range.start,
        // An insertion at either boundary of a replacement is ambiguous: both
        // valid orders change the result.  Treat it as a conventional conflict.
        (true, false) => {
            right.range.start <= left.range.start && left.range.start <= right.range.end
        }
        (false, true) => {
            left.range.start <= right.range.start && right.range.start <= left.range.end
        }
        (false, false) => left.range.start < right.range.end && right.range.start < left.range.end,
    }
}

/// Counts the leading edits on each side that form one connected overlap group.
/// Edits on an individual side never overlap, so every selected edit forms a
/// prefix of that side's remaining list.
fn overlapping_edit_counts(left: &[Edit], right: &[Edit], seed_is_left: bool) -> (usize, usize) {
    let mut left_count = usize::from(seed_is_left);
    let mut right_count = usize::from(!seed_is_left);
    loop {
        let mut grew = false;
        while left_count < left.len()
            && right[..right_count]
                .iter()
                .any(|other| edits_intersect_for_merge(&left[left_count], other))
        {
            left_count += 1;
            grew = true;
        }
        while right_count < right.len()
            && left[..left_count]
                .iter()
                .any(|other| edits_intersect_for_merge(other, &right[right_count]))
        {
            right_count += 1;
            grew = true;
        }
        if !grew {
            return (left_count, right_count);
        }
    }
}

fn render_edits(base: &[String], start: usize, end: usize, edits: &[Edit]) -> Vec<String> {
    let mut rendered = Vec::new();
    let mut cursor = start;
    for edit in edits {
        debug_assert!(cursor <= edit.range.start);
        rendered.extend_from_slice(&base[cursor..edit.range.start]);
        rendered.extend(edit.replacement.clone());
        cursor = edit.range.end;
    }
    rendered.extend_from_slice(&base[cursor..end]);
    rendered
}
fn lines(text: &str) -> Vec<String> {
    text.strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .map(str::to_owned)
        .collect()
}
fn edits(base: &[String], side: &[String]) -> Vec<Edit> {
    let base_refs: Vec<&str> = base.iter().map(String::as_str).collect();
    let side_refs: Vec<&str> = side.iter().map(String::as_str).collect();
    let diff = SimilarTextDiff::from_slices(&base_refs, &side_refs);
    let mut result = Vec::new();
    let mut active: Option<Edit> = None;
    let mut base_cursor = 0;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                if let Some(edit) = active.take() {
                    result.push(edit);
                }
                base_cursor += 1;
            }
            ChangeTag::Delete => {
                let edit = active.get_or_insert_with(|| Edit {
                    range: base_cursor..base_cursor,
                    replacement: Vec::new(),
                });
                base_cursor += 1;
                edit.range.end = base_cursor;
            }
            ChangeTag::Insert => {
                let edit = active.get_or_insert_with(|| Edit {
                    range: base_cursor..base_cursor,
                    replacement: Vec::new(),
                });
                edit.replacement.push(change.value().to_owned());
            }
        }
    }
    if let Some(edit) = active {
        result.push(edit);
    }
    result
}

#[cfg(test)]
#[path = "merge/tests.rs"]
mod tests;
