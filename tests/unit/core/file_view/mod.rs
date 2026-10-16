use super::alignment::{ALIGNMENT_CHARACTER_WORK_BUDGET, line_pair_cost};
use super::*;
use crate::{CompareErrorKind, DEFAULT_TEXT_SIZE_LIMIT};
use std::{
    fs,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

fn sandbox(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "versus-file-view-{name}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn paths(left: Option<PathBuf>, right: Option<PathBuf>) -> [Option<PathBuf>; 2] {
    [left, right]
}

#[test]
fn aligns_anchors_replacements_and_one_sided_lines() {
    let root = sandbox("alignment");
    let left = root.join("left");
    let right = root.join("right");
    fs::write(&left, "anchor\nold\nremove\ntail\n").unwrap();
    fs::write(&right, "anchor\nnew\ntail\nadded\n").unwrap();
    let result = load_file_comparison(&paths(Some(left), Some(right)), &AtomicBool::new(false))
        .unwrap()
        .unwrap();
    assert_eq!(
        result
            .rows
            .iter()
            .map(|row| row.state.clone())
            .collect::<Vec<_>>(),
        vec![
            DirectoryEntryState::Same,
            DirectoryEntryState::Different,
            DirectoryEntryState::LeftOnly,
            DirectoryEntryState::Same,
            DirectoryEntryState::RightOnly,
        ]
    );
    assert_eq!(result.rows[3].left.as_ref().unwrap().0, 4);
    assert_eq!(result.rows[3].right.as_ref().unwrap().0, 3);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn handles_empty_files_and_missing_sides() {
    let root = sandbox("empty");
    let file = root.join("file");
    let empty_left = root.join("empty-left");
    let empty_right = root.join("empty-right");
    fs::write(&empty_left, "").unwrap();
    fs::write(&empty_right, "").unwrap();
    let empty = load_file_comparison(
        &paths(Some(empty_left), Some(empty_right)),
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    assert!(empty.rows.is_empty());
    assert_eq!(empty.message, None);
    fs::write(&file, "one\ntwo\n").unwrap();
    let absent_file = load_file_comparison(
        &paths(Some(root.join("missing-left")), Some(file.clone())),
        &AtomicBool::new(false),
    );
    assert!(absent_file.is_err());
    let right_only = load_file_comparison(&paths(None, Some(file)), &AtomicBool::new(false))
        .unwrap()
        .unwrap();
    assert!(
        right_only
            .rows
            .iter()
            .all(|row| row.left.is_none() && row.state == DirectoryEntryState::RightOnly)
    );
    let no_files = load_file_comparison(&paths(None, None), &AtomicBool::new(false))
        .unwrap()
        .unwrap();
    assert!(no_files.rows.is_empty());
    assert_eq!(no_files.message.as_deref(), Some("No file selected."));

    let empty_left_only = load_file_comparison(
        &paths(Some(root.join("empty-left")), None),
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        empty_left_only.message.as_deref(),
        Some("Only the left file is available; it is empty.")
    );
    let empty_right_only = load_file_comparison(
        &paths(None, Some(root.join("empty-right"))),
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        empty_right_only.message.as_deref(),
        Some("Only the right file is available; it is empty.")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn normalizes_utf8_bom_and_all_common_line_endings_for_display() {
    let root = sandbox("line-endings");
    let left = root.join("left");
    let right = root.join("right");
    fs::write(&left, b"\xef\xbb\xbfone\r\ntwo\rthree\n").unwrap();
    fs::write(&right, "one\ntwo\nthree\n").unwrap();
    let result = load_file_comparison(&paths(Some(left), Some(right)), &AtomicBool::new(false))
        .unwrap()
        .unwrap();
    assert_eq!(result.rows.len(), 3);
    assert!(
        result
            .rows
            .iter()
            .all(|row| row.state == DirectoryEntryState::Same)
    );
    assert_eq!(result.rows[0].left.as_ref().unwrap().1, "one");
    assert_eq!(result.rows[2].right.as_ref().unwrap().1, "three");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn discards_approximate_rows_when_the_diff_timeout_is_reached() {
    let root = sandbox("timeout");
    let left = root.join("left");
    let right = root.join("right");
    fs::write(&left, "one\ntwo\n").unwrap();
    fs::write(&right, "one\nchanged\n").unwrap();
    let result = load_file_comparison_with_timeout(
        &paths(Some(left), Some(right)),
        &AtomicBool::new(false),
        &FileViewOptions::default(),
        Duration::ZERO,
    )
    .unwrap()
    .unwrap();
    assert!(result.rows.is_empty());
    assert_eq!(result.message.as_deref(), Some(DIFF_TIMEOUT_MESSAGE));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reports_binary_oversize_and_cancellation_without_reading_text() {
    let root = sandbox("unavailable");
    let binary = root.join("binary");
    fs::write(&binary, [0, 1, 2]).unwrap();
    let binary_result = load_file_comparison(&paths(Some(binary), None), &AtomicBool::new(false))
        .unwrap()
        .unwrap();
    assert!(binary_result.message.unwrap().contains("binary"));

    let too_large = root.join("large");
    let file = fs::File::create(&too_large).unwrap();
    file.set_len(DEFAULT_TEXT_SIZE_LIMIT + 1).unwrap();
    let large_result = load_file_comparison(&paths(Some(too_large), None), &AtomicBool::new(false))
        .unwrap()
        .unwrap();
    assert!(large_result.message.unwrap().contains("limit"));
    assert_eq!(
        load_file_comparison(&paths(None, None), &AtomicBool::new(true)).unwrap(),
        None
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_directories_as_file_inputs() {
    let root = sandbox("directory");
    let error = load_file_comparison(&paths(Some(root.clone()), None), &AtomicBool::new(false))
        .unwrap_err();
    assert_eq!(error.kind, CompareErrorKind::InvalidPath);
    assert!(error.message.contains("regular"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn short_and_unicode_containment_uses_character_similarity() {
    let cancellation = AtomicBool::new(false);
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut ascii_budget = ALIGNMENT_CHARACTER_WORK_BUDGET;
    let mut unicode_budget = ALIGNMENT_CHARACTER_WORK_BUDGET;
    let ascii = line_pair_cost("a", "//a", &cancellation, deadline, &mut ascii_budget).unwrap();
    let unicode = line_pair_cost("α", "//α", &cancellation, deadline, &mut unicode_budget).unwrap();
    assert_eq!(ascii, unicode);
    assert!(ascii < 150);
}

#[cfg(unix)]
#[test]
fn rejects_symlink_inputs() {
    use std::os::unix::fs::symlink;

    let root = sandbox("symlink");
    let target = root.join("target");
    let link = root.join("link");
    fs::write(&target, "data\n").unwrap();
    symlink(&target, &link).unwrap();
    let error =
        load_file_comparison(&paths(Some(link), None), &AtomicBool::new(false)).unwrap_err();
    assert_eq!(error.kind, CompareErrorKind::InvalidPath);
    assert!(error.message.contains("non-symlink"));
    fs::remove_dir_all(root).unwrap();
}
