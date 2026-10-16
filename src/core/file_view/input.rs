//! Bounded, cancellable file input and decoding for file comparisons.

use super::{FileViewOptions, cancelled};
use crate::core::{CompareError, CompareErrorKind, DEFAULT_BUFFER_SIZE, DEFAULT_TEXT_SIZE_LIMIT};
use std::{
    fs,
    io::{BufReader, Read},
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

pub(super) enum OptionalText {
    Missing,
    Text(String),
    Unavailable(String),
}

pub(super) fn read_optional_text(
    path: Option<&Path>,
    cancellation: &AtomicBool,
    progress: Option<&Arc<super::super::ComparisonProgress>>,
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
        return Ok(OptionalText::Unavailable(too_large_message(path)));
    }
    let bytes = match read_bytes_cancellable(path, metadata.len(), cancellation, progress)? {
        ReadBytes::Cancelled => return Ok(OptionalText::Missing),
        ReadBytes::TooLarge => return Ok(OptionalText::Unavailable(too_large_message(path))),
        ReadBytes::Bytes(bytes) => bytes,
    };
    match super::super::decode_text(path, &bytes) {
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
    progress: Option<&Arc<super::super::ComparisonProgress>>,
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

pub(super) fn begin_reading_progress(paths: &[Option<PathBuf>; 2], options: &FileViewOptions) {
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
    progress.begin(super::super::ProgressStage::Reading, Some(total));
}

fn too_large_message(path: &Path) -> String {
    format!(
        "{} exceeds the {} MiB text display limit.",
        path.display(),
        DEFAULT_TEXT_SIZE_LIMIT / (1024 * 1024)
    )
}
