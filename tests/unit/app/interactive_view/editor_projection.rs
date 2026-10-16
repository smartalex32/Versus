//! Editable projection, decorations, and caret behavior.

use super::*;
use crate::app::visuals::theme::set_theme;

#[test]
fn failed_diff_refresh_drops_provisional_decoration_without_losing_edits() {
    for disconnected in [false, true] {
        let ctx = egui::Context::default();
        let mut view = view("old\n", "new\n");
        view.run_diff = true;
        view.diff = compare_loaded_text(
            ["old\n", "new\n"],
            &AtomicBool::new(false),
            &FileViewOptions::default(),
            [&[], &[]],
        )
        .unwrap();
        view.workspace
            .as_mut()
            .unwrap()
            .edit_bytes(TwoWaySide::Left, 0..0, "!".into())
            .unwrap();
        view.invalidate_edited_diff(0, "old\n", 0..0);
        assert!(view.presentation.has_pending());
        let (sender, receiver) = mpsc::channel();
        if !disconnected {
            sender.send(Err("Comparison failed".into())).unwrap();
        }
        drop(sender);
        view.diff_job = Some(DiffJob {
            receiver,
            cancel: Arc::default(),
            progress: Arc::default(),
        });
        view.diff_dirty = false;
        view.poll_diff(&ctx);
        assert!(!view.presentation.has_pending() && view.diff.is_none());
        assert!(view.error.is_some());
        assert_eq!(view.working_text(0), Some("!old\n"));
        view.selected_side = 0;
        view.move_history(false);
        assert_eq!(view.working_text(0), Some("old\n"));
    }
}

#[test]
fn pending_single_line_edits_keep_source_color_and_precise_inline_ranges() {
    let mut decoration = MergeDecoration::default();
    decoration
        .lines_mut()
        .insert(0, (DirectoryEntryState::LeftOnly, vec![2..4, 6..8]));
    decoration.rebase("abcdefgh\n", 0..0, "文abcdefgh\n");
    assert_eq!(
        decoration.lines()[&0],
        (DirectoryEntryState::LeftOnly, vec![0..3, 5..7, 9..11])
    );
    decoration.rebase("文abcdefgh\n", 5..7, "文abefgh\n");
    assert_eq!(
        decoration.lines()[&0],
        (DirectoryEntryState::LeftOnly, vec![0..3, 7..9])
    );
    decoration.rebase("文abefgh\n", 3..5, "文Xefgh\n");
    assert_eq!(
        decoration.lines()[&0],
        (DirectoryEntryState::LeftOnly, vec![0..4, 6..8])
    );
}

#[test]
fn pending_diff_decorations_follow_exact_byte_anchors_and_line_endings() {
    let before = "x\r\nx\nchanged\rkeep\r\n";
    let mut decoration = MergeDecoration::default();
    decoration
        .lines_mut()
        .insert(1, (DirectoryEntryState::Different, vec![0..1]));
    decoration
        .lines_mut()
        .insert(2, (DirectoryEntryState::Different, vec![0..7]));
    decoration
        .endings_mut()
        .insert(2, versus::DisplayLineEnding::Cr);
    let after = format!("x\r\n{before}");
    decoration.rebase(before, 0..0, &after);
    assert_eq!(decoration.lines()[&2].1, [0..1]);
    assert_eq!(decoration.lines()[&3].1, [0..7]);
    assert_eq!(decoration.ending(3).unwrap(), versus::DisplayLineEnding::Cr);
    // UTF-8 text inserted into that moved line gets valid highlight ranges.
    let byte = after.find("changed").unwrap();
    let mut next = after.clone();
    next.insert_str(byte, "文");
    decoration.rebase(&after, byte..byte, &next);
    assert_eq!(decoration.lines()[&3].1, [0.."文changed".len()]);
    assert_eq!(decoration.ending(3).unwrap(), versus::DisplayLineEnding::Cr);
}

#[test]
fn typing_with_unfiltered_diff_preserves_other_line_decorations_on_both_sides() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for side in 0..2usize {
            let ctx = egui::Context::default();
            set_theme(&ctx, theme);
            let mut view = view("same\r\nold\r\nother\r\n", "same\nnew\nanother\n");
            view.run_diff = true;
            view.merge_options = [false, false, true];
            view.diff = compare_loaded_text(
                [view.working_text(0).unwrap(), view.working_text(1).unwrap()],
                &AtomicBool::new(false),
                &FileViewOptions::default(),
                [&[], &[]],
            )
            .unwrap();
            for _ in 0..3 {
                frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
            }
            let original_peer = view.working_text(1 - side).unwrap().to_owned();
            let untouched = view.merge_decoration(side).lines()[&2].clone();
            view.caret[side] = 1;
            view.sync_editor_cursor(side);
            ctx.memory_mut(|memory| memory.request_focus(view.editor_id(side)));
            frame(&mut view, &ctx, vec![egui::Event::Text("文".into())])
                .drop_without_applying_deltas();
            view.diff_not_before = Some(Instant::now() + Duration::from_secs(1));
            frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
            assert_eq!(view.working_text(1 - side), Some(original_peer.as_str()));
            assert!(view.working_text(side).unwrap().contains(if side == 0 {
                "文old\r\n"
            } else {
                "文new\n"
            }));
            assert_eq!(view.merge_decoration(side).lines()[&2], untouched);
            assert_eq!(
                view.presentation
                    .editor_projection(side)
                    .unwrap()
                    .source_lines(),
                [0, 1, 2]
            );
            view.set_run_diff(false);
            assert!(!view.presentation.has_pending() && view.diff_job.is_none());
        }
    }
}

#[test]
fn typing_with_diff_keeps_filtered_content_and_highlights_between_results() {
    let left = "prefix\nold\nsame\nlater\nsuffix\n";
    let right = "prefix\nnew\nsame\nlaterR\nsuffix\n";
    let ctx = egui::Context::default();
    let mut view = view(left, right);
    view.run_diff = true;
    view.merge_options = [true; 3];
    view.diff = compare_loaded_text(
        [left, right],
        &AtomicBool::new(false),
        &FileViewOptions::default(),
        [&[], &[]],
    )
    .unwrap();
    for _ in 0..3 {
        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
    }
    view.diff_rows = view
        .diff
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| (row.state != DirectoryEntryState::Same).then_some(index))
        .collect();
    let counts = view.footer_legend_counts();
    let label = view.change_label();
    let stale_comparison = view.diff.as_ref().unwrap().clone();
    let (sender, receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    view.diff_job = Some(DiffJob {
        receiver,
        cancel: cancel.clone(),
        progress: Arc::default(),
    });
    let id = view.editor_id(0);
    let mut state = egui::TextEdit::load_state(&ctx, id).unwrap();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(0),
        )));
    state.store(&ctx, id);
    ctx.memory_mut(|memory| memory.request_focus(id));
    frame(&mut view, &ctx, vec![egui::Event::Text("!".into())]).drop_without_applying_deltas();
    assert!(cancel.load(Ordering::Relaxed));
    sender.send(Ok(Some(stale_comparison))).unwrap();
    view.diff_not_before = Some(Instant::now() + Duration::from_secs(1));
    frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
    assert!(
        view.loaded_pair_comparison().is_none(),
        "pending display metadata is not a completed comparison"
    );
    assert_eq!(view.footer_legend_counts(), counts);
    assert_eq!(view.change_label(), label);
    assert_eq!(
        view.working_text(0),
        Some("prefix\n!old\nsame\nlater\nsuffix\n")
    );
    assert_eq!(
        view.presentation.editor_projection(0).unwrap().text(),
        "!old\nlater\n",
        "editing must not briefly expand hidden unchanged lines while diff is pending"
    );
    assert_eq!(
        view.presentation.editor_projection(1).unwrap().text(),
        "new\nlaterR\n"
    );
    assert_eq!(
        view.merge_decoration(0).lines()[&1].0,
        DirectoryEntryState::Different,
        "diff decoration should remain visible while refreshing"
    );
    // Continue typing before any worker returns. Newlines must update the
    // compact projection's source coordinates without exposing hidden text.
    frame(&mut view, &ctx, vec![egui::Event::Text("β\nsecond".into())])
        .drop_without_applying_deltas();
    view.diff_not_before = Some(Instant::now() + Duration::from_secs(1));
    frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
    assert_eq!(
        view.presentation.editor_projection(0).unwrap().text(),
        "!β\nsecondold\nlater\n"
    );
    assert_eq!(
        view.presentation
            .editor_projection(0)
            .unwrap()
            .source_lines(),
        [1, 2, 4]
    );
    frame(&mut view, &ctx, vec![egui::Event::Text("?".into())]).drop_without_applying_deltas();
    assert_eq!(
        view.working_text(0),
        Some("prefix\n!β\nsecond?old\nsame\nlater\nsuffix\n")
    );
    // A deliberate selection joins only the two adjacent edited lines.
    frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
    let mut state = egui::TextEdit::load_state(&ctx, id).unwrap();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(2),
            egui::text::CCursor::new(3),
        )));
    state.store(&ctx, id);
    frame(
        &mut view,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::Backspace,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    )
    .drop_without_applying_deltas();
    view.diff_not_before = Some(Instant::now() + Duration::from_secs(1));
    frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
    assert_eq!(
        view.presentation.editor_projection(0).unwrap().text(),
        "!βsecond?old\nlater\n"
    );
    assert_eq!(
        view.presentation
            .editor_projection(0)
            .unwrap()
            .source_lines(),
        [1, 3]
    );
    assert!(view.filtered_merge_range_hidden(TwoWaySide::Left, &(1..4)));
    // Real comparison results take over without resetting editing/history.
    view.diff_not_before = None;
    for _ in 0..100 {
        view.poll_diff(&ctx);
        if view.diff.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(view.diff.is_some() && !view.presentation.has_pending());
    frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
    assert_eq!(
        view.presentation.editor_projection(0).unwrap().text(),
        "!βsecond?old\nlater\n"
    );
    view.move_history(false);
    assert_eq!(
        view.working_text(0),
        Some("prefix\n!β\nsecond?old\nsame\nlater\nsuffix\n")
    );
    view.set_run_diff(false);
    assert!(!view.presentation.has_pending() && view.diff.is_none());
}

#[test]
fn filtered_transfer_restores_the_destination_caret_before_further_typing() {
    let left = "prefix\nold\nsame\nlater\nsuffix\n";
    let right = "prefix\nnew\nsame\nlaterR\nsuffix\n";
    let ctx = egui::Context::default();
    set_theme(&ctx, egui::Theme::Dark);
    let mut view = view(left, right);
    view.editor_context = Some(ctx.clone());
    view.run_diff = true;
    view.merge_options = [true; 3];
    view.diff = compare_loaded_text(
        [left, right],
        &AtomicBool::new(false),
        &FileViewOptions::default(),
        [&[], &[]],
    )
    .unwrap();
    for _ in 0..3 {
        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
    }
    view.caret[1] = 3;
    view.transfer(TwoWaySide::Left, 3..4);
    frame(&mut view, &ctx, vec![egui::Event::Text("!".into())]).drop_without_applying_deltas();
    assert_eq!(
        view.working_text(1),
        Some("prefix\nnew\nsame\nlater\n!laterR\nsuffix\n")
    );
    assert_eq!(view.working_text(0), Some(left));
    view.move_history(false);
    view.move_history(false);
    assert_eq!(view.working_text(1), Some(right));
}

#[test]
fn cursor_remap_keeps_the_same_source_line_when_filtering_changes() {
    let full = EditorProjection::new("same\nchanged\nsame\nlater\n".into(), vec![0, 1, 2, 3]);
    let filtered = EditorProjection::new("changed\nlater\n".into(), vec![1, 3]);
    let cursor = egui::text::CCursorRange::one(egui::text::CCursor::new(18));
    let remapped = remap_projection_cursor(cursor, &full, &filtered);
    assert_eq!(usize::from(remapped.primary.index), 8);
    let restored = remap_projection_cursor(remapped, &filtered, &full);
    assert_eq!(usize::from(restored.primary.index), 18);
}
#[test]
fn folder_navigation_uses_changed_paths_without_comparing_text() {
    let ctx = egui::Context::default();
    let mut view = InteractiveView::default();
    view.editing = true;
    view.folder_overview = true;
    view.set_folder_differences(vec![PathBuf::from("nested/a"), PathBuf::from("b")]);
    assert!(!view.has_differences());
    view.run_diff = true;
    assert!(view.has_differences());
    view.jump_diff(true, &ctx);
    assert_eq!(view.take_folder_jump(), Some(PathBuf::from("nested/a")));
    view.jump_diff(true, &ctx);
    view.jump_diff(true, &ctx);
    assert_eq!(view.take_folder_jump(), Some(PathBuf::from("b")));
    view.jump_diff(false, &ctx);
    assert_eq!(view.take_folder_jump(), Some(PathBuf::from("nested/a")));
    assert!(view.diff_job.is_none());
}
#[test]
fn incomplete_optional_comparison_is_never_cached_as_identical() {
    let mut view = view("left\n", "right\n");
    view.run_diff = true;
    view.diff = Some(FileComparison {
        rows: Vec::new(),
        message: Some("Comparison budget reached".into()),
    });
    assert_eq!(
        view.loaded_pair_comparison().unwrap().1,
        DirectoryEntryState::Different
    );
}
#[test]
fn keyboard_redo_is_consumed_before_the_less_specific_undo_shortcut() {
    let ctx = egui::Context::default();
    let mut view = view("left\n", "right\n");
    view.caret[1] = 1;
    view.transfer(TwoWaySide::Left, 0..1);
    let key = |modifiers| egui::Event::Key {
        key: egui::Key::Z,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    };
    frame(&mut view, &ctx, vec![key(egui::Modifiers::COMMAND)]).drop_without_applying_deltas();
    assert_eq!(view.working_text(1), Some("right\n"));
    frame(
        &mut view,
        &ctx,
        vec![key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT)],
    )
    .drop_without_applying_deltas();
    assert_eq!(view.working_text(1), Some("right\nleft\n"));
}
#[test]
fn transfer_undo_targets_the_destination_after_source_edits() {
    let ctx = egui::Context::default();
    let mut view = view("left\n", "right\n");
    view.workspace
        .as_mut()
        .unwrap()
        .set_text(TwoWaySide::Left, "edited\n");
    frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
    view.selected_side = 0;
    view.caret[1] = 1;
    view.transfer(TwoWaySide::Left, 0..1);
    view.move_history(false);
    assert_eq!(view.working_text(0), Some("edited\n"));
    assert_eq!(view.working_text(1), Some("right\n"));
    view.move_history(true);
    assert_eq!(view.working_text(1), Some("right\nedited\n"));
}
