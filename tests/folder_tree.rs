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
