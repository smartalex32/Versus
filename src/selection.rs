//! Native source selection and type classification for a comparison side.

use std::path::{Path, PathBuf};

/// The kinds of sources that the comparison workspace understands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceKind {
    File,
    Folder,
    Other,
}

/// The comparison workflow selected by the header buttons.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComparisonMode {
    Folder,
    File,
}

impl ComparisonMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Folder => "Folder Diff",
            Self::File => "File Diff",
        }
    }
    pub fn workspace_label(self, merge: bool) -> &'static str {
        if merge {
            match self {
                Self::Folder => "Folder Merge",
                Self::File => "File Merge",
            }
        } else {
            self.label()
        }
    }

    pub fn source_label(self) -> &'static str {
        match self {
            Self::Folder => "folder",
            Self::File => "file",
        }
    }
}

/// The workspace that can be rendered for the currently selected sources.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionMode {
    Empty,
    File,
    Folder,
    Incompatible,
}

/// Select the appropriate comparison workspace from the two source kinds.
///
/// A single regular file or folder opens its corresponding workspace so that a
/// second source can be selected there. Any special file, symbolic link, or
/// mixed file/folder pair has no valid comparison workspace.
pub fn selection_mode(kinds: [Option<SourceKind>; 2]) -> SelectionMode {
    match kinds {
        [None, None] => SelectionMode::Empty,
        [Some(SourceKind::File), None] | [None, Some(SourceKind::File)] => SelectionMode::File,
        [Some(SourceKind::Folder), None] | [None, Some(SourceKind::Folder)] => {
            SelectionMode::Folder
        }
        [Some(SourceKind::File), Some(SourceKind::File)] => SelectionMode::File,
        [Some(SourceKind::Folder), Some(SourceKind::Folder)] => SelectionMode::Folder,
        _ => SelectionMode::Incompatible,
    }
}

/// Classify a source without following symbolic links.
///
/// This is intentionally suitable for a background task: caller code must not
/// perform this filesystem operation while rendering a frame.
pub fn classify_path(path: &Path) -> Result<SourceKind, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("Cannot inspect {}: {error}", path.display()))?;
    let file_type = metadata.file_type();

    if file_type.is_symlink() {
        Ok(SourceKind::Other)
    } else if file_type.is_file() {
        Ok(SourceKind::File)
    } else if file_type.is_dir() {
        Ok(SourceKind::Folder)
    } else {
        Ok(SourceKind::Other)
    }
}

/// The directory the next native dialog should display first.
pub fn initial_directory(current: &str, current_kind: Option<SourceKind>) -> Option<PathBuf> {
    if current.is_empty() {
        return None;
    }

    let path = Path::new(current);
    match current_kind {
        Some(SourceKind::Folder) => Some(path.to_path_buf()),
        Some(SourceKind::File) => path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf),
        Some(SourceKind::Other) | None => None,
    }
}

fn initial_file_name(current: &str, current_kind: Option<SourceKind>) -> Option<String> {
    (current_kind == Some(SourceKind::File))
        .then(|| Path::new(current).file_name())
        .flatten()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
}

fn configured_dialog(
    side: usize,
    current: &str,
    current_kind: Option<SourceKind>,
    mode: ComparisonMode,
) -> rfd::FileDialog {
    let source_name = if side == 0 { "left" } else { "right" };
    let mut dialog =
        rfd::FileDialog::new().set_title(format!("Choose {source_name} {}", mode.source_label()));

    if let Some(directory) = initial_directory(current, current_kind) {
        dialog = dialog.set_directory(directory);
    }
    if let Some(file_name) = initial_file_name(current, current_kind) {
        dialog = dialog.set_file_name(file_name);
    }
    dialog
}

/// Open the selected workflow's native picker directly on every platform.
pub fn pick_path(
    side: usize,
    current: &str,
    current_kind: Option<SourceKind>,
    mode: ComparisonMode,
) -> Option<PathBuf> {
    let dialog = configured_dialog(side, current, current_kind, mode);
    match mode {
        ComparisonMode::Folder => dialog.pick_folder(),
        ComparisonMode::File => dialog.pick_file(),
    }
}

#[cfg(test)]
mod tests;
