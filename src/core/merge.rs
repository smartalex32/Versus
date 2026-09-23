use super::{CompareError, DEFAULT_TEXT_SIZE_LIMIT, FileCompareOptions, read_text};
use similar::{ChangeTag, TextDiff as SimilarTextDiff};
use std::{ops::Range, path::Path};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MergeHunkKind {
    LeftOnly,
    RightOnly,
    Identical,
    Conflict,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MergeChoice {
    Left,
    Right,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeHunk {
    pub base_range: Range<usize>,
    pub merged_range: Range<usize>,
    pub left: Vec<String>,
    pub right: Vec<String>,
    pub kind: MergeHunkKind,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreeWayDiff {
    pub base: Vec<String>,
    pub left: Vec<String>,
    pub right: Vec<String>,
    pub hunks: Vec<MergeHunk>,
    pub merged: String,
}
#[derive(Clone, Debug)]
struct Edit {
    range: Range<usize>,
    replacement: Vec<String>,
}

pub fn compare_three_way(
    base: impl AsRef<Path>,
    left: impl AsRef<Path>,
    right: impl AsRef<Path>,
) -> Result<ThreeWayDiff, CompareError> {
    compare_three_way_text(
        &read_text(base.as_ref(), DEFAULT_TEXT_SIZE_LIMIT)?,
        &read_text(left.as_ref(), DEFAULT_TEXT_SIZE_LIMIT)?,
        &read_text(right.as_ref(), DEFAULT_TEXT_SIZE_LIMIT)?,
    )
}
pub fn compare_three_way_text(
    base: &str,
    left: &str,
    right: &str,
) -> Result<ThreeWayDiff, CompareError> {
    let base_lines = lines(base);
    let left_lines = lines(left);
    let right_lines = lines(right);
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
        let left_active = next_l.is_some_and(|e| e.range.start == start);
        let concurrent = match (next_l, next_r) {
            (Some(a), Some(b)) => {
                // An insertion at the same anchor as a replacement has no stable
                // independent ordering. Keep both edits in one conflict region.
                (a.range.start == b.range.start && (a.range.is_empty() || b.range.is_empty()))
                    || (a.range.start < b.range.end && b.range.start < a.range.end)
            }
            _ => false,
        };
        if concurrent {
            let le = next_l.unwrap();
            let re = next_r.unwrap();
            let end = le.range.end.max(re.range.end);
            let kind = if le.range == re.range && le.replacement == re.replacement {
                MergeHunkKind::Identical
            } else {
                MergeHunkKind::Conflict
            };
            if kind == MergeHunkKind::Conflict {
                output.push("<<<<<<< LEFT".into());
                output.extend(le.replacement.clone());
                output.push("=======".into());
                output.extend(re.replacement.clone());
                output.push(">>>>>>> RIGHT".into());
            } else {
                output.extend(le.replacement.clone());
            }
            let merged_start = output
                .len()
                .saturating_sub(if kind == MergeHunkKind::Conflict {
                    le.replacement.len() + re.replacement.len() + 3
                } else {
                    le.replacement.len()
                });
            hunks.push(MergeHunk {
                base_range: start..end,
                merged_range: merged_start..output.len(),
                left: le.replacement.clone(),
                right: re.replacement.clone(),
                kind,
            });
            index = end;
            l += 1;
            r += 1;
        } else if left_active {
            let e = next_l.unwrap();
            let merged_start = output.len();
            output.extend(e.replacement.clone());
            hunks.push(MergeHunk {
                base_range: e.range.clone(),
                merged_range: merged_start..output.len(),
                left: e.replacement.clone(),
                right: Vec::new(),
                kind: MergeHunkKind::LeftOnly,
            });
            index = e.range.end;
            l += 1;
        } else {
            let e = next_r.unwrap();
            let merged_start = output.len();
            output.extend(e.replacement.clone());
            hunks.push(MergeHunk {
                base_range: e.range.clone(),
                merged_range: merged_start..output.len(),
                left: Vec::new(),
                right: e.replacement.clone(),
                kind: MergeHunkKind::RightOnly,
            });
            index = e.range.end;
            r += 1;
        }
    }
    output.extend_from_slice(&base_lines[index..]);
    Ok(ThreeWayDiff {
        base: base_lines,
        left: left_lines,
        right: right_lines,
        hunks,
        merged: output.join("\n"),
    })
}
/// Compares editable buffers. Merge semantics normalize CRLF/LF; whitespace options only
/// affect 2-way display, because discarding whitespace during a merge would lose source data.
pub fn compare_three_texts(
    base: &str,
    left: &str,
    right: &str,
    _options: &FileCompareOptions,
) -> Result<ThreeWayDiff, CompareError> {
    compare_three_way_text(base, left, right)
}

/// Returns a new merged buffer with one hunk resolved. `merged_range` is line based,
/// so the UI can use it directly for a selected hunk without parsing conflict markers.
pub fn resolve_merge_hunk(
    diff: &ThreeWayDiff,
    hunk_index: usize,
    use_left: bool,
) -> Option<String> {
    let hunk = diff.hunks.get(hunk_index)?;
    let mut lines: Vec<String> = diff.merged.split('\n').map(str::to_owned).collect();
    let replacement = if use_left { &hunk.left } else { &hunk.right };
    lines.splice(hunk.merged_range.clone(), replacement.iter().cloned());
    Some(lines.join("\n"))
}

/// Renders a merge using a choice for each hunk. Missing choices retain the current
/// hunk rendering, including conflict markers, so callers may resolve incrementally.
pub fn render_three_way(diff: &ThreeWayDiff, choices: &[Option<MergeChoice>]) -> String {
    let mut lines: Vec<String> = diff.merged.split('\n').map(str::to_owned).collect();
    for (index, hunk) in diff.hunks.iter().enumerate().rev() {
        let Some(Some(choice)) = choices.get(index) else {
            continue;
        };
        let replacement = match choice {
            MergeChoice::Left => &hunk.left,
            MergeChoice::Right => &hunk.right,
        };
        lines.splice(hunk.merged_range.clone(), replacement.iter().cloned());
    }
    lines.join("\n")
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
