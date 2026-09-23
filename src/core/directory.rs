use super::{
    CompareError, CompareErrorKind, DEFAULT_BUFFER_SIZE, buffered_files_equal_cancellable,
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
}
impl Default for DirectoryCompareOptions {
    fn default() -> Self {
        Self {
            buffer_size: DEFAULT_BUFFER_SIZE,
            cancellation: None,
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
    for relative_path in unique {
        if cancelled(options) {
            return Ok(DirectoryDiff {
                entries,
                cancelled: true,
            });
        }
        let left_item = left_entries.remove(&relative_path);
        let right_item = right_entries.remove(&relative_path);
        let (kind, state) = match (left_item, right_item) {
            (Some(Err(error)), _) | (_, Some(Err(error))) => {
                (DirectoryEntryKind::Other, DirectoryEntryState::Error(error))
            }
            (Some(Ok(left_kind)), None) => (left_kind, DirectoryEntryState::LeftOnly),
            (None, Some(Ok(right_kind))) => (right_kind, DirectoryEntryState::RightOnly),
            (Some(Ok(left_kind)), Some(Ok(right_kind))) if left_kind != right_kind => {
                (left_kind, DirectoryEntryState::TypeMismatch)
            }
            (Some(Ok(kind @ DirectoryEntryKind::File)), Some(Ok(_))) => {
                let state = match buffered_files_equal_cancellable(
                    left.join(&relative_path),
                    right.join(&relative_path),
                    options.buffer_size,
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
                (kind, state)
            }
            (Some(Ok(kind @ DirectoryEntryKind::Symlink)), Some(Ok(_))) => {
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
                (kind, state)
            }
            (Some(Ok(kind)), Some(Ok(_))) => (kind, DirectoryEntryState::Same),
            (None, None) => unreachable!(),
        };
        entries.push(DirectoryEntry {
            relative_path,
            kind,
            state,
        });
    }
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
fn collect(
    root: &Path,
    options: &DirectoryCompareOptions,
) -> Result<BTreeMap<PathBuf, Result<DirectoryEntryKind, CompareError>>, CompareError> {
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
                            entries.insert(child_relative, Ok(kind));
                        }
                    }
                }
            }
        }
    }
    Ok(entries)
}
fn cancelled(options: &DirectoryCompareOptions) -> bool {
    options
        .cancellation
        .as_ref()
        .is_some_and(|value| value.load(Ordering::Relaxed))
}
