use super::{
    CompareError, CompareErrorKind, DEFAULT_BUFFER_SIZE, DEFAULT_TEXT_SIZE_LIMIT,
    DirectoryEntryState,
};
use similar::{ChangeTag, TextDiff as SimilarTextDiff};
use std::{
    fs,
    io::{BufReader, Read},
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// A UI-independent, aligned display model for comparing two text files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileComparison {
    pub rows: Vec<FileComparisonRow>,
    /// Explains why no line comparison is available, such as binary or oversized input.
    pub message: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileComparisonRow {
    /// One-based line number and rendered line content for the left file.
    pub left: Option<(usize, String)>,
    /// One-based line number and rendered line content for the right file.
    pub right: Option<(usize, String)>,
    pub state: DirectoryEntryState,
    /// Byte ranges in the rendered left text that differ from its aligned peer.
    pub left_changed: Vec<Range<usize>>,
    /// Byte ranges in the rendered right text that differ from its aligned peer.
    pub right_changed: Vec<Range<usize>>,
    /// The original line terminator, if any, for the left rendered line.
    pub left_ending: Option<DisplayLineEnding>,
    /// The original line terminator, if any, for the right rendered line.
    pub right_ending: Option<DisplayLineEnding>,
}

/// A source line ending retained separately from the rendered line text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayLineEnding {
    None,
    Lf,
    CrLf,
    Cr,
}

impl DisplayLineEnding {
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "No ending",
            Self::Lf => "LF",
            Self::CrLf => "CRLF",
            Self::Cr => "CR",
        }
    }
}

/// Display comparison options. The default preserves the historical behavior of
/// accepting line-ending-only changes while still comparing all other text exactly.
#[derive(Clone, Debug)]
pub struct FileViewOptions {
    pub ignore_whitespace: bool,
    pub ignore_line_endings: bool,
    pub progress: Option<Arc<super::ComparisonProgress>>,
}

impl Default for FileViewOptions {
    fn default() -> Self {
        Self {
            ignore_whitespace: false,
            ignore_line_endings: true,
            progress: None,
        }
    }
}

const DIFF_TIMEOUT: Duration = Duration::from_secs(2);
const DIFF_TIMEOUT_MESSAGE: &str =
    "File comparison exceeded the processing time limit; no line comparison is available.";

/// Reads up to two regular files and creates aligned rows for a side-by-side text display.
///
/// A missing side is represented by `None`; every available line is then marked as existing
/// only on its side. `Ok(None)` means a caller cancelled the work.
pub fn load_file_comparison(
    paths: &[Option<PathBuf>; 2],
    cancellation: &AtomicBool,
) -> Result<Option<FileComparison>, CompareError> {
    load_file_comparison_with_options(paths, cancellation, &FileViewOptions::default())
}

/// Reads and aligns files using the selected comparison rules.
pub fn load_file_comparison_with_options(
    paths: &[Option<PathBuf>; 2],
    cancellation: &AtomicBool,
    options: &FileViewOptions,
) -> Result<Option<FileComparison>, CompareError> {
    load_file_comparison_with_timeout(paths, cancellation, options, DIFF_TIMEOUT)
}

fn load_file_comparison_with_timeout(
    paths: &[Option<PathBuf>; 2],
    cancellation: &AtomicBool,
    options: &FileViewOptions,
    timeout: Duration,
) -> Result<Option<FileComparison>, CompareError> {
    if cancelled(cancellation) {
        return Ok(None);
    }

    begin_reading_progress(paths, options);
    let left = read_optional_text(paths[0].as_deref(), cancellation, options.progress.as_ref())?;
    if cancelled(cancellation) {
        return Ok(None);
    }
    let right = read_optional_text(paths[1].as_deref(), cancellation, options.progress.as_ref())?;
    if cancelled(cancellation) {
        return Ok(None);
    }

    let (left, right) = match (left, right) {
        (OptionalText::Missing, OptionalText::Missing) => {
            finish_progress(options);
            return Ok(Some(FileComparison {
                rows: Vec::new(),
                message: Some("No file selected.".into()),
            }));
        }
        (OptionalText::Unavailable(message), _) | (_, OptionalText::Unavailable(message)) => {
            finish_progress(options);
            return Ok(Some(FileComparison {
                rows: Vec::new(),
                message: Some(message),
            }));
        }
        (OptionalText::Missing, OptionalText::Text(right)) => {
            return rows_for_one_side(right, false, cancellation, options);
        }
        (OptionalText::Text(left), OptionalText::Missing) => {
            return rows_for_one_side(left, true, cancellation, options);
        }
        (OptionalText::Text(left), OptionalText::Text(right)) => (left, right),
    };

    let left_lines = lines_for_view(&left);
    let right_lines = lines_for_view(&right);
    drop(left);
    drop(right);
    let left_compare = comparable_lines(&left_lines, options);
    let right_compare = comparable_lines(&right_lines, options);
    let left_refs: Vec<_> = left_compare.iter().map(String::as_str).collect();
    let right_refs: Vec<_> = right_compare.iter().map(String::as_str).collect();
    if let Some(progress) = &options.progress {
        progress.begin(super::ProgressStage::ComparingLines, None);
    }
    // A cancellation request cannot interrupt Similar while it computes a diff, so
    // give its worst-case work a finite bound before observing cancellation again.
    let diff_started = Instant::now();
    let diff = SimilarTextDiff::configure()
        .timeout(timeout)
        .diff_slices(&left_refs, &right_refs);
    if diff_started.elapsed() >= timeout {
        finish_progress(options);
        return Ok(Some(FileComparison {
            rows: Vec::new(),
            message: Some(DIFF_TIMEOUT_MESSAGE.into()),
        }));
    }
    let highlight_deadline = diff_started + timeout;
    if cancelled(cancellation) {
        return Ok(None);
    }
    let mut rows = Vec::new();
    let mut left_index = 0;
    let mut right_index = 0;
    let mut pending_left = Vec::new();
    let mut pending_right = Vec::new();
    if let Some(progress) = &options.progress {
        let changed = diff
            .ops()
            .iter()
            .filter(|op| op.tag() != similar::DiffTag::Equal)
            .map(|op| op.old_range().len().max(op.new_range().len()) as u64)
            .sum();
        progress.begin(super::ProgressStage::Highlighting, Some(changed));
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
                    left: Some((left_index + 1, left_lines[left_index].content.clone())),
                    right: Some((right_index + 1, right_lines[right_index].content.clone())),
                    state: DirectoryEntryState::Same,
                    left_changed: Vec::new(),
                    right_changed: Vec::new(),
                    left_ending: Some(left_lines[left_index].ending),
                    right_ending: Some(right_lines[right_index].ending),
                });
                left_index += 1;
                right_index += 1;
            }
            ChangeTag::Delete => {
                pending_left.push((left_index + 1, left_lines[left_index].clone()));
                left_index += 1;
            }
            ChangeTag::Insert => {
                pending_right.push((right_index + 1, right_lines[right_index].clone()));
                right_index += 1;
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
    Ok(Some(FileComparison {
        rows,
        message: None,
    }))
}

enum OptionalText {
    Missing,
    Text(String),
    Unavailable(String),
}

fn read_optional_text(
    path: Option<&Path>,
    cancellation: &AtomicBool,
    progress: Option<&Arc<super::ComparisonProgress>>,
) -> Result<OptionalText, CompareError> {
    let Some(path) = path else {
        return Ok(OptionalText::Missing);
    };
    let metadata = fs::symlink_metadata(path).map_err(|error| CompareError::io(path, error))?;
    if !metadata.is_file() {
        return Err(CompareError {
            path: Some(path.to_path_buf()),
            kind: CompareErrorKind::InvalidPath,
            message: "file comparison requires regular, non-symlink files".into(),
        });
    }
    if metadata.len() > DEFAULT_TEXT_SIZE_LIMIT {
        return Ok(OptionalText::Unavailable(format!(
            "{} exceeds the {} MiB text display limit.",
            path.display(),
            DEFAULT_TEXT_SIZE_LIMIT / (1024 * 1024)
        )));
    }
    let bytes = match read_bytes_cancellable(path, metadata.len(), cancellation, progress)? {
        ReadBytes::Cancelled => return Ok(OptionalText::Missing),
        ReadBytes::TooLarge => {
            return Ok(OptionalText::Unavailable(format!(
                "{} exceeds the {} MiB text display limit.",
                path.display(),
                DEFAULT_TEXT_SIZE_LIMIT / (1024 * 1024)
            )));
        }
        ReadBytes::Bytes(bytes) => bytes,
    };
    match super::decode_text(path, &bytes) {
        Ok(text) => Ok(OptionalText::Text(text)),
        Err(error) if error.kind == CompareErrorKind::InvalidText => Ok(OptionalText::Unavailable(
            format!("{} is binary or is not valid UTF-8 text.", path.display()),
        )),
        Err(error) => Err(error),
    }
}

enum ReadBytes {
    Cancelled,
    TooLarge,
    Bytes(Vec<u8>),
}

fn read_bytes_cancellable(
    path: &Path,
    expected_length: u64,
    cancellation: &AtomicBool,
    progress: Option<&Arc<super::ComparisonProgress>>,
) -> Result<ReadBytes, CompareError> {
    let capacity = usize::try_from(expected_length.min(DEFAULT_TEXT_SIZE_LIMIT)).unwrap_or(0);
    let mut bytes = Vec::with_capacity(capacity);
    let mut reader = BufReader::with_capacity(
        DEFAULT_BUFFER_SIZE,
        fs::File::open(path).map_err(|error| CompareError::io(path, error))?,
    );
    let mut buffer = [0_u8; DEFAULT_BUFFER_SIZE];
    loop {
        if cancelled(cancellation) {
            return Ok(ReadBytes::Cancelled);
        }
        let count = reader
            .read(&mut buffer)
            .map_err(|error| CompareError::io(path, error))?;
        if count == 0 {
            return Ok(ReadBytes::Bytes(bytes));
        }
        if bytes.len().saturating_add(count) > DEFAULT_TEXT_SIZE_LIMIT as usize {
            return Ok(ReadBytes::TooLarge);
        }
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(progress) = progress {
            progress.advance(count as u64);
        }
    }
}

fn rows_for_one_side(
    text: String,
    left_side: bool,
    cancellation: &AtomicBool,
    options: &FileViewOptions,
) -> Result<Option<FileComparison>, CompareError> {
    let mut rows = Vec::new();
    let lines = lines_for_view(&text);
    if let Some(progress) = &options.progress {
        progress.begin(super::ProgressStage::Highlighting, Some(lines.len() as u64));
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

fn append_changed_rows(
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
const ALIGNMENT_CHARACTER_WORK_BUDGET: usize = 128 * 1024;

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
            // A diagonal is preferred when it is cheaper. When an unequal run
            // is entirely unrelated, taking the gap at its far edge preserves
            // the conventional first-to-first replacement pairing.
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
) -> Vec<std::borrow::Cow<'a, str>> {
    lines
        .iter()
        .map(|(_, line)| {
            if options.ignore_whitespace {
                std::borrow::Cow::Owned(
                    line.content
                        .chars()
                        .filter(|character| !character.is_whitespace())
                        .collect(),
                )
            } else {
                std::borrow::Cow::Borrowed(line.content.as_str())
            }
        })
        .collect()
}

fn line_pair_cost(
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
    // Prefix/suffix handles the common cheap case. For text that differs at
    // both ends (for example a wrapped comment), use a bounded character diff
    // to recognize its common interior without knowing any comment syntax.
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
    // Keep every unequal substitution cheaper than two gaps so a same-sized
    // unrelated replacement remains paired. Above a small noise floor, each
    // additional shared character lowers the cost. That preserves the strict
    // preference for an exact original line over a near-duplicate insertion.
    150_u32.saturating_sub(shared_percent.saturating_sub(20) as u32)
}

#[derive(Clone, Debug)]
struct DisplayLine {
    content: String,
    ending: DisplayLineEnding,
}

fn lines_for_view(text: &str) -> Vec<DisplayLine> {
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

fn comparable_lines(lines: &[DisplayLine], options: &FileViewOptions) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            let mut content = if options.ignore_whitespace {
                line.content
                    .chars()
                    .filter(|character| !character.is_whitespace())
                    .collect()
            } else {
                line.content.clone()
            };
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

fn whole_range(text: &str) -> Option<Range<usize>> {
    (!text.is_empty()).then_some(0..text.len())
}

const HIGHLIGHT_LINE_LIMIT: usize = 16 * 1024;

struct HighlightText {
    normalized: String,
    byte_ranges: Vec<Range<usize>>,
}

fn changed_ranges_with_options(
    left: &str,
    right: &str,
    options: &FileViewOptions,
    cancellation: &AtomicBool,
    deadline: Instant,
) -> Option<(Vec<Range<usize>>, Vec<Range<usize>>)> {
    if cancelled(cancellation) {
        return None;
    }
    // Bound character-index bookkeeping before allocating one range per
    // character. A single large source line can otherwise dwarf the file limit.
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
        if let Some(previous) = merged.last_mut()
            && range.start <= previous.end
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

fn begin_reading_progress(paths: &[Option<PathBuf>; 2], options: &FileViewOptions) {
    let Some(progress) = &options.progress else {
        return;
    };
    let total = paths
        .iter()
        .flatten()
        .filter_map(|path| fs::symlink_metadata(path).ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum();
    progress.begin(super::ProgressStage::Reading, Some(total));
}

fn finish_progress(options: &FileViewOptions) {
    if let Some(progress) = &options.progress {
        progress.begin(super::ProgressStage::Finished, Some(0));
    }
}

fn cancelled(cancellation: &AtomicBool) -> bool {
    cancellation.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn sandbox(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "versus-file-view-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn paths(left: Option<PathBuf>, right: Option<PathBuf>) -> [Option<PathBuf>; 2] {
        [left, right]
    }

    #[test]
    fn aligns_anchors_replacements_and_one_sided_lines() {
        let root = sandbox("alignment");
        let left = root.join("left");
        let right = root.join("right");
        fs::write(&left, "anchor\nold\nremove\ntail\n").unwrap();
        fs::write(&right, "anchor\nnew\ntail\nadded\n").unwrap();
        let result = load_file_comparison(&paths(Some(left), Some(right)), &AtomicBool::new(false))
            .unwrap()
            .unwrap();
        assert_eq!(
            result
                .rows
                .iter()
                .map(|row| row.state.clone())
                .collect::<Vec<_>>(),
            vec![
                DirectoryEntryState::Same,
                DirectoryEntryState::Different,
                DirectoryEntryState::LeftOnly,
                DirectoryEntryState::Same,
                DirectoryEntryState::RightOnly,
            ]
        );
        assert_eq!(result.rows[3].left.as_ref().unwrap().0, 4);
        assert_eq!(result.rows[3].right.as_ref().unwrap().0, 3);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn handles_empty_files_and_missing_sides() {
        let root = sandbox("empty");
        let file = root.join("file");
        let empty_left = root.join("empty-left");
        let empty_right = root.join("empty-right");
        fs::write(&empty_left, "").unwrap();
        fs::write(&empty_right, "").unwrap();
        let empty = load_file_comparison(
            &paths(Some(empty_left), Some(empty_right)),
            &AtomicBool::new(false),
        )
        .unwrap()
        .unwrap();
        assert!(empty.rows.is_empty());
        assert_eq!(empty.message, None);
        fs::write(&file, "one\ntwo\n").unwrap();
        let absent_file = load_file_comparison(
            &paths(Some(root.join("missing-left")), Some(file.clone())),
            &AtomicBool::new(false),
        );
        assert!(absent_file.is_err());
        let right_only = load_file_comparison(&paths(None, Some(file)), &AtomicBool::new(false))
            .unwrap()
            .unwrap();
        assert!(
            right_only
                .rows
                .iter()
                .all(|row| row.left.is_none() && row.state == DirectoryEntryState::RightOnly)
        );
        let no_files = load_file_comparison(&paths(None, None), &AtomicBool::new(false))
            .unwrap()
            .unwrap();
        assert!(no_files.rows.is_empty());
        assert_eq!(no_files.message.as_deref(), Some("No file selected."));

        let empty_left_only = load_file_comparison(
            &paths(Some(root.join("empty-left")), None),
            &AtomicBool::new(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            empty_left_only.message.as_deref(),
            Some("Only the left file is available; it is empty.")
        );
        let empty_right_only = load_file_comparison(
            &paths(None, Some(root.join("empty-right"))),
            &AtomicBool::new(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            empty_right_only.message.as_deref(),
            Some("Only the right file is available; it is empty.")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn normalizes_utf8_bom_and_all_common_line_endings_for_display() {
        let root = sandbox("line-endings");
        let left = root.join("left");
        let right = root.join("right");
        fs::write(&left, b"\xef\xbb\xbfone\r\ntwo\rthree\n").unwrap();
        fs::write(&right, "one\ntwo\nthree\n").unwrap();
        let result = load_file_comparison(&paths(Some(left), Some(right)), &AtomicBool::new(false))
            .unwrap()
            .unwrap();
        assert_eq!(result.rows.len(), 3);
        assert!(
            result
                .rows
                .iter()
                .all(|row| row.state == DirectoryEntryState::Same)
        );
        assert_eq!(result.rows[0].left.as_ref().unwrap().1, "one");
        assert_eq!(result.rows[2].right.as_ref().unwrap().1, "three");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discards_approximate_rows_when_the_diff_timeout_is_reached() {
        let root = sandbox("timeout");
        let left = root.join("left");
        let right = root.join("right");
        fs::write(&left, "one\ntwo\n").unwrap();
        fs::write(&right, "one\nchanged\n").unwrap();
        let result = load_file_comparison_with_timeout(
            &paths(Some(left), Some(right)),
            &AtomicBool::new(false),
            &FileViewOptions::default(),
            Duration::ZERO,
        )
        .unwrap()
        .unwrap();
        assert!(result.rows.is_empty());
        assert_eq!(result.message.as_deref(), Some(DIFF_TIMEOUT_MESSAGE));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reports_binary_oversize_and_cancellation_without_reading_text() {
        let root = sandbox("unavailable");
        let binary = root.join("binary");
        fs::write(&binary, [0, 1, 2]).unwrap();
        let binary_result =
            load_file_comparison(&paths(Some(binary), None), &AtomicBool::new(false))
                .unwrap()
                .unwrap();
        assert!(binary_result.message.unwrap().contains("binary"));

        let too_large = root.join("large");
        let file = fs::File::create(&too_large).unwrap();
        file.set_len(DEFAULT_TEXT_SIZE_LIMIT + 1).unwrap();
        let large_result =
            load_file_comparison(&paths(Some(too_large), None), &AtomicBool::new(false))
                .unwrap()
                .unwrap();
        assert!(large_result.message.unwrap().contains("limit"));
        assert_eq!(
            load_file_comparison(&paths(None, None), &AtomicBool::new(true)).unwrap(),
            None
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_directories_as_file_inputs() {
        let root = sandbox("directory");
        let error = load_file_comparison(&paths(Some(root.clone()), None), &AtomicBool::new(false))
            .unwrap_err();
        assert_eq!(error.kind, CompareErrorKind::InvalidPath);
        assert!(error.message.contains("regular"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn short_and_unicode_containment_uses_character_similarity() {
        let cancellation = AtomicBool::new(false);
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut ascii_budget = ALIGNMENT_CHARACTER_WORK_BUDGET;
        let mut unicode_budget = ALIGNMENT_CHARACTER_WORK_BUDGET;
        let ascii = line_pair_cost("a", "//a", &cancellation, deadline, &mut ascii_budget).unwrap();
        let unicode =
            line_pair_cost("α", "//α", &cancellation, deadline, &mut unicode_budget).unwrap();
        assert_eq!(ascii, unicode);
        assert!(ascii < 150);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_inputs() {
        use std::os::unix::fs::symlink;

        let root = sandbox("symlink");
        let target = root.join("target");
        let link = root.join("link");
        fs::write(&target, "data\n").unwrap();
        symlink(&target, &link).unwrap();
        let error =
            load_file_comparison(&paths(Some(link), None), &AtomicBool::new(false)).unwrap_err();
        assert_eq!(error.kind, CompareErrorKind::InvalidPath);
        assert!(error.message.contains("non-symlink"));
        fs::remove_dir_all(root).unwrap();
    }
}
