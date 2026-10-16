//! Diff-independent state for interactive two-way editing.
//!
//! This model owns working copies, history, and automatic structural/paragraph
//! chunks. It deliberately has no protected ranges or user-defined chunks.

use super::chunk_detection::detect_chunks;
use super::text_lines::lines;
use std::{
    fmt,
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);
pub fn revision() -> u64 {
    NEXT_REVISION.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum HistoryTag {
    Local(u64),
    FolderBatch(u64),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum TwoWaySide {
    Left,
    Right,
}
impl TwoWaySide {
    pub fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
    fn index(self) -> usize {
        matches!(self, Self::Right) as usize
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferPlacement {
    Insert,
    Replace,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChunkKind {
    Structural,
    Paragraph,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InteractiveChunk {
    pub id: u64,
    pub range: Range<usize>,
    pub kind: ChunkKind,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TwoWayMergeError {
    InvalidLineRange {
        side: TwoWaySide,
        range: Range<usize>,
    },
    InvalidInsertionRange(Range<usize>),
    InvalidByteRange(Range<usize>),
    OverlappingDestinationRanges,
    UnknownChunk(u64),
}
impl fmt::Display for TwoWayMergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLineRange { side, range } => write!(
                f,
                "invalid {side:?} line range {}..{}",
                range.start, range.end
            ),
            Self::InvalidInsertionRange(range) => write!(
                f,
                "an insertion target must have equal bounds, got {}..{}",
                range.start, range.end
            ),
            Self::InvalidByteRange(range) => {
                write!(f, "invalid byte range {}..{}", range.start, range.end)
            }
            Self::OverlappingDestinationRanges => {
                f.write_str("batch transfer destination ranges overlap")
            }
            Self::UnknownChunk(id) => write!(f, "chunk {id} no longer exists"),
        }
    }
}
impl std::error::Error for TwoWayMergeError {}

#[derive(Clone, Debug)]
struct Snapshot {
    text: String,
}
#[derive(Clone, Debug)]
struct DocumentHistory {
    tag: HistoryTag,
    timestamp: u64,
    before: Arc<Snapshot>,
    after: Arc<Snapshot>,
}

/// Canonical editable state for one file, including transactional history.
#[derive(Clone, Debug)]
pub struct InteractiveDocument {
    text: String,
    saved: String,
    undo: Vec<DocumentHistory>,
    redo: Vec<DocumentHistory>,
    revision: u64,
}
impl InteractiveDocument {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            saved: text.clone(),
            text,
            undo: Vec::new(),
            redo: Vec::new(),
            revision: revision(),
        }
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn is_dirty(&self) -> bool {
        self.text != self.saved
    }
    pub fn baseline(&self) -> &str {
        &self.saved
    }
    pub fn mark_saved_text(&mut self, saved: impl Into<String>) {
        self.saved = saved.into();
        self.revision = revision();
    }
    pub fn next_undo_tag(&self) -> Option<HistoryTag> {
        self.undo.last().map(|entry| entry.tag)
    }
    pub fn next_redo_tag(&self) -> Option<HistoryTag> {
        self.redo.last().map(|entry| entry.tag)
    }
    fn undo_timestamp(&self) -> Option<u64> {
        self.undo.last().map(|entry| entry.timestamp)
    }
    fn redo_timestamp(&self) -> Option<u64> {
        self.redo.last().map(|entry| entry.timestamp)
    }
    pub fn undo_expected(&mut self, tag: HistoryTag) -> bool {
        let Some(entry) = self.undo.last() else {
            return false;
        };
        if entry.tag != tag {
            return false;
        };
        let entry = self.undo.pop().expect("history entry checked");
        self.text = entry.before.text.clone();
        self.redo.push(entry);
        self.revision = revision();
        true
    }
    pub fn redo_expected(&mut self, tag: HistoryTag) -> bool {
        let Some(entry) = self.redo.last() else {
            return false;
        };
        if entry.tag != tag {
            return false;
        };
        let entry = self.redo.pop().expect("history entry checked");
        self.text = entry.after.text.clone();
        self.undo.push(entry);
        self.revision = revision();
        true
    }
    /// Applies disjoint UTF-8 byte edits as one transaction.
    pub fn apply_byte_edits(
        &mut self,
        tag: HistoryTag,
        mut edits: Vec<(Range<usize>, String)>,
    ) -> Result<(), TwoWayMergeError> {
        if edits.is_empty() {
            return Ok(());
        }
        for (range, _) in &edits {
            if range.start > range.end
                || range.end > self.text.len()
                || !self.text.is_char_boundary(range.start)
                || !self.text.is_char_boundary(range.end)
            {
                return Err(TwoWayMergeError::InvalidByteRange(range.clone()));
            }
        }
        edits.sort_by(|left, right| right.0.start.cmp(&left.0.start));
        if edits.windows(2).any(|pair| pair[1].0.end > pair[0].0.start) {
            return Err(TwoWayMergeError::OverlappingDestinationRanges);
        }
        let before = Snapshot {
            text: self.text.clone(),
        };
        for (range, replacement) in edits {
            self.text.replace_range(range, &replacement);
        }
        self.push_history(
            tag,
            before,
            Snapshot {
                text: self.text.clone(),
            },
        );
        self.revision = revision();
        Ok(())
    }
    fn push_history(&mut self, tag: HistoryTag, before: Snapshot, after: Snapshot) {
        self.undo.push(DocumentHistory {
            tag,
            timestamp: revision(),
            before: Arc::new(before),
            after: Arc::new(after),
        });
        self.redo.clear();
        let mut bytes: usize = self
            .undo
            .iter()
            .map(|entry| entry.before.text.len() + entry.after.text.len())
            .sum();
        while self.undo.len() > 1 && (self.undo.len() > 256 || bytes > 64 * 1024 * 1024) {
            let oldest = self.undo.remove(0);
            bytes -= oldest.before.text.len() + oldest.after.text.len();
        }
    }
}

/// Two editable working copies. All line ranges are zero-based and half-open.
#[derive(Clone, Debug)]
pub struct TwoWayWorkspace {
    documents: [InteractiveDocument; 2],
    text: [String; 2],
    saved: [String; 2],
    pending_before: [Option<Snapshot>; 2],
}
impl TwoWayWorkspace {
    pub fn new(left: impl Into<String>, right: impl Into<String>) -> Self {
        let text = [left.into(), right.into()];
        Self {
            documents: [
                InteractiveDocument::new(text[0].clone()),
                InteractiveDocument::new(text[1].clone()),
            ],
            saved: text.clone(),
            text,
            pending_before: [None, None],
        }
    }
    pub fn from_documents(documents: [InteractiveDocument; 2]) -> Self {
        let mut workspace = Self::new(documents[0].text.clone(), documents[1].text.clone());
        workspace.saved = [documents[0].saved.clone(), documents[1].saved.clone()];
        workspace.documents = documents;
        workspace
    }
    pub fn document(&self, side: TwoWaySide) -> &InteractiveDocument {
        &self.documents[side.index()]
    }
    pub fn text(&self, side: TwoWaySide) -> &str {
        &self.text[side.index()]
    }
    pub fn line_count(&self, side: TwoWaySide) -> usize {
        lines(self.text(side)).len()
    }
    pub fn is_dirty(&self, side: TwoWaySide) -> bool {
        self.text[side.index()] != self.saved[side.index()]
    }
    pub fn has_unsaved_changes(&self) -> bool {
        self.is_dirty(TwoWaySide::Left) || self.is_dirty(TwoWaySide::Right)
    }
    pub fn mark_saved_text(&mut self, side: TwoWaySide, saved: impl Into<String>) {
        self.saved[side.index()] = saved.into();
        self.documents[side.index()].mark_saved_text(self.saved[side.index()].clone());
    }
    pub fn chunks(&self, side: TwoWaySide) -> Vec<InteractiveChunk> {
        detect_chunks(self.text(side))
    }
    pub fn set_text(&mut self, side: TwoWaySide, replacement: impl Into<String>) {
        let replacement = replacement.into();
        if self.text(side) == replacement {
            return;
        }
        self.snapshot(side);
        self.text[side.index()] = replacement;
        self.finish(side);
    }
    pub fn edit_bytes(
        &mut self,
        side: TwoWaySide,
        range: Range<usize>,
        replacement: String,
    ) -> Result<(), TwoWayMergeError> {
        self.documents[side.index()]
            .apply_byte_edits(HistoryTag::Local(revision()), vec![(range, replacement)])?;
        self.project_documents();
        Ok(())
    }
    pub fn transfer(
        &mut self,
        source: TwoWaySide,
        source_range: Range<usize>,
        destination_range: Range<usize>,
        placement: TransferPlacement,
    ) -> Result<(), TwoWayMergeError> {
        let destination = source.other();
        let mut replacement = self.prepare(
            source,
            &source_range,
            destination,
            &destination_range,
            placement,
        )?;
        if placement == TransferPlacement::Insert {
            replacement = separated_insertion(
                &replacement,
                self.text(destination),
                destination_range.start,
            );
        }
        self.snapshot(destination);
        self.replace_lines(destination, destination_range, replacement);
        self.finish(destination);
        Ok(())
    }
    pub fn transfer_batch(
        &mut self,
        source: TwoWaySide,
        transfers: Vec<(Range<usize>, Range<usize>, TransferPlacement)>,
    ) -> Result<(), TwoWayMergeError> {
        if transfers.is_empty() {
            return Ok(());
        };
        let destination = source.other();
        let destination_text = self.text(destination).to_owned();
        let mut prepared: Vec<_> = transfers
            .into_iter()
            .map(|(source_range, destination_range, placement)| {
                self.prepare(
                    source,
                    &source_range,
                    destination,
                    &destination_range,
                    placement,
                )
                .map(|replacement| (source_range, destination_range, placement, replacement))
            })
            .collect::<Result<_, _>>()?;
        validate_batch_targets(&prepared)?;
        prepared.sort_by(|left, right| {
            right
                .1
                .start
                .cmp(&left.1.start)
                .then_with(|| right.1.end.cmp(&left.1.end))
                .then_with(|| right.0.start.cmp(&left.0.start))
        });
        self.snapshot(destination);
        let mut index = 0;
        while index < prepared.len() {
            let (_, target, placement, _) = &prepared[index];
            if *placement == TransferPlacement::Insert {
                let target = target.clone();
                let mut end = index + 1;
                while end < prepared.len()
                    && prepared[end].2 == TransferPlacement::Insert
                    && prepared[end].1 == target
                {
                    end += 1;
                }
                let joined = prepared[index..end]
                    .iter()
                    .rev()
                    .map(|entry| entry.3.as_str())
                    .collect::<String>();
                self.replace_lines(
                    destination,
                    target.clone(),
                    separated_insertion(&joined, &destination_text, target.start),
                );
                index = end;
            } else {
                self.replace_lines(destination, target.clone(), prepared[index].3.clone());
                index += 1;
            }
        }
        self.finish(destination);
        Ok(())
    }
    pub fn transfer_chunk(
        &mut self,
        source: TwoWaySide,
        chunk_id: u64,
        destination_range: Range<usize>,
        placement: TransferPlacement,
    ) -> Result<(), TwoWayMergeError> {
        let range = self
            .chunks(source)
            .into_iter()
            .find(|chunk| chunk.id == chunk_id)
            .map(|chunk| chunk.range)
            .ok_or(TwoWayMergeError::UnknownChunk(chunk_id))?;
        self.transfer(source, range, destination_range, placement)
    }
    pub fn can_undo(&self) -> bool {
        self.next_undo_tag().is_some()
    }
    pub fn can_redo(&self) -> bool {
        self.next_redo_tag().is_some()
    }
    pub fn undo(&mut self) -> bool {
        let Some(tag) = self.next_undo_tag() else {
            return false;
        };
        let changed = self.documents[self.side_for_tag(tag, false).index()].undo_expected(tag);
        if changed {
            self.project_documents();
        }
        changed
    }
    pub fn redo(&mut self) -> bool {
        let Some(tag) = self.next_redo_tag() else {
            return false;
        };
        let changed = self.documents[self.side_for_tag(tag, true).index()].redo_expected(tag);
        if changed {
            self.project_documents();
        }
        changed
    }
    pub fn next_undo_tag(&self) -> Option<HistoryTag> {
        match (
            self.documents[0].undo_timestamp(),
            self.documents[1].undo_timestamp(),
        ) {
            (Some(left), Some(right)) => self.documents[usize::from(right > left)].next_undo_tag(),
            (Some(_), None) => self.documents[0].next_undo_tag(),
            (None, Some(_)) => self.documents[1].next_undo_tag(),
            (None, None) => None,
        }
    }
    pub fn next_redo_tag(&self) -> Option<HistoryTag> {
        match (
            self.documents[0].redo_timestamp(),
            self.documents[1].redo_timestamp(),
        ) {
            (Some(left), Some(right)) => self.documents[usize::from(right < left)].next_redo_tag(),
            (Some(_), None) => self.documents[0].next_redo_tag(),
            (None, Some(_)) => self.documents[1].next_redo_tag(),
            (None, None) => None,
        }
    }
    fn validate(&self, side: TwoWaySide, range: &Range<usize>) -> Result<(), TwoWayMergeError> {
        if range.start > range.end || range.end > self.line_count(side) {
            Err(TwoWayMergeError::InvalidLineRange {
                side,
                range: range.clone(),
            })
        } else {
            Ok(())
        }
    }
    fn prepare(
        &self,
        source: TwoWaySide,
        source_range: &Range<usize>,
        destination: TwoWaySide,
        destination_range: &Range<usize>,
        placement: TransferPlacement,
    ) -> Result<String, TwoWayMergeError> {
        self.validate(source, source_range)?;
        self.validate(destination, destination_range)?;
        if placement == TransferPlacement::Insert
            && destination_range.start != destination_range.end
        {
            return Err(TwoWayMergeError::InvalidInsertionRange(
                destination_range.clone(),
            ));
        }
        Ok(lines(self.text(source))[source_range.clone()].concat())
    }
    fn replace_lines(&mut self, side: TwoWaySide, range: Range<usize>, replacement: String) {
        let segments = lines(self.text(side));
        let start = byte_at_line(&segments, range.start);
        let end = byte_at_line(&segments, range.end);
        self.text[side.index()].replace_range(start..end, &replacement);
    }
    fn snapshot(&mut self, side: TwoWaySide) {
        self.pending_before[side.index()] = Some(Snapshot {
            text: self.documents[side.index()].text.clone(),
        });
    }
    fn finish(&mut self, side: TwoWaySide) {
        let Some(before) = self.pending_before[side.index()].take() else {
            return;
        };
        let document = &mut self.documents[side.index()];
        document.text = self.text[side.index()].clone();
        document.push_history(
            HistoryTag::Local(revision()),
            before,
            Snapshot {
                text: document.text.clone(),
            },
        );
        document.revision = revision();
    }
    fn project_documents(&mut self) {
        for (index, document) in self.documents.iter().enumerate() {
            self.text[index] = document.text.clone();
            self.saved[index] = document.saved.clone();
        }
    }
    fn side_for_tag(&self, tag: HistoryTag, redo: bool) -> TwoWaySide {
        if (if redo {
            self.documents[1].next_redo_tag()
        } else {
            self.documents[1].next_undo_tag()
        }) == Some(tag)
        {
            TwoWaySide::Right
        } else {
            TwoWaySide::Left
        }
    }
}

fn byte_at_line(lines: &[&str], line: usize) -> usize {
    lines.iter().take(line).map(|line| line.len()).sum()
}
fn has_line_ending(text: &str) -> bool {
    text.ends_with('\n') || text.ends_with('\r')
}
fn preferred_ending(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else if text.contains('\n') {
        "\n"
    } else if text.contains('\r') {
        "\r"
    } else {
        "\n"
    }
}
fn separated_insertion(source: &str, destination: &str, at_line: usize) -> String {
    if source.is_empty() {
        return String::new();
    };
    let lines = lines(destination);
    let mut result = String::new();
    if at_line > 0 && !has_line_ending(lines[at_line - 1]) {
        result.push_str(preferred_ending(destination));
    }
    result.push_str(source);
    if at_line < lines.len() && !has_line_ending(&result) {
        result.push_str(preferred_ending(destination));
    }
    result
}
fn overlaps(first: &Range<usize>, second: &Range<usize>) -> bool {
    !first.is_empty() && !second.is_empty() && first.start < second.end && second.start < first.end
}
fn validate_batch_targets(
    transfers: &[(Range<usize>, Range<usize>, TransferPlacement, String)],
) -> Result<(), TwoWayMergeError> {
    for (index, (_, first, first_placement, _)) in transfers.iter().enumerate() {
        for (_, second, second_placement, _) in &transfers[index + 1..] {
            let conflict = match (*first_placement, *second_placement) {
                (TransferPlacement::Insert, TransferPlacement::Insert) => false,
                (TransferPlacement::Replace, TransferPlacement::Replace) => overlaps(first, second),
                (TransferPlacement::Insert, TransferPlacement::Replace) => {
                    second.start <= first.start && first.start <= second.end
                }
                (TransferPlacement::Replace, TransferPlacement::Insert) => {
                    first.start <= second.start && second.start <= first.end
                }
            };
            if conflict {
                return Err(TwoWayMergeError::OverlappingDestinationRanges);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "interactive/tests.rs"]
mod tests;
