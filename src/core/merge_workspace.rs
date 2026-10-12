//! Editable, UI-independent merge session state.
//!
//! Source text stays immutable. Each action tracks a byte range in the live result;
//! edits translate unaffected ranges and invalidate overlapping targets.

use super::{CompareError, CompareErrorKind, DEFAULT_TEXT_SIZE_LIMIT};
use similar::{DiffTag, TextDiff};
use std::{
    fmt,
    io::Read,
    ops::Range,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MergeSide {
    Incoming,
    Current,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MergeActionKind {
    Safe,
    Conflict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MergeLineEnding {
    None,
    Lf,
    Crlf,
    Cr,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeDisplayLine {
    pub number: usize,
    pub text: String,
    pub ending: MergeLineEnding,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeDisplayRow {
    pub incoming: Option<MergeDisplayLine>,
    pub current: Option<MergeDisplayLine>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeAction {
    pub id: usize,
    pub kind: MergeActionKind,
    /// Ordered, nonoverlapping display rows. A conflict always covers one row;
    /// consecutive safe rows share one action, even when they come from both sides.
    pub row_range: Range<usize>,
    pub is_pending: bool,
    /// Live result byte range; None after a choice or an overlapping manual edit.
    pub target_range: Option<Range<usize>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeWorkspaceError(pub String);
impl fmt::Display for MergeWorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for MergeWorkspaceError {}

#[derive(Clone, Debug)]
struct Edit {
    range: Range<usize>,
    replacement: Vec<String>,
}
#[derive(Clone, Debug)]
struct ActionData {
    public: MergeAction,
    incoming: Vec<String>,
    current: Vec<String>,
    automatic: Vec<String>,
    target: Option<Range<usize>>,
}

/// A merge buffer.  Incoming is normally applied into Current; choosing Current
/// resolves an action without changing its text.
#[derive(Clone, Debug)]
pub struct MergeWorkspace {
    rows: Vec<MergeDisplayRow>,
    actions: Vec<ActionData>,
    result: String,
    undo: Vec<WorkspaceSnapshot>,
    saved_result: String,
}
#[derive(Clone, Debug)]
struct WorkspaceSnapshot {
    result: String,
    actions: Vec<(bool, Option<Range<usize>>)>,
}

impl MergeWorkspace {
    pub fn new(base: Option<&str>, incoming: &str, current: &str) -> Self {
        let incoming_lines = source_lines(incoming);
        let current_lines = source_lines(current);
        let (rows, actions) = build_merge(
            base.map(source_lines).as_deref(),
            &incoming_lines,
            &current_lines,
        );
        Self {
            rows,
            actions,
            result: current.to_owned(),
            undo: Vec::new(),
            saved_result: current.to_owned(),
        }
    }
    pub fn rows(&self) -> &[MergeDisplayRow] {
        &self.rows
    }
    /// The returned action values are a render snapshot. Fetch again after editing.
    pub fn actions(&self) -> Vec<MergeAction> {
        self.actions.iter().map(|a| a.public.clone()).collect()
    }
    pub fn result(&self) -> &str {
        &self.result
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn is_dirty(&self) -> bool {
        self.result != self.saved_result
    }
    pub fn mark_saved(&mut self) {
        self.saved_result = self.result.clone();
    }
    pub fn mark_saved_result(&mut self, saved: &str) {
        self.saved_result = saved.to_owned();
    }
    pub fn edit_result(&mut self, replacement: String) {
        if self.result == replacement {
            return;
        }
        self.snapshot();
        let (old, replacement_len) = changed_range(&self.result, &replacement);
        self.result = replacement;
        self.translate_targets(old, replacement_len, None);
    }
    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.result = previous.result;
        for (action, (pending, target)) in self.actions.iter_mut().zip(previous.actions) {
            action.public.is_pending = pending;
            action.public.target_range = target.clone();
            action.target = target;
        }
        true
    }
    pub fn action_range(&self, id: usize) -> Result<Range<usize>, MergeWorkspaceError> {
        self.action(id)?.target.clone().ok_or_else(|| {
            MergeWorkspaceError(
                "cannot apply this merge action because its target was edited".into(),
            )
        })
    }
    pub fn apply_action(&mut self, id: usize, side: MergeSide) -> Result<(), MergeWorkspaceError> {
        let index = self
            .actions
            .iter()
            .position(|a| a.public.id == id)
            .ok_or_else(|| MergeWorkspaceError("merge action no longer exists".into()))?;
        if !self.actions[index].public.is_pending {
            return Ok(());
        }
        let range = self.action_range(id)?;
        let replacement = match side {
            MergeSide::Incoming => join_lines(&self.actions[index].incoming),
            MergeSide::Current => join_lines(&self.actions[index].current),
        };
        self.snapshot();
        self.replace_live(index, range, replacement);
        self.actions[index].public.is_pending = false;
        Ok(())
    }
    /// Applies every independently safe incoming change as one undoable operation.
    pub fn auto_merge(&mut self) -> Result<usize, MergeWorkspaceError> {
        let pending: Vec<usize> = self
            .actions
            .iter()
            .filter(|a| {
                a.public.is_pending && a.public.kind == MergeActionKind::Safe && a.target.is_some()
            })
            .map(|a| a.public.id)
            .collect();
        if pending.is_empty() {
            return Ok(0);
        }
        let ranges: Vec<(usize, Range<usize>)> = pending
            .iter()
            .copied()
            .map(|id| {
                let index = self.actions.iter().position(|a| a.public.id == id).unwrap();
                self.action_range(id).map(|range| (index, range))
            })
            .collect::<Result<_, _>>()?;
        self.snapshot();
        for (index, range) in ranges.into_iter().rev() {
            let replacement = join_lines(&self.actions[index].automatic);
            self.replace_live(index, range, replacement);
            self.actions[index].public.is_pending = false;
        }
        Ok(pending.len())
    }
    fn action(&self, id: usize) -> Result<&ActionData, MergeWorkspaceError> {
        self.actions
            .iter()
            .find(|a| a.public.id == id)
            .ok_or_else(|| MergeWorkspaceError("merge action no longer exists".into()))
    }
    fn snapshot(&mut self) {
        self.undo.push(WorkspaceSnapshot {
            result: self.result.clone(),
            actions: self
                .actions
                .iter()
                .map(|action| (action.public.is_pending, action.target.clone()))
                .collect(),
        });
        // Bound session memory while retaining at least the latest undo, even
        // for a large buffer. Source replacements are immutable and never copied.
        let mut bytes: usize = self
            .undo
            .iter()
            .map(|snapshot| {
                snapshot.result.len()
                    + snapshot.actions.len() * std::mem::size_of::<(bool, Option<Range<usize>>)>()
            })
            .sum();
        while self.undo.len() > 1 && (self.undo.len() > 256 || bytes > 64 * 1024 * 1024) {
            let oldest = self.undo.remove(0);
            bytes -= oldest.result.len()
                + oldest.actions.len() * std::mem::size_of::<(bool, Option<Range<usize>>)>();
        }
    }
    fn replace_live(&mut self, chosen: usize, range: Range<usize>, replacement: String) {
        let replacement_len = replacement.len();
        self.result.replace_range(range.clone(), &replacement);
        self.translate_targets(range, replacement_len, Some(chosen));
        self.actions[chosen].target = None;
        self.actions[chosen].public.target_range = None;
    }
    fn translate_targets(
        &mut self,
        old: Range<usize>,
        replacement_len: usize,
        chosen: Option<usize>,
    ) {
        let delta = replacement_len as isize - (old.end - old.start) as isize;
        for (index, action) in self.actions.iter_mut().enumerate() {
            let Some(range) = action.target.clone() else {
                continue;
            };
            let next = if old.is_empty() && range.is_empty() && range.start == old.start {
                chosen.map(|chosen| {
                    if index > chosen {
                        shift_range(range.clone(), delta)
                    } else {
                        range.clone()
                    }
                })
            } else if range.end <= old.start {
                Some(range)
            } else if range.start >= old.end {
                Some(shift_range(range, delta))
            } else {
                None
            };
            action.target = next.clone();
            action.public.target_range = next;
        }
    }
}

/// Loads regular, non-symlink UTF-8 files for a background merge worker.
/// `Ok(None)` reports cancellation.
pub fn load_merge_workspace(
    base: Option<&Path>,
    incoming: &Path,
    current: &Path,
    cancellation: &AtomicBool,
) -> Result<Option<MergeWorkspace>, CompareError> {
    if cancelled(cancellation) {
        return Ok(None);
    }
    let base = match base {
        Some(path) => Some(read_merge_text(path, cancellation)?),
        None => None,
    };
    if cancelled(cancellation) {
        return Ok(None);
    }
    let incoming = read_merge_text(incoming, cancellation)?;
    if cancelled(cancellation) {
        return Ok(None);
    }
    let current = read_merge_text(current, cancellation)?;
    if cancelled(cancellation) {
        return Ok(None);
    }
    let workspace = MergeWorkspace::new(base.as_deref(), &incoming, &current);
    Ok((!cancelled(cancellation)).then_some(workspace))
}

fn read_merge_text(path: &Path, cancellation: &AtomicBool) -> Result<String, CompareError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| CompareError::io(path, e))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CompareError {
            path: Some(path.to_path_buf()),
            kind: CompareErrorKind::InvalidPath,
            message: "merge requires a regular file, not a symlink".into(),
        });
    }
    if metadata.len() > DEFAULT_TEXT_SIZE_LIMIT {
        return Err(CompareError {
            path: Some(path.to_path_buf()),
            kind: CompareErrorKind::TooLarge,
            message: format!("file exceeds the text limit of {DEFAULT_TEXT_SIZE_LIMIT} bytes"),
        });
    }
    if cancelled(cancellation) {
        return Err(CompareError {
            path: Some(path.to_path_buf()),
            kind: CompareErrorKind::Cancelled,
            message: "merge loading was cancelled".into(),
        });
    }
    let mut reader = std::fs::File::open(path).map_err(|e| CompareError::io(path, e))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        if cancelled(cancellation) {
            return Err(CompareError {
                path: Some(path.to_path_buf()),
                kind: CompareErrorKind::Cancelled,
                message: "merge loading was cancelled".into(),
            });
        }
        let count = reader
            .read(&mut buffer)
            .map_err(|e| CompareError::io(path, e))?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() as u64 > DEFAULT_TEXT_SIZE_LIMIT {
            return Err(CompareError {
                path: Some(path.to_path_buf()),
                kind: CompareErrorKind::TooLarge,
                message: format!("file exceeds the text limit of {DEFAULT_TEXT_SIZE_LIMIT} bytes"),
            });
        }
    }
    if cancelled(cancellation) {
        return Err(CompareError {
            path: Some(path.to_path_buf()),
            kind: CompareErrorKind::Cancelled,
            message: "merge loading was cancelled".into(),
        });
    }
    if bytes.contains(&0) {
        return Err(CompareError::invalid_text(path));
    }
    String::from_utf8(bytes).map_err(|_| CompareError::invalid_text(path))
}
fn cancelled(value: &AtomicBool) -> bool {
    value.load(Ordering::Relaxed)
}

// Connected regions include every edit that overlaps an already connected edit.
// Adjacent replacements stay separate until their safe actions are grouped, so a
// conflict on one line does not absorb independent neighboring changes.
struct Region {
    range: Range<usize>,
    incoming: Vec<Edit>,
    current: Vec<Edit>,
}

fn build_merge(
    base: Option<&[String]>,
    incoming: &[String],
    current: &[String],
) -> (Vec<MergeDisplayRow>, Vec<ActionData>) {
    let mut builder = MergeBuilder::default();
    let Some(base) = base else {
        builder.append(
            incoming,
            current,
            Some(MergeActionKind::Conflict),
            MergeSide::Incoming,
        );
        return (builder.rows, builder.actions);
    };
    let mut edits: Vec<(Edit, MergeSide)> = edits(base, incoming)
        .into_iter()
        .map(|edit| (edit, MergeSide::Incoming))
        .chain(
            edits(base, current)
                .into_iter()
                .map(|edit| (edit, MergeSide::Current)),
        )
        .collect();
    edits.sort_by_key(|(edit, _)| (edit.range.start, edit.range.end));
    let mut regions: Vec<Region> = Vec::new();
    for (edit, side) in edits {
        if !regions
            .last()
            .is_some_and(|region| ranges_overlap(&region.range, &edit.range))
        {
            regions.push(Region {
                range: edit.range.clone(),
                incoming: Vec::new(),
                current: Vec::new(),
            });
        }
        let region = regions.last_mut().unwrap();
        region.range.end = region.range.end.max(edit.range.end);
        match side {
            MergeSide::Incoming => region.incoming.push(edit),
            MergeSide::Current => region.current.push(edit),
        }
    }
    let mut cursor = 0;
    for region in regions {
        builder.append(
            &base[cursor..region.range.start],
            &base[cursor..region.range.start],
            None,
            MergeSide::Current,
        );
        let left = render_region(base, &region.range, &region.incoming);
        let right = render_region(base, &region.range, &region.current);
        let kind = if left == right {
            None
        } else if region.incoming.is_empty() || region.current.is_empty() {
            Some(MergeActionKind::Safe)
        } else {
            Some(MergeActionKind::Conflict)
        };
        let automatic = if region.incoming.is_empty() {
            MergeSide::Current
        } else {
            MergeSide::Incoming
        };
        builder.append(&left, &right, kind, automatic);
        cursor = region.range.end;
    }
    builder.append(&base[cursor..], &base[cursor..], None, MergeSide::Current);
    (builder.rows, builder.actions)
}

#[derive(Default)]
struct MergeBuilder {
    rows: Vec<MergeDisplayRow>,
    actions: Vec<ActionData>,
    incoming_number: usize,
    current_number: usize,
    current_byte: usize,
}
impl MergeBuilder {
    fn append(
        &mut self,
        incoming: &[String],
        current: &[String],
        kind: Option<MergeActionKind>,
        automatic: MergeSide,
    ) {
        let diff = TextDiff::from_slices(&refs(incoming), &refs(current));
        for op in diff.ops() {
            let left = op.old_range();
            let right = op.new_range();
            for offset in 0..left.len().max(right.len()) {
                let incoming = (offset < left.len()).then(|| incoming[left.start + offset].clone());
                let current = (offset < right.len()).then(|| current[right.start + offset].clone());
                let row = self.rows.len();
                let target =
                    self.current_byte..self.current_byte + current.as_ref().map_or(0, String::len);
                self.rows.push(MergeDisplayRow {
                    incoming: incoming.as_ref().map(|line| {
                        self.incoming_number += 1;
                        display_line(self.incoming_number, line)
                    }),
                    current: current.as_ref().map(|line| {
                        self.current_number += 1;
                        display_line(self.current_number, line)
                    }),
                });
                self.current_byte = target.end;
                let Some(kind) = kind.filter(|_| incoming != current) else {
                    continue;
                };
                let automatic_text = match automatic {
                    MergeSide::Incoming => incoming.clone(),
                    MergeSide::Current => current.clone(),
                };
                if kind == MergeActionKind::Safe
                    && self.actions.last().is_some_and(|action| {
                        action.public.kind == MergeActionKind::Safe
                            && action.public.row_range.end == row
                    })
                {
                    let action = self.actions.last_mut().unwrap();
                    action.incoming.extend(incoming);
                    action.current.extend(current);
                    action.automatic.extend(automatic_text);
                    action.public.row_range.end = row + 1;
                    action.target.as_mut().unwrap().end = target.end;
                    action.public.target_range = action.target.clone();
                } else {
                    self.actions.push(ActionData {
                        public: MergeAction {
                            id: self.actions.len(),
                            kind,
                            row_range: row..row + 1,
                            is_pending: true,
                            target_range: Some(target.clone()),
                        },
                        incoming: incoming.into_iter().collect(),
                        current: current.into_iter().collect(),
                        automatic: automatic_text.into_iter().collect(),
                        target: Some(target),
                    });
                }
            }
        }
    }
}

fn ranges_overlap(a: &Range<usize>, b: &Range<usize>) -> bool {
    if a.is_empty() && b.is_empty() {
        return a.start == b.start;
    }
    if a.is_empty() {
        return b.start <= a.start && a.start < b.end;
    }
    if b.is_empty() {
        return a.start <= b.start && b.start < a.end;
    }
    a.start < b.end && b.start < a.end
}
fn render_region(base: &[String], range: &Range<usize>, edits: &[Edit]) -> Vec<String> {
    let mut output = Vec::new();
    let mut cursor = range.start;
    for edit in edits {
        output.extend_from_slice(&base[cursor..edit.range.start]);
        output.extend_from_slice(&edit.replacement);
        cursor = edit.range.end;
    }
    output.extend_from_slice(&base[cursor..range.end]);
    output
}
fn edits(base: &[String], side: &[String]) -> Vec<Edit> {
    let diff = TextDiff::from_slices(&refs(base), &refs(side));
    let mut out = Vec::new();
    for op in diff.ops() {
        if op.tag() != DiffTag::Equal {
            let old = op.old_range();
            let new = op.new_range();
            if old.len() == new.len() {
                for offset in 0..old.len() {
                    out.push(Edit {
                        range: old.start + offset..old.start + offset + 1,
                        replacement: vec![side[new.start + offset].clone()],
                    });
                }
            } else {
                out.push(Edit {
                    range: old,
                    replacement: side[new].to_vec(),
                });
            }
        }
    }
    out
}
fn refs(lines: &[String]) -> Vec<&str> {
    lines.iter().map(String::as_str).collect()
}
fn shift_range(range: Range<usize>, delta: isize) -> Range<usize> {
    ((range.start as isize + delta) as usize)..((range.end as isize + delta) as usize)
}
fn changed_range(old: &str, new: &str) -> (Range<usize>, usize) {
    let mut prefix = 0;
    for (a, b) in old.chars().zip(new.chars()) {
        if a != b {
            break;
        }
        prefix += a.len_utf8();
    }
    let mut old_end = old.len();
    let mut new_end = new.len();
    while old_end > prefix && new_end > prefix {
        let a = old[..old_end].chars().next_back().unwrap();
        let b = new[..new_end].chars().next_back().unwrap();
        if a != b {
            break;
        }
        old_end -= a.len_utf8();
        new_end -= b.len_utf8();
    }
    (prefix..old_end, new_end - prefix)
}
fn join_lines(lines: &[String]) -> String {
    lines.concat()
}
fn source_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            let end = i + 1;
            out.push(text[start..end].to_owned());
            start = end;
        } else if bytes[i] == b'\r' {
            let end = if bytes.get(i + 1) == Some(&b'\n') {
                i + 2
            } else {
                i + 1
            };
            out.push(text[start..end].to_owned());
            start = end;
            i = end - 1;
        }
        i += 1;
    }
    if start < text.len() {
        out.push(text[start..].to_owned());
    }
    out
}
fn display_line(number: usize, line: &str) -> MergeDisplayLine {
    let (text, ending) = if let Some(value) = line.strip_suffix("\r\n") {
        (value, MergeLineEnding::Crlf)
    } else if let Some(value) = line.strip_suffix('\n') {
        (value, MergeLineEnding::Lf)
    } else if let Some(value) = line.strip_suffix('\r') {
        (value, MergeLineEnding::Cr)
    } else {
        (line, MergeLineEnding::None)
    };
    MergeDisplayLine {
        number,
        text: text.to_owned(),
        ending,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auto_merges_disjoint_edits_and_undoes_once() {
        let mut w = MergeWorkspace::new(Some("a\nb\nc\n"), "A\nb\nc\n", "a\nb\nC\n");
        assert_eq!(w.auto_merge().unwrap(), 2);
        assert_eq!(w.result(), "A\nb\nC\n");
        assert!(w.undo());
        assert_eq!(w.result(), "a\nb\nC\n");
    }
    #[test]
    fn overlap_is_conflict() {
        let w = MergeWorkspace::new(Some("a\nb\nc\n"), "a\nB\nc\n", "a\nX\nc\n");
        assert_eq!(w.actions()[0].kind, MergeActionKind::Conflict);
    }
    #[test]
    fn manual_edit_before_target_rebases_and_inside_refuses() {
        let mut w = MergeWorkspace::new(Some("a\nb\nc\n"), "a\nb\nC\n", "a\nb\nc\n");
        let id = w.actions()[0].id;
        w.edit_result("z\na\nb\nc\n".into());
        w.apply_action(id, MergeSide::Incoming).unwrap();
        assert_eq!(w.result(), "z\na\nb\nC\n");
        let mut w = MergeWorkspace::new(Some("a\nb\nc\n"), "a\nb\nC\n", "a\nb\nc\n");
        let id = w.actions()[0].id;
        w.edit_result("a\nb\nchanged\n".into());
        assert!(w.apply_action(id, MergeSide::Incoming).is_err());
    }
    #[test]
    fn preserves_unicode_and_endings() {
        let mut w = MergeWorkspace::new(Some("α\r\nβ\r\n"), "α\r\nγ\r\n", "α\r\nβ\r\n");
        w.auto_merge().unwrap();
        assert_eq!(w.result(), "α\r\nγ\r\n");
    }
    #[test]
    fn save_snapshot_tracks_later_edits() {
        let mut w = MergeWorkspace::new(None, "a\n", "b\n");
        w.edit_result("c\n".into());
        w.mark_saved_result("c\n");
        assert!(!w.is_dirty());
        w.edit_result("d\n".into());
        w.mark_saved_result("c\n");
        assert!(w.is_dirty());
    }
    #[test]
    fn mixed_safe_replacements_are_grouped_around_a_line_conflict() {
        let base = "title=Workspace\ncolor=blue\nsize=small\nmode=normal\nnotes=ready\n";
        let incoming = "title=Workspace\ncolor=green\nsize=large\nmode=incoming\nnotes=ready\n";
        let current = "title=Workspace\ncolor=blue\nsize=small\nmode=current\nnotes=ready\n";
        let mut workspace = MergeWorkspace::new(Some(base), incoming, current);
        let actions = workspace.actions();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].kind, MergeActionKind::Safe);
        assert_eq!(actions[1].kind, MergeActionKind::Conflict);
        assert!(actions.iter().all(|action| action.target_range.is_some()));
        assert_eq!(workspace.auto_merge().unwrap(), 1);
        assert_eq!(
            workspace.result(),
            "title=Workspace\ncolor=green\nsize=large\nmode=current\nnotes=ready\n"
        );
    }

    fn source_from_rows(rows: &[MergeDisplayRow], side: MergeSide) -> String {
        rows.iter()
            .filter_map(|row| {
                if side == MergeSide::Incoming {
                    row.incoming.as_ref()
                } else {
                    row.current.as_ref()
                }
            })
            .map(|line| {
                format!(
                    "{}{}",
                    line.text,
                    match line.ending {
                        MergeLineEnding::None => "",
                        MergeLineEnding::Lf => "\n",
                        MergeLineEnding::Crlf => "\r\n",
                        MergeLineEnding::Cr => "\r",
                    }
                )
            })
            .collect()
    }

    #[test]
    fn small_inputs_preserve_sources_and_all_choices_in_both_orders() {
        let inputs = [
            "",
            "a",
            "a\n",
            "a\nb\n",
            "a\nb\nc\n",
            "a\nx\nb\n",
            "x\nb\ny\n",
            "x\n",
            "b\n",
            "a\nc\n",
            "α\r\nβ",
            "a\na\na\n",
        ];
        for base in inputs.iter().copied().map(Some).chain([None]) {
            for incoming in inputs {
                for current in inputs {
                    let workspace = MergeWorkspace::new(base, incoming, current);
                    assert_eq!(
                        source_from_rows(workspace.rows(), MergeSide::Incoming),
                        incoming,
                        "base {base:?}"
                    );
                    assert_eq!(
                        source_from_rows(workspace.rows(), MergeSide::Current),
                        current,
                        "base {base:?}"
                    );
                    let actions = workspace.actions();
                    for pair in actions.windows(2) {
                        assert!(pair[0].row_range.end <= pair[1].row_range.start);
                    }
                    for action in &actions {
                        assert!(action.row_range.end <= workspace.rows().len());
                        if action.kind == MergeActionKind::Conflict {
                            assert_eq!(action.row_range.len(), 1);
                        }
                        let range = action.target_range.clone().unwrap();
                        assert!(
                            current.is_char_boundary(range.start)
                                && current.is_char_boundary(range.end)
                        );
                    }
                    for reverse in [false, true] {
                        let mut order: Vec<_> = actions.iter().map(|action| action.id).collect();
                        if reverse {
                            order.reverse();
                        }
                        for side in [MergeSide::Incoming, MergeSide::Current] {
                            let mut merged = workspace.clone();
                            for id in &order {
                                merged.apply_action(*id, side).unwrap();
                            }
                            assert_eq!(
                                merged.result(),
                                if side == MergeSide::Incoming {
                                    incoming
                                } else {
                                    current
                                },
                                "base={base:?}, incoming={incoming:?}, current={current:?}, reverse={reverse}, side={side:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn spanning_conflicts_close_over_all_overlapping_edits_and_split_into_lines() {
        let base = "head\na\nb\nc\nd\ntail\n";
        let incoming = "head\nreplacement\ntail\n";
        let current = "head\nA\nb\nC\nd\ntail\n";
        let mut workspace = MergeWorkspace::new(Some(base), incoming, current);
        let actions = workspace.actions();
        assert_eq!(actions.len(), 4);
        assert!(
            actions
                .iter()
                .all(|action| action.kind == MergeActionKind::Conflict
                    && action.row_range.len() == 1)
        );
        assert_eq!(workspace.auto_merge().unwrap(), 0);
        for action in actions {
            workspace
                .apply_action(action.id, MergeSide::Incoming)
                .unwrap();
        }
        assert_eq!(workspace.result(), incoming);
    }

    #[test]
    fn adjacent_changes_from_different_sides_auto_merge_as_one_safe_block() {
        let mut workspace = MergeWorkspace::new(Some("a\nb\n"), "a\ninsert\nb\n", "A\nb\n");
        assert_eq!(workspace.actions().len(), 1);
        assert_eq!(workspace.auto_merge().unwrap(), 1);
        assert_eq!(workspace.result(), "A\ninsert\nb\n");
        assert!(workspace.undo());
        assert_eq!(workspace.result(), "A\nb\n");
        assert!(workspace.actions()[0].is_pending);
    }

    #[test]
    fn duplicate_lines_and_multiple_insertions_have_stable_targets() {
        for order in [[0, 1, 2], [2, 1, 0], [1, 0, 2]] {
            let mut workspace = MergeWorkspace::new(None, "a\nb\nc\n", "");
            for id in order {
                workspace.apply_action(id, MergeSide::Incoming).unwrap();
            }
            assert_eq!(workspace.result(), "a\nb\nc\n");
        }
        let mut workspace = MergeWorkspace::new(
            Some("same\nsame\nsame\n"),
            "first\nsame\nlast\n",
            "same\nsame\nsame\n",
        );
        assert!(
            workspace
                .actions()
                .iter()
                .all(|action| action.target_range.is_some())
        );
        workspace.auto_merge().unwrap();
        assert_eq!(workspace.result(), "first\nsame\nlast\n");
    }

    #[test]
    fn manual_edits_do_not_block_other_safe_changes_and_undo_restores_metadata() {
        let mut workspace = MergeWorkspace::new(Some("a\nb\nc\n"), "A\nb\nC\n", "a\nb\nc\n");
        workspace.edit_result("hand typed\nb\nc\n".into());
        assert_eq!(workspace.auto_merge().unwrap(), 1);
        assert_eq!(workspace.result(), "hand typed\nb\nC\n");
        assert!(workspace.undo());
        assert_eq!(workspace.result(), "hand typed\nb\nc\n");
        assert!(workspace.undo());
        assert_eq!(workspace.result(), "a\nb\nc\n");
        assert!(
            workspace
                .actions()
                .iter()
                .all(|action| action.is_pending && action.target_range.is_some())
        );
        workspace.apply_action(0, MergeSide::Current).unwrap();
        assert_eq!(workspace.result(), "a\nb\nc\n");
        assert!(!workspace.actions()[0].is_pending);
        assert!(workspace.undo());
        assert!(workspace.actions()[0].is_pending);
    }

    #[test]
    fn loader_rejects_nontext_special_and_large_inputs_and_preserves_bom() {
        let directory = std::env::temp_dir().join(format!(
            "versus-merge-loader-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("text");
        let cancelled = AtomicBool::new(false);
        std::fs::write(&path, "\u{feff}α\r\nβ").unwrap();
        let workspace = load_merge_workspace(None, &path, &path, &cancelled)
            .unwrap()
            .unwrap();
        assert_eq!(workspace.result(), "\u{feff}α\r\nβ");
        assert!(load_merge_workspace(None, &directory, &path, &cancelled).is_err());
        std::fs::write(&path, [0_u8, 1]).unwrap();
        assert_eq!(
            load_merge_workspace(None, &path, &path, &cancelled)
                .unwrap_err()
                .kind,
            CompareErrorKind::InvalidText
        );
        std::fs::File::create(&path)
            .unwrap()
            .set_len(DEFAULT_TEXT_SIZE_LIMIT + 1)
            .unwrap();
        assert_eq!(
            load_merge_workspace(None, &path, &path, &cancelled)
                .unwrap_err()
                .kind,
            CompareErrorKind::TooLarge
        );
        cancelled.store(true, Ordering::Relaxed);
        assert!(
            load_merge_workspace(None, &path, &path, &cancelled)
                .unwrap()
                .is_none()
        );
        #[cfg(unix)]
        {
            let link = directory.join("link");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            cancelled.store(false, Ordering::Relaxed);
            assert_eq!(
                load_merge_workspace(None, &link, &path, &cancelled)
                    .unwrap_err()
                    .kind,
                CompareErrorKind::InvalidPath
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
