use std::path::PathBuf;
use versus::core::*;

fn entry(path: &str, kind: DirectoryEntryKind, state: DirectoryEntryState) -> DirectoryEntry {
    let left_exists = !matches!(&state, DirectoryEntryState::RightOnly);
    let right_exists = !matches!(&state, DirectoryEntryState::LeftOnly);
    DirectoryEntry {
        relative_path: PathBuf::from(path),
        left_exists,
        right_exists,
        left_kind: left_exists.then(|| kind.clone()),
        right_kind: right_exists.then(|| kind.clone()),
        left_size: None,
        right_size: None,
        kind,
        state,
    }
}

fn type_mismatch(
    path: &str,
    left_kind: DirectoryEntryKind,
    right_kind: DirectoryEntryKind,
) -> DirectoryEntry {
    DirectoryEntry {
        relative_path: PathBuf::from(path),
        left_exists: true,
        right_exists: true,
        left_kind: Some(left_kind.clone()),
        right_kind: Some(right_kind),
        left_size: None,
        right_size: None,
        kind: left_kind,
        state: DirectoryEntryState::TypeMismatch,
    }
}

fn tree(entries: Vec<DirectoryEntry>) -> FolderTree {
    FolderTree::from_diff(&DirectoryDiff {
        entries,
        cancelled: false,
    })
}

fn node<'a>(root: &'a TreeNode, path: &str) -> &'a TreeNode {
    path.split('/').fold(root, |current, component| {
        current
            .children
            .iter()
            .find(|child| child.name == component)
            .unwrap()
    })
}

#[test]
fn nested_changes_mark_shared_ancestor_directories_different() {
    let tree = tree(vec![
        entry(
            "src",
            DirectoryEntryKind::Directory,
            DirectoryEntryState::Same,
        ),
        entry(
            "src/lib.rs",
            DirectoryEntryKind::File,
            DirectoryEntryState::Different,
        ),
    ]);
    let src = node(tree.root(), "src");
    assert_eq!(src.state, DirectoryEntryState::Different);
    assert_eq!(src.left.kind, Some(DirectoryEntryKind::Directory));
    assert_eq!(src.right.kind, Some(DirectoryEntryKind::Directory));
}

#[test]
fn one_sided_subtrees_preserve_presence_and_order_directories_first() {
    let tree = tree(vec![
        entry(
            "z-file",
            DirectoryEntryKind::File,
            DirectoryEntryState::Same,
        ),
        entry(
            "a-file",
            DirectoryEntryKind::File,
            DirectoryEntryState::Same,
        ),
        entry(
            "right",
            DirectoryEntryKind::Directory,
            DirectoryEntryState::RightOnly,
        ),
        entry(
            "right/nested",
            DirectoryEntryKind::File,
            DirectoryEntryState::RightOnly,
        ),
        entry(
            "left",
            DirectoryEntryKind::Directory,
            DirectoryEntryState::LeftOnly,
        ),
    ]);
    let names: Vec<_> = tree
        .root()
        .children
        .iter()
        .map(|child| child.name.to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["left", "right", "a-file", "z-file"]);
    let right = node(tree.root(), "right");
    assert_eq!(right.state, DirectoryEntryState::RightOnly);
    assert!(!right.left.exists);
    assert_eq!(right.right.kind, Some(DirectoryEntryKind::Directory));
}

#[test]
fn expansion_is_shared_by_relative_path_and_controls_visible_rows() {
    let mut tree = tree(vec![
        entry(
            "folder",
            DirectoryEntryKind::Directory,
            DirectoryEntryState::Same,
        ),
        entry(
            "folder/file",
            DirectoryEntryKind::File,
            DirectoryEntryState::Same,
        ),
        entry("other", DirectoryEntryKind::File, DirectoryEntryState::Same),
    ]);
    assert_eq!(tree.visible_rows().len(), 2);
    assert!(tree.toggle_expanded("folder"));
    let rows = tree.visible_rows();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1].node.relative_path, PathBuf::from("folder/file"));
    assert_eq!(rows[1].depth, 1);
    assert!(!tree.toggle_expanded("folder"));
    assert_eq!(tree.visible_rows().len(), 2);
}

#[test]
fn type_mismatches_retain_the_exact_kind_on_each_side() {
    let tree = tree(vec![
        type_mismatch(
            "item",
            DirectoryEntryKind::File,
            DirectoryEntryKind::Directory,
        ),
        entry(
            "item/child",
            DirectoryEntryKind::File,
            DirectoryEntryState::RightOnly,
        ),
        type_mismatch(
            "empty",
            DirectoryEntryKind::File,
            DirectoryEntryKind::Directory,
        ),
    ]);
    let item = node(tree.root(), "item");
    assert_eq!(item.state, DirectoryEntryState::TypeMismatch);
    assert_eq!(item.left.kind, Some(DirectoryEntryKind::File));
    assert_eq!(item.right.kind, Some(DirectoryEntryKind::Directory));
    let empty = node(tree.root(), "empty");
    assert!(empty.right.exists);
    assert_eq!(empty.right.kind, Some(DirectoryEntryKind::Directory));
}

#[test]
fn errors_propagate_to_shared_ancestors() {
    let error = CompareError {
        path: Some(PathBuf::from("left/src/blocked")),
        kind: CompareErrorKind::Io,
        message: "permission denied".into(),
    };
    let tree = tree(vec![
        entry(
            "src",
            DirectoryEntryKind::Directory,
            DirectoryEntryState::Same,
        ),
        entry(
            "src/blocked",
            DirectoryEntryKind::Other,
            DirectoryEntryState::Error(error.clone()),
        ),
    ]);
    assert_eq!(
        node(tree.root(), "src").state,
        DirectoryEntryState::Error(error)
    );
}

#[test]
fn filesystem_comparison_preserves_empty_folders_and_mismatched_side_types() {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };
    let root = std::env::temp_dir().join(format!(
        "versus-tree-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let left = root.join("left");
    let right = root.join("right");
    for path in [
        left.join("empty"),
        right.join("empty"),
        right.join("mismatch"),
        left.join("left-only"),
        right.join("right-only"),
    ] {
        fs::create_dir_all(path).unwrap();
    }
    fs::write(left.join("mismatch"), b"file opposite an empty folder").unwrap();
    let diff = compare_directories(&left, &right, &DirectoryCompareOptions::default()).unwrap();
    let tree = FolderTree::from_diff(&diff);
    let empty = node(tree.root(), "empty");
    assert_eq!(empty.state, DirectoryEntryState::Same);
    assert!(empty.left.exists && empty.right.exists && empty.children.is_empty());
    let mismatch = node(tree.root(), "mismatch");
    assert_eq!(mismatch.state, DirectoryEntryState::TypeMismatch);
    assert_eq!(mismatch.left.kind, Some(DirectoryEntryKind::File));
    assert_eq!(mismatch.right.kind, Some(DirectoryEntryKind::Directory));
    assert!(!node(tree.root(), "left-only").right.exists);
    assert!(!node(tree.root(), "right-only").left.exists);
    assert_eq!(
        fs::read(left.join("mismatch")).unwrap(),
        b"file opposite an empty folder"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn filesystem_comparison_reports_recursive_sizes_per_side() {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };
    let root = std::env::temp_dir().join(format!(
        "versus-tree-sizes-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let left = root.join("left");
    let right = root.join("right");
    fs::create_dir_all(left.join("nested")).unwrap();
    fs::create_dir_all(right.join("nested")).unwrap();
    fs::create_dir_all(left.join("empty")).unwrap();
    fs::create_dir_all(right.join("empty")).unwrap();
    fs::create_dir_all(right.join("mismatch")).unwrap();
    fs::write(left.join("nested/shared.bin"), b"abc").unwrap();
    fs::write(right.join("nested/shared.bin"), b"abc").unwrap();
    fs::write(left.join("nested/left.bin"), b"wxyz").unwrap();
    fs::write(right.join("right.bin"), b"12").unwrap();
    fs::write(left.join("mismatch"), b"12345").unwrap();

    let diff = compare_directories(&left, &right, &DirectoryCompareOptions::default()).unwrap();
    let tree = FolderTree::from_diff(&diff);
    let nested = node(tree.root(), "nested");
    assert_eq!(nested.left.size, Some(7));
    assert_eq!(nested.right.size, Some(3));
    assert_eq!(node(tree.root(), "empty").left.size, Some(0));
    assert_eq!(node(tree.root(), "empty").right.size, Some(0));
    let mismatch = node(tree.root(), "mismatch");
    assert_eq!(mismatch.left.size, Some(5));
    assert_eq!(mismatch.right.size, Some(0));
    assert_eq!(tree.root().left.size, Some(12));
    assert_eq!(tree.root().right.size, Some(5));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn filesystem_comparison_does_not_count_symlink_targets_in_folder_sizes() {
    use std::{
        fs,
        os::unix::fs::symlink,
        time::{SystemTime, UNIX_EPOCH},
    };
    let root = std::env::temp_dir().join(format!(
        "versus-tree-links-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let left = root.join("left");
    let right = root.join("right");
    fs::create_dir_all(&left).unwrap();
    fs::create_dir_all(&right).unwrap();
    fs::write(left.join("file"), b"abc").unwrap();
    fs::write(right.join("file"), b"abc").unwrap();
    symlink("file", left.join("file-link")).unwrap();
    symlink("file", right.join("file-link")).unwrap();

    let diff = compare_directories(&left, &right, &DirectoryCompareOptions::default()).unwrap();
    let tree = FolderTree::from_diff(&diff);
    assert_eq!(node(tree.root(), "file-link").left.size, None);
    assert_eq!(node(tree.root(), "file-link").right.size, None);
    assert_eq!(tree.root().left.size, Some(3));
    assert_eq!(tree.root().right.size, Some(3));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unknown_directory_size_propagates_only_on_its_own_side() {
    let mut directory = entry(
        "unreadable-child",
        DirectoryEntryKind::Directory,
        DirectoryEntryState::Same,
    );
    directory.left_size = None;
    directory.right_size = Some(0);

    let tree = tree(vec![directory]);
    assert_eq!(tree.root().left.size, None);
    assert_eq!(tree.root().right.size, Some(0));
}

#[test]
fn root_scan_error_does_not_get_replaced_with_an_empty_folder_size() {
    let error = CompareError {
        path: Some(PathBuf::from("left")),
        kind: CompareErrorKind::Io,
        message: "directory iteration failed".into(),
    };
    let tree = tree(vec![DirectoryEntry {
        relative_path: PathBuf::new(),
        left_exists: true,
        right_exists: false,
        left_kind: None,
        right_kind: None,
        left_size: None,
        right_size: None,
        kind: DirectoryEntryKind::Other,
        state: DirectoryEntryState::Error(error),
    }]);
    assert_eq!(tree.root().left.size, None);
    assert_eq!(tree.root().right.size, Some(0));
}
