//! Line-ending normalization and inline text highlighting for comparison rows.

use super::{DisplayLineEnding, FileViewOptions, cancelled};
use similar::{ChangeTag, TextDiff as SimilarTextDiff};
use std::{ops::Range, sync::atomic::AtomicBool, time::Instant};

#[derive(Clone, Debug)]
pub(super) struct DisplayLine {
    pub(super) content: String,
    pub(super) ending: DisplayLineEnding,
}

pub(super) fn lines_for_view(text: &str) -> Vec<DisplayLine> {
    let mut lines = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let (ending, width) = match bytes[index] {
            b'\n' => (DisplayLineEnding::Lf, 1),
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => (DisplayLineEnding::CrLf, 2),
            b'\r' => (DisplayLineEnding::Cr, 1),
            _ => {
                index += 1;
                continue;
            }
        };
        lines.push(DisplayLine {
            content: text[start..index].to_owned(),
            ending,
        });
        index += width;
        start = index;
    }
    if start < text.len() {
        lines.push(DisplayLine {
            content: text[start..].to_owned(),
            ending: DisplayLineEnding::None,
        });
    }
    lines
}

pub(super) fn comparable_lines(lines: &[DisplayLine], options: &FileViewOptions) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            let mut content = normalized_text(&line.content, options.ignore_whitespace);
            if !options.ignore_line_endings {
                content.push('\u{0}');
                content.push(match line.ending {
                    DisplayLineEnding::None => '0',
                    DisplayLineEnding::Lf => '1',
                    DisplayLineEnding::CrLf => '2',
                    DisplayLineEnding::Cr => '3',
                });
            }
            content
        })
        .collect()
}

pub(super) fn whole_range(text: &str) -> Option<Range<usize>> {
    (!text.is_empty()).then_some(0..text.len())
}

const HIGHLIGHT_LINE_LIMIT: usize = 16 * 1024;

struct HighlightText {
    normalized: String,
    byte_ranges: Vec<Range<usize>>,
}

pub(super) fn changed_ranges_with_options(
    left: &str,
    right: &str,
    options: &FileViewOptions,
    cancellation: &AtomicBool,
    deadline: Instant,
) -> Option<(Vec<Range<usize>>, Vec<Range<usize>>)> {
    if cancelled(cancellation) {
        return None;
    }
    if left.len().saturating_add(right.len()) > HIGHLIGHT_LINE_LIMIT {
        let equal = left == right
            || (options.ignore_whitespace
                && left
                    .chars()
                    .filter(|character| !character.is_whitespace())
                    .eq(right.chars().filter(|character| !character.is_whitespace())));
        if cancelled(cancellation) {
            return None;
        }
        return Some(if equal {
            (Vec::new(), Vec::new())
        } else {
            (
                whole_range(left).into_iter().collect(),
                whole_range(right).into_iter().collect(),
            )
        });
    }
    let left = highlight_text(left, options.ignore_whitespace);
    let right = highlight_text(right, options.ignore_whitespace);
    if left.normalized == right.normalized {
        return Some((Vec::new(), Vec::new()));
    }
    if Instant::now() >= deadline {
        return Some((
            merge_ranges(left.byte_ranges),
            merge_ranges(right.byte_ranges),
        ));
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    let diff = SimilarTextDiff::configure()
        .timeout(remaining)
        .diff_chars(left.normalized.as_str(), right.normalized.as_str());
    if cancelled(cancellation) {
        return None;
    }
    if Instant::now() >= deadline {
        return Some((
            merge_ranges(left.byte_ranges),
            merge_ranges(right.byte_ranges),
        ));
    }
    let mut left_changed = Vec::new();
    let mut right_changed = Vec::new();
    let mut left_index = 0;
    let mut right_index = 0;
    for change in diff.iter_all_changes() {
        if cancelled(cancellation) {
            return None;
        }
        match change.tag() {
            ChangeTag::Delete => {
                left_changed.push(left.byte_ranges[left_index].clone());
                left_index += 1;
            }
            ChangeTag::Insert => {
                right_changed.push(right.byte_ranges[right_index].clone());
                right_index += 1;
            }
            ChangeTag::Equal => {
                left_index += 1;
                right_index += 1;
            }
        }
    }
    Some((merge_ranges(left_changed), merge_ranges(right_changed)))
}

fn normalized_text(text: &str, ignore_whitespace: bool) -> String {
    if ignore_whitespace {
        text.chars()
            .filter(|character| !character.is_whitespace())
            .collect()
    } else {
        text.to_owned()
    }
}

fn highlight_text(text: &str, ignore_whitespace: bool) -> HighlightText {
    let mut normalized = String::new();
    let mut byte_ranges = Vec::new();
    for (start, character) in text.char_indices() {
        if ignore_whitespace && character.is_whitespace() {
            continue;
        }
        normalized.push(character);
        byte_ranges.push(start..start + character.len_utf8());
    }
    HighlightText {
        normalized,
        byte_ranges,
    }
}

fn merge_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_unstable_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(previous) if range.start <= previous.end => {
                previous.end = previous.end.max(range.end);
            }
            _ => merged.push(range),
        }
    }
    merged
}
