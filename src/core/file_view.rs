use super::{
    CompareError, CompareErrorKind, DEFAULT_BUFFER_SIZE, DEFAULT_TEXT_SIZE_LIMIT,
    DirectoryEntryState,
};
use similar::{ChangeTag, TextDiff as SimilarTextDiff};
use std::{
    fs,
    io::{BufReader, Read},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
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
    load_file_comparison_with_timeout(paths, cancellation, DIFF_TIMEOUT)
}

fn load_file_comparison_with_timeout(
    paths: &[Option<PathBuf>; 2],
    cancellation: &AtomicBool,
    timeout: Duration,
) -> Result<Option<FileComparison>, CompareError> {
    if cancelled(cancellation) {
        return Ok(None);
    }

    let left = read_optional_text(paths[0].as_deref(), cancellation)?;
    if cancelled(cancellation) {
        return Ok(None);
    }
    let right = read_optional_text(paths[1].as_deref(), cancellation)?;
    if cancelled(cancellation) {
        return Ok(None);
    }

    let (left, right) = match (left, right) {
        (OptionalText::Missing, OptionalText::Missing) => {
            return Ok(Some(FileComparison {
                rows: Vec::new(),
                message: Some("No file selected.".into()),
            }));
        }
        (OptionalText::Unavailable(message), _) | (_, OptionalText::Unavailable(message)) => {
            return Ok(Some(FileComparison {
                rows: Vec::new(),
                message: Some(message),
            }));
        }
        (OptionalText::Missing, OptionalText::Text(right)) => {
            return rows_for_one_side(right, false, cancellation);
        }
        (OptionalText::Text(left), OptionalText::Missing) => {
            return rows_for_one_side(left, true, cancellation);
        }
        (OptionalText::Text(left), OptionalText::Text(right)) => (left, right),
    };

    let left_lines = display_lines_for_view(&left);
    let right_lines = display_lines_for_view(&right);
    drop(left);
    drop(right);
    let left_refs: Vec<_> = left_lines.iter().map(String::as_str).collect();
    let right_refs: Vec<_> = right_lines.iter().map(String::as_str).collect();
    // A cancellation request cannot interrupt Similar while it computes a diff, so
    // give its worst-case work a finite bound before observing cancellation again.
    let diff_started = Instant::now();
    let diff = SimilarTextDiff::configure()
        .timeout(timeout)
        .diff_slices(&left_refs, &right_refs);
    if diff_started.elapsed() >= timeout {
        return Ok(Some(FileComparison {
            rows: Vec::new(),
            message: Some(DIFF_TIMEOUT_MESSAGE.into()),
        }));
    }
    if cancelled(cancellation) {
        return Ok(None);
    }
    let mut rows = Vec::new();
    let mut left_index = 0;
    let mut right_index = 0;
    let mut pending_left = Vec::new();
    let mut pending_right = Vec::new();
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
                ) {
                    return Ok(None);
                }
                rows.push(FileComparisonRow {
                    left: Some((left_index + 1, left_lines[left_index].clone())),
                    right: Some((right_index + 1, right_lines[right_index].clone())),
                    state: DirectoryEntryState::Same,
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
    ) {
        return Ok(None);
    }
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
    let bytes = match read_bytes_cancellable(path, metadata.len(), cancellation)? {
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
    }
}

fn rows_for_one_side(
    text: String,
    left_side: bool,
    cancellation: &AtomicBool,
) -> Result<Option<FileComparison>, CompareError> {
    let mut rows = Vec::new();
    for (index, line) in display_lines_for_view(&text).into_iter().enumerate() {
        if cancelled(cancellation) {
            return Ok(None);
        }
        let line = Some((index + 1, line));
        rows.push(FileComparisonRow {
            left: left_side.then(|| line.clone()).flatten(),
            right: (!left_side).then_some(line).flatten(),
            state: if left_side {
                DirectoryEntryState::LeftOnly
            } else {
                DirectoryEntryState::RightOnly
            },
        });
    }
    let message = rows.is_empty().then(|| {
        format!(
            "Only the {} file is available; it is empty.",
            if left_side { "left" } else { "right" }
        )
    });
    Ok(Some(FileComparison { rows, message }))
}

fn append_changed_rows(
    rows: &mut Vec<FileComparisonRow>,
    left: impl Iterator<Item = (usize, String)>,
    right: impl Iterator<Item = (usize, String)>,
    cancellation: &AtomicBool,
) -> bool {
    let mut left = left;
    let mut right = right;
    loop {
        if cancelled(cancellation) {
            return false;
        }
        match (left.next(), right.next()) {
            (Some(left), Some(right)) => rows.push(FileComparisonRow {
                left: Some(left),
                right: Some(right),
                state: DirectoryEntryState::Different,
            }),
            (Some(left), None) => rows.push(FileComparisonRow {
                left: Some(left),
                right: None,
                state: DirectoryEntryState::LeftOnly,
            }),
            (None, Some(right)) => rows.push(FileComparisonRow {
                left: None,
                right: Some(right),
                state: DirectoryEntryState::RightOnly,
            }),
            (None, None) => break,
        }
    }
    true
}

fn display_lines_for_view(text: &str) -> Vec<String> {
    super::display_lines(&text.replace("\r\n", "\n").replace('\r', "\n"))
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
