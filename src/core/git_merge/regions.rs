use std::{ops::Range, time::Instant};

use similar::{DiffTag, TextDiff};

/// Returns byte ranges for lines, including their terminators.  Keeping
/// terminators in the range makes replacements preserve CRLF and final-newline
/// choices exactly.
fn lines(text: &str) -> Option<Vec<Range<usize>>> {
    const MAX_LINES: usize = 200_000;
    let mut result = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let terminator = match bytes[index] {
            b'\n' => 1,
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => 2,
            b'\r' => 1,
            _ => {
                index += 1;
                continue;
            }
        };
        let end = index + terminator;
        if result.len() == MAX_LINES {
            return None;
        }
        result.push(start..end);
        start = end;
        index = end;
    }
    if start < text.len() {
        if result.len() == MAX_LINES {
            return None;
        }
        result.push(start..text.len());
    }
    Some(result)
}

#[derive(Clone, Debug)]
pub(super) struct Hunk {
    pub base: Range<usize>,
    pub target: Range<usize>,
}

/// Produces exact changed line spans with `similar`'s bounded Myers algorithm.
/// The algorithm avoids the quadratic allocation of a table-sized LCS.
pub(super) fn hunks(base: &str, target: &str, deadline: Instant) -> Option<Vec<Hunk>> {
    let base_lines = lines(base)?;
    let target_lines = lines(target)?;
    let base_values: Vec<_> = base_lines
        .iter()
        .map(|range| &base[range.clone()])
        .collect();
    let target_values: Vec<_> = target_lines
        .iter()
        .map(|range| &target[range.clone()])
        .collect();
    let diff = TextDiff::configure()
        .deadline(deadline)
        .diff_slices(&base_values, &target_values);
    if Instant::now() >= deadline {
        return None;
    }
    Some(
        diff.ops()
            .iter()
            .filter(|operation| operation.tag() != DiffTag::Equal)
            .map(|operation| Hunk {
                base: offset(&base_lines, operation.old_range().start, base.len())
                    ..offset(&base_lines, operation.old_range().end, base.len()),
                target: offset(&target_lines, operation.new_range().start, target.len())
                    ..offset(&target_lines, operation.new_range().end, target.len()),
            })
            .collect(),
    )
}

fn offset(lines: &[Range<usize>], index: usize, text_len: usize) -> usize {
    lines.get(index).map_or(text_len, |range| range.start)
}

pub(super) fn union_ranges(hunks: &[&[Hunk]]) -> Vec<Range<usize>> {
    let values: Vec<_> = hunks
        .iter()
        .flat_map(|set| set.iter().map(|hunk| hunk.target.clone()))
        .collect();
    close_ranges(values)
}

pub(super) fn close_ranges(mut values: Vec<Range<usize>>) -> Vec<Range<usize>> {
    values.sort_by_key(|range| (range.start, range.end));
    let mut output: Vec<Range<usize>> = Vec::new();
    for value in values {
        if let Some(last) = output.last_mut()
            && intersects_or_touches(last, &value)
        {
            last.end = last.end.max(value.end);
        } else {
            output.push(value);
        }
    }
    output
}

pub(super) fn map_range(base: &Range<usize>, hunks: &[Hunk], target_len: usize) -> Range<usize> {
    let start = map_boundary(base.start, false, hunks, target_len);
    let end = map_boundary(base.end, true, hunks, target_len);
    start.min(end)..start.max(end)
}

fn map_boundary(point: usize, end: bool, hunks: &[Hunk], target_len: usize) -> usize {
    let touching = hunks.iter().find(|hunk| {
        if hunk.base.is_empty() {
            return hunk.base.start == point;
        }
        if end {
            hunk.base.start < point && point <= hunk.base.end
        } else {
            hunk.base.start <= point && point < hunk.base.end
        }
    });
    if let Some(hunk) = touching {
        return if end {
            hunk.target.end
        } else {
            hunk.target.start
        };
    }
    let delta: isize = hunks
        .iter()
        .filter(|hunk| hunk.base.end <= point)
        .map(|hunk| hunk.target.len() as isize - hunk.base.len() as isize)
        .sum();
    point.saturating_add_signed(delta).min(target_len)
}

fn intersects_or_touches(left: &Range<usize>, right: &Range<usize>) -> bool {
    if left.is_empty() && right.is_empty() {
        return left.start == right.start;
    }
    left.start <= right.end && right.start <= left.end
}
