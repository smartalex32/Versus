//! Display mapping for editable Merge views.
//!
//! The workspace owns canonical documents and their history. This module only
//! retains derived decorations and translates between those source documents
//! and the compact, changed-lines editor presentation.

use super::*;
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone, Default)]
pub(super) struct MergeDecoration {
    lines: BTreeMap<usize, (DirectoryEntryState, Vec<Range<usize>>)>,
    endings: BTreeMap<usize, versus::DisplayLineEnding>,
}

pub(super) struct PendingMergePresentation {
    sides: [MergeDecoration; 2],
    pub(super) changes: usize,
    pub(super) legend_counts: Option<[usize; 6]>,
}

impl PendingMergePresentation {
    pub(super) fn rebase(&mut self, side: usize, before: &str, range: Range<usize>, after: &str) {
        self.sides[side].rebase(before, range, after);
    }
}

/// Derived state for a Merge comparison while a fresh comparison is pending.
/// It deliberately cannot provide comparison rows or merge targets.
#[derive(Default)]
pub(super) struct MergePresentation {
    pending: Option<PendingMergePresentation>,
    editor_projections: [Option<EditorProjection>; 2],
}

impl MergePresentation {
    pub(super) fn clear(&mut self) {
        self.pending = None;
    }

    pub(super) fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub(super) fn pending_counts(&self) -> Option<[usize; 6]> {
        self.pending
            .as_ref()
            .and_then(|pending| pending.legend_counts)
    }

    pub(super) fn pending_changes(&self) -> Option<usize> {
        self.pending.as_ref().map(|pending| pending.changes)
    }

    pub(super) fn decoration(
        &self,
        side: usize,
        comparison: Option<&FileComparison>,
        ignore_line_endings: bool,
    ) -> MergeDecoration {
        comparison.map_or_else(
            || {
                self.pending
                    .as_ref()
                    .map(|pending| pending.sides[side].clone())
                    .unwrap_or_default()
            },
            |comparison| MergeDecoration::from_comparison(comparison, side, ignore_line_endings),
        )
    }

    pub(super) fn capture(
        &self,
        comparison: Option<&FileComparison>,
        ignore_line_endings: bool,
        legend_counts: Option<[usize; 6]>,
        changes: usize,
    ) -> PendingMergePresentation {
        PendingMergePresentation {
            sides: std::array::from_fn(|side| {
                self.decoration(side, comparison, ignore_line_endings)
            }),
            changes: self.pending_changes().unwrap_or(changes),
            legend_counts,
        }
    }

    pub(super) fn replace_pending(&mut self, pending: Option<PendingMergePresentation>) {
        self.pending = pending;
    }

    pub(super) fn update_editor_cursor(
        &mut self,
        side: usize,
        ctx: &egui::Context,
        id: egui::Id,
        next: EditorProjection,
    ) {
        if let Some(previous) = self.editor_projections[side].as_ref() {
            if previous != &next {
                if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
                    if let Some(cursor) = state.cursor.char_range() {
                        state
                            .cursor
                            .set_char_range(Some(remap_projection_cursor(cursor, previous, &next)));
                    }
                    state.store(ctx, id);
                }
            }
        }
        self.editor_projections[side] = Some(next);
    }

    pub(super) fn set_full_editor(&mut self, side: usize, text: String) {
        self.editor_projections[side] = Some(EditorProjection::full(text));
    }
}

impl MergeDecoration {
    pub(super) fn from_parts(
        lines: BTreeMap<usize, (DirectoryEntryState, Vec<Range<usize>>)>,
        endings: BTreeMap<usize, versus::DisplayLineEnding>,
    ) -> Self {
        Self { lines, endings }
    }

    pub(super) fn into_parts(
        self,
    ) -> (
        BTreeMap<usize, (DirectoryEntryState, Vec<Range<usize>>)>,
        BTreeMap<usize, versus::DisplayLineEnding>,
    ) {
        (self.lines, self.endings)
    }

    pub(super) fn is_changed(&self, line: usize) -> bool {
        self.lines
            .get(&line)
            .is_some_and(|(state, _)| *state != DirectoryEntryState::Same)
    }
    fn from_comparison(
        comparison: &FileComparison,
        side: usize,
        ignore_line_endings: bool,
    ) -> Self {
        let mut decoration = Self::default();
        for row in &comparison.rows {
            let (line, changed, ending) = if side == 0 {
                (&row.left, &row.left_changed, row.left_ending)
            } else {
                (&row.right, &row.right_changed, row.right_ending)
            };
            if let Some((number, _)) = line {
                decoration
                    .lines
                    .insert(number - 1, (row.state.clone(), changed.clone()));
                if !ignore_line_endings && row.left_ending != row.right_ending {
                    if let Some(ending) = ending {
                        decoration.endings.insert(number - 1, ending);
                    }
                }
            }
        }
        decoration
    }

    pub(super) fn rebase(&mut self, before: &str, range: Range<usize>, after: &str) {
        let old = text_segments(before);
        let new = text_segments(after);
        let line_at = |byte: usize| {
            let mut offset = 0;
            for (line, (text, ending)) in old.iter().enumerate() {
                offset += text.len() + ending.len();
                if byte < offset || (byte == offset && ending.is_empty()) {
                    return line;
                }
            }
            old.len()
        };
        let start = line_at(range.start);
        let end = (line_at(range.end) + 1).min(old.len());
        let delta = new.len() as isize - old.len() as isize;
        let new_end = end.saturating_add_signed(delta).min(new.len());
        let previous = self.lines.get(&start).cloned();
        let ending = (end == start + 1 && new_end == end && old[start].1 == new[start].1)
            .then(|| self.endings.get(&start).copied())
            .flatten();
        let inline = if end == start + 1 && new_end == end {
            let line_start: usize = old[..start]
                .iter()
                .map(|(text, ending)| text.len() + ending.len())
                .sum();
            (range.end <= line_start + old[start].0.len()).then(|| {
                let edit = range.start - line_start..range.end - line_start;
                let inserted = new[start].0.len() + edit.len() - old[start].0.len();
                let shift = inserted as isize - edit.len() as isize;
                let mut ranges: Vec<Range<usize>> = previous
                    .as_ref()
                    .into_iter()
                    .flat_map(|(_, ranges)| ranges)
                    .map(|span| {
                        let a = if span.start <= edit.start {
                            span.start
                        } else if span.start >= edit.end {
                            span.start.saturating_add_signed(shift)
                        } else {
                            edit.start
                        };
                        let b = if span.end <= edit.start {
                            span.end
                        } else if span.end >= edit.end {
                            span.end.saturating_add_signed(shift)
                        } else {
                            edit.start + inserted
                        };
                        a..b
                    })
                    .filter(|span| !span.is_empty())
                    .collect();
                if inserted > 0 {
                    ranges.push(edit.start..edit.start + inserted);
                }
                ranges.sort_by_key(|span| span.start);
                let mut merged: Vec<Range<usize>> = Vec::new();
                for span in ranges {
                    if let Some(last) = merged.last_mut().filter(|last| span.start <= last.end) {
                        last.end = last.end.max(span.end);
                    } else {
                        merged.push(span);
                    }
                }
                merged
            })
        } else {
            None
        };
        let remap = |line: usize| {
            if line < start {
                Some(line)
            } else if line >= end {
                Some(line.saturating_add_signed(delta))
            } else {
                None
            }
        };
        self.lines = std::mem::take(&mut self.lines)
            .into_iter()
            .filter_map(|(line, value)| Some((remap(line)?, value)))
            .collect();
        self.endings = std::mem::take(&mut self.endings)
            .into_iter()
            .filter_map(|(line, value)| Some((remap(line)?, value)))
            .collect();
        if let Some(ending) = ending {
            self.endings.insert(start, ending);
        }
        for (line, (text, _)) in new.iter().enumerate().take(new_end).skip(start) {
            self.lines.insert(
                line,
                (
                    previous
                        .as_ref()
                        .map(|(state, _)| state)
                        .filter(|state| **state != DirectoryEntryState::Same)
                        .cloned()
                        .unwrap_or(DirectoryEntryState::Different),
                    inline.clone().unwrap_or_else(|| {
                        if text.is_empty() {
                            Vec::new()
                        } else {
                            vec![0..text.len()]
                        }
                    }),
                ),
            );
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct MergeProjection {
    text: String,
    spans: Vec<ProjectionSpan>,
}

#[derive(Clone, Debug)]
struct ProjectionSpan {
    pub(super) source_line: usize,
    source: Range<usize>,
    projected: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct EditorProjection {
    text: String,
    source_lines: Vec<usize>,
}

impl EditorProjection {
    pub(super) fn new(text: String, source_lines: Vec<usize>) -> Self {
        Self { text, source_lines }
    }

    fn full(text: String) -> Self {
        let source_lines = (0..text_segments(&text).len()).collect();
        Self { text, source_lines }
    }
}

impl MergeProjection {
    pub(super) fn text(&self) -> &str {
        &self.text
    }

    pub(super) fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    pub(super) fn source_lines(&self) -> impl Iterator<Item = usize> + '_ {
        self.spans.iter().map(|span| span.source_line)
    }
}

pub(super) fn merge_projection(displayed: &str, changed: &MergeDecoration) -> MergeProjection {
    let mut projection = MergeProjection {
        text: String::new(),
        spans: Vec::new(),
    };
    let mut source_start = 0;
    for (line, content) in displayed.split_inclusive('\n').enumerate() {
        let source = source_start..source_start + content.len();
        source_start = source.end;
        if !changed
            .lines
            .get(&line)
            .is_some_and(|(state, _)| *state != DirectoryEntryState::Same)
        {
            continue;
        }
        let projected_start = projection.text.len();
        projection.text.push_str(content);
        projection.spans.push(ProjectionSpan {
            source_line: line,
            source,
            projected: projected_start..projection.text.len(),
        });
    }
    projection
}

pub(super) fn visible_chunk_runs(
    range: &Range<usize>,
    visible_lines: &[usize],
) -> Vec<Range<usize>> {
    let mut runs: Vec<Range<usize>> = Vec::new();
    for line in visible_lines
        .iter()
        .copied()
        .filter(|line| range.contains(line))
    {
        match runs.last_mut() {
            Some(last) if last.end == line => last.end += 1,
            _ => runs.push(line..line + 1),
        }
    }
    runs
}

pub(super) fn remap_projection_cursor(
    cursor: egui::text::CCursorRange,
    old: &EditorProjection,
    new: &EditorProjection,
) -> egui::text::CCursorRange {
    let source_position = |projection: &EditorProjection, cursor: egui::text::CCursor| {
        let index: usize = cursor.index.into();
        let prefix: String = projection.text.chars().take(index).collect();
        let display_line = prefix
            .chars()
            .filter(|character| *character == '\n')
            .count();
        let column = prefix.rsplit_once('\n').map_or_else(
            || prefix.chars().count(),
            |(_, column)| column.chars().count(),
        );
        let source_line = projection
            .source_lines
            .get(display_line)
            .copied()
            .or_else(|| projection.source_lines.last().map(|line| line + 1))
            .unwrap_or(0);
        (source_line, column)
    };
    let projected_cursor = |(source_line, column): (usize, usize)| {
        let display_line = new
            .source_lines
            .iter()
            .position(|line| *line == source_line)
            .unwrap_or_else(|| {
                new.source_lines
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, line)| line.abs_diff(source_line))
                    .map(|(index, _)| index)
                    .unwrap_or(0)
            });
        let line_start = new
            .text
            .split_inclusive('\n')
            .take(display_line)
            .map(|line| line.chars().count())
            .sum::<usize>();
        let line = new
            .text
            .split_inclusive('\n')
            .nth(display_line)
            .unwrap_or("");
        egui::text::CCursor::new(
            line_start + column.min(line.trim_end_matches('\n').chars().count()),
        )
    };
    egui::text::CCursorRange::two(
        projected_cursor(source_position(old, cursor.primary)),
        projected_cursor(source_position(old, cursor.secondary)),
    )
}

pub(super) fn projected_editor_edit(
    original: &str,
    projection: &MergeProjection,
    edited: &str,
    selection: Option<Range<usize>>,
) -> Result<(Range<usize>, String), &'static str> {
    let (projected_range, replacement) = editor_byte_edit(&projection.text, edited, selection);
    let first = projection
        .spans
        .iter()
        .position(|span| {
            span.projected.start <= projected_range.start
                && (projected_range.start < span.projected.end
                    || (projected_range.start == projection.text.len()
                        && span.projected.end == projection.text.len()))
        })
        .ok_or("This edit is outside the visible changed lines.")?;
    let last = if projected_range.is_empty() {
        first
    } else {
        projection.spans.iter().position(|span| {
            span.projected.start <= projected_range.end && projected_range.end <= span.projected.end
        }).ok_or("This edit crosses hidden unchanged lines. Turn off Show only differences to edit across them.")?
    };
    let start_span = &projection.spans[first];
    let end_span = &projection.spans[last];
    let normalized_original = original.replace("\r\n", "\n").replace('\r', "\n");
    if projection.spans[first..=last]
        .windows(2)
        .any(|pair| pair[1].source_line != pair[0].source_line + 1)
    {
        return Err(
            "This edit crosses hidden unchanged lines. Turn off Show only differences to edit across them.",
        );
    }
    if projection.spans[first..=last]
        .iter()
        .enumerate()
        .any(|(offset, span)| {
            let index = first + offset;
            let next_is_hidden = projection
                .spans
                .get(index + 1)
                .map_or(span.source.end < normalized_original.len(), |next| {
                    next.source_line != span.source_line + 1
                });
            let newline = span
                .projected
                .end
                .checked_sub(1)
                .and_then(|index| projection.text.as_bytes().get(index))
                == Some(&b'\n');
            next_is_hidden
                && newline
                && projected_range.start <= span.projected.end - 1
                && span.projected.end <= projected_range.end
        })
    {
        return Err(
            "This edit would join hidden unchanged lines. Turn off Show only differences to make that change.",
        );
    }
    let normalized = start_span.source.start + (projected_range.start - start_span.projected.start)
        ..end_span.source.start + (projected_range.end - end_span.projected.start);
    let range = normalized_byte_to_original(original, normalized.start)
        ..normalized_byte_to_original(original, normalized.end);
    let replacement = replacement.replace('\n', line_ending_at(original, &range));
    Ok((range, replacement))
}

pub(super) fn text_segments(text: &str) -> Vec<(&str, &str)> {
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    let mut lines = Vec::new();
    while i < bytes.len() {
        if bytes[i] == b'\n' || bytes[i] == b'\r' {
            let len = if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
            lines.push((&text[start..i], &text[i..i + len]));
            i += len;
            start = i;
        } else {
            i += 1;
        }
    }
    if start < text.len() {
        lines.push((&text[start..], ""));
    }
    lines
}

pub(super) fn normalized_byte_to_original(original: &str, normalized: usize) -> usize {
    let mut byte = 0;
    let mut shown = 0;
    while shown < normalized && byte < original.len() {
        byte += if original.as_bytes()[byte] == b'\r'
            && original.as_bytes().get(byte + 1) == Some(&b'\n')
        {
            2
        } else {
            1
        };
        shown += 1;
    }
    byte
}

pub(super) fn original_byte_to_normalized_chars(original: &str, byte_offset: usize) -> usize {
    let mut byte = 0;
    let mut chars = 0;
    while byte < byte_offset && byte < original.len() {
        if original.as_bytes()[byte] == b'\r' && original.as_bytes().get(byte + 1) == Some(&b'\n') {
            byte += 2;
        } else {
            byte += original[byte..].chars().next().map_or(1, char::len_utf8);
        }
        chars += 1;
    }
    chars
}

pub(super) fn editor_byte_edit(
    original: &str,
    edited: &str,
    selection: Option<Range<usize>>,
) -> (Range<usize>, String) {
    let displayed = original.replace("\r\n", "\n").replace('\r', "\n");
    let char_byte = |index: usize| {
        displayed
            .char_indices()
            .nth(index)
            .map_or(displayed.len(), |(byte, _)| byte)
    };
    let valid = |range: &Range<usize>| -> Option<Range<usize>> {
        if range.start > range.end
            || range.end > displayed.len()
            || !displayed.is_char_boundary(range.start)
            || !displayed.is_char_boundary(range.end)
        {
            return None;
        }
        let suffix = displayed.len() - range.end;
        let end = edited.len().checked_sub(suffix)?;
        if range.start > end
            || !edited.is_char_boundary(range.start)
            || !edited.is_char_boundary(end)
        {
            return None;
        }
        (displayed[..range.start] == edited[..range.start]
            && displayed[range.end..] == edited[end..])
            .then_some(range.start..end)
    };
    let mut candidates = Vec::new();
    if let Some(selection) = selection {
        let selected = char_byte(selection.start)..char_byte(selection.end);
        candidates.push(selected.clone());
        if selected.is_empty() && displayed.len() > edited.len() {
            let deleted = displayed.len() - edited.len();
            if selected.start >= deleted {
                candidates.push(selected.start - deleted..selected.start);
            }
            candidates.push(selected.start..selected.start.saturating_add(deleted));
        }
    }
    let (old, new) = candidates
        .into_iter()
        .find_map(|range| valid(&range).map(|replacement| (range, replacement)))
        .unwrap_or_else(|| {
            let start = displayed
                .chars()
                .zip(edited.chars())
                .take_while(|(a, b)| a == b)
                .map(|(c, _)| c.len_utf8())
                .sum::<usize>();
            let suffix = displayed[start..]
                .chars()
                .rev()
                .zip(edited[start..].chars().rev())
                .take_while(|(a, b)| a == b)
                .map(|(c, _)| c.len_utf8())
                .sum::<usize>();
            (
                start..displayed.len() - suffix,
                start..edited.len() - suffix,
            )
        });
    let range = normalized_byte_to_original(original, old.start)
        ..normalized_byte_to_original(original, old.end);
    (
        range.clone(),
        edited[new].replace('\n', line_ending_at(original, &range)),
    )
}

fn line_ending_at<'a>(original: &'a str, range: &Range<usize>) -> &'a str {
    let mut offset = 0;
    text_segments(original)
        .iter()
        .find_map(|(text, ending)| {
            let end = offset + text.len() + ending.len();
            let contains = range.start < end;
            offset = end;
            (contains && !ending.is_empty()).then_some(*ending)
        })
        .or_else(|| {
            text_segments(original)
                .iter()
                .find_map(|(_, ending)| (!ending.is_empty()).then_some(*ending))
        })
        .unwrap_or("\n")
}

#[cfg(test)]
mod test_support;
