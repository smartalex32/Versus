//! Inline transfers, copy-all, line endings, and paired saves.

use super::*;

#[test]
fn accept_all_replaces_destination_as_one_undoable_operation() {
    let mut view = view("whole\nleft\n", "right\n");
    view.copy_all(TwoWaySide::Left);
    assert_eq!(view.working_text(1), view.working_text(0));
    assert!(view.workspace.as_mut().unwrap().undo());
    assert_eq!(view.working_text(1), Some("right\n"));
    assert!(!view.workspace.as_ref().unwrap().can_undo());
    assert!(view.workspace.as_mut().unwrap().redo());
    assert_eq!(view.working_text(1), Some("whole\nleft\n"));
}

#[test]
fn repeated_insertions_follow_the_original_target_in_source_order() {
    let mut view = view("first\nsecond\n", "keep\n");
    view.caret[1] = 0;
    view.transfer(TwoWaySide::Left, 0..1);
    view.transfer(TwoWaySide::Left, 1..2);
    assert_eq!(view.working_text(1), Some("first\nsecond\nkeep\n"));
    assert!(view.workspace.as_mut().unwrap().undo());
    assert_eq!(view.working_text(1), Some("first\nkeep\n"));
}

#[test]
fn editing_preserves_unaffected_mixed_endings() {
    assert_eq!(
        restore_line_endings("one\r\ntwo\nthree\r", "one\nchanged\nthree\n"),
        "one\r\nchanged\nthree\r"
    );
    assert_eq!(restore_line_endings("a\r\nb", "a\nb!"), "a\r\nb!");
}
#[test]
fn save_both_writes_both_snapshots_and_retains_newer_edits() {
    let root = std::env::temp_dir().join(format!("versus-ui-save-both-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let paths = [root.join("left"), root.join("right")];
    std::fs::write(&paths[0], "old-left\n").unwrap();
    std::fs::write(&paths[1], "old-right\n").unwrap();
    let mut view = view("old-left\n", "old-right\n");
    view.files = Some(std::array::from_fn(|side| {
        load_working_file(Some(&paths[side])).unwrap()
    }));
    view.paths = Some(paths.clone());
    view.workspace
        .as_mut()
        .unwrap()
        .set_text(TwoWaySide::Left, "saved-left\n");
    view.workspace
        .as_mut()
        .unwrap()
        .set_text(TwoWaySide::Right, "saved-right\n");
    view.save_sides(vec![0, 1]);
    view.workspace
        .as_mut()
        .unwrap()
        .set_text(TwoWaySide::Left, "newer\n");
    let ctx = egui::Context::default();
    for _ in 0..100 {
        view.poll_save(&ctx);
        if !view.is_saving() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!view.is_saving());
    assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "saved-left\n");
    assert_eq!(std::fs::read_to_string(&paths[1]).unwrap(), "saved-right\n");
    assert_eq!(view.working_text(0), Some("newer\n"));
    assert!(view.workspace.as_ref().unwrap().is_dirty(TwoWaySide::Left));
    assert!(!view.workspace.as_ref().unwrap().is_dirty(TwoWaySide::Right));
    view.workspace.as_mut().unwrap().undo();
    assert!(!view.workspace.as_ref().unwrap().is_dirty(TwoWaySide::Left));
    let _ = std::fs::remove_dir_all(root);
}
