use super::*;

const FIRST: &str = "<<<<<<< current\nC1\n=======\nI1\n>>>>>>> incoming\n";
const SECOND: &str = "<<<<<<< current\nC2\n=======\nI2\n>>>>>>> incoming\n";

#[test]
fn broad_marker_edits_and_history_never_duplicate_conflict_identities() {
    let initial = format!("{FIRST}between\n{SECOND}");
    let mut merge = model(None, "I1\nbetween\nI2\n", "C1\nbetween\nC2\n", &initial);
    let edited = initial.replace("C1", "D1").replace("C2", "D2");
    merge.edit_result(edited.clone()).unwrap();
    assert_eq!(merge.unresolved_count(), 2);
    let revision = merge.revision();
    merge.edit_result(edited).unwrap();
    assert_eq!(merge.revision(), revision);
    assert_eq!(merge.unresolved_count(), 2);
    assert!(merge.undo());
    assert_eq!(merge.unresolved_count(), 2);
    assert!(merge.redo());
    assert_eq!(merge.unresolved_count(), 2);
}

#[test]
fn editing_inside_a_resolved_conflict_retains_its_unchanged_suffix() {
    let initial = "<<<<<<< current\nA\nB\nC\n=======\na\nb\nc\n>>>>>>> incoming\n";
    let mut merge = model(Some("base\n"), "a\nb\nc\n", "A\nB\nC\n", initial);
    let id = merge.regions()[0].id;
    merge
        .choose(id, merge.revision(), MergeChoice::Incoming)
        .unwrap();
    merge.edit_result("a\nchanged\nc\n".into()).unwrap();
    assert_eq!(merge.unresolved_count(), 1);
    assert_eq!(merge.conflicts[0].envelope, 0..merge.result().len());
    let id = merge
        .regions()
        .iter()
        .find(|region| region.unresolved)
        .unwrap()
        .id;
    merge
        .choose(id, merge.revision(), MergeChoice::Current)
        .unwrap();
    assert_eq!(merge.result(), "A\nB\nC\n");
}

#[test]
fn deleted_conflict_stays_actionable_when_folded_into_a_nonempty_change() {
    let initial = format!("A\n{FIRST}Z\n");
    let mut merge = model(Some("A\nbase\nZ\n"), "A\nI1\nZ\n", "A\nC1\nZ\n", &initial);
    merge.edit_result("A\nZ\n".into()).unwrap();
    assert!(merge.conflicts[0].envelope.is_empty());
    merge.edit_result("B\nZ\n".into()).unwrap();
    let id = merge
        .regions()
        .iter()
        .find(|region| region.unresolved)
        .unwrap()
        .id;
    merge.mark_resolved(id, merge.revision()).unwrap();
    assert!(merge.can_finish());
    assert!(merge.undo());
    assert_eq!(merge.unresolved_count(), 1);
}

#[test]
fn a_new_wrapper_elsewhere_keeps_the_original_manual_resolution_pending() {
    let initial = format!("{FIRST}separator\n");
    let mut merge = model(None, "I1\nseparator\n", "C1\nseparator\n", &initial);
    merge.edit_result("manual\nseparator\n".into()).unwrap();
    merge
        .edit_result(format!("manual\nseparator\n{SECOND}"))
        .unwrap();
    assert_eq!(merge.unresolved_count(), 2);
    assert!(!merge.can_finish());
}

#[test]
fn malformed_marker_text_cannot_be_marked_resolved() {
    let mut merge = model(None, "I1\n", "C1\n", FIRST);
    merge
        .edit_result("<<<<<<< current\nunfinished\n".into())
        .unwrap();
    let id = merge.regions()[0].id;
    assert_eq!(
        merge.mark_resolved(id, merge.revision()),
        Err(GitMergeError::MalformedConflictMarkers)
    );
    assert_eq!(merge.unresolved_count(), 1);
}

#[test]
fn oversized_manual_edits_are_rejected_atomically() {
    let mut merge = model(None, "incoming", "current", "partial");
    let oversized = "x".repeat(crate::core::DEFAULT_TEXT_SIZE_LIMIT as usize + 1);
    assert_eq!(
        merge.edit_result(oversized),
        Err(GitMergeError::InputTooLarge)
    );
    assert_eq!(merge.result(), "partial");
    assert_eq!(merge.revision(), 0);
    assert!(!merge.can_undo());
}

#[test]
fn mapping_budget_failure_keeps_manual_editing_available_and_arrows_disabled() {
    let mut merge = model(None, "", "", &"\n".repeat(200_001));
    assert!(!merge.mapping_available());
    let before = merge.result().to_owned();
    assert_eq!(
        merge.choose(0, merge.revision(), MergeChoice::Incoming),
        Err(GitMergeError::MappingUnavailable)
    );
    assert_eq!(merge.result(), before);
    merge.edit_result("manually reduced\n".into()).unwrap();
    assert!(merge.mapping_available());
    assert_eq!(merge.result(), "manually reduced\n");
    assert!(merge.undo());
    assert_eq!(merge.result(), before);
}

#[test]
fn both_choices_keep_the_requested_order_and_result_line_ending() {
    for ending in ["\n", "\r\n", "\r"] {
        let initial = format!("partial{ending}");
        let mut merge = model(None, "incoming", "current", &initial);
        let id = merge.regions()[0].id;
        merge
            .choose(id, merge.revision(), MergeChoice::IncomingThenCurrent)
            .unwrap();
        assert_eq!(merge.result(), format!("incoming{ending}current"));
        assert!(merge.undo());
        let id = merge.regions()[0].id;
        merge
            .choose(id, merge.revision(), MergeChoice::CurrentThenIncoming)
            .unwrap();
        assert_eq!(merge.result(), format!("current{ending}incoming"));
    }
}
