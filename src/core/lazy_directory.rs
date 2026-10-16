//! Lazy, metadata-only directory listings for folder merge navigation.

use super::{CompareError, CompareErrorKind, DirectoryEntryKind};
use std::{
    collections::BTreeMap,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LazyDirectorySide {
    pub kind: DirectoryEntryKind,
    pub size: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LazyDirectoryEntry {
    pub relative_path: PathBuf,
    pub left: Option<LazyDirectorySide>,
    pub right: Option<LazyDirectorySide>,
}

/// Metadata for one child in a single directory tree. This is deliberately
/// separate from paired directory entries so a merge workspace can browse one
/// selected source without inventing a second root or reading its contents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LazyDirectoryItem {
    pub relative_path: PathBuf,
    pub side: LazyDirectorySide,
}

/// Lists one directory level below a single root. This reads names and file
/// metadata only; it does not hash or open file contents and rejects symlink
/// roots or ancestors.
pub fn directory_entries_for_root(
    root: &Path,
    relative: &Path,
) -> Result<Vec<LazyDirectoryItem>, CompareError> {
    validate_relative_directory_path(relative)?;
    validate_directory_root(root)?;
    let mut entries: Vec<_> = list_child_directory(root, relative)?
        .into_iter()
        .map(|(name, side)| LazyDirectoryItem {
            relative_path: relative.join(name),
            side,
        })
        .collect();
    entries.sort_by(|left, right| {
        (right.side.kind == DirectoryEntryKind::Directory)
            .cmp(&(left.side.kind == DirectoryEntryKind::Directory))
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });
    Ok(entries)
}

/// Lists just one directory level. It reads metadata only and never hashes or
/// opens file contents; symlinks are returned as non-navigable entries.
pub fn directory_entries_at(
    left_root: &Path,
    right_root: &Path,
    relative: &Path,
) -> Result<Vec<LazyDirectoryEntry>, CompareError> {
    validate_relative_directory_path(relative)?;
    validate_directory_root(left_root)?;
    validate_directory_root(right_root)?;
    let left = list_child_directory(left_root, relative)?;
    let right = list_child_directory(right_root, relative)?;
    let mut names = BTreeMap::<
        std::ffi::OsString,
        (Option<LazyDirectorySide>, Option<LazyDirectorySide>),
    >::new();
    for (name, side) in left {
        names.entry(name).or_insert((None, None)).0 = Some(side);
    }
    for (name, side) in right {
        names.entry(name).or_insert((None, None)).1 = Some(side);
    }
    let mut entries: Vec<_> = names
        .into_iter()
        .map(|(name, (left, right))| LazyDirectoryEntry {
            relative_path: relative.join(name),
            left,
            right,
        })
        .collect();
    entries.sort_by(|left, right| {
        let left_is_directory = left
            .left
            .as_ref()
            .is_some_and(|side| side.kind == DirectoryEntryKind::Directory)
            || left
                .right
                .as_ref()
                .is_some_and(|side| side.kind == DirectoryEntryKind::Directory);
        let right_is_directory = right
            .left
            .as_ref()
            .is_some_and(|side| side.kind == DirectoryEntryKind::Directory)
            || right
                .right
                .as_ref()
                .is_some_and(|side| side.kind == DirectoryEntryKind::Directory);
        right_is_directory
            .cmp(&left_is_directory)
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });
    Ok(entries)
}

fn validate_relative_directory_path(relative: &Path) -> Result<(), CompareError> {
    if relative.is_absolute()
        || relative.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(CompareError {
            path: Some(relative.to_path_buf()),
            kind: CompareErrorKind::InvalidPath,
            message: "directory relative path must stay below its root".into(),
        });
    }
    Ok(())
}

fn validate_directory_root(path: &Path) -> Result<(), CompareError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| CompareError::io(path, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CompareError {
            path: Some(path.to_path_buf()),
            kind: CompareErrorKind::NotDirectory,
            message: "directory navigation requires a non-symlink directory root".into(),
        });
    }
    Ok(())
}

fn list_child_directory(
    root: &Path,
    relative: &Path,
) -> Result<Vec<(std::ffi::OsString, LazyDirectorySide)>, CompareError> {
    let mut path = root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(part) = component else {
            continue;
        };
        path.push(part);
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(CompareError::io(&path, error)),
        };
        // Validate every component rather than only the final path. A
        // `link/child` path must never make directory browsing follow `link`.
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Ok(Vec::new());
        }
    }
    fs::read_dir(&path)
        .map_err(|error| CompareError::io(&path, error))?
        .map(|entry| {
            let entry = entry.map_err(|error| CompareError::io(&path, error))?;
            let meta = fs::symlink_metadata(entry.path())
                .map_err(|error| CompareError::io(entry.path(), error))?;
            let kind = if meta.file_type().is_symlink() {
                DirectoryEntryKind::Symlink
            } else if meta.is_dir() {
                DirectoryEntryKind::Directory
            } else if meta.is_file() {
                DirectoryEntryKind::File
            } else {
                DirectoryEntryKind::Other
            };
            Ok((
                entry.file_name(),
                LazyDirectorySide {
                    size: (kind == DirectoryEntryKind::File).then_some(meta.len()),
                    kind,
                },
            ))
        })
        .collect()
}
