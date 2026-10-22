use super::*;

mod ledger;

fn model(base: Option<&str>, incoming: &str, current: &str, result: &str) -> GitMergeModel {
    GitMergeModel::new(
        base.map(str::to_owned),
        incoming.to_owned(),
        current.to_owned(),
        result.to_owned(),
    )
    .unwrap()
}

#[test]
fn preserves_partial_result_and_maps_prior_identical_change() {
    let merge = model(
        Some("one\ntwo\n"),
        "one\nchanged\n",
        "one\nchanged\n",
        "one\nchanged\n",
    );
    assert_eq!(merge.result(), "one\nchanged\n");
    assert!(
        merge
            .regions()
            .iter()
            .any(|region| region.result == (4..12))
    );
}

#[test]
fn chooses_conflict_source_and_undo_restores_text_and_state() {
    let mut merge = model(
        Some("base\n"),
        "incoming\n",
        "current\n",
        "<<<<<<< current\ncurrent\n=======\nincoming\n>>>>>>> incoming\n",
    );
    let conflict = merge
        .regions()
        .iter()
        .find(|region| region.unresolved)
        .unwrap()
        .clone();
    merge
        .choose(conflict.id, merge.revision(), MergeChoice::Incoming)
        .unwrap();
    assert_eq!(merge.result(), "incoming\n");
    assert!(merge.can_finish());
    assert!(merge.undo());
    assert!(merge.result().starts_with("<<<<<<< current"));
    assert_eq!(merge.unresolved_count(), 1);
    assert!(merge.redo());
    assert_eq!(merge.result(), "incoming\n");
}

#[test]
fn stale_region_cannot_overwrite_a_later_edit() {
    let mut merge = model(
        Some("a\nb\n"),
        "a\nincoming\n",
        "a\ncurrent\n",
        "a\npartial\n",
    );
    let region = merge.regions()[0].clone();
    merge.edit_result("a\nmanually edited\n".into()).unwrap();
    assert!(matches!(
        merge.choose(region.id, 0, MergeChoice::Incoming),
        Err(GitMergeError::StaleRegion { .. })
    ));
    assert_eq!(merge.result(), "a\nmanually edited\n");
}

#[test]
fn restore_base_is_one_undoable_operation_and_reopens_known_conflicts() {
    let mut merge = model(
        Some("base\n"),
        "incoming\n",
        "current\n",
        "<<<<<<< current\ncurrent\n=======\nincoming\n>>>>>>> incoming\n",
    );
    let id = merge
        .regions()
        .iter()
        .find(|region| region.unresolved)
        .unwrap()
        .id;
    merge.choose(id, 0, MergeChoice::Incoming).unwrap();
    merge.restore_base().unwrap();
    assert_eq!(merge.result(), "base\n");
    assert_eq!(merge.unresolved_count(), 1);
    assert!(merge.undo());
    assert_eq!(merge.result(), "incoming\n");
    assert!(merge.can_finish());
}

#[test]
fn missing_base_is_distinct_from_empty_base() {
    let mut absent = model(None, "incoming", "current", "partial");
    assert!(matches!(
        absent.restore_base(),
        Err(GitMergeError::BaseUnavailable)
    ));
    let mut empty = model(Some(""), "incoming", "current", "partial");
    empty.restore_base().unwrap();
    assert_eq!(empty.result(), "");
}

#[test]
fn direct_edits_allow_intermediate_malformed_markers_but_cannot_finish() {
    let mut merge = model(Some("base\n"), "incoming\n", "current\n", "clean\n");
    merge
        .edit_result("<<<<<<< current\nunfinished\n".into())
        .unwrap();
    assert!(!merge.can_finish());
    assert!(matches!(
        merge.choose(1_000_000, merge.revision(), MergeChoice::Incoming),
        Err(GitMergeError::MalformedConflictMarkers)
    ));
}

#[test]
fn parses_diff3_crlf_conflicts_with_wide_markers() {
    let result = "<<<<<<<< current\r\ncurrent\r\n|||||||| base\r\nbase\r\n========\r\nincoming\r\n>>>>>>>> remote\r\n";
    let merge = model(Some("base\r\n"), "incoming\r\n", "current\r\n", result);
    assert_eq!(merge.unresolved_count(), 1);
    assert!(!merge.can_finish());
}

#[test]
fn parses_cr_only_conflicts() {
    let result = "<<<<<<< current\rcurrent\r=======\rincoming\r>>>>>>> incoming\r";
    let merge = model(Some("base\r"), "incoming\r", "current\r", result);
    assert_eq!(merge.unresolved_count(), 1);
}

#[test]
fn direct_conflict_edit_requires_explicit_resolution_and_survives_marker_removal() {
    let original = "<<<<<<< current\ncurrent\n=======\nincoming\n>>>>>>> incoming\n";
    let mut merge = model(Some("base\n"), "incoming\n", "current\n", original);
    merge.edit_result("manual\n".into()).unwrap();
    assert_eq!(merge.unresolved_count(), 1);
    let conflict = merge
        .regions()
        .iter()
        .find(|region| region.unresolved)
        .unwrap()
        .id;
    merge.mark_resolved(conflict, merge.revision()).unwrap();
    assert!(merge.can_finish());
}

#[test]
fn malformed_initial_conflicts_are_rejected() {
    assert!(matches!(
        GitMergeModel::new(
            None,
            "incoming".into(),
            "current".into(),
            "<<<<<<< current\n".into()
        ),
        Err(GitMergeError::MalformedConflictMarkers)
    ));
}

#[test]
fn one_manual_edit_can_explicitly_resolve_two_conflicts_and_undo_restores_both() {
    let result = "<<<<<<< current\nC1\n=======\nI1\n>>>>>>> incoming\nplain\n<<<<<<< current\nC2\n=======\nI2\n>>>>>>> incoming\n";
    let mut merge = model(
        Some("base\nplain\nbase\n"),
        "I1\nplain\nI2\n",
        "C1\nplain\nC2\n",
        result,
    );
    merge.edit_result("manual\n".into()).unwrap();
    let region = merge
        .regions()
        .iter()
        .find(|region| region.unresolved)
        .unwrap()
        .clone();
    merge.mark_resolved(region.id, merge.revision()).unwrap();
    assert!(merge.can_finish());
    assert!(merge.undo());
    assert_eq!(merge.unresolved_count(), 2);
}

#[test]
fn touching_a_resolved_conflict_reopens_it() {
    let result = "<<<<<<< current\ncurrent\n=======\nincoming\n>>>>>>> incoming\n";
    let mut merge = model(Some("base\n"), "incoming\n", "current\n", result);
    let region = merge
        .regions()
        .iter()
        .find(|region| region.unresolved)
        .unwrap()
        .clone();
    merge
        .choose(region.id, merge.revision(), MergeChoice::Incoming)
        .unwrap();
    assert!(merge.can_finish());
    merge.edit_result("changed manually\n".into()).unwrap();
    assert_eq!(merge.unresolved_count(), 1);
    assert!(!merge.can_finish());
}

#[test]
fn choosing_after_restore_base_uses_the_full_immutable_source() {
    let result = "<<<<<<< current\ncurrent\n=======\nincoming\n>>>>>>> incoming\n";
    let mut merge = model(Some("base\n"), "incoming\n", "current\n", result);
    merge.restore_base().unwrap();
    let region = merge
        .regions()
        .iter()
        .find(|region| region.unresolved)
        .unwrap()
        .clone();
    merge
        .choose(region.id, merge.revision(), MergeChoice::Incoming)
        .unwrap();
    assert_eq!(merge.result(), "incoming\n");
}

#[test]
fn base_marker_after_divider_is_malformed() {
    let result =
        "<<<<<<< current\ncurrent\n=======\nincoming\n||||||| base\nbase\n>>>>>>> incoming\n";
    assert!(matches!(
        GitMergeModel::new(None, "incoming\n".into(), "current\n".into(), result.into()),
        Err(GitMergeError::MalformedConflictMarkers)
    ));
}
