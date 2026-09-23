use super::{CompareError, CompareErrorKind};
use similar::{ChangeTag, TextDiff as SimilarTextDiff};
use std::{
    fs,
    io::{BufReader, Read},
    ops::Range,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub const DEFAULT_TEXT_SIZE_LIMIT: u64 = 32 * 1024 * 1024;
pub const DEFAULT_BUFFER_SIZE: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct FileCompareOptions {
    pub text_size_limit: u64,
    pub buffer_size: usize,
    pub ignore_line_endings: bool,
    pub ignore_whitespace: bool,
}
impl Default for FileCompareOptions {
    fn default() -> Self {
        Self {
            text_size_limit: DEFAULT_TEXT_SIZE_LIMIT,
            buffer_size: DEFAULT_BUFFER_SIZE,
            ignore_line_endings: true,
            ignore_whitespace: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileDiffKind {
    Text,
    Binary,
    TooLarge,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HunkKind {
    Added,
    Removed,
    Changed,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffHunk {
    pub left_range: Range<usize>,
    pub right_range: Range<usize>,
    pub kind: HunkKind,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextDiff {
    pub left_lines: Vec<String>,
    pub right_lines: Vec<String>,
    pub hunks: Vec<DiffHunk>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDiff {
    pub kind: FileDiffKind,
    pub equal: bool,
    pub text: Option<TextDiff>,
}

/// Compares editable UTF-8 buffers using the same normalization options as file comparison.
pub fn compare_texts(left: &str, right: &str, options: &FileCompareOptions) -> TextDiff {
    let left_lines = display_lines(left);
    let right_lines = display_lines(right);
    let left_compare = normalized(left, options);
    let right_compare = normalized(right, options);
    TextDiff {
        left_lines,
        right_lines,
        hunks: build_hunks(&left_compare, &right_compare),
    }
}

pub fn compare_files(
    left: impl AsRef<Path>,
    right: impl AsRef<Path>,
    options: &FileCompareOptions,
) -> Result<FileDiff, CompareError> {
    let left = left.as_ref();
    let right = right.as_ref();
    let left_metadata = fs::metadata(left).map_err(|e| CompareError::io(left, e))?;
    let right_metadata = fs::metadata(right).map_err(|e| CompareError::io(right, e))?;
    if !left_metadata.is_file() || !right_metadata.is_file() {
        return Err(CompareError {
            path: None,
            kind: CompareErrorKind::InvalidPath,
            message: "file comparison requires two regular files".into(),
        });
    }
    let equal = buffered_files_equal(left, right, options.buffer_size)?;
    if left_metadata.len() > options.text_size_limit
        || right_metadata.len() > options.text_size_limit
    {
        return Ok(FileDiff {
            kind: FileDiffKind::TooLarge,
            equal,
            text: None,
        });
    }
    let left_bytes = fs::read(left).map_err(|e| CompareError::io(left, e))?;
    let right_bytes = fs::read(right).map_err(|e| CompareError::io(right, e))?;
    let left_text = match decode_text(left, &left_bytes) {
        Ok(text) => text,
        Err(_) => {
            return Ok(FileDiff {
                kind: FileDiffKind::Binary,
                equal,
                text: None,
            });
        }
    };
    let right_text = match decode_text(right, &right_bytes) {
        Ok(text) => text,
        Err(_) => {
            return Ok(FileDiff {
                kind: FileDiffKind::Binary,
                equal,
                text: None,
            });
        }
    };
    let text = compare_texts(&left_text, &right_text, options);
    Ok(FileDiff {
        kind: FileDiffKind::Text,
        equal: text.hunks.is_empty(),
        text: Some(text),
    })
}

pub fn buffered_files_equal(
    left: impl AsRef<Path>,
    right: impl AsRef<Path>,
    buffer_size: usize,
) -> Result<bool, CompareError> {
    Ok(buffered_files_equal_cancellable(left, right, buffer_size, None)?.unwrap_or(false))
}

/// Compares two files in bounded buffers. `Ok(None)` means cancellation was requested.
pub fn buffered_files_equal_cancellable(
    left: impl AsRef<Path>,
    right: impl AsRef<Path>,
    buffer_size: usize,
    cancellation: Option<&AtomicBool>,
) -> Result<Option<bool>, CompareError> {
    let left = left.as_ref();
    let right = right.as_ref();
    if is_cancelled(cancellation) {
        return Ok(None);
    }
    let left_len = fs::metadata(left)
        .map_err(|e| CompareError::io(left, e))?
        .len();
    if left_len
        != fs::metadata(right)
            .map_err(|e| CompareError::io(right, e))?
            .len()
    {
        return Ok(Some(false));
    }
    let size = buffer_size.max(1);
    let mut left_reader = BufReader::with_capacity(
        size,
        fs::File::open(left).map_err(|e| CompareError::io(left, e))?,
    );
    let mut right_reader = BufReader::with_capacity(
        size,
        fs::File::open(right).map_err(|e| CompareError::io(right, e))?,
    );
    let mut left_buffer = vec![0; size];
    let mut right_buffer = vec![0; size];
    loop {
        if is_cancelled(cancellation) {
            return Ok(None);
        }
        let a = left_reader
            .read(&mut left_buffer)
            .map_err(|e| CompareError::io(left, e))?;
        let b = right_reader
            .read(&mut right_buffer)
            .map_err(|e| CompareError::io(right, e))?;
        if a != b || left_buffer[..a] != right_buffer[..b] {
            return Ok(Some(false));
        }
        if a == 0 {
            return Ok(Some(true));
        }
    }
}

fn is_cancelled(cancellation: Option<&AtomicBool>) -> bool {
    cancellation.is_some_and(|value| value.load(Ordering::Relaxed))
}

pub(crate) fn read_text(path: &Path, limit: u64) -> Result<String, CompareError> {
    let metadata = fs::metadata(path).map_err(|e| CompareError::io(path, e))?;
    if metadata.len() > limit {
        return Err(CompareError {
            path: Some(path.to_path_buf()),
            kind: CompareErrorKind::TooLarge,
            message: format!("file exceeds the text limit of {limit} bytes"),
        });
    }
    let bytes = fs::read(path).map_err(|e| CompareError::io(path, e))?;
    decode_text(path, &bytes)
}
pub(crate) fn decode_text(path: &Path, bytes: &[u8]) -> Result<String, CompareError> {
    if bytes.contains(&0) {
        return Err(CompareError::invalid_text(path));
    }
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    String::from_utf8(bytes.to_vec()).map_err(|_| CompareError::invalid_text(path))
}
pub(crate) fn display_lines(text: &str) -> Vec<String> {
    text.split_inclusive('\n')
        .map(|s| {
            let line = s.strip_suffix('\n').unwrap_or(s);
            line.strip_suffix('\r').unwrap_or(line).to_owned()
        })
        .collect()
}
pub(crate) fn normalized(text: &str, options: &FileCompareOptions) -> Vec<String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let source = if options.ignore_line_endings {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text.to_owned()
    };
    source
        .split_inclusive('\n')
        .map(|line| {
            let line = line.strip_suffix('\n').unwrap_or(line);
            if options.ignore_whitespace {
                line.chars().filter(|c| !c.is_whitespace()).collect()
            } else {
                line.to_owned()
            }
        })
        .collect()
}
pub(crate) fn build_hunks(left: &[String], right: &[String]) -> Vec<DiffHunk> {
    let left_refs: Vec<&str> = left.iter().map(String::as_str).collect();
    let right_refs: Vec<&str> = right.iter().map(String::as_str).collect();
    let diff = SimilarTextDiff::from_slices(&left_refs, &right_refs);
    let mut hunks = Vec::new();
    let mut start_left = 0;
    let mut start_right = 0;
    let mut end_left = 0;
    let mut end_right = 0;
    let mut has_delete = false;
    let mut has_insert = false;
    let mut active = false;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                if active {
                    hunks.push(DiffHunk {
                        left_range: start_left..end_left,
                        right_range: start_right..end_right,
                        kind: if has_delete && has_insert {
                            HunkKind::Changed
                        } else if has_delete {
                            HunkKind::Removed
                        } else {
                            HunkKind::Added
                        },
                    });
                    active = false;
                    has_delete = false;
                    has_insert = false;
                }
            }
            ChangeTag::Delete => {
                if !active {
                    start_left = change.old_index().unwrap_or(end_left);
                    start_right = change.new_index().unwrap_or(end_right);
                    active = true;
                }
                end_left = change.old_index().unwrap_or(end_left) + 1;
                end_right = change.new_index().unwrap_or(end_right);
                has_delete = true;
            }
            ChangeTag::Insert => {
                if !active {
                    start_left = change.old_index().unwrap_or(end_left);
                    start_right = change.new_index().unwrap_or(end_right);
                    active = true;
                }
                end_left = change.old_index().unwrap_or(end_left);
                end_right = change.new_index().unwrap_or(end_right) + 1;
                has_insert = true;
            }
        }
    }
    if active {
        hunks.push(DiffHunk {
            left_range: start_left..end_left,
            right_range: start_right..end_right,
            kind: if has_delete && has_insert {
                HunkKind::Changed
            } else if has_delete {
                HunkKind::Removed
            } else {
                HunkKind::Added
            },
        });
    }
    hunks
}
