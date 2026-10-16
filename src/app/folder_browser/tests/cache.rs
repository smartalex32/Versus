//! Folder browser cache capability regressions.

use super::*;

#[test]
fn merged_states_reaggregate_nested_parents_and_preserve_intrinsic_states() {
    let mut mixed_type = tree_entry(
        "file-folder",
        DirectoryEntryKind::File,
        DirectoryEntryState::TypeMismatch,
    );
    mixed_type.left_kind = Some(DirectoryEntryKind::Directory);
    mixed_type.left_size = None;
    let tree = folder_tree(vec![
        mixed_type,
        tree_entry(
            "src",
            DirectoryEntryKind::Directory,
            DirectoryEntryState::Same,
        ),
        tree_entry(
            "src/changed.rs",
            DirectoryEntryKind::File,
            DirectoryEntryState::Different,
        ),
        tree_entry(
            "src/clean.rs",
            DirectoryEntryKind::File,
            DirectoryEntryState::Same,
        ),
        tree_entry(
            "only-left",
            DirectoryEntryKind::Directory,
            DirectoryEntryState::LeftOnly,
        ),
        tree_entry(
            "mismatch",
            DirectoryEntryKind::File,
            DirectoryEntryState::TypeMismatch,
        ),
        tree_entry(
            "only-right.txt",
            DirectoryEntryKind::File,
            DirectoryEntryState::RightOnly,
        ),
        tree_entry(
            "unreadable",
            DirectoryEntryKind::Directory,
            DirectoryEntryState::Error(CompareError::io(
                "unreadable",
                std::io::Error::other("denied"),
            )),
        ),
    ]);
    let mut overlays = BTreeMap::new();
    overlays.insert(PathBuf::from("src/changed.rs"), DirectoryEntryState::Same);
    let cache = build_merged_folder_cache(Some(&tree), &overlays, Some(1));
    assert_eq!(
        cache.states[std::path::Path::new("src")],
        DirectoryEntryState::Same,
        "a clean overlay must restore all nested parent badges"
    );
    assert_eq!(
        cache.states[std::path::Path::new("only-left")],
        DirectoryEntryState::LeftOnly
    );
    assert_eq!(
        cache.states[std::path::Path::new("mismatch")],
        DirectoryEntryState::TypeMismatch
    );
    assert_eq!(
        cache.states[std::path::Path::new("only-right.txt")],
        DirectoryEntryState::RightOnly
    );
    assert!(matches!(
        cache.states[std::path::Path::new("unreadable")],
        DirectoryEntryState::Error(_)
    ));
    assert_eq!(
        cache.difference_paths.as_slice(),
        [
            PathBuf::from("file-folder"),
            PathBuf::from("mismatch"),
            PathBuf::from("only-right.txt")
        ]
    );
    for terminal in [
        DirectoryEntryState::TypeMismatch,
        DirectoryEntryState::LeftOnly,
        DirectoryEntryState::RightOnly,
        DirectoryEntryState::Error(CompareError::io(
            "src",
            std::io::Error::other("overlay failed"),
        )),
    ] {
        overlays.insert(PathBuf::from("src"), terminal.clone());
        let cache = build_merged_folder_cache(Some(&tree), &overlays, Some(1));
        assert_eq!(
            cache.states[std::path::Path::new("src")],
            terminal,
            "a direct terminal overlay must survive parent aggregation"
        );
    }
}

#[test]
fn merged_state_cache_is_reused_for_large_identical_trees_until_invalidated() {
    let mut entries = Vec::new();
    for index in 0..8_000 {
        entries.push(tree_entry(
            &format!("dir-{index}"),
            DirectoryEntryKind::Directory,
            DirectoryEntryState::Same,
        ));
        entries.push(tree_entry(
            &format!("dir-{index}/file.txt"),
            DirectoryEntryKind::File,
            DirectoryEntryState::Same,
        ));
    }
    let tree = folder_tree(entries);
    let mut browser = FolderBrowser::new_search([None, None]);
    let first = browser.merged_folder_cache(Some(&tree)).states.clone();
    let first_paths = browser
        .merged_folder_cache(Some(&tree))
        .difference_paths
        .clone();
    let second = browser.merged_folder_cache(Some(&tree));
    assert!(Arc::ptr_eq(&first, &second.states));
    assert!(Arc::ptr_eq(&first_paths, &second.difference_paths));
    assert!(second.difference_paths.is_empty());
    browser.invalidate_comparison_cache();
    let rebuilt = browser.merged_folder_cache(Some(&tree));
    assert!(!Arc::ptr_eq(&first, &rebuilt.states));
}
