//! Bounded line and character alignment for changed comparison blocks.

use super::{
    FileComparison, FileComparisonRow, FileViewOptions, cancelled, finish_progress,
    text::{DisplayLine, changed_ranges_with_options, lines_for_view, whole_range},
};
use crate::core::{CompareError, DirectoryEntryState};
use similar::{ChangeTag, TextDiff as SimilarTextDiff};
use std::{
    borrow::Cow,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

pub(super) fn compare_line_sets(
    lines: [Vec<DisplayLine>; 2],
    comparable: [Vec<String>; 2],
    maps: [Vec<usize>; 2],
    cancellation: &AtomicBool,
    options: &FileViewOptions,
    timeout: Duration,
) -> Result<Option<FileComparison>, CompareError> {
    let references = [
        comparable[0].iter().map(String::as_str).collect::<Vec<_>>(),
        comparable[1].iter().map(String::as_str).collect::<Vec<_>>(),
    ];
    if let Some(progress) = &options.progress {
        progress.begin(super::super::ProgressStage::ComparingLines, None);
    }
    // A cancellation request cannot interrupt Similar while it computes a diff, so
    // give its worst-case work a finite bound before observing cancellation again.
    let diff_started = Instant::now();
    let diff = SimilarTextDiff::configure()
        .timeout(timeout)
        .diff_slices(&references[0], &references[1]);
    if diff_started.elapsed() >= timeout {
        finish_progress(options);
        return Ok(Some(FileComparison {
            rows: Vec::new(),
            message: Some(super::DIFF_TIMEOUT_MESSAGE.into()),
        }));
    }
    let highlight_deadline = diff_started + timeout;
    if cancelled(cancellation) {
        return Ok(None);
    }
    let mut rows = Vec::new();
    let mut indexes = [0, 0];
    let mut pending_left = Vec::new();
    let mut pending_right = Vec::new();
    if let Some(progress) = &options.progress {
        let changed = diff
            .ops()
            .iter()
            .filter(|op| op.tag() != similar::DiffTag::Equal)
            .map(|op| op.old_range().len().max(op.new_range().len()) as u64)
            .sum();
        progress.begin(super::super::ProgressStage::Highlighting, Some(changed));
    }
    for change in diff.iter_all_changes() {
        if cancelled(cancellation) {
            return Ok(None);
        }
        match change.tag() {
            ChangeTag::Equal => {
                if !append_changed_rows(
                    &mut rows,
                    pending_left.drain(..),
                    pending_right.drain(..),
                    cancellation,
                    options,
                    highlight_deadline,
                ) {
                    return Ok(None);
                }
                rows.push(FileComparisonRow {
                    left: Some((indexes[0] + 1, lines[0][indexes[0]].content.clone())),
                    right: Some((indexes[1] + 1, lines[1][indexes[1]].content.clone())),
                    state: DirectoryEntryState::Same,
                    left_changed: Vec::new(),
                    right_changed: Vec::new(),
                    left_ending: Some(lines[0][indexes[0]].ending),
                    right_ending: Some(lines[1][indexes[1]].ending),
                });
                indexes[0] += 1;
                indexes[1] += 1;
            }
            ChangeTag::Delete => {
                pending_left.push((indexes[0] + 1, lines[0][indexes[0]].clone()));
                indexes[0] += 1;
            }
            ChangeTag::Insert => {
                pending_right.push((indexes[1] + 1, lines[1][indexes[1]].clone()));
                indexes[1] += 1;
            }
        }
    }
    if !append_changed_rows(
        &mut rows,
        pending_left.drain(..),
        pending_right.drain(..),
        cancellation,
        options,
        highlight_deadline,
    ) {
        return Ok(None);
    }
    finish_progress(options);
    for row in &mut rows {
        if let Some((number, _)) = &mut row.left {
            *number = maps[0][*number - 1] + 1;
        }
        if let Some((number, _)) = &mut row.right {
            *number = maps[1][*number - 1] + 1;
        }
    }
    Ok(Some(FileComparison {
        rows,
        message: None,
    }))
}

pub(super) fn rows_for_one_side(
    text: String,
    left_side: bool,
    cancellation: &AtomicBool,
    options: &FileViewOptions,
) -> Result<Option<FileComparison>, CompareError> {
    let mut rows = Vec::new();
    let lines = lines_for_view(&text);
    if let Some(progress) = &options.progress {
        progress.begin(
            super::super::ProgressStage::Highlighting,
            Some(lines.len() as u64),
        );
    }
    for (index, line) in lines.into_iter().enumerate() {
        if cancelled(cancellation) {
            return Ok(None);
        }
        let ending = Some(line.ending);
        let content = line.content;
        let line = Some((index + 1, content.clone()));
        rows.push(FileComparisonRow {
            left: left_side.then(|| line.clone()).flatten(),
            right: (!left_side).then_some(line).flatten(),
            state: if left_side {
                DirectoryEntryState::LeftOnly
            } else {
                DirectoryEntryState::RightOnly
            },
            left_changed: left_side
                .then(|| whole_range(&content))
                .flatten()
                .into_iter()
                .collect(),
            right_changed: (!left_side)
                .then(|| whole_range(&content))
                .flatten()
                .into_iter()
                .collect(),
            left_ending: left_side.then_some(ending).flatten(),
            right_ending: (!left_side).then_some(ending).flatten(),
        });
        if let Some(progress) = &options.progress {
            progress.advance(1);
        }
    }
    let message = rows.is_empty().then(|| {
        format!(
            "Only the {} file is available; it is empty.",
            if left_side { "left" } else { "right" }
        )
    });
    finish_progress(options);
    Ok(Some(FileComparison { rows, message }))
}

pub(super) fn append_changed_rows(
    rows: &mut Vec<FileComparisonRow>,
    left: impl Iterator<Item = (usize, DisplayLine)>,
    right: impl Iterator<Item = (usize, DisplayLine)>,
    cancellation: &AtomicBool,
    options: &FileViewOptions,
    highlight_deadline: Instant,
) -> bool {
    let left: Vec<_> = left.collect();
    let right: Vec<_> = right.collect();
    let alignment = align_changed_block(&left, &right, cancellation, options, highlight_deadline);
    for (left_index, right_index) in alignment {
        if cancelled(cancellation) {
            return false;
        }
        match (
            left_index.map(|index| &left[index]),
            right_index.map(|index| &right[index]),
        ) {
            (Some((left_number, left)), Some((right_number, right))) => {
                let Some((left_changed, right_changed)) = changed_ranges_with_options(
                    &left.content,
                    &right.content,
                    options,
                    cancellation,
                    highlight_deadline,
                ) else {
                    return false;
                };
                rows.push(FileComparisonRow {
                    left: Some((*left_number, left.content.clone())),
                    right: Some((*right_number, right.content.clone())),
                    state: DirectoryEntryState::Different,
                    left_changed,
                    right_changed,
                    left_ending: Some(left.ending),
                    right_ending: Some(right.ending),
                });
            }
            (Some((left_number, left)), None) => rows.push(FileComparisonRow {
                left: Some((*left_number, left.content.clone())),
                right: None,
                state: DirectoryEntryState::LeftOnly,
                left_changed: whole_range(&left.content).into_iter().collect(),
                right_changed: Vec::new(),
                left_ending: Some(left.ending),
                right_ending: None,
            }),
            (None, Some((right_number, right))) => rows.push(FileComparisonRow {
                left: None,
                right: Some((*right_number, right.content.clone())),
                state: DirectoryEntryState::RightOnly,
                left_changed: Vec::new(),
                right_changed: whole_range(&right.content).into_iter().collect(),
                left_ending: None,
                right_ending: Some(right.ending),
            }),
            (None, None) => unreachable!("a changed-block alignment always contains a line"),
        }
        if let Some(progress) = &options.progress {
            progress.advance(1);
        }
    }
    true
}

const ALIGNMENT_GAP_COST: u32 = 100;
const MAX_BLOCK_ALIGNMENT_LINES: usize = 128;
const MAX_BLOCK_ALIGNMENT_CELLS: usize = 16 * 1024;
const MAX_BLOCK_ALIGNMENT_BYTES: usize = 64 * 1024;
pub(super) const ALIGNMENT_CHARACTER_WORK_BUDGET: usize = 128 * 1024;

/// Aligns a run of inserted/deleted lines inside a larger line diff. `similar`
/// deliberately leaves these runs opaque, so simply zipping them makes one
/// inserted line shift every following replacement. This bounded second pass
/// favours a pair with shared text over two gaps, while retaining a positional
/// pairing for unrelated equal-sized replacements.
fn align_changed_block(
    left: &[(usize, DisplayLine)],
    right: &[(usize, DisplayLine)],
    cancellation: &AtomicBool,
    options: &FileViewOptions,
    deadline: Instant,
) -> Vec<(Option<usize>, Option<usize>)> {
    let can_align = left.len() <= MAX_BLOCK_ALIGNMENT_LINES
        && right.len() <= MAX_BLOCK_ALIGNMENT_LINES
        && left.len().saturating_mul(right.len()) <= MAX_BLOCK_ALIGNMENT_CELLS
        && left
            .iter()
            .chain(right)
            .map(|(_, line)| line.content.len())
            .sum::<usize>()
            <= MAX_BLOCK_ALIGNMENT_BYTES
        && !cancelled(cancellation)
        && Instant::now() < deadline;
    if !can_align {
        return positional_alignment(left.len(), right.len());
    }

    let left_text = alignment_text(left, options);
    let right_text = alignment_text(right, options);
    let width = right.len() + 1;
    let mut character_work_remaining = ALIGNMENT_CHARACTER_WORK_BUDGET;
    let mut costs = vec![0_u32; (left.len() + 1) * width];
    let mut steps = vec![0_u8; costs.len()];
    for left_index in 1..=left.len() {
        costs[left_index * width] = left_index as u32 * ALIGNMENT_GAP_COST;
        steps[left_index * width] = 1;
    }
    for right_index in 1..=right.len() {
        costs[right_index] = right_index as u32 * ALIGNMENT_GAP_COST;
        steps[right_index] = 2;
    }
    for left_index in 1..=left.len() {
        for right_index in 1..=right.len() {
            if cancelled(cancellation) || Instant::now() >= deadline {
                return positional_alignment(left.len(), right.len());
            }
            let diagonal = costs[(left_index - 1) * width + right_index - 1]
                + match line_pair_cost(
                    &left_text[left_index - 1],
                    &right_text[right_index - 1],
                    cancellation,
                    deadline,
                    &mut character_work_remaining,
                ) {
                    Some(cost) => cost,
                    None => return positional_alignment(left.len(), right.len()),
                };
            let left_only = costs[(left_index - 1) * width + right_index] + ALIGNMENT_GAP_COST;
            let right_only = costs[left_index * width + right_index - 1] + ALIGNMENT_GAP_COST;
            let (cost, step) = if diagonal < left_only && diagonal < right_only {
                (diagonal, 0)
            } else if left_only <= right_only {
                (left_only, 1)
            } else {
                (right_only, 2)
            };
            let offset = left_index * width + right_index;
            costs[offset] = cost;
            steps[offset] = step;
        }
    }

    let mut aligned = Vec::with_capacity(left.len().max(right.len()));
    let (mut left_index, mut right_index) = (left.len(), right.len());
    while left_index != 0 || right_index != 0 {
        match steps[left_index * width + right_index] {
            0 => {
                left_index -= 1;
                right_index -= 1;
                aligned.push((Some(left_index), Some(right_index)));
            }
            1 => {
                left_index -= 1;
                aligned.push((Some(left_index), None));
            }
            2 => {
                right_index -= 1;
                aligned.push((None, Some(right_index)));
            }
            _ => unreachable!("invalid changed-block alignment step"),
        }
    }
    aligned.reverse();
    aligned
}

fn positional_alignment(left_len: usize, right_len: usize) -> Vec<(Option<usize>, Option<usize>)> {
    (0..left_len.max(right_len))
        .map(|index| {
            (
                (index < left_len).then_some(index),
                (index < right_len).then_some(index),
            )
        })
        .collect()
}

fn alignment_text<'a>(
    lines: &'a [(usize, DisplayLine)],
    options: &FileViewOptions,
) -> Vec<Cow<'a, str>> {
    lines
        .iter()
        .map(|(_, line)| {
            if options.ignore_whitespace {
                Cow::Owned(
                    line.content
                        .chars()
                        .filter(|character| !character.is_whitespace())
                        .collect(),
                )
            } else {
                Cow::Borrowed(line.content.as_str())
            }
        })
        .collect()
}

pub(super) fn line_pair_cost(
    left: &str,
    right: &str,
    cancellation: &AtomicBool,
    deadline: Instant,
    character_work_remaining: &mut usize,
) -> Option<u32> {
    if cancelled(cancellation) || Instant::now() >= deadline {
        return None;
    }
    if left == right {
        return Some(0);
    }
    let left_length = left.chars().count();
    let right_length = right.chars().count();
    let max_length = left_length.max(right_length);
    if max_length == 0 {
        return Some(145);
    }
    let prefix = left
        .chars()
        .zip(right.chars())
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = left
        .chars()
        .rev()
        .zip(right.chars().rev())
        .take(left_length.min(right_length).saturating_sub(prefix))
        .take_while(|(left, right)| left == right)
        .count();
    let shared_percent = (prefix + suffix) * 100 / max_length;
    if shared_percent >= 70 {
        return Some(pair_cost_for_percent(shared_percent));
    }
    let character_work = left.len().saturating_mul(right.len());
    if character_work <= *character_work_remaining {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        *character_work_remaining -= character_work;
        let diff = SimilarTextDiff::configure()
            .timeout(remaining)
            .diff_chars(left, right);
        if cancelled(cancellation) || Instant::now() >= deadline {
            return None;
        }
        let common = diff
            .iter_all_changes()
            .filter(|change| change.tag() == ChangeTag::Equal)
            .map(|change| change.value().chars().count())
            .sum::<usize>();
        return Some(pair_cost_for_percent(common * 100 / max_length));
    }
    Some(pair_cost_for_percent(shared_percent))
}

fn pair_cost_for_percent(shared_percent: usize) -> u32 {
    150_u32.saturating_sub(shared_percent.saturating_sub(20) as u32)
}
