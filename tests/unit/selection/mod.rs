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
