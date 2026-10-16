//! Retained document, side loading, and pair lifecycle behavior.

use super::*;

#[test]
fn single_side_edits_undo_redo_and_repeated_saves_use_fresh_fingerprints() {
    let root = std::env::temp_dir().join(format!("versus-single-save-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("only.txt");
    std::fs::write(&path, "old\n").unwrap();
    let mut view = partial_view(path.clone(), "old\n");
    view.workspace
        .as_mut()
        .unwrap()
        .edit_bytes(TwoWaySide::Left, 0..0, "first ".into())
        .unwrap();
    view.selected_side = 0;
    view.move_history(false);
    assert_eq!(view.working_text(0), Some("old\n"));
    view.move_history(true);
    assert_eq!(view.working_text(0), Some("first old\n"));
    let ctx = egui::Context::default();
    view.save_sides(vec![0]);
    settle_save(&mut view, &ctx);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "first old\n");
    assert!(!view.workspace.as_ref().unwrap().is_dirty(TwoWaySide::Left));
    view.workspace
        .as_mut()
        .unwrap()
        .set_text(TwoWaySide::Left, "second\n");
    view.save_sides(vec![0]);
    settle_save(&mut view, &ctx);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "second\n");
    assert!(!view.workspace.as_ref().unwrap().is_dirty(TwoWaySide::Left));
    let _ = std::fs::remove_dir_all(root);
}
#[test]
fn pending_or_failed_peer_load_preserves_healthy_side_editor_and_history() {
    let mut view = partial_view(PathBuf::from("left.txt"), "left\n");
    view.workspace
        .as_mut()
        .unwrap()
        .edit_bytes(TwoWaySide::Left, 0..0, "edited ".into())
        .unwrap();
    view.publish_active();
    view.selected_side = 0;
    view.scroll[0] = 37.0;
    let editor = egui::Id::new(("two-way-edit", view.side_generation[0], 0usize));
    let (sender, receiver) = mpsc::channel();
    view.side_sources[1] = Some(PathBuf::from("missing.txt"));
    view.side_generation[1] = view.side_generation[1].wrapping_add(1);
    view.activate_side_workspace();
    view.side_loading[1] = Some(SideLoad {
        receiver,
        path: PathBuf::from("missing.txt"),
    });
    assert_eq!(view.working_text(0), Some("edited left\n"));
    assert_eq!(view.scroll[0], 37.0);
    assert_eq!(
        editor,
        egui::Id::new(("two-way-edit", view.side_generation[0], 0usize))
    );
    view.move_history(false);
    assert_eq!(view.working_text(0), Some("left\n"));
    view.move_history(true);
    assert_eq!(view.working_text(0), Some("edited left\n"));
    sender.send(Err("unavailable".into())).unwrap();
    view.poll(&egui::Context::default());
    assert!(
        view.error
            .as_deref()
            .is_some_and(|error| error.contains("unavailable"))
    );
    assert_eq!(view.working_text(0), Some("edited left\n"));
    assert!(!view.has_actual_pair());
    let ctx = egui::Context::default();
    frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
    assert!(ctx.read_response(editor).is_some());
    assert!(
        ctx.read_response(egui::Id::new((
            "two-way-edit",
            view.side_generation[1],
            1usize,
        )))
        .is_none()
    );
}
#[test]
fn cached_side_reopen_promotes_to_a_canonical_pair() {
    let mut view = InteractiveView {
        editing: true,
        ..Default::default()
    };
    for (side, text) in ["left\n", "right\n"].into_iter().enumerate() {
        let path = PathBuf::from(if side == 0 { "left.txt" } else { "right.txt" });
        view.register_file(
            path.clone(),
            WorkingFile {
                identity: None,
                path: Some(path),
                text: text.into(),
                encoding: versus::TextEncoding::Utf8 { bom: false },
                fingerprint: versus::FileFingerprint::Bytes(text.as_bytes().to_vec()),
            },
        );
    }
    view.open_sources([None, None]);
    view.open_sources([
        Some(PathBuf::from("left.txt")),
        Some(PathBuf::from("right.txt")),
    ]);
    assert_eq!(
        view.pair_paths().unwrap(),
        &[PathBuf::from("left.txt"), PathBuf::from("right.txt")]
    );
    assert!(view.files.is_some() && view.workspace.is_some());
    view.set_run_diff(true);
    assert!(view.loaded_pair_comparison().is_none());
}
