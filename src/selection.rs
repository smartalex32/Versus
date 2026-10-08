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
            Self::Folder => "Folder Compare",
            Self::File => "File Compare",
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
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_directory() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("versus-selection-{unique}"));
        fs::create_dir_all(&directory).expect("create test directory");
        directory
    }

    #[test]
    fn classifies_regular_files_folders_missing_paths_and_symlinks() {
        let directory = test_directory();
        let file = directory.join("example.txt");
        fs::write(&file, "contents").expect("write test file");

        assert_eq!(classify_path(&directory), Ok(SourceKind::Folder));
        assert_eq!(classify_path(&file), Ok(SourceKind::File));
        assert!(classify_path(&directory.join("missing")).is_err());

        #[cfg(unix)]
        {
            let link = directory.join("example-link");
            std::os::unix::fs::symlink(&file, &link).expect("create test symlink");
            assert_eq!(classify_path(&link), Ok(SourceKind::Other));
        }

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn chooses_empty_single_matching_and_incompatible_modes() {
        assert_eq!(selection_mode([None, None]), SelectionMode::Empty);
        assert_eq!(
            selection_mode([Some(SourceKind::File), None]),
            SelectionMode::File
        );
        assert_eq!(
            selection_mode([None, Some(SourceKind::Folder)]),
            SelectionMode::Folder
        );
        assert_eq!(
            selection_mode([Some(SourceKind::File), Some(SourceKind::File)]),
            SelectionMode::File
        );
        assert_eq!(
            selection_mode([Some(SourceKind::Folder), Some(SourceKind::Folder)]),
            SelectionMode::Folder
        );
        assert_eq!(
            selection_mode([Some(SourceKind::File), Some(SourceKind::Folder)]),
            SelectionMode::Incompatible
        );
        assert_eq!(
            selection_mode([Some(SourceKind::Other), None]),
            SelectionMode::Incompatible
        );
    }

    #[test]
    fn derives_native_dialog_starting_locations_from_current_source() {
        assert_eq!(
            initial_directory("/workspace/left", Some(SourceKind::Folder)),
            Some(PathBuf::from("/workspace/left"))
        );
        assert_eq!(
            initial_directory("/workspace/left/item.rs", Some(SourceKind::File)),
            Some(PathBuf::from("/workspace/left"))
        );
        assert_eq!(initial_directory("item.rs", Some(SourceKind::File)), None);
        assert_eq!(initial_directory("", Some(SourceKind::Folder)), None);
        assert_eq!(
            initial_file_name("/workspace/left/item.rs", Some(SourceKind::File)),
            Some("item.rs".into())
        );
    }
}
