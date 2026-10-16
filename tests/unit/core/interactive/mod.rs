use super::*;
#[test]
fn chunk_transfer_is_undoable_without_a_diff() {
    let mut workspace = TwoWayWorkspace::new("fn source() {\r\n}\r\n", "unrelated\n");
    let id = workspace.chunks(TwoWaySide::Left)[0].id;
    workspace
        .transfer_chunk(TwoWaySide::Left, id, 1..1, TransferPlacement::Insert)
        .unwrap();
    assert_eq!(
        workspace.text(TwoWaySide::Right),
        "unrelated\nfn source() {\r\n}\r\n"
    );
    assert!(workspace.undo());
}
#[test]
fn byte_edits_are_utf8_safe() {
    let mut document = InteractiveDocument::new("αβ\n");
    assert!(matches!(
        document.apply_byte_edits(HistoryTag::Local(revision()), vec![(1..2, "x".into())]),
        Err(TwoWayMergeError::InvalidByteRange(_))
    ));
    let tag = HistoryTag::FolderBatch(revision());
    document
        .apply_byte_edits(tag, vec![(0..2, "A".into())])
        .unwrap();
    assert!(document.undo_expected(tag));
}
#[test]
fn structural_chunks_are_detected() {
    let workspace = TwoWayWorkspace::new("fn first() {\n}\nfn second() {\n}\n", "");
    assert_eq!(workspace.chunks(TwoWaySide::Left).len(), 2);
}

#[test]
fn structural_chunk_boundaries_handle_rust_python_and_c_lexing() {
    let rust = TwoWayWorkspace::new(
        "fn first() {\n    call(\"}\"); // { ignored\n}\nfn second() {\n}\n",
        "",
    );
    assert_eq!(
        rust.chunks(TwoWaySide::Left)
            .iter()
            .map(|chunk| chunk.range.clone())
            .collect::<Vec<_>>(),
        vec![0..3, 3..5]
    );

    let python = TwoWayWorkspace::new(
        "def first():\n    value = 1\n\n    return value\n\ndef second():\n    return 2\n",
        "",
    );
    assert_eq!(
        python.chunks(TwoWaySide::Left)[0].range,
        0..5,
        "blank lines remain in an indented Python suite"
    );

    let c_style = TwoWayWorkspace::new(
        "void first() {\n  const char *value = \"{ not a block }\";\n  /* { comment } */\n}\nint second()\n{\n  return 2; // } ignored\n}\n",
        "",
    );
    assert_eq!(
        c_style
            .chunks(TwoWaySide::Left)
            .iter()
            .map(|chunk| chunk.range.clone())
            .collect::<Vec<_>>(),
        vec![0..4, 4..8]
    );
}

#[test]
fn batch_insert_uses_original_anchors_and_one_undo_step() {
    let mut workspace = TwoWayWorkspace::new("α\r\nβ\r\nγ\r\n", "zero\nend\n");
    workspace
        .transfer_batch(
            TwoWaySide::Left,
            vec![
                (0..1, 1..1, TransferPlacement::Insert),
                (1..2, 1..1, TransferPlacement::Insert),
                (2..3, 2..2, TransferPlacement::Insert),
            ],
        )
        .unwrap();
    assert_eq!(
        workspace.text(TwoWaySide::Right),
        "zero\nα\r\nβ\r\nend\nγ\r\n"
    );
    assert!(workspace.undo());
    assert_eq!(workspace.text(TwoWaySide::Right), "zero\nend\n");
    assert!(!workspace.can_undo());
}

#[test]
fn facade_undo_keeps_the_other_document_live() {
    let mut workspace = TwoWayWorkspace::from_documents([
        InteractiveDocument::new("a\n"),
        InteractiveDocument::new("b\n"),
    ]);
    workspace.set_text(TwoWaySide::Left, "A\n");
    workspace.set_text(TwoWaySide::Right, "B\n");
    workspace.set_text(TwoWaySide::Left, "AA\n");
    assert!(workspace.undo());
    assert_eq!(workspace.text(TwoWaySide::Left), "A\n");
    assert_eq!(workspace.text(TwoWaySide::Right), "B\n");
    assert_eq!(workspace.document(TwoWaySide::Right).text(), "B\n");
}

#[test]
fn facade_redo_replays_documents_in_original_order() {
    let mut workspace = TwoWayWorkspace::new("a\n", "b\n");
    workspace.set_text(TwoWaySide::Left, "A\n");
    workspace.set_text(TwoWaySide::Right, "B\n");
    assert!(workspace.undo());
    assert!(workspace.undo());
    assert!(workspace.redo());
    assert_eq!(workspace.text(TwoWaySide::Left), "A\n");
    assert_eq!(workspace.text(TwoWaySide::Right), "b\n");
    assert!(workspace.redo());
    assert_eq!(workspace.text(TwoWaySide::Right), "B\n");
}

#[test]
fn byte_edits_are_atomic_and_require_the_expected_history_tag() {
    let mut document = InteractiveDocument::new("zero\nkeep\nlast\n");
    let tag = HistoryTag::FolderBatch(revision());
    document
        .apply_byte_edits(
            tag,
            vec![(0..4, "ZERO".into()), (5..9, "KEEP\nPLUS".into())],
        )
        .unwrap();
    assert_eq!(document.text(), "ZERO\nKEEP\nPLUS\nlast\n");
    assert!(!document.undo_expected(HistoryTag::Local(revision())));
    assert_eq!(document.text(), "ZERO\nKEEP\nPLUS\nlast\n");
    assert!(document.undo_expected(tag));
    assert_eq!(document.text(), "zero\nkeep\nlast\n");

    let before = document.text().to_owned();
    assert!(matches!(
        document.apply_byte_edits(
            HistoryTag::Local(revision()),
            vec![(0..1, "X".into()), (0..2, "Y".into())],
        ),
        Err(TwoWayMergeError::OverlappingDestinationRanges)
    ));
    assert_eq!(document.text(), before);
}
