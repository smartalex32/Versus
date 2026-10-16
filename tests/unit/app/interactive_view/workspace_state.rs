//! Workspace state, edit transactions, and pending-diff behavior.

use super::*;
use crate::app::visuals::theme::ROW_HEIGHT;
use crate::app::visuals::theme::{comparison_font, set_theme};

#[test]
fn equal_folder_difference_results_adopt_the_new_cache_without_resetting_navigation() {
    let mut view = InteractiveView::default();
    let original = Arc::new(vec![PathBuf::from("changed.txt")]);
    view.set_folder_differences(original);
    view.folder_difference_index = Some(0);
    let refreshed = Arc::new(vec![PathBuf::from("changed.txt")]);
    view.set_folder_differences(refreshed.clone());
    assert!(Arc::ptr_eq(&view.folder_differences, &refreshed));
    assert_eq!(view.folder_difference_index, Some(0));
}

#[test]
fn diff_starts_disabled() {
    assert!(!InteractiveView::default().run_diff_enabled())
}
#[test]
fn activity_status_requires_a_running_action() {
    let ctx = egui::Context::default();
    let mut view = view("left\n", "right\n");
    view.status = "Saved 2 working copies.".into();
    let output = ctx.run_ui(egui::RawInput::default(), |ui| {
        assert!(!view.render_activity_status(ui));
    });
    output.drop_without_applying_deltas();
    assert_eq!(view.guidance_message(), "Saved 2 working copies.");
    let (_sender, receiver) = mpsc::channel();
    view.side_loading[0] = Some(SideLoad {
        receiver,
        path: "left.rs".into(),
    });
    let output = ctx.run_ui(egui::RawInput::default(), |ui| {
        assert!(view.render_activity_status(ui));
    });
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "Loading selected files…")));
    output.drop_without_applying_deltas();
    view.side_loading[0] = None;
    view.error = Some("Load failed".into());
    ctx.run_ui(egui::RawInput::default(), |ui| {
        assert!(
            !view.render_activity_status(ui),
            "errors are shown by the separate summary"
        );
    })
    .drop_without_applying_deltas();
}
#[test]
fn change_label_reports_count_before_navigation_and_position_afterward() {
    let ctx = egui::Context::default();
    let mut view = InteractiveView::default();
    view.folder_overview = true;
    view.folder_differences = Arc::new(vec![PathBuf::from("one"), PathBuf::from("two")]);
    assert_eq!(view.change_label(), "2 changes");
    view.jump_diff(true, &ctx);
    assert_eq!(view.change_label(), "Change 1 of 2");
}
#[test]
fn keyboard_edit_before_repeated_lines_preserves_caret_endings_and_history() {
    let ctx = egui::Context::default();
    let mut view = view("x\r\nx\np\r", "right\n");
    frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
    view.caret[0] = 0;
    view.sync_editor_cursor(0);
    ctx.memory_mut(|memory| memory.request_focus(view.editor_id(0)));
    frame(&mut view, &ctx, vec![egui::Event::Text("x\n".into())]).drop_without_applying_deltas();
    assert_eq!(view.working_text(0), Some("x\r\nx\r\nx\np\r"));
    view.move_history(false);
    assert_eq!(view.working_text(0), Some("x\r\nx\np\r"));
    view.move_history(true);
    assert_eq!(view.working_text(0), Some("x\r\nx\r\nx\np\r"));
    assert!(view.diff_job.is_none());
}

#[test]
fn byte_edit_handles_unicode_selection_backspace_and_delete() {
    for (original, edited, cursor, range, replacement) in [
        ("é\r\na\n", "é\nβ\n", 2..3, 4..5, "β"),
        ("x\r\nx\r\n", "xx\n", 2..2, 1..3, ""),
        ("x\r\nx\r\n", "x\nx", 3..3, 4..6, ""),
    ] {
        let edit = editor_byte_edit(original, edited, Some(cursor));
        assert_eq!(edit, (range, replacement.to_owned()));
    }
}

#[test]
fn filtered_merge_projection_edits_only_visible_source_lines_and_guards_hidden_seams() {
    let mut changed = BTreeMap::new();
    changed.insert(1, (DirectoryEntryState::Different, Vec::new()));
    changed.insert(3, (DirectoryEntryState::Different, Vec::new()));
    let original = "same\r\nold\r\nsame\r\nlater\r\ntrailing\r\n";
    let displayed = original.replace("\r\n", "\n");
    let projection = merge_projection(
        &displayed,
        &MergeDecoration::from_parts(changed, BTreeMap::new()),
    );
    assert_eq!(projection.text(), "old\nlater\n");
    assert_eq!(projection.source_lines().collect::<Vec<_>>(), [1, 3]);

    // Typing at the start of the second visible block maps to its source
    // line, rather than the preceding hidden seam.
    let (range, replacement) =
        projected_editor_edit(original, &projection, "old\n!later\n", None).unwrap();
    assert_eq!(&original[range.clone()], "");
    assert_eq!(replacement, "!");
    let mut edited = original.to_owned();
    edited.replace_range(range, &replacement);
    assert_eq!(edited, "same\r\nold\r\nsame\r\n!later\r\ntrailing\r\n");

    // Removing the newline before the omitted middle line must not join
    // the visible and hidden text in the backing document.
    assert!(projected_editor_edit(original, &projection, "oldlater\n", None).is_err());
    // The final visible newline is also protected when the remainder of
    // the source is hidden.
    assert!(projected_editor_edit(original, &projection, "old\nlater", None).is_err());
}

#[test]
fn filtered_merge_chunk_runs_never_cross_hidden_source_lines() {
    assert_eq!(
        visible_chunk_runs(&(0..6), &[0, 2, 3, 5]),
        vec![0..1, 2..4, 5..6]
    );
}

#[test]
fn merge_caret_uses_font_height_without_changing_row_spacing() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for zoom in [1.0, 2.0] {
            for text in ["first\nsecond\n", "first\n\nthird\n", ""] {
                for side in 0..2usize {
                    for run_diff in [false, true] {
                        let ctx = egui::Context::default();
                        set_theme(&ctx, theme);
                        ctx.set_zoom_factor(zoom);
                        ctx.style_mut_of(theme, |style| style.visuals.text_cursor.blink = false);
                        let stroke = ctx.style_of(theme).visuals.text_cursor.stroke;
                        let mut view = view(text, text);
                        view.run_diff = run_diff;
                        view.merge_options = [false; 3];
                        if run_diff {
                            view.diff = compare_loaded_text(
                                [text, text],
                                &AtomicBool::new(false),
                                &FileViewOptions::default(),
                                [&[], &[]],
                            )
                            .unwrap();
                        }
                        for _ in 0..3 {
                            frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                        }
                        view.caret[side] = usize::from(!text.is_empty());
                        view.sync_editor_cursor(side);
                        ctx.memory_mut(|memory| memory.request_focus(view.editor_id(side)));
                        let output = frame(&mut view, &ctx, vec![]);
                        assert_eq!(
                            ctx.style_of(theme).visuals.text_cursor.max_height,
                            None,
                            "merge caret styling must not leak to other text fields"
                        );
                        let font = comparison_font(&ctx);
                        let font_height = ctx.fonts_mut(|fonts| fonts.row_height(&font));
                        let cursors: Vec<_> = output
                            .shapes
                            .iter()
                            .filter_map(|shape| match &shape.shape {
                                egui::Shape::LineSegment {
                                    points,
                                    stroke: painted,
                                } if *painted == stroke && points[0].x == points[1].x => {
                                    Some((points[1].y - points[0].y).abs())
                                }
                                _ => None,
                            })
                            .collect();
                        assert_eq!(
                            cursors.len(),
                            1,
                            "one focused editor caret: {theme:?}, zoom={zoom}, side={side}"
                        );
                        assert!(
                            (cursors[0] - (font_height + 3.0)).abs() < 0.1,
                            "caret should use the font height with native caret margins, not padded rows: height={}, font={font_height}, {theme:?}, zoom={zoom}, side={side}, diff={run_diff}, text={text:?}",
                            cursors[0]
                        );
                        for shape in &output.shapes {
                            if let egui::Shape::Text(text_shape) = &shape.shape {
                                if !text.is_empty() && text_shape.galley.text() == text {
                                    let rows = &text_shape.galley.rows;
                                    assert!(
                                        (rows[1].pos.y
                                            - rows[0].pos.y
                                            - ROW_HEIGHT.max(font_height.ceil() + 8.0))
                                        .abs()
                                            < 0.1
                                    );
                                }
                            }
                        }
                        output.drop_without_applying_deltas();
                    }
                }
            }
        }
    }
}

#[test]
fn merge_undo_redo_shortcuts_keep_the_diff_view_stable() {
    let left = "prefix\r\nold\r\nsame\r\nlater\r\nsuffix\r\n";
    let right = "prefix\nnew\nsame\nlaterR\nsuffix\n";
    for side in 0..2usize {
        for filtered in [true, false] {
            for settled in [false, true] {
                let ctx = egui::Context::default();
                let mut view = view(left, right);
                view.run_diff = true;
                view.merge_options = [filtered, true, true];
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
                view.caret[side] = 1;
                view.sync_editor_cursor(side);
                ctx.memory_mut(|memory| memory.request_focus(view.editor_id(side)));
                frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                frame(
                    &mut view,
                    &ctx,
                    vec![egui::Event::Text("!β\nsecond".into())],
                )
                .drop_without_applying_deltas();
                view.diff_not_before = Some(Instant::now() + Duration::from_secs(60));
                frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                if settled {
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
                }
                let edited = view.working_text(side).unwrap().to_owned();
                let edited_projection = view
                    .presentation
                    .editor_projection(side)
                    .unwrap()
                    .text()
                    .to_owned();
                let original = if side == 0 { left } else { right };
                for key in [egui::Key::Z, egui::Key::Y] {
                    let events = [true, false]
                        .into_iter()
                        .map(|pressed| egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed,
                            repeat: false,
                            modifiers: egui::Modifiers::CTRL,
                        })
                        .collect();
                    frame(&mut view, &ctx, events).drop_without_applying_deltas();
                    let expected = if key == egui::Key::Z {
                        if filtered {
                            if side == 0 {
                                "old\nlater\n".into()
                            } else {
                                "new\nlaterR\n".into()
                            }
                        } else {
                            original.replace("\r\n", "\n")
                        }
                    } else {
                        edited_projection.clone()
                    };
                    assert_eq!(
                        view.working_text(side),
                        Some(if key == egui::Key::Z {
                            original
                        } else {
                            &edited
                        })
                    );
                    assert_eq!(
                        view.presentation.editor_projection(side).unwrap().text(),
                        expected,
                        "Ctrl+{key:?} must not expand/redecorate the file while refreshing: side={side}, filtered={filtered}"
                    );
                    assert!(view.presentation.has_pending());
                    view.diff_not_before = Some(Instant::now() + Duration::from_secs(60));
                }
                frame(&mut view, &ctx, vec![egui::Event::Text("?".into())])
                    .drop_without_applying_deltas();
                assert!(
                    view.working_text(side).unwrap().contains("second?"),
                    "typing after redo should continue at the edited position: side={side}, filtered={filtered}, text={:?}",
                    view.working_text(side)
                );
                assert_eq!(
                    view.working_text(1 - side),
                    Some(if side == 0 { right } else { left })
                );
            }
        }
    }
}

#[test]
fn replacement_batch_undo_redo_preserves_both_diff_panes() {
    let left = "prefix\nleft match\nsuffix\n";
    let right = "prefix\nright match\nsuffix\n";
    let ctx = egui::Context::default();
    let mut view = view(left, right);
    let preview = preview_for(
        &view,
        &[("left.rs", 12..17, "found"), ("right.rs", 13..18, "found")],
    );
    view.apply_search_preview(&preview).unwrap();
    view.run_diff = true;
    view.merge_options = [true; 3];
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
    for (key, word) in [(egui::Key::Z, "match"), (egui::Key::Y, "found")] {
        let events = [true, false]
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::CTRL,
            })
            .collect();
        frame(&mut view, &ctx, events).drop_without_applying_deltas();
        assert_eq!(
            view.presentation.editor_projection(0).unwrap().text(),
            format!("left {word}\n")
        );
        assert_eq!(
            view.presentation.editor_projection(1).unwrap().text(),
            format!("right {word}\n")
        );
        assert_eq!(
            view.working_text(0),
            Some(format!("prefix\nleft {word}\nsuffix\n").as_str())
        );
        assert_eq!(
            view.working_text(1),
            Some(format!("prefix\nright {word}\nsuffix\n").as_str())
        );
        assert!(view.presentation.has_pending());
        view.diff_not_before = Some(Instant::now() + Duration::from_secs(60));
    }
}
