//! Search replacement and transactional history behavior.

use super::*;

#[test]
fn refreshed_replacement_preserves_earlier_edit_history() {
    let mut view = view("before match\nkept\n", "right\n");
    view.workspace
        .as_mut()
        .unwrap()
        .set_text(TwoWaySide::Left, "old match\nkept\n");
    view.invalidate_diff();
    let mut saved = view.export_working_files();
    saved
        .get_mut(&PathBuf::from("left.rs"))
        .unwrap()
        .fingerprint = versus::FileFingerprint::Bytes(b"old match\nkept\n".to_vec());
    view.update_saved_files(&saved);
    let preview = refreshed_preview(&view, &[("left.rs", "new\nold match\nkept\n")]);
    view.apply_search_preview(&preview).unwrap();
    assert_eq!(view.working_text(0), Some("new\nold found\nkept\n"));
    view.undo_batch(false).unwrap();
    assert_eq!(view.working_text(0), Some("new\nold match\nkept\n"));
    view.selected_side = 0;
    view.move_history(false);
    assert_eq!(view.working_text(0), Some("old match\nkept\n"));
    for _ in 0..3 {
        view.move_history(false);
    }
    assert_eq!(view.working_text(0), Some("before match\nkept\n"));
    assert!(!view.run_diff_enabled());
}

#[test]
fn refreshed_replacement_rejects_dirty_documents_atomically() {
    let mut view = view("left match\n", "right match\n");
    let preview = refreshed_preview(
        &view,
        &[
            ("left.rs", "new left match\n"),
            ("right.rs", "new right match\n"),
        ],
    );
    view.workspace
        .as_mut()
        .unwrap()
        .set_text(TwoWaySide::Right, "unsaved match\n");
    view.invalidate_diff();
    let before = view.export_working_files();
    let revision = view.folder_revision_signature();
    assert!(view.apply_search_preview(&preview).is_err());
    assert_eq!(view.export_working_files(), before);
    assert_eq!(view.folder_revision_signature(), revision);
    assert!(!view.can_undo_batch());
}

#[test]
fn external_refresh_splice_is_utf8_safe_and_preserves_exact_line_endings() {
    for (before, after) in [
        ("é match\r\n", "ê match\r\n"),
        ("a\r\n", "a\n"),
        ("", "文"),
        ("文", ""),
    ] {
        let (range, replacement) = external_refresh_edit(before, after).unwrap();
        let mut text = before.to_owned();
        text.replace_range(range, &replacement);
        assert_eq!(text, after);
    }
    assert_eq!(external_refresh_edit("same", "same"), None);
}
#[test]
fn folder_batch_and_file_history_undo_and_redo_in_transaction_order() {
    let mut view = view("old-left\n", "old-right\n");
    view.workspace
        .as_mut()
        .unwrap()
        .set_text(TwoWaySide::Left, "manual-left\n");
    view.invalidate_diff();
    let preview = preview_for(
        &view,
        &[("left.rs", 0..6, "batch"), ("right.rs", 0..3, "batch")],
    );
    view.apply_search_preview(&preview).unwrap();
    assert_eq!(view.working_text(0), Some("batch-left\n"));
    assert_eq!(view.working_text(1), Some("batch-right\n"));
    view.selected_side = 0;
    view.move_history(false);
    assert_eq!(view.working_text(0), Some("manual-left\n"));
    assert_eq!(view.working_text(1), Some("old-right\n"));
    view.move_history(false);
    assert_eq!(view.working_text(0), Some("old-left\n"));
    assert!(!view.can_redo_batch());
    view.move_history(true);
    assert_eq!(view.working_text(0), Some("manual-left\n"));
    assert!(view.can_redo_batch());
    view.move_history(true);
    assert_eq!(view.working_text(0), Some("batch-left\n"));
    assert_eq!(view.working_text(1), Some("batch-right\n"));
}
#[test]
fn later_manual_edits_block_atomic_batch_undo_until_their_undo() {
    let mut view = view("left\n", "right\n");
    let preview = preview_for(
        &view,
        &[("left.rs", 0..4, "batch"), ("right.rs", 0..5, "batch")],
    );
    view.apply_search_preview(&preview).unwrap();
    view.workspace
        .as_mut()
        .unwrap()
        .set_text(TwoWaySide::Right, "newer\n");
    view.invalidate_diff();
    view.selected_side = 0;
    view.move_history(false);
    assert_eq!(view.working_text(0), Some("batch\n"));
    assert_eq!(view.working_text(1), Some("newer\n"));
    assert!(view.error.is_some());
    view.selected_side = 1;
    view.move_history(false);
    assert_eq!(view.working_text(1), Some("batch\n"));
    view.move_history(false);
    assert_eq!(view.working_text(0), Some("left\n"));
    assert_eq!(view.working_text(1), Some("right\n"));
}

#[test]
fn alternate_pairing_retains_canonical_file_text_and_undo_without_changing_partner() {
    let mut view = view("left\n", "right\n");
    view.workspace
        .as_mut()
        .unwrap()
        .set_text(TwoWaySide::Left, "first\n");
    view.invalidate_diff();
    let mut extra = BTreeMap::new();
    extra.insert(
        PathBuf::from("third.rs"),
        WorkingFile {
            identity: None,
            path: Some(PathBuf::from("third.rs")),
            text: "third\n".into(),
            encoding: versus::TextEncoding::Utf8 { bom: false },
            fingerprint: versus::FileFingerprint::Bytes(b"third\n".to_vec()),
        },
    );
    view.ingest_working_files(&extra);
    view.open_pair([PathBuf::from("left.rs"), PathBuf::from("third.rs")]);
    view.workspace
        .as_mut()
        .unwrap()
        .set_text(TwoWaySide::Left, "second\n");
    view.invalidate_diff();
    view.open_pair([PathBuf::from("left.rs"), PathBuf::from("right.rs")]);
    assert_eq!(view.working_text(0), Some("second\n"));
    assert_eq!(view.working_text(1), Some("right\n"));
    view.selected_side = 0;
    view.move_history(false);
    assert_eq!(view.working_text(0), Some("first\n"));
    assert_eq!(view.working_text(1), Some("right\n"));
    assert_eq!(
        view.export_working_files()[&PathBuf::from("left.rs")].text,
        "first\n"
    );
}
#[test]
fn failed_multifile_batch_preflight_changes_no_text_or_history() {
    let mut view = view("left\n", "right\n");
    let mut preview = preview_for(
        &view,
        &[("left.rs", 0..4, "batch"), ("right.rs", 0..5, "batch")],
    );
    preview
        .changes
        .iter_mut()
        .find(|change| change.path == PathBuf::from("right.rs"))
        .unwrap()
        .edits = vec![(0..999, "invalid".into())];
    assert!(view.apply_search_preview(&preview).is_err());
    assert_eq!(view.working_text(0), Some("left\n"));
    assert_eq!(view.working_text(1), Some("right\n"));
    assert!(!view.can_undo_batch());
    assert!(
        view.workspace
            .as_ref()
            .unwrap()
            .document(TwoWaySide::Left)
            .next_undo_tag()
            .is_none()
    );
}
#[test]
fn save_after_batch_then_undo_uses_the_saved_baseline_and_retains_batch_redo() {
    let mut view = view("left\n", "right\n");
    let preview = preview_for(
        &view,
        &[("left.rs", 0..4, "batch"), ("right.rs", 0..5, "batch")],
    );
    view.apply_search_preview(&preview).unwrap();
    let mut saved = view.export_working_files();
    for file in saved.values_mut() {
        file.fingerprint = versus::FileFingerprint::Bytes(file.text.as_bytes().to_vec());
    }
    view.update_saved_files(&saved);
    assert!(!view.any_dirty());
    view.move_history(false);
    assert!(view.any_dirty());
    assert_eq!(view.working_text(0), Some("left\n"));
    view.move_history(true);
    assert!(!view.any_dirty());
}
