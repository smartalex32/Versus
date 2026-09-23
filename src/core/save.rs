use super::CompareError;
use std::{
    fs,
    io::{ErrorKind, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineEnding {
    Lf,
    Crlf,
}
#[derive(Clone, Debug)]
pub struct SaveOptions {
    pub overwrite: bool,
    pub line_ending: LineEnding,
}
impl Default for SaveOptions {
    fn default() -> Self {
        Self {
            overwrite: false,
            line_ending: LineEnding::Lf,
        }
    }
}
pub fn save_text_safely(
    path: impl AsRef<Path>,
    contents: &str,
    options: &SaveOptions,
) -> Result<(), CompareError> {
    let path = path.as_ref();
    if destination_exists(path)? && !options.overwrite {
        return Err(CompareError {
            path: Some(path.to_path_buf()),
            kind: super::CompareErrorKind::Io,
            message: "destination already exists; explicit overwrite is required".into(),
        });
    }
    let parent = path.parent().ok_or_else(|| CompareError {
        path: Some(path.to_path_buf()),
        kind: super::CompareErrorKind::InvalidPath,
        message: "destination has no parent directory".into(),
    })?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(".{name}.versus-{stamp}.tmp"));
    let backup = parent.join(format!(".{name}.versus-{stamp}.backup"));
    let rendered = match options.line_ending {
        LineEnding::Lf => contents.replace("\r\n", "\n"),
        LineEnding::Crlf => contents.replace("\r\n", "\n").replace('\n', "\r\n"),
    };
    let write_result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|e| CompareError::io(&temporary, e))?;
        file.write_all(rendered.as_bytes())
            .map_err(|e| CompareError::io(&temporary, e))?;
        file.sync_all()
            .map_err(|e| CompareError::io(&temporary, e))?;
        // `rename` cannot replace an existing file on Windows. Move the old file aside
        // first so a failed replacement can restore it instead of discarding it.
        let had_destination = destination_exists(path)?;
        if had_destination && !options.overwrite {
            return Err(CompareError {
                path: Some(path.to_path_buf()),
                kind: super::CompareErrorKind::Io,
                message: "destination appeared while saving; explicit overwrite is required".into(),
            });
        }
        if had_destination {
            fs::rename(path, &backup).map_err(|e| CompareError::io(path, e))?;
        }
        if let Err(error) = fs::rename(&temporary, path) {
            if had_destination {
                if let Err(restore_error) = fs::rename(&backup, path) {
                    return Err(CompareError {
                        path: Some(backup.clone()),
                        kind: super::CompareErrorKind::Io,
                        message: format!(
                            "failed to replace destination ({error}) and restore backup ({restore_error})"
                        ),
                    });
                }
            }
            return Err(CompareError::io(path, error));
        }
        if had_destination {
            // The new destination has already been written. A stale recovery backup is
            // preferable to reporting a failed save after that point.
            let _ = fs::remove_file(&backup);
        }
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

fn destination_exists(path: &Path) -> Result<bool, CompareError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(CompareError::io(path, error)),
    }
}
