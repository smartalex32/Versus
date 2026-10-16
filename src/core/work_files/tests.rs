use super::*;
use crate::core::DirectoryEntryKind;
use std::fs;
fn temp(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("versus-work-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}
#[test]
fn utf16_and_external_changes_are_preserved_and_detected() {
    let root = temp("utf16");
    let file = root.join("a.txt");
    fs::write(&file, [0xff, 0xfe, b'h', 0, b'i', 0, b'\r', 0, b'\n', 0]).unwrap();
    let mut working = load_working_file(Some(&file)).unwrap();
    assert_eq!(working.text, "hi\r\n");
    working.text.push('!');
    save_working_file(&mut working).unwrap();
    assert_eq!(&fs::read(&file).unwrap()[..2], &[0xff, 0xfe]);
    fs::write(&file, "outside").unwrap();
    working.text.push('?');
    assert!(save_working_file(&mut working).is_err());
    let _ = fs::remove_dir_all(root);
}
#[test]
fn directory_listing_is_one_level_and_metadata_only() {
    let root = temp("listing");
    let left = root.join("left");
    let right = root.join("right");
    fs::create_dir_all(left.join("nested")).unwrap();
    fs::create_dir_all(&right).unwrap();
    fs::write(left.join("nested/secret"), b"x").unwrap();
    fs::write(left.join("a"), b"left").unwrap();
    fs::write(right.join("b"), b"right").unwrap();
    let entries = directory_entries_at(&left, &right, Path::new("")).unwrap();
    assert_eq!(entries.len(), 3);
    assert!(
        entries
            .iter()
            .all(|entry| entry.relative_path.components().count() == 1)
    );
    let _ = fs::remove_dir_all(root);
}
#[test]
fn single_root_listing_does_not_need_or_invent_a_peer() {
    let root = temp("single-listing");
    fs::create_dir_all(root.join("directory")).unwrap();
    fs::write(root.join("file.txt"), b"metadata only").unwrap();
    let entries = directory_entries_for_root(&root, Path::new("")).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].side.kind, DirectoryEntryKind::Directory);
    assert!(
        entries
            .iter()
            .all(|entry| entry.relative_path.parent() == Some(Path::new("")))
    );
    let _ = fs::remove_dir_all(root);
}
#[test]
fn one_sided_directory_remains_browsable_when_peer_is_regular_file() {
    let root = temp("type-mismatch");
    let left = root.join("left");
    let right = root.join("right");
    fs::create_dir_all(&left).unwrap();
    fs::create_dir_all(right.join("node")).unwrap();
    fs::write(left.join("node"), b"not a directory").unwrap();
    fs::write(right.join("node/child"), b"child").unwrap();
    let entries = directory_entries_at(&left, &right, Path::new("node")).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].left.is_none());
    assert_eq!(
        entries[0].right.as_ref().unwrap().kind,
        DirectoryEntryKind::File
    );
    let _ = fs::remove_dir_all(root);
}
#[test]
fn missing_save_parent_is_created_without_losing_the_destination() {
    let root = temp("nested-save");
    let destination = root.join("new/nested/file.txt");
    let mut file = load_working_file(Some(destination.as_path())).unwrap();
    file.text = "created\n".into();
    save_working_file(&mut file).unwrap();
    assert_eq!(fs::read_to_string(destination).unwrap(), "created\n");
    let _ = fs::remove_dir_all(root);
}
#[test]
fn working_file_key_is_lexical_and_does_not_require_the_path_to_exist() {
    assert_eq!(
        working_file_key(Path::new("missing/./tree/../file.txt")),
        PathBuf::from("missing/file.txt")
    );
    assert_eq!(
        working_file_key(Path::new("../../missing")),
        PathBuf::from("../../missing")
    );
}
#[cfg(unix)]
#[test]
fn save_rejects_a_parent_alias_that_changes_physical_destination() {
    use std::os::unix::fs::symlink;
    let root = temp("changed-parent");
    let first = root.join("first");
    let second = root.join("second");
    let link = root.join("link");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::write(first.join("file.txt"), "first").unwrap();
    fs::write(second.join("file.txt"), "second").unwrap();
    symlink(&first, &link).unwrap();
    let alias = link.join("file.txt");
    let mut file = load_working_file(Some(alias.as_path())).unwrap();
    fs::remove_file(&link).unwrap();
    symlink(&second, &link).unwrap();
    file.text = "edited".into();
    assert!(save_working_file(&mut file).is_err());
    assert_eq!(
        fs::read_to_string(second.join("file.txt")).unwrap(),
        "second"
    );
    let _ = fs::remove_dir_all(root);
}
#[cfg(unix)]
#[test]
fn symlink_relative_parent_is_never_followed_when_listing_children() {
    use std::os::unix::fs::symlink;
    let root = temp("symlink-parent");
    let left = root.join("left");
    let right = root.join("right");
    let external = root.join("external");
    fs::create_dir_all(external.join("deeper")).unwrap();
    fs::create_dir_all(right.join("shortcut/deeper")).unwrap();
    fs::create_dir_all(&left).unwrap();
    fs::write(external.join("deeper/secret"), b"must not list").unwrap();
    fs::write(right.join("shortcut/deeper/peer"), b"visible peer").unwrap();
    symlink(&external, left.join("shortcut")).unwrap();
    let entries = directory_entries_at(&left, &right, Path::new("shortcut/deeper")).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].relative_path,
        PathBuf::from("shortcut/deeper/peer")
    );
    assert!(entries[0].left.is_none());
    assert_eq!(
        entries[0].right.as_ref().unwrap().kind,
        DirectoryEntryKind::File
    );
    let _ = fs::remove_dir_all(root);
}
