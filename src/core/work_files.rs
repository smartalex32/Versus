//! Working copies for interactive merge.  This module deliberately does not
//! compare files: callers load only the file a user opened and save only after
//! an explicit request.

use super::{CompareError, CompareErrorKind, DEFAULT_TEXT_SIZE_LIMIT, DirectoryEntryKind};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs,
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Returns a filesystem-free identity for a working-copy path.  It removes
/// lexical `.`/`..` aliases without resolving symlinks or reading metadata, so
/// it is safe to use while Merge Mode intentionally has no diff work running.
pub fn working_file_key(path: &Path) -> PathBuf {
    let mut key = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => key.push(key_component(prefix.as_os_str())),
            std::path::Component::RootDir => key.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                let can_pop_normal = matches!(
                    key.components().next_back(),
                    Some(std::path::Component::Normal(_))
                );
                if can_pop_normal {
                    key.pop();
                } else if !key.is_absolute() {
                    key.push("..");
                }
            }
            std::path::Component::Normal(part) => key.push(key_component(part)),
        }
    }
    if key.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        key
    }
}

#[cfg(windows)]
fn key_component(value: &OsStr) -> OsString {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    OsString::from_wide(
        &value
            .encode_wide()
            .map(|unit| {
                if unit <= 0x7f {
                    (unit as u8 as char).to_ascii_lowercase() as u16
                } else {
                    unit
                }
            })
            .collect::<Vec<_>>(),
    )
}
#[cfg(not(windows))]
fn key_component(value: &OsStr) -> OsString {
    value.to_os_string()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextEncoding {
    Utf8 { bom: bool },
    Utf16Le,
    Utf16Be,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileFingerprint {
    Missing,
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkingFile {
    pub path: Option<PathBuf>,
    /// Physical target identity captured by the background loader.  `path`
    /// retains the native spelling selected by the user for saving.
    pub identity: Option<PathBuf>,
    pub text: String,
    pub encoding: TextEncoding,
    pub fingerprint: FileFingerprint,
}

impl WorkingFile {
    pub fn empty(path: Option<PathBuf>) -> Self {
        Self {
            path,
            identity: None,
            text: String::new(),
            encoding: TextEncoding::Utf8 { bom: false },
            fingerprint: FileFingerprint::Missing,
        }
    }

    pub fn is_dirty(&self) -> bool {
        match &self.fingerprint {
            FileFingerprint::Missing => !self.text.is_empty(),
            FileFingerprint::Bytes(original) => {
                encode_text(&self.text, self.encoding).map_or(true, |bytes| &bytes != original)
            }
        }
    }

    /// Original decoded content, used to retain dirty state for staged files.
    pub fn baseline_text(&self) -> Result<String, CompareError> {
        match &self.fingerprint {
            FileFingerprint::Missing => Ok(String::new()),
            FileFingerprint::Bytes(bytes) => {
                decode_text(self.path.as_deref().unwrap_or(Path::new("")), bytes)
                    .map(|(text, _)| text)
            }
        }
    }

    pub fn set_path(&mut self, path: PathBuf) {
        self.path = Some(path);
    }

    pub fn identity_key(&self) -> PathBuf {
        self.identity.clone().unwrap_or_else(|| {
            self.path
                .as_deref()
                .map(working_file_key)
                .unwrap_or_else(|| PathBuf::from("."))
        })
    }
}

/// Resolves a physical identity off the UI thread.  Existing inputs are
/// canonicalized; for a missing destination its nearest existing parent is
/// canonicalized and the remaining lexical components are appended.
pub fn resolve_working_file_identity(path: &Path) -> Result<PathBuf, CompareError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(CompareError {
                    path: Some(path.to_path_buf()),
                    kind: CompareErrorKind::InvalidPath,
                    message: "working file identity requires a regular, non-symlink file".into(),
                });
            }
            return fs::canonicalize(path).map_err(|error| CompareError::io(path, error));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(CompareError::io(path, error)),
    }
    let ancestors: Vec<&Path> = path.ancestors().collect();
    for (index, ancestor) in ancestors.iter().enumerate() {
        match fs::canonicalize(ancestor) {
            Ok(parent) => {
                let mut identity = parent;
                for component in ancestors[..index]
                    .iter()
                    .rev()
                    .flat_map(|part| part.file_name())
                {
                    identity.push(component);
                }
                return Ok(working_file_key(&identity));
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(CompareError::io(ancestor, error)),
        }
    }
    Err(CompareError {
        path: Some(path.to_path_buf()),
        kind: CompareErrorKind::InvalidPath,
        message: "path has no canonicalizable parent".into(),
    })
}

pub fn load_working_file(path: Option<&Path>) -> Result<WorkingFile, CompareError> {
    let Some(path) = path else {
        return Ok(WorkingFile::empty(None));
    };
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let mut file = WorkingFile::empty(Some(path.to_path_buf()));
            file.identity = Some(resolve_working_file_identity(path)?);
            return Ok(file);
        }
        Err(error) => return Err(CompareError::io(path, error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CompareError {
            path: Some(path.to_path_buf()),
            kind: CompareErrorKind::InvalidPath,
            message: "working copies require a regular, non-symlink file".into(),
        });
    }
    if metadata.len() > DEFAULT_TEXT_SIZE_LIMIT {
        return Err(CompareError {
            path: Some(path.to_path_buf()),
            kind: CompareErrorKind::TooLarge,
            message: format!("file exceeds the text limit of {DEFAULT_TEXT_SIZE_LIMIT} bytes"),
        });
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(path)
        .map_err(|e| CompareError::io(path, e))?
        .take(DEFAULT_TEXT_SIZE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| CompareError::io(path, e))?;
    if bytes.len() as u64 > DEFAULT_TEXT_SIZE_LIMIT {
        return Err(CompareError {
            path: Some(path.to_path_buf()),
            kind: CompareErrorKind::TooLarge,
            message: "file grew beyond the text size limit while loading".into(),
        });
    }
    let (text, encoding) = decode_text(path, &bytes)?;
    Ok(WorkingFile {
        path: Some(path.to_path_buf()),
        identity: Some(resolve_working_file_identity(path)?),
        text,
        encoding,
        fingerprint: FileFingerprint::Bytes(bytes),
    })
}

/// Saves a working copy only if the original bytes still match the disk file.
/// This protects a user's external edits without relying on timestamps.
pub fn save_working_file(file: &mut WorkingFile) -> Result<(), CompareError> {
    let path = file.path.as_ref().ok_or_else(|| CompareError {
        path: None,
        kind: CompareErrorKind::InvalidPath,
        message: "choose a destination before saving this missing file".into(),
    })?;
    let current_identity = resolve_working_file_identity(path)?;
    if file
        .identity
        .as_ref()
        .is_some_and(|identity| identity != &current_identity)
    {
        return Err(CompareError {
            path: Some(path.clone()),
            kind: CompareErrorKind::Io,
            message: "save destination identity changed outside Versus; reload before saving"
                .into(),
        });
    }
    let unchanged = match (&file.fingerprint, read_current(path)?) {
        (FileFingerprint::Missing, FileFingerprint::Missing) => true,
        (FileFingerprint::Bytes(expected), FileFingerprint::Bytes(actual)) => expected == &actual,
        _ => false,
    };
    if !unchanged {
        return Err(CompareError {
            path: Some(path.clone()),
            kind: CompareErrorKind::Io,
            message: "file changed outside Versus; reload or save to a new destination".into(),
        });
    }
    let bytes = encode_text(&file.text, file.encoding)?;
    write_bytes_safely(path, &bytes, &file.fingerprint)?;
    file.fingerprint = FileFingerprint::Bytes(bytes);
    file.identity = Some(resolve_working_file_identity(path)?);
    Ok(())
}

fn read_current(path: &Path) -> Result<FileFingerprint, CompareError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(CompareError {
                path: Some(path.to_path_buf()),
                kind: CompareErrorKind::InvalidPath,
                message: "save destination is not a regular, non-symlink file".into(),
            })
        }
        Ok(metadata) if metadata.len() > DEFAULT_TEXT_SIZE_LIMIT => Err(CompareError {
            path: Some(path.to_path_buf()),
            kind: CompareErrorKind::TooLarge,
            message: format!("file exceeds the text limit of {DEFAULT_TEXT_SIZE_LIMIT} bytes"),
        }),
        Ok(_) => fs::read(path)
            .map(FileFingerprint::Bytes)
            .map_err(|error| CompareError::io(path, error)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(FileFingerprint::Missing),
        Err(error) => Err(CompareError::io(path, error)),
    }
}

fn decode_text(path: &Path, bytes: &[u8]) -> Result<(String, TextEncoding), CompareError> {
    if let Some(body) = bytes.strip_prefix(&[0xff, 0xfe]) {
        return decode_utf16(path, body, true).map(|text| (text, TextEncoding::Utf16Le));
    }
    if let Some(body) = bytes.strip_prefix(&[0xfe, 0xff]) {
        return decode_utf16(path, body, false).map(|text| (text, TextEncoding::Utf16Be));
    }
    let (body, bom) = match bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        Some(body) => (body, true),
        None => (bytes, false),
    };
    if body.contains(&0) {
        return Err(CompareError::invalid_text(path));
    }
    String::from_utf8(body.to_vec())
        .map(|text| (text, TextEncoding::Utf8 { bom }))
        .map_err(|_| CompareError::invalid_text(path))
}

fn decode_utf16(path: &Path, bytes: &[u8], little: bool) -> Result<String, CompareError> {
    if bytes.len() % 2 != 0 {
        return Err(CompareError::invalid_text(path));
    }
    let words: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| {
            if little {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        })
        .collect();
    String::from_utf16(&words).map_err(|_| CompareError::invalid_text(path))
}

fn encode_text(text: &str, encoding: TextEncoding) -> Result<Vec<u8>, CompareError> {
    let mut bytes = Vec::new();
    match encoding {
        TextEncoding::Utf8 { bom } => {
            if bom {
                bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
            }
            bytes.extend_from_slice(text.as_bytes());
        }
        TextEncoding::Utf16Le => {
            bytes.extend_from_slice(&[0xff, 0xfe]);
            for word in text.encode_utf16() {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
        }
        TextEncoding::Utf16Be => {
            bytes.extend_from_slice(&[0xfe, 0xff]);
            for word in text.encode_utf16() {
                bytes.extend_from_slice(&word.to_be_bytes());
            }
        }
    }
    Ok(bytes)
}

fn write_bytes_safely(
    path: &Path,
    bytes: &[u8],
    expected: &FileFingerprint,
) -> Result<(), CompareError> {
    let parent = path.parent().ok_or_else(|| CompareError {
        path: Some(path.to_path_buf()),
        kind: CompareErrorKind::InvalidPath,
        message: "destination has no parent directory".into(),
    })?;
    ensure_directory_chain(parent)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temporary = parent.join(format!(".{name}.versus-{stamp}.tmp"));
    // Reserve a unique directory rather than a backup filename.  That keeps a
    // concurrent save from replacing somebody else's recovery copy between a
    // check and rename.
    let backup_directory = parent.join(format!(".{name}.versus-{stamp}.backup"));
    let backup = backup_directory.join("original");
    let result = (|| {
        fs::create_dir(&backup_directory)
            .map_err(|error| CompareError::io(&backup_directory, error))?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| CompareError::io(&temporary, error))?;
        output
            .write_all(bytes)
            .map_err(|error| CompareError::io(&temporary, error))?;
        output
            .sync_all()
            .map_err(|error| CompareError::io(&temporary, error))?;
        let existing = matches!(expected, FileFingerprint::Bytes(_));
        // Check again immediately before moving the old file aside.  After it is
        // moved we validate the backup bytes before allowing replacement.
        if read_current(path)? != *expected {
            return Err(CompareError {
                path: Some(path.to_path_buf()),
                kind: CompareErrorKind::Io,
                message: "file changed outside Versus while saving".into(),
            });
        }
        let permissions = if existing {
            Some(
                fs::metadata(path)
                    .map_err(|error| CompareError::io(path, error))?
                    .permissions(),
            )
        } else {
            None
        };
        if let Some(permissions) = permissions.as_ref() {
            // Applying the inherited mode before replacement avoids a window
            // where a replacement has default permissions after the old file
            // was already removed.
            fs::set_permissions(&temporary, permissions.clone())
                .map_err(|error| CompareError::io(&temporary, error))?;
        }
        if existing {
            fs::rename(path, &backup).map_err(|error| CompareError::io(path, error))?;
            if read_current(&backup)? != *expected {
                let _ = fs::rename(&backup, path);
                return Err(CompareError {
                    path: Some(path.to_path_buf()),
                    kind: CompareErrorKind::Io,
                    message: "file changed outside Versus while saving".into(),
                });
            }
        }
        if let Err(error) = fs::rename(&temporary, path) {
            if existing {
                let _ = fs::rename(&backup, path);
            }
            return Err(CompareError::io(path, error));
        }
        if existing {
            let _ = fs::remove_file(backup);
        }
        let _ = fs::remove_dir(&backup_directory);
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
        // Keep a recovery backup if restoration failed.  It is safer to leave
        // a uniquely named directory than to remove another writer's data.
    }
    result
}

fn ensure_directory_chain(parent: &Path) -> Result<(), CompareError> {
    let ancestors: Vec<&Path> = parent.ancestors().collect();
    // Anchor at the nearest existing component.  Checking every OS-provided
    // ancestor would reject normal macOS paths below `/var` (a system alias),
    // while this still refuses a selected symlink destination or any symlink
    // introduced below the chosen existing directory.
    let existing = ancestors
        .iter()
        .position(|directory| {
            !directory.as_os_str().is_empty() && fs::symlink_metadata(directory).is_ok()
        })
        .ok_or_else(|| CompareError {
            path: Some(parent.to_path_buf()),
            kind: CompareErrorKind::InvalidPath,
            message: "save destination has no existing directory ancestor".into(),
        })?;
    let anchor = ancestors[existing];
    let metadata = fs::symlink_metadata(anchor).map_err(|error| CompareError::io(anchor, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CompareError {
            path: Some(anchor.to_path_buf()),
            kind: CompareErrorKind::InvalidPath,
            message: "save destination parent contains a non-directory or symlink".into(),
        });
    }
    for directory in ancestors[..existing].iter().rev() {
        match fs::symlink_metadata(directory) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(CompareError {
                    path: Some(directory.to_path_buf()),
                    kind: CompareErrorKind::InvalidPath,
                    message: "save destination parent contains a non-directory or symlink".into(),
                });
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {
                fs::create_dir(directory).map_err(|error| CompareError::io(directory, error))?
            }
            Err(error) => return Err(CompareError::io(directory, error)),
        }
    }
    Ok(())
}

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

/// Lists just one directory level.  It reads metadata only and never hashes or
/// opens file contents; symlinks are returned as non-navigable entries.
pub fn directory_entries_at(
    left_root: &Path,
    right_root: &Path,
    relative: &Path,
) -> Result<Vec<LazyDirectoryEntry>, CompareError> {
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
        // Validate every component rather than only the final path.  A
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn temp(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("versus-work-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn utf16_and_external_changes_are_preserved_and_detected() {
        let root = temp("utf16");
        let file = root.join("a.txt");
        fs::write(&file, [0xff, 0xfe, b'h', 0, b'i', 0, b'\r', 0, b'\n', 0]).unwrap();
        let mut working = load_working_file(Some(&file)).unwrap();
        assert_eq!(working.text, "hi\r\n");
        working.text.push('!');
        save_working_file(&mut working).unwrap();
        assert_eq!(&fs::read(&file).unwrap()[..2], &[0xff, 0xfe]);
        fs::write(&file, "outside").unwrap();
        working.text.push('?');
        assert!(save_working_file(&mut working).is_err());
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn directory_listing_is_one_level_and_metadata_only() {
        let root = temp("listing");
        let left = root.join("left");
        let right = root.join("right");
        fs::create_dir_all(left.join("nested")).unwrap();
        fs::create_dir_all(&right).unwrap();
        fs::write(left.join("nested/secret"), b"x").unwrap();
        fs::write(left.join("a"), b"left").unwrap();
        fs::write(right.join("b"), b"right").unwrap();
        let entries = directory_entries_at(&left, &right, Path::new("")).unwrap();
        assert_eq!(entries.len(), 3);
        assert!(
            entries
                .iter()
                .all(|entry| entry.relative_path.components().count() == 1)
        );
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn one_sided_directory_remains_browsable_when_peer_is_regular_file() {
        let root = temp("type-mismatch");
        let left = root.join("left");
        let right = root.join("right");
        fs::create_dir_all(&left).unwrap();
        fs::create_dir_all(right.join("node")).unwrap();
        fs::write(left.join("node"), b"not a directory").unwrap();
        fs::write(right.join("node/child"), b"child").unwrap();
        let entries = directory_entries_at(&left, &right, Path::new("node")).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].left.is_none());
        assert_eq!(
            entries[0].right.as_ref().unwrap().kind,
            DirectoryEntryKind::File
        );
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn missing_save_parent_is_created_without_losing_the_destination() {
        let root = temp("nested-save");
        let destination = root.join("new/nested/file.txt");
        let mut file = load_working_file(Some(destination.as_path())).unwrap();
        file.text = "created\n".into();
        save_working_file(&mut file).unwrap();
        assert_eq!(fs::read_to_string(destination).unwrap(), "created\n");
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn working_file_key_is_lexical_and_does_not_require_the_path_to_exist() {
        assert_eq!(
            working_file_key(Path::new("missing/./tree/../file.txt")),
            PathBuf::from("missing/file.txt")
        );
        assert_eq!(
            working_file_key(Path::new("../../missing")),
            PathBuf::from("../../missing")
        );
    }
    #[cfg(unix)]
    #[test]
    fn save_rejects_a_parent_alias_that_changes_physical_destination() {
        use std::os::unix::fs::symlink;
        let root = temp("changed-parent");
        let first = root.join("first");
        let second = root.join("second");
        let link = root.join("link");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join("file.txt"), "first").unwrap();
        fs::write(second.join("file.txt"), "second").unwrap();
        symlink(&first, &link).unwrap();
        let alias = link.join("file.txt");
        let mut file = load_working_file(Some(alias.as_path())).unwrap();
        fs::remove_file(&link).unwrap();
        symlink(&second, &link).unwrap();
        file.text = "edited".into();
        assert!(save_working_file(&mut file).is_err());
        assert_eq!(
            fs::read_to_string(second.join("file.txt")).unwrap(),
            "second"
        );
        let _ = fs::remove_dir_all(root);
    }
    #[cfg(unix)]
    #[test]
    fn symlink_relative_parent_is_never_followed_when_listing_children() {
        use std::os::unix::fs::symlink;
        let root = temp("symlink-parent");
        let left = root.join("left");
        let right = root.join("right");
        let external = root.join("external");
        fs::create_dir_all(external.join("deeper")).unwrap();
        fs::create_dir_all(right.join("shortcut/deeper")).unwrap();
        fs::create_dir_all(&left).unwrap();
        fs::write(external.join("deeper/secret"), b"must not list").unwrap();
        fs::write(right.join("shortcut/deeper/peer"), b"visible peer").unwrap();
        symlink(&external, left.join("shortcut")).unwrap();
        let entries = directory_entries_at(&left, &right, Path::new("shortcut/deeper")).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].relative_path,
            PathBuf::from("shortcut/deeper/peer")
        );
        assert!(entries[0].left.is_none());
        assert_eq!(
            entries[0].right.as_ref().unwrap().kind,
            DirectoryEntryKind::File
        );
        let _ = fs::remove_dir_all(root);
    }
}
