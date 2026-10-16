use super::*;

#[test]
fn closes_over_multiple_overlapping_edits() {
    let merged = merge_three_way_text("a\nb\nc\nd\n", "a\n", "a\nB\nc\nD\n").unwrap();
    assert_eq!(merged.hunks.len(), 1);
    assert_eq!(merged.hunks[0].kind, MergeHunkKind::Conflict);
    assert_eq!(
        merged.merged,
        "a\n<<<<<<< OURS\n=======\nB\nc\nD\n>>>>>>> THEIRS\n"
    );
}

#[test]
fn preserves_disjoint_edits_from_both_sides() {
    let merged = merge_three_way_text("a\nb\nc\n", "a\nours\nc\n", "a\nb\ntheirs\n").unwrap();
    assert_eq!(merged.merged, "a\nours\ntheirs\n");
    assert!(
        merged
            .hunks
            .iter()
            .all(|hunk| hunk.kind != MergeHunkKind::Conflict)
    );
}
