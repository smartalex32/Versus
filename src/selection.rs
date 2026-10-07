//! Native source selection and type classification for a comparison side.

use std::path::{Path, PathBuf};

/// The kinds of sources that the comparison workspace understands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceKind {
    File,
    Folder,
    Other,
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
) -> rfd::FileDialog {
    let source_name = if side == 0 { "left" } else { "right" };
    let mut dialog =
        rfd::FileDialog::new().set_title(format!("Choose {source_name} file or folder"));

    if let Some(directory) = initial_directory(current, current_kind) {
        dialog = dialog.set_directory(directory);
    }
    if let Some(file_name) = initial_file_name(current, current_kind) {
        dialog = dialog.set_file_name(file_name);
    }
    dialog
}

// Some native backends return standard Yes/No values even when custom labels
// were requested. Keep the mapping explicit so those choices never become cancel.
#[cfg(any(not(target_os = "macos"), test))]
fn picker_kind(choice: rfd::MessageDialogResult) -> Option<SourceKind> {
    match choice {
        rfd::MessageDialogResult::Yes => Some(SourceKind::File),
        rfd::MessageDialogResult::No => Some(SourceKind::Folder),
        rfd::MessageDialogResult::Custom(choice) if choice == "File" => Some(SourceKind::File),
        rfd::MessageDialogResult::Custom(choice) if choice == "Folder" => Some(SourceKind::Folder),
        _ => None,
    }
}

/// Show a native source picker for one side of the comparison.
///
/// macOS supplies a combined file-or-folder picker. Other supported platforms
/// first ask which source kind to choose, then open the appropriate native
/// picker.
pub fn pick_path(side: usize, current: &str, current_kind: Option<SourceKind>) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        configured_dialog(side, current, current_kind).pick_file_or_folder()
    }

    #[cfg(not(target_os = "macos"))]
    {
        // Windows' default MessageBox backend cannot label custom buttons.
        #[cfg(target_os = "windows")]
        let (description, buttons) = (
            "Select Yes to choose a file, No to choose a folder, or Cancel to keep the current selection.",
            rfd::MessageButtons::YesNoCancel,
        );
        #[cfg(not(target_os = "windows"))]
        let (description, buttons) = (
            "Choose a file or folder to compare. Select Cancel to keep the current source.",
            rfd::MessageButtons::YesNoCancelCustom("File".into(), "Folder".into(), "Cancel".into()),
        );
        let choice = rfd::MessageDialog::new()
            .set_title("Choose comparison source")
            .set_description(description)
            .set_buttons(buttons)
            .show();

        match picker_kind(choice) {
            Some(SourceKind::File) => configured_dialog(side, current, current_kind).pick_file(),
            Some(SourceKind::Folder) => {
                configured_dialog(side, current, current_kind).pick_folder()
            }
            _ => None,
        }
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
    fn maps_standard_and_custom_native_choices_without_confusing_cancel() {
        use rfd::MessageDialogResult::{Cancel, Custom, No, Yes};
        for choice in [Yes, Custom("File".into())] {
            assert_eq!(picker_kind(choice), Some(SourceKind::File));
        }
        for choice in [No, Custom("Folder".into())] {
            assert_eq!(picker_kind(choice), Some(SourceKind::Folder));
        }
        for choice in [Cancel, Custom("Cancel".into())] {
            assert_eq!(picker_kind(choice), None);
        }
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
