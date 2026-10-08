use super::{
    CompareError, CompareErrorKind, ComparisonProgress, DEFAULT_BUFFER_SIZE, ProgressStage,
    files_equal_with_ignores_cancellable,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Debug)]
pub struct DirectoryCompareOptions {
    pub buffer_size: usize,
    pub cancellation: Option<Arc<AtomicBool>>,
    /// Ignore inline Unicode whitespace in valid UTF-8 files at most 32 MiB.
    /// Binary, invalid UTF-8, and larger files are compared byte-for-byte.
    pub ignore_whitespace: bool,
    /// Normalize CR, LF, and CRLF (including a final line ending) in valid UTF-8
    /// files at most 32 MiB. Other files remain byte-compared.
    pub ignore_line_endings: bool,
    pub progress: Option<Arc<ComparisonProgress>>,
}
impl Default for DirectoryCompareOptions {
    fn default() -> Self {
        Self {
            buffer_size: DEFAULT_BUFFER_SIZE,
            cancellation: None,
            ignore_whitespace: false,
            ignore_line_endings: false,
            progress: None,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DirectoryEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DirectoryEntryState {
    Same,
    Different,
    LeftOnly,
    RightOnly,
    TypeMismatch,
    Error(CompareError),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryEntry {
    pub relative_path: PathBuf,
    pub left_exists: bool,
    pub right_exists: bool,
    pub left_kind: Option<DirectoryEntryKind>,
    pub right_kind: Option<DirectoryEntryKind>,
    /// The byte size on each side. Files use their metadata length and directories
    /// use the sum of all regular files below them. Symlinks are never followed.
    pub left_size: Option<u64>,
    pub right_size: Option<u64>,
    pub kind: DirectoryEntryKind,
    pub state: DirectoryEntryState,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryDiff {
    pub entries: Vec<DirectoryEntry>,
    pub cancelled: bool,
}

pub fn compare_directories(
    left: impl AsRef<Path>,
    right: impl AsRef<Path>,
    options: &DirectoryCompareOptions,
) -> Result<DirectoryDiff, CompareError> {
    let left = left.as_ref();
    let right = right.as_ref();
    validate_directory(left)?;
    validate_directory(right)?;
    begin_progress(options, ProgressStage::Scanning, None);
    let mut left_entries = collect(left, options)?;
    if cancelled(options) {
        return Ok(DirectoryDiff {
            entries: Vec::new(),
            cancelled: true,
        });
    }
    let mut right_entries = collect(right, options)?;
    if cancelled(options) {
        return Ok(DirectoryDiff {
            entries: Vec::new(),
            cancelled: true,
        });
    }
    let mut entries = Vec::new();
    let paths: Vec<PathBuf> = left_entries
        .keys()
        .chain(right_entries.keys())
        .cloned()
        .collect();
    let mut unique = std::collections::BTreeSet::new();
    for relative_path in paths {
        unique.insert(relative_path);
    }
    begin_progress(
        options,
        ProgressStage::ComparingFiles,
        Some(unique.len() as u64),
    );
    for relative_path in unique {
        if cancelled(options) {
            return Ok(DirectoryDiff {
                entries,
                cancelled: true,
            });
        }
        let left_item = left_entries.remove(&relative_path);
        let right_item = right_entries.remove(&relative_path);
        let left_exists = left_item.is_some();
        let right_exists = right_item.is_some();
        let left_kind = left_item
            .as_ref()
            .and_then(|item| item.as_ref().ok().map(|item| item.kind.clone()));
        let right_kind = right_item
            .as_ref()
            .and_then(|item| item.as_ref().ok().map(|item| item.kind.clone()));
        let left_size = left_item
            .as_ref()
            .and_then(|item| item.as_ref().ok().and_then(|item| item.size));
        let right_size = right_item
            .as_ref()
            .and_then(|item| item.as_ref().ok().and_then(|item| item.size));
        let (kind, state) = match (left_item, right_item) {
            (Some(Err(error)), _) | (_, Some(Err(error))) => {
                (DirectoryEntryKind::Other, DirectoryEntryState::Error(error))
            }
            (Some(Ok(left_item)), None) => (left_item.kind, DirectoryEntryState::LeftOnly),
            (None, Some(Ok(right_item))) => (right_item.kind, DirectoryEntryState::RightOnly),
            (Some(Ok(left_item)), Some(Ok(right_item))) if left_item.kind != right_item.kind => {
                (left_item.kind, DirectoryEntryState::TypeMismatch)
            }
            (
                Some(Ok(CollectedEntry {
                    kind: DirectoryEntryKind::File,
                    ..
                })),
                Some(Ok(_)),
            ) => {
                let state = match files_equal_with_ignores_cancellable(
                    left.join(&relative_path),
                    right.join(&relative_path),
                    options.buffer_size,
                    options.ignore_whitespace,
                    options.ignore_line_endings,
                    options.cancellation.as_deref(),
                ) {
                    Ok(Some(true)) => DirectoryEntryState::Same,
                    Ok(Some(false)) => DirectoryEntryState::Different,
                    Ok(None) => {
                        return Ok(DirectoryDiff {
                            entries,
                            cancelled: true,
                        });
                    }
                    Err(error) => DirectoryEntryState::Error(error),
                };
                (DirectoryEntryKind::File, state)
            }
            (
                Some(Ok(CollectedEntry {
                    kind: DirectoryEntryKind::Symlink,
                    ..
                })),
                Some(Ok(_)),
            ) => {
                let state = match (
                    fs::read_link(left.join(&relative_path)),
                    fs::read_link(right.join(&relative_path)),
                ) {
                    (Ok(a), Ok(b)) if a == b => DirectoryEntryState::Same,
                    (Ok(_), Ok(_)) => DirectoryEntryState::Different,
                    (Err(e), _) => {
                        DirectoryEntryState::Error(CompareError::io(left.join(&relative_path), e))
                    }
                    (_, Err(e)) => {
                        DirectoryEntryState::Error(CompareError::io(right.join(&relative_path), e))
                    }
                };
                (DirectoryEntryKind::Symlink, state)
            }
            (Some(Ok(item)), Some(Ok(_))) => (item.kind, DirectoryEntryState::Same),
            (None, None) => unreachable!(),
        };
        entries.push(DirectoryEntry {
            relative_path,
            left_exists,
            right_exists,
            left_kind,
            right_kind,
            left_size,
            right_size,
            kind,
            state,
        });
        advance_progress(options, 1);
    }
    begin_progress(options, ProgressStage::Finished, Some(entries.len() as u64));
    advance_progress(options, entries.len() as u64);
    Ok(DirectoryDiff {
        entries,
        cancelled: false,
    })
}
fn validate_directory(path: &Path) -> Result<(), CompareError> {
    let metadata = fs::metadata(path).map_err(|e| CompareError::io(path, e))?;
    if metadata.is_dir() {
        Ok(())
    } else {
        Err(CompareError {
            path: Some(path.to_path_buf()),
            kind: CompareErrorKind::NotDirectory,
            message: "path is not a directory".into(),
        })
    }
}
#[derive(Clone, Debug)]
struct CollectedEntry {
    kind: DirectoryEntryKind,
    size: Option<u64>,
}

fn collect(
    root: &Path,
    options: &DirectoryCompareOptions,
) -> Result<BTreeMap<PathBuf, Result<CollectedEntry, CompareError>>, CompareError> {
    let mut entries = BTreeMap::new();
    let mut todo = vec![PathBuf::new()];
    while let Some(relative) = todo.pop() {
        if cancelled(options) {
            break;
        }
        let path = root.join(&relative);
        let reader = match fs::read_dir(&path) {
            Ok(reader) => reader,
            Err(error) if relative.as_os_str().is_empty() => {
                return Err(CompareError::io(&path, error));
            }
            Err(error) => {
                entries.insert(relative.clone(), Err(CompareError::io(&path, error)));
                continue;
            }
        };
        for child in reader {
            if cancelled(options) {
                break;
            }
            match child {
                Err(error) => {
                    entries.insert(relative.clone(), Err(CompareError::io(&path, error)));
                }
                Ok(child) => {
                    let child_relative = relative.join(child.file_name());
                    match fs::symlink_metadata(child.path()) {
                        Err(error) => {
                            entries
                                .insert(child_relative, Err(CompareError::io(child.path(), error)));
                        }
                        Ok(metadata) => {
                            let kind = if metadata.file_type().is_symlink() {
                                DirectoryEntryKind::Symlink
                            } else if metadata.is_file() {
                                DirectoryEntryKind::File
                            } else if metadata.is_dir() {
                                DirectoryEntryKind::Directory
                            } else {
                                DirectoryEntryKind::Other
                            };
                            if kind == DirectoryEntryKind::Directory {
                                todo.push(child_relative.clone());
                            }
                            let size = match kind {
                                DirectoryEntryKind::File => Some(metadata.len()),
                                DirectoryEntryKind::Directory => None,
                                DirectoryEntryKind::Symlink | DirectoryEntryKind::Other => None,
                            };
                            entries.insert(child_relative, Ok(CollectedEntry { kind, size }));
                        }
                    }
                }
            }
            advance_progress(options, 1);
        }
    }
    calculate_directory_sizes(&mut entries, options);
    Ok(entries)
}

/// Calculate totals only when every descendant was scanned successfully. This
/// prevents an unreadable child from being shown as a deceptively partial total.
fn calculate_directory_sizes(
    entries: &mut BTreeMap<PathBuf, Result<CollectedEntry, CompareError>>,
    options: &DirectoryCompareOptions,
) {
    let mut totals = BTreeMap::new();
    let mut directories = Vec::new();
    for (path, entry) in entries.iter() {
        if cancelled(options) {
            return;
        }
        if matches!(
            entry,
            Ok(CollectedEntry {
                kind: DirectoryEntryKind::Directory,
                ..
            })
        ) {
            totals.insert(path.clone(), Some(0));
            directories.push(path.clone());
        }
    }
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));

    // Seed each directory with its direct regular-file sizes, and mark a parent
    // unknown as soon as one of its children could not be scanned.
    for (path, entry) in entries.iter() {
        if cancelled(options) {
            return;
        }
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        let Some(parent_total) = totals.get_mut(parent) else {
            continue;
        };
        match entry {
            Ok(CollectedEntry {
                kind: DirectoryEntryKind::File,
                size: Some(size),
            }) => add_to_total(parent_total, Some(*size)),
            Err(_) => *parent_total = None,
            Ok(_) => {}
        }
    }

    // Every child directory has already been finalized when it is added to its
    // parent, so this makes one post-order pass over the collected entries.
    for directory in directories {
        if cancelled(options) {
            return;
        }
        let total = totals.remove(&directory).unwrap_or(None);
        if let Some(Ok(entry)) = entries.get_mut(&directory) {
            entry.size = total;
        }
        let parent = directory.parent().unwrap_or_else(|| Path::new(""));
        if let Some(parent_total) = totals.get_mut(parent) {
            add_to_total(parent_total, total);
        }
    }
}

fn add_to_total(total: &mut Option<u64>, value: Option<u64>) {
    *total = total.and_then(|total| value.and_then(|value| total.checked_add(value)));
}
fn cancelled(options: &DirectoryCompareOptions) -> bool {
    options
        .cancellation
        .as_ref()
        .is_some_and(|value| value.load(Ordering::Relaxed))
}

fn begin_progress(options: &DirectoryCompareOptions, stage: ProgressStage, total: Option<u64>) {
    if let Some(progress) = &options.progress {
        progress.begin(stage, total);
    }
}

fn advance_progress(options: &DirectoryCompareOptions, amount: u64) {
    if let Some(progress) = &options.progress {
        progress.advance(amount);
    }
}
