use std::{fmt, io, path::PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompareErrorKind {
    Io,
    InvalidPath,
    NotDirectory,
    InvalidText,
    TooLarge,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompareError {
    pub path: Option<PathBuf>,
    pub kind: CompareErrorKind,
    pub message: String,
}

impl CompareError {
    pub fn io(path: impl Into<PathBuf>, error: io::Error) -> Self {
        Self {
            path: Some(path.into()),
            kind: CompareErrorKind::Io,
            message: error.to_string(),
        }
    }

    pub(crate) fn invalid_text(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(path.into()),
            kind: CompareErrorKind::InvalidText,
            message: "file is not valid UTF-8 text".into(),
        }
    }
}

impl fmt::Display for CompareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(path) => write!(f, "{}: {}", path.display(), self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for CompareError {}
