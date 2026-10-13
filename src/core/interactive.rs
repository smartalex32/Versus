//! Diff-independent state for an interactive two-way merge session.
//!
//! This module deliberately has no dependency on the comparison engine.  It owns
//! two editable working copies and exposes byte-preserving, line-addressed
//! transfers for the UI.  All line ranges are zero-based and half-open.

use std::{
    fmt,
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

/// Returns a process-wide, monotonic revision suitable for publishing changes.
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
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferPlacement {
    /// Insert before `destination_range.start`.  Its end must equal its start.
    Insert,
    /// Replace every line in the destination range.
    Replace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChunkKind {
    Structural,
    Paragraph,
    Manual,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InteractiveChunk {
    /// Stable for a session while this chunk's range is unchanged.
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
    ProtectedRange {
        side: TwoWaySide,
        range: Range<usize>,
    },
    InvalidInsertionRange(Range<usize>),
    InvalidByteRange(Range<usize>),
    ProtectedByteRange(Range<usize>),
    OverlappingDestinationRanges,
    IgnoredRangeNotFound(Range<usize>),
    UnknownChunk(u64),
    CannotCombineChunks,
}

impl fmt::Display for TwoWayMergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLineRange { side, range } => {
                write!(
                    f,
                    "invalid {side:?} line range {}..{}",
                    range.start, range.end
                )
            }
            Self::ProtectedRange { side, range } => {
                write!(
                    f,
                    "{side:?} lines {}..{} are protected",
                    range.start, range.end
                )
            }
            Self::InvalidInsertionRange(range) => write!(
                f,
                "an insertion target must have equal bounds, got {}..{}",
                range.start, range.end
            ),
            Self::InvalidByteRange(range) => {
                write!(f, "invalid byte range {}..{}", range.start, range.end)
            }
            Self::ProtectedByteRange(range) => {
                write!(f, "byte range {}..{} is protected", range.start, range.end)
            }
            Self::OverlappingDestinationRanges => {
                f.write_str("batch transfer destination ranges overlap")
            }
            Self::IgnoredRangeNotFound(range) => {
                write!(
                    f,
                    "ignored range {}..{} no longer exists",
                    range.start, range.end
                )
            }
            Self::UnknownChunk(id) => write!(f, "chunk {id} no longer exists"),
            Self::CannotCombineChunks => f.write_str("only adjacent chunks can be combined"),
        }
    }
}

impl std::error::Error for TwoWayMergeError {}

#[derive(Clone, Debug)]
struct Snapshot {
    text: [String; 2],
    ignored: [Vec<Range<usize>>; 2],
    manual: [Vec<InteractiveChunk>; 2],
    next_chunk_id: u64,
}

#[derive(Clone, Debug)]
struct DocumentHistory {
    tag: HistoryTag,
    timestamp: u64,
    before: Arc<Snapshot>,
    after: Arc<Snapshot>,
}

/// Canonical editable state for one file. It is independent from its paired
/// document so folder-wide actions can coordinate explicit `FolderBatch` tags
/// without stale paired snapshots overwriting another file's edits.
#[derive(Clone, Debug)]
pub struct InteractiveDocument {
    text: String,
    saved: String,
    ignored: Vec<Range<usize>>,
    manual: Vec<InteractiveChunk>,
    next_chunk_id: u64,
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
            ignored: Vec::new(),
            manual: Vec::new(),
            next_chunk_id: 1,
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
    pub fn ignored_ranges(&self) -> &[Range<usize>] {
        &self.ignored
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
        }
        let entry = self.undo.pop().unwrap();
        self.restore_document(entry.before.as_ref().clone());
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
        }
        let entry = self.redo.pop().unwrap();
        self.restore_document(entry.after.as_ref().clone());
        self.undo.push(entry);
        self.revision = revision();
        true
    }
    pub fn apply_byte_edits(
        &mut self,
        tag: HistoryTag,
        edits: Vec<(Range<usize>, String)>,
    ) -> Result<(), TwoWayMergeError> {
        self.apply_byte_edits_inner(tag, edits, true)
    }

    fn apply_manual_byte_edit(
        &mut self,
        range: Range<usize>,
        replacement: String,
    ) -> Result<(), TwoWayMergeError> {
        self.apply_byte_edits_inner(
            HistoryTag::Local(revision()),
            vec![(range, replacement)],
            false,
        )
    }

    fn apply_byte_edits_inner(
        &mut self,
        tag: HistoryTag,
        mut edits: Vec<(Range<usize>, String)>,
        protect_ignored: bool,
    ) -> Result<(), TwoWayMergeError> {
        if edits.is_empty() {
            return Ok(());
        }
        for (range, replacement) in &edits {
            if range.start > range.end
                || range.end > self.text.len()
                || !self.text.is_char_boundary(range.start)
                || !self.text.is_char_boundary(range.end)
            {
                return Err(TwoWayMergeError::InvalidByteRange(range.clone()));
            }
            let protected = protect_ignored
                && self.ignored.iter().any(|ignored| {
                    let segments = lines(&self.text);
                    let start = byte_at_line(&segments, ignored.start);
                    let end = byte_at_line(&segments, ignored.end);
                    if range.is_empty() {
                        if start < range.start && range.start < end {
                            true
                        } else if range.start == start {
                            // At a protected line boundary an inserted complete line
                            // stays outside it; a partial prefix would alter it.
                            !has_line_ending(replacement)
                        } else {
                            false
                        }
                    } else {
                        range.start < end && start < range.end
                    }
                });
            if protected {
                return Err(TwoWayMergeError::ProtectedByteRange(range.clone()));
            }
        }
        edits.sort_by(|left, right| right.0.start.cmp(&left.0.start));
        for pair in edits.windows(2) {
            if pair[1].0.end > pair[0].0.start {
                return Err(TwoWayMergeError::OverlappingDestinationRanges);
            }
        }
        let before = self.document_snapshot();
        for (range, replacement) in edits {
            let start_line = line_anchor_for_byte(&self.text, range.start);
            let starts_at_boundary = is_line_boundary(&self.text, range.start);
            let removed = line_break_count(&self.text[range.clone()]);
            let inserted = line_break_count(&replacement);
            self.text.replace_range(range.clone(), &replacement);
            if removed != 0 || inserted != 0 {
                let anchor = if range.is_empty() && !starts_at_boundary {
                    start_line + 1
                } else {
                    start_line
                };
                self.remap_document(anchor..anchor + removed, inserted);
            }
        }
        let after = self.document_snapshot();
        self.push_history(tag, before, after);
        self.revision = revision();
        Ok(())
    }
    fn document_snapshot(&self) -> Snapshot {
        Snapshot {
            text: [self.text.clone(), String::new()],
            ignored: [self.ignored.clone(), Vec::new()],
            manual: [self.manual.clone(), Vec::new()],
            next_chunk_id: self.next_chunk_id,
        }
    }
    fn restore_document(&mut self, snapshot: Snapshot) {
        self.text = snapshot.text[0].clone();
        self.ignored = snapshot.ignored[0].clone();
        self.manual = snapshot.manual[0].clone();
        self.next_chunk_id = snapshot.next_chunk_id;
    }
    fn remap_document(&mut self, old: Range<usize>, replacement_len: usize) {
        for range in &mut self.ignored {
            *range = remap_range(range.clone(), old.clone(), replacement_len);
        }
        self.ignored.retain(|range| !range.is_empty());
        normalize_ranges(&mut self.ignored);
        for chunk in &mut self.manual {
            chunk.range = remap_range(chunk.range.clone(), old.clone(), replacement_len);
        }
        self.manual.retain(|chunk| !chunk.range.is_empty());
    }
    fn push_history(&mut self, tag: HistoryTag, before: Snapshot, after: Snapshot) {
        self.undo.push(DocumentHistory {
            tag,
            timestamp: revision(),
            before: Arc::new(before),
            after: Arc::new(after),
        });
        self.redo.clear();
        let size = |snapshot: &Snapshot| {
            snapshot.text.iter().map(String::len).sum::<usize>()
                + snapshot.ignored.iter().flatten().count() * std::mem::size_of::<Range<usize>>()
                + snapshot.manual.iter().flatten().count() * std::mem::size_of::<InteractiveChunk>()
        };
        let mut bytes: usize = self
            .undo
            .iter()
            .map(|entry| size(&entry.before) + size(&entry.after))
            .sum();
        while self.undo.len() > 1 && (self.undo.len() > 256 || bytes > 64 * 1024 * 1024) {
            let oldest = self.undo.remove(0);
            bytes -= size(&oldest.before) + size(&oldest.after);
        }
    }
}

/// Session-local editable copies of two files.
///
/// `mark_saved` only changes a baseline; callers perform filesystem IO outside
/// this type.  Mutations are transactional and become one undo item each.
#[derive(Clone, Debug)]
pub struct TwoWayWorkspace {
    documents: [InteractiveDocument; 2],
    text: [String; 2],
    saved: [String; 2],
    ignored: [Vec<Range<usize>>; 2],
    manual: [Vec<InteractiveChunk>; 2],
    next_chunk_id: u64,
    pending_document_before: [Option<Snapshot>; 2],
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
            ignored: [Vec::new(), Vec::new()],
            manual: [Vec::new(), Vec::new()],
            next_chunk_id: 1,
            pending_document_before: [None, None],
        }
    }

    /// Constructs a facade from canonical file documents. New UI code should
    /// retain and coordinate the documents directly for folder-wide batches.
    pub fn from_documents(documents: [InteractiveDocument; 2]) -> Self {
        let mut workspace = Self::new(documents[0].text.clone(), documents[1].text.clone());
        workspace.saved = [documents[0].saved.clone(), documents[1].saved.clone()];
        workspace.ignored = [documents[0].ignored.clone(), documents[1].ignored.clone()];
        workspace.manual = [documents[0].manual.clone(), documents[1].manual.clone()];
        workspace.next_chunk_id = documents[0].next_chunk_id.max(documents[1].next_chunk_id);
        workspace.documents = documents;
        workspace
    }

    pub fn document(&self, side: TwoWaySide) -> &InteractiveDocument {
        &self.documents[side.index()]
    }

    pub fn next_undo_tag(&self) -> Option<HistoryTag> {
        match (
            self.documents[0].undo_timestamp(),
            self.documents[1].undo_timestamp(),
        ) {
            (Some(left), Some(right)) => {
                self.documents[if right > left { 1 } else { 0 }].next_undo_tag()
            }
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
            // Redo replays the oldest transaction that was undone first.
            (Some(left), Some(right)) => {
                self.documents[if right < left { 1 } else { 0 }].next_redo_tag()
            }
            (Some(_), None) => self.documents[0].next_redo_tag(),
            (None, Some(_)) => self.documents[1].next_redo_tag(),
            (None, None) => None,
        }
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

    pub fn mark_saved(&mut self, side: TwoWaySide) {
        self.saved[side.index()] = self.text[side.index()].clone();
        self.documents[side.index()].mark_saved_text(self.saved[side.index()].clone());
    }

    pub fn mark_saved_text(&mut self, side: TwoWaySide, saved: impl Into<String>) {
        self.saved[side.index()] = saved.into();
        self.documents[side.index()].mark_saved_text(self.saved[side.index()].clone());
    }

    pub fn ignored_ranges(&self, side: TwoWaySide) -> &[Range<usize>] {
        &self.ignored[side.index()]
    }

    /// Source line numbers and bytes that remain eligible for optional diff work.
    /// This is deliberately a view: it does no comparison and never loads files.
    pub fn comparison_lines(&self, side: TwoWaySide) -> Vec<(usize, &str)> {
        lines(self.text(side))
            .into_iter()
            .enumerate()
            .filter(|(line, _)| {
                !self.ignored[side.index()]
                    .iter()
                    .any(|range| range.contains(line))
            })
            .collect()
    }

    /// Returns structural code blocks where recognizable, otherwise paragraphs.
    /// Explicit manual chunks are included and replace overlapping automatic ones.
    pub fn chunks(&self, side: TwoWaySide) -> Vec<InteractiveChunk> {
        let index = side.index();
        let mut automatic = detect_chunks(self.text(side));
        let manual = &self.manual[index];
        automatic = automatic
            .into_iter()
            .flat_map(|chunk| split_automatic_chunk_around_manual(chunk, manual))
            .collect();
        automatic.extend(manual.iter().cloned());
        automatic.sort_by_key(|chunk| (chunk.range.start, chunk.range.end));
        automatic
    }

    /// Replaces all text on one side. Ranges outside the edited span retain their
    /// meaning; ranges crossing it expand/shrink with the replacement.
    pub fn set_text(&mut self, side: TwoWaySide, replacement: impl Into<String>) {
        let replacement = replacement.into();
        if self.text(side) == replacement {
            return;
        }
        let old_lines = lines(self.text(side));
        let new_lines = lines(&replacement);
        let (old_range, new_len) = changed_line_range(&old_lines, &new_lines);
        self.snapshot(side);
        self.text[side.index()] = replacement;
        self.remap_side(side, old_range, new_len);
        self.finish_document_change(side);
    }

    /// Applies an explicitly located editor change. Unlike automatic transfers,
    /// direct typing is allowed inside protected ranges; the exact byte anchor
    /// keeps line metadata correct even when surrounding text is duplicated.
    pub fn edit_bytes(
        &mut self,
        side: TwoWaySide,
        range: Range<usize>,
        replacement: String,
    ) -> Result<(), TwoWayMergeError> {
        self.documents[side.index()].apply_manual_byte_edit(range, replacement)?;
        self.project_documents();
        Ok(())
    }

    /// Replaces a selected line range. `replacement` retains its original line
    /// endings, so transferring CRLF text into an LF file does not alter it.
    pub fn replace_lines(
        &mut self,
        side: TwoWaySide,
        range: Range<usize>,
        replacement: impl Into<String>,
    ) -> Result<(), TwoWayMergeError> {
        self.validate_range(side, &range)?;
        self.ensure_unprotected(side, &range)?;
        self.snapshot(side);
        self.replace_lines_after_snapshot(side, range, replacement.into());
        self.finish_document_change(side);
        Ok(())
    }

    /// Copies the exact selected source lines to the other working copy.
    /// `Insert` requires a zero-length target and `Replace` rejects protected
    /// destination lines. Source protected ranges are deliberately readable: they
    /// protect automatic targets, not the user's ability to copy content out.
    pub fn transfer(
        &mut self,
        source: TwoWaySide,
        source_range: Range<usize>,
        destination_range: Range<usize>,
        placement: TransferPlacement,
    ) -> Result<(), TwoWayMergeError> {
        let destination = source.other();
        let mut replacement = self.prepare_transfer(
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
        self.replace_lines_after_snapshot(destination, destination_range, replacement);
        self.finish_document_change(destination);
        Ok(())
    }

    /// Applies source copies as one history operation. Every destination range
    /// is interpreted against the destination before this call, independent of
    /// other transfers in the batch.
    pub fn transfer_batch(
        &mut self,
        source: TwoWaySide,
        transfers: Vec<(Range<usize>, Range<usize>, TransferPlacement)>,
    ) -> Result<(), TwoWayMergeError> {
        if transfers.is_empty() {
            return Ok(());
        }
        let destination = source.other();
        let destination_text = self.text(destination).to_owned();
        let mut prepared: Vec<(Range<usize>, Range<usize>, TransferPlacement, String)> = transfers
            .into_iter()
            .map(|(source_range, destination_range, placement)| {
                let replacement = self.prepare_transfer(
                    source,
                    &source_range,
                    destination,
                    &destination_range,
                    placement,
                )?;
                Ok((source_range, destination_range, placement, replacement))
            })
            .collect::<Result<_, TwoWayMergeError>>()?;
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
                let mut group_end = index + 1;
                while group_end < prepared.len()
                    && prepared[group_end].2 == TransferPlacement::Insert
                    && prepared[group_end].1 == target
                {
                    group_end += 1;
                }
                // Entries were sorted by descending source start. Reversing a
                // same-anchor group keeps the resulting inserted lines in source
                // order even though all inserts share the original anchor.
                let joined = prepared[index..group_end]
                    .iter()
                    .rev()
                    .map(|entry| entry.3.as_str())
                    .collect::<String>();
                let replacement = separated_insertion(&joined, &destination_text, target.start);
                self.replace_lines_after_snapshot(destination, target, replacement);
                index = group_end;
            } else {
                self.replace_lines_after_snapshot(
                    destination,
                    target.clone(),
                    prepared[index].3.clone(),
                );
                index += 1;
            }
        }
        self.finish_document_change(destination);
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

    pub fn add_ignored_range(
        &mut self,
        side: TwoWaySide,
        range: Range<usize>,
    ) -> Result<(), TwoWayMergeError> {
        self.validate_range(side, &range)?;
        if range.is_empty()
            || self.ignored[side.index()]
                .iter()
                .any(|current| current == &range)
        {
            return Ok(());
        }
        self.snapshot(side);
        let ranges = &mut self.ignored[side.index()];
        ranges.push(range);
        normalize_ranges(ranges);
        self.finish_document_change(side);
        Ok(())
    }

    pub fn remove_ignored_range(&mut self, side: TwoWaySide, range: Range<usize>) -> bool {
        let Some(index) = self.ignored[side.index()]
            .iter()
            .position(|current| current == &range)
        else {
            return false;
        };
        self.snapshot(side);
        self.ignored[side.index()].remove(index);
        self.finish_document_change(side);
        true
    }

    /// Changes one protected range as a single undoable operation. The original
    /// range must still exist exactly; callers can refetch `ignored_ranges` after
    /// edits because line references may have moved.
    pub fn update_ignored_range(
        &mut self,
        side: TwoWaySide,
        existing: Range<usize>,
        replacement: Range<usize>,
    ) -> Result<(), TwoWayMergeError> {
        self.validate_range(side, &existing)?;
        self.validate_range(side, &replacement)?;
        if replacement.is_empty() {
            return Err(TwoWayMergeError::InvalidLineRange {
                side,
                range: replacement,
            });
        }
        let Some(index) = self.ignored[side.index()]
            .iter()
            .position(|range| range == &existing)
        else {
            return Err(TwoWayMergeError::IgnoredRangeNotFound(existing));
        };
        if existing == replacement {
            return Ok(());
        }
        self.snapshot(side);
        self.ignored[side.index()][index] = replacement;
        normalize_ranges(&mut self.ignored[side.index()]);
        self.finish_document_change(side);
        Ok(())
    }

    pub fn clear_ignored_ranges(&mut self, side: TwoWaySide) {
        if self.ignored[side.index()].is_empty() {
            return;
        }
        self.snapshot(side);
        self.ignored[side.index()].clear();
        self.finish_document_change(side);
    }

    /// Adds an arbitrary contiguous manual chunk, replacing automatic chunks it
    /// overlaps. The returned id can be used by `transfer_chunk` or splitting.
    pub fn create_chunk(
        &mut self,
        side: TwoWaySide,
        range: Range<usize>,
    ) -> Result<u64, TwoWayMergeError> {
        self.validate_range(side, &range)?;
        if range.is_empty() {
            return Err(TwoWayMergeError::InvalidLineRange { side, range });
        }
        self.snapshot(side);
        let id = self.allocate_chunk_id();
        let chunks = &mut self.manual[side.index()];
        chunks.retain(|chunk| !overlaps(&chunk.range, &range));
        chunks.push(InteractiveChunk {
            id,
            range,
            kind: ChunkKind::Manual,
        });
        chunks.sort_by_key(|chunk| chunk.range.start);
        self.finish_document_change(side);
        Ok(id)
    }

    pub fn adjust_chunk(
        &mut self,
        side: TwoWaySide,
        chunk_id: u64,
        range: Range<usize>,
    ) -> Result<(), TwoWayMergeError> {
        self.validate_range(side, &range)?;
        if range.is_empty() {
            return Err(TwoWayMergeError::InvalidLineRange { side, range });
        }
        let existing = self
            .chunk_by_id(side, chunk_id)
            .ok_or(TwoWayMergeError::UnknownChunk(chunk_id))?;
        self.snapshot(side);
        let chunks = &mut self.manual[side.index()];
        chunks.retain(|chunk| chunk.id != existing.id && !overlaps(&chunk.range, &range));
        chunks.push(InteractiveChunk {
            id: existing.id,
            range,
            kind: ChunkKind::Manual,
        });
        chunks.sort_by_key(|chunk| chunk.range.start);
        self.finish_document_change(side);
        Ok(())
    }

    pub fn split_chunk(
        &mut self,
        side: TwoWaySide,
        chunk_id: u64,
        at_line: usize,
    ) -> Result<(u64, u64), TwoWayMergeError> {
        let chunk = self
            .chunk_by_id(side, chunk_id)
            .ok_or(TwoWayMergeError::UnknownChunk(chunk_id))?;
        if at_line <= chunk.range.start || at_line >= chunk.range.end {
            return Err(TwoWayMergeError::InvalidLineRange {
                side,
                range: chunk.range,
            });
        }
        self.snapshot(side);
        let left_id = self.allocate_chunk_id();
        let right_id = self.allocate_chunk_id();
        let chunks = &mut self.manual[side.index()];
        chunks.retain(|candidate| {
            candidate.id != chunk_id && !overlaps(&candidate.range, &chunk.range)
        });
        chunks.push(InteractiveChunk {
            id: left_id,
            range: chunk.range.start..at_line,
            kind: ChunkKind::Manual,
        });
        chunks.push(InteractiveChunk {
            id: right_id,
            range: at_line..chunk.range.end,
            kind: ChunkKind::Manual,
        });
        chunks.sort_by_key(|candidate| candidate.range.start);
        self.finish_document_change(side);
        Ok((left_id, right_id))
    }

    pub fn combine_chunks(
        &mut self,
        side: TwoWaySide,
        first_id: u64,
        second_id: u64,
    ) -> Result<u64, TwoWayMergeError> {
        let first = self
            .chunk_by_id(side, first_id)
            .ok_or(TwoWayMergeError::UnknownChunk(first_id))?;
        let second = self
            .chunk_by_id(side, second_id)
            .ok_or(TwoWayMergeError::UnknownChunk(second_id))?;
        let (first, second) = if first.range.start <= second.range.start {
            (first, second)
        } else {
            (second, first)
        };
        if first.range.end != second.range.start {
            return Err(TwoWayMergeError::CannotCombineChunks);
        }
        self.snapshot(side);
        let id = self.allocate_chunk_id();
        let chunks = &mut self.manual[side.index()];
        chunks.retain(|candidate| candidate.id != first.id && candidate.id != second.id);
        chunks.push(InteractiveChunk {
            id,
            range: first.range.start..second.range.end,
            kind: ChunkKind::Manual,
        });
        chunks.sort_by_key(|candidate| candidate.range.start);
        self.finish_document_change(side);
        Ok(id)
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
        let side = self.side_for_undo_tag(tag, false);
        let changed = self.documents[side.index()].undo_expected(tag);
        if changed {
            self.project_documents();
        }
        changed
    }

    pub fn redo(&mut self) -> bool {
        let Some(tag) = self.next_redo_tag() else {
            return false;
        };
        let side = self.side_for_undo_tag(tag, true);
        let changed = self.documents[side.index()].redo_expected(tag);
        if changed {
            self.project_documents();
        }
        changed
    }

    fn validate_range(
        &self,
        side: TwoWaySide,
        range: &Range<usize>,
    ) -> Result<(), TwoWayMergeError> {
        if range.start > range.end || range.end > self.line_count(side) {
            return Err(TwoWayMergeError::InvalidLineRange {
                side,
                range: range.clone(),
            });
        }
        Ok(())
    }

    fn prepare_transfer(
        &self,
        source: TwoWaySide,
        source_range: &Range<usize>,
        destination: TwoWaySide,
        destination_range: &Range<usize>,
        placement: TransferPlacement,
    ) -> Result<String, TwoWayMergeError> {
        self.validate_range(source, source_range)?;
        self.validate_range(destination, destination_range)?;
        if placement == TransferPlacement::Insert
            && destination_range.start != destination_range.end
        {
            return Err(TwoWayMergeError::InvalidInsertionRange(
                destination_range.clone(),
            ));
        }
        self.ensure_unprotected(destination, destination_range)?;
        let replacement = lines(self.text(source))[source_range.clone()].concat();
        Ok(replacement)
    }

    fn ensure_unprotected(
        &self,
        side: TwoWaySide,
        range: &Range<usize>,
    ) -> Result<(), TwoWayMergeError> {
        let protected = if range.is_empty() {
            self.ignored[side.index()]
                .iter()
                // Inserting exactly before or after a protected range does not
                // alter it.  Inserting in its interior would split it.
                .any(|ignored| ignored.start < range.start && range.start < ignored.end)
        } else {
            self.ignored[side.index()]
                .iter()
                .any(|ignored| overlaps(ignored, range))
        };
        if protected {
            return Err(TwoWayMergeError::ProtectedRange {
                side,
                range: range.clone(),
            });
        }
        Ok(())
    }

    fn replace_lines_after_snapshot(
        &mut self,
        side: TwoWaySide,
        range: Range<usize>,
        replacement: String,
    ) {
        let segments = lines(self.text(side));
        let start = byte_at_line(&segments, range.start);
        let end = byte_at_line(&segments, range.end);
        self.text[side.index()].replace_range(start..end, &replacement);
        self.remap_side(side, range, lines(&replacement).len());
    }

    fn remap_side(&mut self, side: TwoWaySide, old: Range<usize>, replacement_len: usize) {
        let index = side.index();
        for ignored in &mut self.ignored[index] {
            *ignored = remap_range(ignored.clone(), old.clone(), replacement_len);
        }
        self.ignored[index].retain(|range| !range.is_empty());
        normalize_ranges(&mut self.ignored[index]);
        for chunk in &mut self.manual[index] {
            chunk.range = remap_range(chunk.range.clone(), old.clone(), replacement_len);
        }
        self.manual[index].retain(|chunk| !chunk.range.is_empty());
        self.manual[index].sort_by_key(|chunk| chunk.range.start);
    }

    fn chunk_by_id(&self, side: TwoWaySide, id: u64) -> Option<InteractiveChunk> {
        self.chunks(side).into_iter().find(|chunk| chunk.id == id)
    }

    fn allocate_chunk_id(&mut self) -> u64 {
        let id = self.next_chunk_id;
        self.next_chunk_id = self.next_chunk_id.saturating_add(1);
        id
    }

    fn snapshot(&mut self, side: TwoWaySide) {
        self.pending_document_before[side.index()] =
            Some(self.documents[side.index()].document_snapshot());
    }

    fn finish_document_change(&mut self, side: TwoWaySide) {
        let Some(before) = self.pending_document_before[side.index()].take() else {
            return;
        };
        let document = &mut self.documents[side.index()];
        document.text = self.text[side.index()].clone();
        document.ignored = self.ignored[side.index()].clone();
        document.manual = self.manual[side.index()].clone();
        document.next_chunk_id = self.next_chunk_id;
        let after = document.document_snapshot();
        document.push_history(HistoryTag::Local(revision()), before, after);
        document.revision = revision();
    }

    fn project_documents(&mut self) {
        for (index, document) in self.documents.iter().enumerate() {
            self.text[index] = document.text.clone();
            self.saved[index] = document.saved.clone();
            self.ignored[index] = document.ignored.clone();
            self.manual[index] = document.manual.clone();
        }
        self.next_chunk_id = self.documents[0]
            .next_chunk_id
            .max(self.documents[1].next_chunk_id);
    }

    fn side_for_undo_tag(&self, tag: HistoryTag, redo: bool) -> TwoWaySide {
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

fn lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let bytes = text.as_bytes();
    let mut result = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        let length = match bytes[index] {
            b'\n' => 1,
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => 2,
            b'\r' => 1,
            _ => {
                index += 1;
                continue;
            }
        };
        result.push(&text[start..index + length]);
        index += length;
        start = index;
    }
    if start < text.len() {
        result.push(&text[start..]);
    }
    result
}

fn byte_at_line(lines: &[&str], line: usize) -> usize {
    lines.iter().take(line).map(|line| line.len()).sum()
}

fn line_anchor_for_byte(text: &str, byte: usize) -> usize {
    let segments = lines(text);
    let mut offset = 0;
    for (index, segment) in segments.iter().enumerate() {
        if byte < offset + segment.len() {
            return index;
        }
        if byte == offset + segment.len() {
            return index + 1;
        }
        offset += segment.len();
    }
    segments.len()
}

fn is_line_boundary(text: &str, byte: usize) -> bool {
    byte == 0
        || lines(text)
            .iter()
            .scan(0usize, |offset, segment| {
                *offset += segment.len();
                Some(*offset)
            })
            .any(|boundary| boundary == byte)
}

fn line_break_count(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut count = 0;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' => {
                count += 1;
                index += 1;
            }
            b'\r' => {
                count += 1;
                index += usize::from(bytes.get(index + 1) == Some(&b'\n')) + 1;
            }
            _ => index += 1,
        }
    }
    count
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

/// Keep a source's unterminated final line distinct when it is inserted into an
/// existing line-oriented destination. The added separator follows the target's
/// convention; all source bytes themselves remain untouched.
fn separated_insertion(source: &str, destination: &str, at_line: usize) -> String {
    if source.is_empty() {
        return String::new();
    }
    let destination_lines = lines(destination);
    let mut result = String::new();
    if at_line > 0
        && !destination_lines[at_line - 1].is_empty()
        && !has_line_ending(destination_lines[at_line - 1])
    {
        result.push_str(preferred_ending(destination));
    }
    result.push_str(source);
    if at_line < destination_lines.len() && !has_line_ending(&result) {
        result.push_str(preferred_ending(destination));
    }
    result
}

fn changed_line_range(old: &[&str], new: &[&str]) -> (Range<usize>, usize) {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let mut old_end = old.len();
    let mut new_end = new.len();
    while old_end > prefix && new_end > prefix && old[old_end - 1] == new[new_end - 1] {
        old_end -= 1;
        new_end -= 1;
    }
    (prefix..old_end, new_end - prefix)
}

fn remap_range(range: Range<usize>, changed: Range<usize>, replacement_len: usize) -> Range<usize> {
    // An insertion immediately before a range shifts it forward. An insertion
    // immediately after it leaves it alone, keeping ranges attached to their
    // original lines instead of protecting newly inserted text.
    if changed.is_empty() {
        let at = changed.start;
        let start = if at <= range.start {
            range.start + replacement_len
        } else {
            range.start
        };
        let end = if at < range.end {
            range.end + replacement_len
        } else {
            range.end
        };
        return start..end;
    }
    let removed = changed.end - changed.start;
    let delta = replacement_len as isize - removed as isize;
    let shifted = |line: usize| (line as isize + delta).max(0) as usize;
    let start = if range.start >= changed.end {
        shifted(range.start)
    } else if range.start <= changed.start {
        range.start
    } else {
        changed.start
    };
    let end = if range.end <= changed.start {
        range.end
    } else if range.end >= changed.end {
        shifted(range.end)
    } else {
        changed.start + replacement_len
    };
    start..end
}

fn overlaps(first: &Range<usize>, second: &Range<usize>) -> bool {
    if first.is_empty() || second.is_empty() {
        return false;
    }
    first.start < second.end && second.start < first.end
}

fn split_automatic_chunk_around_manual(
    chunk: InteractiveChunk,
    manual: &[InteractiveChunk],
) -> Vec<InteractiveChunk> {
    let mut covered: Vec<&Range<usize>> = manual
        .iter()
        .map(|manual| &manual.range)
        .filter(|range| overlaps(&chunk.range, range))
        .collect();
    if covered.is_empty() {
        return vec![chunk];
    }
    covered.sort_by_key(|range| range.start);
    let mut result = Vec::new();
    let mut cursor = chunk.range.start;
    for range in covered {
        let start = range.start.max(chunk.range.start);
        let end = range.end.min(chunk.range.end);
        if cursor < start {
            let range = cursor..start;
            result.push(InteractiveChunk {
                id: automatic_id(&range),
                range,
                kind: chunk.kind,
            });
        }
        cursor = cursor.max(end);
    }
    if cursor < chunk.range.end {
        let range = cursor..chunk.range.end;
        result.push(InteractiveChunk {
            id: automatic_id(&range),
            range,
            kind: chunk.kind,
        });
    }
    result
}

fn validate_batch_targets(
    transfers: &[(Range<usize>, Range<usize>, TransferPlacement, String)],
) -> Result<(), TwoWayMergeError> {
    for (index, (_, first, first_placement, _)) in transfers.iter().enumerate() {
        for (_, second, second_placement, _) in &transfers[index + 1..] {
            match (*first_placement, *second_placement) {
                (TransferPlacement::Insert, TransferPlacement::Insert) => {
                    // Same anchors are merged as a source-ordered insertion group.
                }
                (TransferPlacement::Replace, TransferPlacement::Replace) => {
                    if overlaps(first, second) {
                        return Err(TwoWayMergeError::OverlappingDestinationRanges);
                    }
                }
                (TransferPlacement::Insert, TransferPlacement::Replace) => {
                    if second.start <= first.start && first.start <= second.end {
                        return Err(TwoWayMergeError::OverlappingDestinationRanges);
                    }
                }
                (TransferPlacement::Replace, TransferPlacement::Insert) => {
                    if first.start <= second.start && second.start <= first.end {
                        return Err(TwoWayMergeError::OverlappingDestinationRanges);
                    }
                }
            }
        }
    }
    Ok(())
}

fn normalize_ranges(ranges: &mut Vec<Range<usize>>) {
    ranges.sort_by_key(|range| range.start);
    let mut normalized: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges.drain(..) {
        if let Some(last) = normalized.last_mut()
            && range.start <= last.end
        {
            last.end = last.end.max(range.end);
        } else {
            normalized.push(range);
        }
    }
    *ranges = normalized;
}

fn detect_chunks(text: &str) -> Vec<InteractiveChunk> {
    let lines = lines(text);
    let mut result = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if lines[index].trim().is_empty() {
            index += 1;
            continue;
        }
        let start = index;
        let structural = structural_kind(lines[index]);
        index += 1;
        if let Some(StructuralKind::Python { indent }) = structural {
            // Python suites may intentionally contain blank lines. Only a
            // nonblank dedent closes the enclosing function or class.
            while index < lines.len() {
                if !lines[index].trim().is_empty() && leading_indent(lines[index]) <= indent {
                    break;
                }
                index += 1;
            }
        } else if structural.is_some() {
            let mut lexical = LexState::default();
            let mut braces = brace_delta_lexical(lines[start], &mut lexical);
            let mut opened = braces > 0;
            while index < lines.len() {
                if !opened
                    && (lines[index].trim().is_empty() || structural_kind(lines[index]).is_some())
                {
                    break;
                }
                braces += brace_delta_lexical(lines[index], &mut lexical);
                opened |= braces > 0;
                index += 1;
                // A matching closing brace ends a C/Rust/Java block, including
                // when another declaration follows with no blank separator.
                if opened && braces <= 0 {
                    break;
                }
            }
        } else {
            while index < lines.len()
                && !lines[index].trim().is_empty()
                && structural_kind(lines[index]).is_none()
            {
                index += 1;
            }
        }
        let range = start..index;
        result.push(InteractiveChunk {
            id: automatic_id(&range),
            range,
            kind: if structural.is_some() {
                ChunkKind::Structural
            } else {
                ChunkKind::Paragraph
            },
        });
    }
    result
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StructuralKind {
    Python { indent: usize },
    Braced,
}

fn structural_kind(line: &str) -> Option<StructuralKind> {
    let indent = leading_indent(line);
    let line = line.trim_start();
    if ["def ", "async def "]
        .iter()
        .any(|prefix| line.starts_with(prefix))
        || (line.starts_with("class ") && line.trim_end().ends_with(':'))
    {
        return Some(StructuralKind::Python { indent });
    }
    if [
        "fn ",
        "pub fn ",
        "async fn ",
        "class ",
        "struct ",
        "enum ",
        "impl ",
        "function ",
        "interface ",
        "type ",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
        || looks_like_braced_declaration(line)
    {
        return Some(StructuralKind::Braced);
    }
    None
}

fn leading_indent(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

fn looks_like_braced_declaration(line: &str) -> bool {
    let line = line.trim_start();
    if ["if", "for", "while", "switch", "catch", "match"]
        .iter()
        .any(|keyword| line.starts_with(keyword) && line[keyword.len()..].starts_with([' ', '(']))
    {
        return false;
    }
    let Some(open) = line.find('(') else {
        return false;
    };
    let Some(close) = line[open..].find(')') else {
        return false;
    };
    let before = line[..open].split_whitespace().count();
    before >= 2 && (line[open + close + 1..].contains('{') || line.trim_end().ends_with(')'))
}

#[derive(Default)]
struct LexState {
    block_comment: bool,
    quote: Option<u8>,
    escaped: bool,
}

/// Count braces while skipping ordinary quoted literals and C-family comments.
/// This is intentionally lexical: chunk detection never needs a parser or diff.
fn brace_delta_lexical(line: &str, state: &mut LexState) -> isize {
    let bytes = line.as_bytes();
    let mut depth = 0;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if state.block_comment {
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                state.block_comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if let Some(quote) = state.quote {
            if state.escaped {
                state.escaped = false;
            } else if byte == b'\\' {
                state.escaped = true;
            } else if byte == quote {
                state.quote = None;
            }
            index += 1;
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            break;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            state.block_comment = true;
            index += 2;
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            state.quote = Some(byte);
        } else if byte == b'{' {
            depth += 1;
        } else if byte == b'}' {
            depth -= 1;
        }
        index += 1;
    }
    if state.quote.is_some() && !state.escaped {
        state.quote = None;
    }
    state.escaped = false;
    depth
}

fn automatic_id(range: &Range<usize>) -> u64 {
    // Automatic ids are kept separate from manually allocated positive ids.
    (1u64 << 63) | ((range.start as u64) << 31) | range.end as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfers_unrelated_text_without_a_diff() {
        let mut workspace =
            TwoWayWorkspace::new("fn source() {\r\n  left();\r\n}\r\n", "unrelated\n");
        let chunk = workspace.chunks(TwoWaySide::Left)[0].clone();
        workspace
            .transfer_chunk(TwoWaySide::Left, chunk.id, 1..1, TransferPlacement::Insert)
            .unwrap();
        assert_eq!(
            workspace.text(TwoWaySide::Right),
            "unrelated\nfn source() {\r\n  left();\r\n}\r\n"
        );
        assert!(workspace.is_dirty(TwoWaySide::Right));
        assert!(workspace.undo());
        assert_eq!(workspace.text(TwoWaySide::Right), "unrelated\n");
        assert!(workspace.redo());
    }

    #[test]
    fn edits_remap_protection_and_manual_chunks() {
        let mut workspace = TwoWayWorkspace::new("one\ntwo\nthree\nfour\n", "target\n");
        workspace.add_ignored_range(TwoWaySide::Left, 2..3).unwrap();
        let id = workspace.create_chunk(TwoWaySide::Left, 1..3).unwrap();
        workspace
            .replace_lines(TwoWaySide::Left, 0..0, "zero\n")
            .unwrap();
        assert_eq!(workspace.ignored_ranges(TwoWaySide::Left), &[3..4]);
        assert_eq!(
            workspace
                .chunks(TwoWaySide::Left)
                .into_iter()
                .find(|chunk| chunk.id == id)
                .unwrap()
                .range,
            2..4
        );
        let error = workspace
            .replace_lines(TwoWaySide::Left, 3..4, "changed\n")
            .unwrap_err();
        assert!(matches!(error, TwoWayMergeError::ProtectedRange { .. }));
    }

    #[test]
    fn transfers_and_ignore_changes_undo_as_atomic_operations() {
        let mut workspace = TwoWayWorkspace::new("a\nb\nc\n", "x\ny\n");
        workspace
            .transfer(TwoWaySide::Left, 1..3, 1..2, TransferPlacement::Replace)
            .unwrap();
        assert_eq!(workspace.text(TwoWaySide::Right), "x\nb\nc\n");
        workspace
            .add_ignored_range(TwoWaySide::Right, 1..2)
            .unwrap();
        assert!(matches!(
            workspace.transfer(TwoWaySide::Left, 0..1, 1..2, TransferPlacement::Replace),
            Err(TwoWayMergeError::ProtectedRange { .. })
        ));
        assert!(workspace.undo());
        assert!(workspace.ignored_ranges(TwoWaySide::Right).is_empty());
        assert!(workspace.undo());
        assert_eq!(workspace.text(TwoWaySide::Right), "x\ny\n");
    }

    #[test]
    fn chunks_can_split_and_combine() {
        let mut workspace = TwoWayWorkspace::new("first\nsecond\nthird\n", "");
        let id = workspace.create_chunk(TwoWaySide::Left, 0..3).unwrap();
        let (first, second) = workspace.split_chunk(TwoWaySide::Left, id, 1).unwrap();
        let combined = workspace
            .combine_chunks(TwoWaySide::Left, first, second)
            .unwrap();
        assert_eq!(
            workspace
                .chunks(TwoWaySide::Left)
                .into_iter()
                .find(|chunk| chunk.id == combined)
                .unwrap()
                .range,
            0..3
        );
    }

    #[test]
    fn unterminated_lines_stay_separate_and_protection_cannot_be_split() {
        let mut workspace = TwoWayWorkspace::new("incoming", "before\nafter\n");
        workspace
            .transfer(TwoWaySide::Left, 0..1, 1..1, TransferPlacement::Insert)
            .unwrap();
        assert_eq!(
            workspace.text(TwoWaySide::Right),
            "before\nincoming\nafter\n"
        );
        workspace
            .add_ignored_range(TwoWaySide::Right, 1..3)
            .unwrap();
        assert!(matches!(
            workspace.transfer(TwoWaySide::Left, 0..1, 2..2, TransferPlacement::Insert),
            Err(TwoWayMergeError::ProtectedRange { .. })
        ));
        // Edits remain explicitly user-directed; protection only blocks automatic
        // replacement targets and transfers.
        workspace.set_text(TwoWaySide::Right, "manual\nedit\n");
        assert_eq!(workspace.ignored_ranges(TwoWaySide::Right), &[0..2]);
    }

    #[test]
    fn structural_chunks_split_adjacent_rust_functions() {
        let workspace = TwoWayWorkspace::new(
            "fn first() {\n    call(\"}\"); // { ignored\n}\nfn second() {\n}\n",
            "",
        );
        let chunks = workspace.chunks(TwoWaySide::Left);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].range, 0..3);
        assert_eq!(chunks[1].range, 3..5);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.kind == ChunkKind::Structural)
        );
    }

    #[test]
    fn python_chunk_keeps_blank_lines_until_a_dedent() {
        let workspace = TwoWayWorkspace::new(
            "def first():\n    value = 1\n\n    return value\n\ndef second():\n    return 2\n",
            "",
        );
        let chunks = workspace.chunks(TwoWaySide::Left);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].range, 0..5);
        assert_eq!(chunks[1].range, 5..7);
    }

    #[test]
    fn c_style_methods_ignore_comment_and_literal_braces() {
        let workspace = TwoWayWorkspace::new(
            "void first() {\n  const char *value = \"{ not a block }\";\n  /* { comment } */\n}\nint second()\n{\n  return 2; // } ignored\n}\n",
            "",
        );
        let chunks = workspace.chunks(TwoWaySide::Left);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].range, 0..4);
        assert_eq!(chunks[1].range, 4..8);
    }

    #[test]
    fn insertion_at_protected_range_boundary_keeps_original_lines_protected() {
        let mut workspace = TwoWayWorkspace::new("new\n", "one\ntwo\nthree\n");
        workspace
            .add_ignored_range(TwoWaySide::Right, 1..2)
            .unwrap();
        workspace
            .transfer(TwoWaySide::Left, 0..1, 1..1, TransferPlacement::Insert)
            .unwrap();
        assert_eq!(workspace.text(TwoWaySide::Right), "one\nnew\ntwo\nthree\n");
        assert_eq!(workspace.ignored_ranges(TwoWaySide::Right), &[2..3]);
        workspace
            .transfer(TwoWaySide::Left, 0..1, 3..3, TransferPlacement::Insert)
            .unwrap();
        assert_eq!(workspace.ignored_ranges(TwoWaySide::Right), &[2..3]);
    }

    #[test]
    fn updating_an_ignored_range_is_one_undoable_change() {
        let mut workspace = TwoWayWorkspace::new("one\ntwo\nthree\n", "");
        workspace.add_ignored_range(TwoWaySide::Left, 0..1).unwrap();
        workspace
            .update_ignored_range(TwoWaySide::Left, 0..1, 1..3)
            .unwrap();
        assert_eq!(workspace.ignored_ranges(TwoWaySide::Left), &[1..3]);
        assert!(workspace.undo());
        assert_eq!(workspace.ignored_ranges(TwoWaySide::Left), &[0..1]);
        assert!(workspace.redo());
        assert_eq!(workspace.ignored_ranges(TwoWaySide::Left), &[1..3]);
    }

    #[test]
    fn manual_chunk_only_replaces_its_part_of_an_automatic_chunk() {
        let mut workspace = TwoWayWorkspace::new("one\ntwo\nthree\nfour\n", "");
        let manual = workspace.create_chunk(TwoWaySide::Left, 1..3).unwrap();
        let chunks = workspace.chunks(TwoWaySide::Left);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].range, 0..1);
        assert_eq!(chunks[1].id, manual);
        assert_eq!(chunks[1].range, 1..3);
        assert_eq!(chunks[2].range, 3..4);
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.range.clone())
                .collect::<Vec<_>>(),
            vec![0..1, 1..3, 3..4]
        );
    }

    #[test]
    fn batch_inserts_use_original_anchors_and_one_undo_step() {
        let mut workspace = TwoWayWorkspace::new("α\r\nβ\r\nγ\r\n", "zero\nend\n");
        workspace
            .transfer_batch(
                TwoWaySide::Left,
                vec![
                    (0..1, 1..1, TransferPlacement::Insert),
                    (1..2, 1..1, TransferPlacement::Insert),
                    (2..3, 2..2, TransferPlacement::Insert),
                ],
            )
            .unwrap();
        assert_eq!(
            workspace.text(TwoWaySide::Right),
            "zero\nα\r\nβ\r\nend\nγ\r\n"
        );
        assert!(workspace.undo());
        assert_eq!(workspace.text(TwoWaySide::Right), "zero\nend\n");
        assert!(!workspace.can_undo());
    }

    #[test]
    fn batch_rejects_protected_target_without_any_partial_transfer() {
        let mut workspace = TwoWayWorkspace::new("a\nb\n", "one\ntwo\nthree\n");
        workspace
            .add_ignored_range(TwoWaySide::Right, 1..2)
            .unwrap();
        let before = workspace.text(TwoWaySide::Right).to_owned();
        let history_before = workspace.can_undo();
        let error = workspace.transfer_batch(
            TwoWaySide::Left,
            vec![
                (0..1, 0..0, TransferPlacement::Insert),
                (1..2, 1..2, TransferPlacement::Replace),
            ],
        );
        assert!(matches!(
            error,
            Err(TwoWayMergeError::ProtectedRange { .. })
        ));
        assert_eq!(workspace.text(TwoWaySide::Right), before);
        assert_eq!(workspace.can_undo(), history_before);
    }

    #[test]
    fn document_byte_edits_preserve_line_metadata_and_expected_history_tags() {
        let mut document = InteractiveDocument::new("zero\nkeep\nprotected\nlast\n");
        document.ignored.push(2..3);
        document.manual.push(InteractiveChunk {
            id: 1,
            range: 1..2,
            kind: ChunkKind::Manual,
        });
        let tag = HistoryTag::FolderBatch(revision());
        document
            .apply_byte_edits(
                tag,
                vec![(0..4, "ZERO".into()), (5..9, "KEEP\nPLUS".into())],
            )
            .unwrap();
        assert_eq!(document.ignored_ranges(), &[3..4]);
        assert!(!document.undo_expected(HistoryTag::Local(revision())));
        assert!(document.undo_expected(tag));
        assert_eq!(document.text(), "zero\nkeep\nprotected\nlast\n");
        assert_eq!(document.ignored_ranges(), &[2..3]);
    }

    #[test]
    fn document_rejects_byte_edits_inside_protected_content() {
        let mut document = InteractiveDocument::new("safe\nprotected\n");
        document.ignored.push(1..2);
        let protected_start = "safe\n".len();
        assert!(matches!(
            document.apply_byte_edits(
                HistoryTag::Local(revision()),
                vec![(protected_start + 1..protected_start + 2, "X".into())]
            ),
            Err(TwoWayMergeError::ProtectedByteRange(_))
        ));
        assert!(
            document
                .apply_byte_edits(HistoryTag::Local(revision()), vec![(0..1, "S".into())])
                .is_ok()
        );
    }

    #[test]
    fn facade_keeps_documents_live_and_undo_does_not_restore_partner() {
        let mut workspace = TwoWayWorkspace::from_documents([
            InteractiveDocument::new("a\n"),
            InteractiveDocument::new("b\n"),
        ]);
        workspace.set_text(TwoWaySide::Left, "A\n");
        workspace.set_text(TwoWaySide::Right, "B\n");
        workspace.set_text(TwoWaySide::Left, "AA\n");
        assert_eq!(workspace.document(TwoWaySide::Left).text(), "AA\n");
        assert_eq!(workspace.document(TwoWaySide::Right).text(), "B\n");
        assert!(workspace.undo());
        assert_eq!(workspace.text(TwoWaySide::Left), "A\n");
        assert_eq!(workspace.text(TwoWaySide::Right), "B\n");
        assert_eq!(workspace.document(TwoWaySide::Right).text(), "B\n");
    }

    #[test]
    fn facade_redo_replays_undone_documents_in_original_order() {
        let mut workspace = TwoWayWorkspace::new("a\n", "b\n");
        workspace.set_text(TwoWaySide::Left, "A\n");
        workspace.set_text(TwoWaySide::Right, "B\n");
        assert!(workspace.undo());
        assert!(workspace.undo());
        assert_eq!(workspace.text(TwoWaySide::Left), "a\n");
        assert_eq!(workspace.text(TwoWaySide::Right), "b\n");
        assert!(workspace.redo());
        assert_eq!(workspace.text(TwoWaySide::Left), "A\n");
        assert_eq!(workspace.text(TwoWaySide::Right), "b\n");
        assert!(workspace.redo());
        assert_eq!(workspace.text(TwoWaySide::Right), "B\n");
    }

    #[test]
    fn byte_edit_anchor_moves_repeated_protected_line_without_content_matching() {
        let mut document = InteractiveDocument::new("x\nx\np\n");
        document.ignored.push(1..2);
        document
            .apply_byte_edits(HistoryTag::Local(revision()), vec![(0..0, "x\n".into())])
            .unwrap();
        assert_eq!(document.ignored_ranges(), &[2..3]);
    }

    #[test]
    fn byte_word_edits_leave_line_metadata_and_disjoint_ranges_intact() {
        let mut document = InteractiveDocument::new("one\nkeep\nprotected\nlast\n");
        document.ignored.push(2..3);
        document.manual.push(InteractiveChunk {
            id: 1,
            range: 1..2,
            kind: ChunkKind::Manual,
        });
        document
            .apply_byte_edits(
                HistoryTag::Local(revision()),
                vec![(0..3, "ONE".into()), (4..8, "KEEP".into())],
            )
            .unwrap();
        assert_eq!(document.ignored_ranges(), &[2..3]);
        assert_eq!(document.manual[0].range, 1..2);
    }

    #[test]
    fn manual_byte_edit_anchors_duplicate_lines_and_can_edit_protected_text() {
        let mut workspace = TwoWayWorkspace::new("x\nx\np\n", "");
        workspace.add_ignored_range(TwoWaySide::Left, 1..2).unwrap();
        // Insert a complete duplicate line before the protected second `x`.
        workspace
            .edit_bytes(TwoWaySide::Left, 0..0, "x\n".into())
            .unwrap();
        assert_eq!(workspace.ignored_ranges(TwoWaySide::Left), &[2..3]);
        assert!(workspace.undo());
        assert_eq!(workspace.text(TwoWaySide::Left), "x\nx\np\n");
        assert_eq!(workspace.ignored_ranges(TwoWaySide::Left), &[1..2]);
        // User typing is explicit and may modify the protected line itself.
        let protected_start = "x\n".len();
        workspace
            .edit_bytes(
                TwoWaySide::Left,
                protected_start..protected_start + 1,
                "X".into(),
            )
            .unwrap();
        assert_eq!(workspace.text(TwoWaySide::Left), "x\nX\np\n");
        assert_eq!(workspace.ignored_ranges(TwoWaySide::Left), &[1..2]);
    }
}
