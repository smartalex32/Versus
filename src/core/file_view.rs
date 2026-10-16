//! UI-independent file comparison orchestration and its public display model.
//!
//! This module coordinates bounded input loading with line alignment. Its internal
//! capabilities keep source decoding, changed-block alignment, and text presentation
//! details local while preserving one public model for the application.

mod alignment;
mod input;
mod text;

use super::{CompareError, DirectoryEntryState};
use std::{
    ops::Range,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use input::{OptionalText, read_optional_text};
use text::{comparable_lines, lines_for_view};

#[cfg(test)]
use alignment::{ALIGNMENT_CHARACTER_WORK_BUDGET, line_pair_cost};

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

    input::begin_reading_progress(paths, options);
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
            return alignment::rows_for_one_side(right, false, cancellation, options);
        }
        (OptionalText::Text(left), OptionalText::Missing) => {
            return alignment::rows_for_one_side(left, true, cancellation, options);
        }
        (OptionalText::Text(left), OptionalText::Text(right)) => (left, right),
    };

    compare_loaded_text_with_timeout([&left, &right], cancellation, options, [&[], &[]], timeout)
}

/// Compare already loaded working copies without any filesystem reads.
/// Ignored ranges are omitted independently and returned line numbers retain
/// their positions in the original working copies.
pub fn compare_loaded_text(
    text: [&str; 2],
    cancellation: &AtomicBool,
    options: &FileViewOptions,
    ignored: [&[Range<usize>]; 2],
) -> Result<Option<FileComparison>, CompareError> {
    compare_loaded_text_with_timeout(text, cancellation, options, ignored, DIFF_TIMEOUT)
}

fn compare_loaded_text_with_timeout(
    text: [&str; 2],
    cancellation: &AtomicBool,
    options: &FileViewOptions,
    ignored: [&[Range<usize>]; 2],
    timeout: Duration,
) -> Result<Option<FileComparison>, CompareError> {
    if cancelled(cancellation) {
        return Ok(None);
    }
    let source_lines = [lines_for_view(text[0]), lines_for_view(text[1])];
    let maps: [Vec<usize>; 2] = std::array::from_fn(|side| {
        (0..source_lines[side].len())
            .filter(|line| !ignored[side].iter().any(|range| range.contains(line)))
            .collect()
    });
    let lines: [Vec<_>; 2] = std::array::from_fn(|side| {
        maps[side]
            .iter()
            .map(|index| source_lines[side][*index].clone())
            .collect()
    });
    let comparable = [
        comparable_lines(&lines[0], options),
        comparable_lines(&lines[1], options),
    ];
    alignment::compare_line_sets(lines, comparable, maps, cancellation, options, timeout)
}

pub(super) fn finish_progress(options: &FileViewOptions) {
    if let Some(progress) = &options.progress {
        progress.begin(super::ProgressStage::Finished, Some(0));
    }
}

pub(super) fn cancelled(cancellation: &AtomicBool) -> bool {
    cancellation.load(Ordering::Relaxed)
}

#[cfg(test)]
#[path = "file_view/tests.rs"]
mod tests;
