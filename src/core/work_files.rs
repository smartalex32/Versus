//! Working copies for interactive merge.  This module deliberately does not
//! compare files: callers load only the file a user opened and save only after
//! an explicit request.

pub use super::lazy_directory::{
    LazyDirectoryEntry, LazyDirectoryItem, LazyDirectorySide, directory_entries_at,
    directory_entries_for_root,
};
use super::{CompareError, CompareErrorKind, DEFAULT_TEXT_SIZE_LIMIT};
use std::{
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

#[cfg(test)]
#[path = "../../tests/unit/core/work_files/mod.rs"]
mod tests;
