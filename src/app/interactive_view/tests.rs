//! Regression coverage for the interactive editor workspace.

use super::*;
use versus::SearchChange;

impl InteractiveView {
    pub(crate) fn current_paths(&self) -> Option<&[PathBuf; 2]> {
        self.paths.as_ref()
    }

    pub(crate) fn working_text(&self, side: usize) -> Option<&str> {
        self.workspace
            .as_ref()
            .map(|workspace| workspace.text(side_of(side)))
    }

    pub(crate) fn editor_id(&self, side: usize) -> egui::Id {
        egui::Id::new(("two-way-edit", self.side_generation[side], side))
    }

    pub(crate) fn set_working_text_for_test(&mut self, side: usize, text: &str) {
        self.workspace
            .as_mut()
            .expect("loaded test file has a workspace")
            .set_text(side_of(side), text);
    }

    pub(crate) fn ingest_working_files(&mut self, files: &BTreeMap<PathBuf, WorkingFile>) {
        self.publish_active();
        for (path, file) in files {
            let key = file.identity_key();
            self.aliases.insert(path.clone(), key.clone());
            self.aliases.insert(key.clone(), key.clone());
            if let Some(retained) = self.documents.get_mut(&key) {
                if retained.document.text() != file.text {
                    let mut facade = TwoWayWorkspace::from_documents([
                        retained.document.clone(),
                        InteractiveDocument::new(""),
                    ]);
                    facade.set_text(TwoWaySide::Left, file.text.clone());
                    retained.document = facade.document(TwoWaySide::Left).clone();
                }
                retained.file = file.clone();
                if !file.is_dirty() {
                    retained.document.mark_saved_text(file.text.clone());
                }
            } else {
                self.documents.insert(key, retained_file(file.clone()));
            }
        }
        self.refresh_active();
        self.invalidate_diff();
    }

    pub(crate) fn render(&mut self, ui: &mut egui::Ui) {
        let (_, status) = self.render_with_sources(ui, |_| {});
        ui.scope_builder(egui::UiBuilder::new().max_rect(status), |ui| {
            let errors: Vec<_> = self
                .error_details()
                .map(|error| error.lines().map(str::to_owned).collect())
                .unwrap_or_default();
            bottom_status_area(ui, |ui| {
                if errors.is_empty() {
                    self.render_activity_status(ui);
                } else {
                    error_summary(ui, &errors);
                }
            });
        });
    }
}

fn restore_line_endings(original: &str, edited: &str) -> String {
    let (range, replacement) = editor_byte_edit(original, edited, None);
    let mut text = original.to_owned();
    text.replace_range(range, &replacement);
    text
}

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
                                            - super::ROW_HEIGHT.max(font_height.ceil() + 8.0))
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
fn view(left: &str, right: &str) -> InteractiveView {
    let paths = [PathBuf::from("left.rs"), PathBuf::from("right.rs")];
    let files = std::array::from_fn(|side| {
        let text = if side == 0 { left } else { right };
        WorkingFile {
            identity: None,
            path: Some(paths[side].clone()),
            text: text.into(),
            encoding: versus::TextEncoding::Utf8 { bom: false },
            fingerprint: versus::FileFingerprint::Bytes(text.as_bytes().to_vec()),
        }
    });
    InteractiveView {
        workspace: Some(TwoWayWorkspace::new(left, right)),
        files: Some(files),
        paths: Some(paths),
        editing: true,
        ..Default::default()
    }
}
fn readonly_comparison(lines: usize) -> FileComparison {
    FileComparison {
        rows: (0..lines)
            .map(|index| versus::FileComparisonRow {
                left: Some((index + 1, format!("left\tline {index}"))),
                right: Some((index + 1, format!("right line {index}"))),
                state: if index % 3 == 0 {
                    DirectoryEntryState::Different
                } else {
                    DirectoryEntryState::Same
                },
                left_changed: Vec::new(),
                right_changed: Vec::new(),
                left_ending: Some(versus::DisplayLineEnding::Lf),
                right_ending: Some(versus::DisplayLineEnding::Lf),
            })
            .collect(),
        message: None,
    }
}
fn partial_view(path: PathBuf, text: &str) -> InteractiveView {
    let file = WorkingFile {
        identity: None,
        path: Some(path.clone()),
        text: text.into(),
        encoding: versus::TextEncoding::Utf8 { bom: false },
        fingerprint: versus::FileFingerprint::Bytes(text.as_bytes().to_vec()),
    };
    let mut view = InteractiveView {
        editing: true,
        ..Default::default()
    };
    view.register_file(path.clone(), file.clone());
    view.side_sources[0] = Some(path);
    view.side_files[0] = Some(file);
    view.activate_side_workspace();
    view
}
fn settle_save(view: &mut InteractiveView, ctx: &egui::Context) {
    for _ in 0..100 {
        view.poll_save(ctx);
        if !view.is_saving() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("save did not finish");
}
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
fn frame(
    view: &mut InteractiveView,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(900.0 / ctx.zoom_factor(), 650.0 / ctx.zoom_factor()),
            )),
            events,
            ..Default::default()
        },
        |ui| view.render(ui),
    )
}
fn click(view: &mut InteractiveView, ctx: &egui::Context, id: egui::Id) {
    click_frame(view, ctx, id).drop_without_applying_deltas();
}
fn click_frame(view: &mut InteractiveView, ctx: &egui::Context, id: egui::Id) -> egui::FullOutput {
    let position = ctx.read_response(id).unwrap().rect.center();
    let pointer = |pressed| {
        vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    frame(view, ctx, pointer(true)).drop_without_applying_deltas();
    frame(view, ctx, pointer(false))
}
#[test]
fn merge_difference_navigation_highlights_and_scrolls_only_corresponding_sides() {
    let mut comparison = readonly_comparison(240);
    for (index, row) in comparison.rows.iter_mut().enumerate() {
        let modified = row.state != DirectoryEntryState::Same;
        row.left.as_mut().unwrap().1 = if modified {
            format!("left change {index}")
        } else {
            format!("shared line {index}")
        };
        row.right.as_mut().unwrap().1 = if modified {
            format!("right change {index}")
        } else {
            format!("shared line {index}")
        };
        if index > 40 {
            row.right.as_mut().unwrap().0 -= 1;
        }
        if index > 130 {
            row.left.as_mut().unwrap().0 -= 1;
        }
    }
    comparison.rows[40].state = DirectoryEntryState::LeftOnly;
    comparison.rows[40].right = None;
    comparison.rows[100].state = DirectoryEntryState::Different;
    comparison.rows[130].state = DirectoryEntryState::RightOnly;
    comparison.rows[130].left = None;
    let texts: [String; 2] = std::array::from_fn(|side| {
        comparison
            .rows
            .iter()
            .filter_map(|row| {
                if side == 0 {
                    row.left.as_ref()
                } else {
                    row.right.as_ref()
                }
            })
            .map(|line| format!("{}\n", line.1))
            .collect()
    });
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for zoom in [1.0, 2.0] {
            for filtered in [false, true] {
                let ctx = egui::Context::default();
                set_theme(&ctx, theme);
                ctx.set_zoom_factor(zoom);
                let mut view = view(&texts[0], &texts[1]);
                view.editor_context = Some(ctx.clone());
                view.run_diff = true;
                view.merge_options = [filtered, false, false];
                view.diff_rows = comparison
                    .rows
                    .iter()
                    .enumerate()
                    .filter_map(|(index, row)| {
                        (row.state != DirectoryEntryState::Same).then_some(index)
                    })
                    .collect();
                view.diff = Some(comparison.clone());
                for (target, forward, source_lines, filtered_lines) in [
                    (
                        0usize,
                        true,
                        [Some(0usize), Some(0usize)],
                        [Some(0usize), Some(0usize)],
                    ),
                    (40, true, [Some(40), None], [Some(14), None]),
                    (100, true, [Some(100), Some(99)], [Some(35), Some(34)]),
                    (130, true, [None, Some(129)], [None, Some(45)]),
                    (100, false, [Some(100), Some(99)], [Some(35), Some(34)]),
                ] {
                    view.scroll = [66.0, 88.0];
                    let target_index = view
                        .diff_rows
                        .iter()
                        .position(|index| *index == target)
                        .unwrap();
                    ctx.memory_mut(|memory| {
                        memory.data.insert_temp(
                            egui::Id::new("interactive-diff-navigation"),
                            if forward {
                                (target_index + view.diff_rows.len() - 1) % view.diff_rows.len()
                            } else {
                                target_index + 1
                            },
                        )
                    });
                    for _ in 0..3 {
                        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                    }
                    let before = view.scroll;
                    click(
                        &mut view,
                        &ctx,
                        egui::Id::new(if forward {
                            "Next difference"
                        } else {
                            "Previous difference"
                        }),
                    );
                    assert_eq!(view.navigation_lines(&ctx), source_lines);
                    let font = comparison_font(&ctx);
                    let row_height = super::ROW_HEIGHT
                        .max(ctx.fonts_mut(|fonts| fonts.row_height(&font)).ceil() + 8.0);
                    for side in 0..2 {
                        let display_line = if filtered {
                            filtered_lines[side]
                        } else {
                            source_lines[side]
                        };
                        assert!(
                            (view.scroll[side]
                                - display_line
                                    .map_or(before[side], |line| line.saturating_sub(1) as f32
                                        * row_height))
                            .abs()
                                < 0.5
                        );
                    }
                    let output = frame(&mut view, &ctx, vec![]);
                    let highlights: Vec<_> = output
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::Shape::Rect(rect)
                                if rect.fill
                                    == Palette::for_context(&ctx).accent.gamma_multiply(0.13) =>
                            {
                                Some((rect.rect, shape.clip_rect))
                            }
                            _ => None,
                        })
                        .collect();
                    assert_eq!(highlights.len(), source_lines.iter().flatten().count());
                    for (rect, clip) in highlights {
                        assert!(
                            rect.intersect(clip).is_positive(),
                            "navigated row must be visible"
                        );
                        let side = usize::from(rect.center().x > ctx.viewport_rect().center().x);
                        if source_lines[side].is_some_and(|line| line > 0) {
                            assert!(
                                rect.top() - clip.top() >= row_height - 1.0,
                                "leave a line above the highlighted change"
                            );
                        }
                        assert!(
                            source_lines[side].is_some(),
                            "do not highlight an unmatched opposite line"
                        );
                    }
                    output.drop_without_applying_deltas();
                }
                view.set_run_diff(false);
                let output = frame(&mut view, &ctx, vec![]);
                assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == Palette::for_context(&ctx).accent.gamma_multiply(0.13))));
                assert_eq!(view.working_text(0), Some(texts[0].as_str()));
                assert_eq!(view.working_text(1), Some(texts[1].as_str()));
                assert!(!view.workspace.as_ref().unwrap().can_undo());
                output.drop_without_applying_deltas();
            }
        }
    }
}

#[test]
fn merge_navigation_near_file_end_settles_on_the_click_frame() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for zoom in [1.0, 2.0] {
            for count in [12, 240] {
                for filtered in [false, true] {
                    let ctx = egui::Context::default();
                    set_theme(&ctx, theme);
                    ctx.set_zoom_factor(zoom);
                    let text: String = (0..count)
                        .map(|line| format!("fn chunk_{line}() {{}}\n"))
                        .collect();
                    let mut view = view(&text, &text);
                    view.editor_context = Some(ctx.clone());
                    view.run_diff = true;
                    view.merge_options = [filtered, false, false];
                    view.diff = Some(readonly_comparison(count));
                    view.diff_rows = vec![count - 3];
                    for _ in 0..3 {
                        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                    }
                    let geometry = |output: &egui::FullOutput| {
                        let highlights: Vec<_> = output
                            .shapes
                            .iter()
                            .filter_map(|shape| match &shape.shape {
                                egui::Shape::Rect(rect)
                                    if rect.fill
                                        == Palette::for_context(&ctx)
                                            .accent
                                            .gamma_multiply(0.13) =>
                                {
                                    Some((rect.rect, shape.clip_rect))
                                }
                                _ => None,
                            })
                            .collect();
                        let text: Vec<_> = output
                            .shapes
                            .iter()
                            .filter_map(|shape| match &shape.shape {
                                egui::Shape::Text(text)
                                    if text.galley.text().contains("chunk_") =>
                                {
                                    Some((text.pos, text.galley.size(), shape.clip_rect))
                                }
                                _ => None,
                            })
                            .collect();
                        (highlights, text)
                    };
                    for action in ["Next difference", "Previous difference", "Next difference"] {
                        let first = click_frame(&mut view, &ctx, egui::Id::new(action));
                        let first_geometry = geometry(&first);
                        first.drop_without_applying_deltas();
                        assert_eq!(first_geometry.0.len(), 2);
                        assert_eq!(first_geometry.1.len(), 2);
                        let clicked_scroll = view.scroll;
                        for _ in 0..4 {
                            let output = frame(&mut view, &ctx, vec![]);
                            let settled_geometry = geometry(&output);
                            output.drop_without_applying_deltas();
                            assert_eq!(
                                first_geometry, settled_geometry,
                                "{action} must settle on the first frame, including when the target cannot scroll to the top: theme={theme:?}, zoom={zoom}, count={count}, filtered={filtered}"
                            );
                            assert_eq!(view.scroll, clicked_scroll);
                        }
                    }
                    assert_eq!(view.working_text(0), Some(text.as_str()));
                    assert_eq!(view.working_text(1), Some(text.as_str()));
                    assert!(!view.workspace.as_ref().unwrap().can_undo());
                }
            }
        }
    }
}

#[test]
fn readonly_difference_navigation_leaves_a_context_row_and_clamps_at_start() {
    for filtered in [false, true] {
        let ctx = egui::Context::default();
        set_theme(&ctx, egui::Theme::Dark);
        let mut view = view("left\n", "right\n");
        view.editing = false;
        view.diff = Some(readonly_comparison(240));
        view.diff_rows = (0..240).step_by(3).collect();
        view.diff_options = [filtered, false, false];
        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        let font = comparison_font(&ctx);
        let row_height =
            super::ROW_HEIGHT.max(ctx.fonts_mut(|fonts| fonts.row_height(&font)).ceil() + 8.0);
        for (forward, display_line) in [
            (true, 0usize),
            (true, if filtered { 1 } else { 3 }),
            (true, if filtered { 2 } else { 6 }),
            (false, if filtered { 1 } else { 3 }),
            (false, 0),
        ] {
            view.jump_diff(forward, &ctx);
            assert_eq!(
                view.scroll,
                [display_line.saturating_sub(1) as f32 * row_height; 2]
            );
        }
    }
}

#[test]
fn readonly_presentation_is_reused_and_rebuilds_for_filter_changes() {
    let ctx = egui::Context::default();
    let mut view = view("left\n", "right\n");
    view.editing = false;
    view.diff = Some(readonly_comparison(12));
    view.diff_rows = vec![0, 3, 6, 9];
    for _ in 0..2 {
        ctx.run_ui(egui::RawInput::default(), |ui| {
            view.ensure_readonly_presentation(ui);
        })
        .drop_without_applying_deltas();
    }
    let first_rows = view.readonly_presentation.as_ref().unwrap().rows.as_ptr();
    let first_widths = view.readonly_presentation.as_ref().unwrap().content_widths;
    ctx.run_ui(egui::RawInput::default(), |ui| {
        view.ensure_readonly_presentation(ui);
    })
    .drop_without_applying_deltas();
    assert_eq!(
        view.readonly_presentation.as_ref().unwrap().rows.as_ptr(),
        first_rows,
        "unchanged frames must reuse the cached row list"
    );
    let longest_left = (0..12)
        .map(|index| format!("left\tline {index}").len() + 3)
        .max()
        .unwrap();
    let expected_left_width = super::file_style::TEXT_OFFSET
        + 28.0
        + longest_left as f32 * view.readonly_presentation.as_ref().unwrap().font.size;
    assert_eq!(first_widths[0], expected_left_width);
    let mut row_height = 0.0;
    ctx.run_ui(egui::RawInput::default(), |ui| {
        row_height = super::comparison_row_height(ui);
    })
    .drop_without_applying_deltas();
    // The old top row is source row 5. Filtering should retain the first
    // surviving source row at or after it (row 6), not reinterpret pixel
    // offset five as display row five.
    view.scroll = [5.0 * row_height + 0.5; 2];
    view.diff_options[0] = true;
    ctx.run_ui(egui::RawInput::default(), |ui| {
        view.ensure_readonly_presentation(ui);
    })
    .drop_without_applying_deltas();
    let cached = view.readonly_presentation.as_ref().unwrap();
    assert_eq!(cached.preferences, [true, false, false]);
    assert_eq!(cached.rows, vec![0, 3, 6, 9]);
    assert!((view.scroll[0] - (2.0 * row_height + 0.5)).abs() < 0.01);
}

#[test]
fn readonly_refresh_remaps_the_source_line_anchor_and_merge_keeps_its_scroll() {
    let ctx = egui::Context::default();
    let mut view = view("left\n", "right\n");
    view.editing = false;
    view.diff = Some(readonly_comparison(12));
    view.diff_rows = vec![0, 3, 6, 9];
    let mut row_height = 0.0;
    ctx.run_ui(egui::RawInput::default(), |ui| {
        row_height = super::comparison_row_height(ui);
        view.ensure_readonly_presentation(ui);
    })
    .drop_without_applying_deltas();
    // Source line 10 is at aligned display index 9 in the old result.
    view.scroll = [9.0 * row_height + 0.5; 2];
    view.invalidate_diff();
    let mut refreshed = readonly_comparison(12);
    for inserted in 0..2 {
        refreshed.rows.insert(
            inserted,
            versus::FileComparisonRow {
                left: None,
                right: Some((inserted + 1, format!("inserted {inserted}"))),
                state: DirectoryEntryState::RightOnly,
                left_changed: Vec::new(),
                right_changed: Vec::new(),
                left_ending: None,
                right_ending: Some(versus::DisplayLineEnding::Lf),
            },
        );
    }
    view.diff = Some(refreshed);
    view.diff_rows = vec![0, 1, 2, 5, 8, 11];
    ctx.run_ui(egui::RawInput::default(), |ui| {
        view.ensure_readonly_presentation(ui);
    })
    .drop_without_applying_deltas();
    let cache = view.readonly_presentation.as_ref().unwrap();
    let top = (view.scroll[0] / row_height).floor() as usize;
    let row = &view.diff.as_ref().unwrap().rows[cache.rows[top]];
    assert_eq!(row.left.as_ref().map(|line| line.0), Some(10));
    assert!((view.scroll[0] % row_height - 0.5).abs() < 0.01);

    view.editing = true;
    view.scroll = [37.0, 71.0];
    view.invalidate_diff();
    assert!(view.readonly_pending_anchor.is_none());
    assert_eq!(view.scroll, [37.0, 71.0]);
}

#[test]
fn merge_editor_keeps_raw_tabs_syntax_and_a_clear_right_transfer_gutter() {
    let ctx = egui::Context::default();
    let mut view = view("\tfn source() {}\n", "target\n");
    let right_chunk = view.workspace.as_ref().unwrap().chunks(TwoWaySide::Right)[0].id;
    // Run Diff is intentionally off: the editable source view still keeps
    // its syntax formatting and byte-identical TextEdit buffer.
    let output = frame(&mut view, &ctx, vec![]);
    let source = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text().contains("fn source") => Some(text),
            _ => None,
        })
        .expect("source line should be rendered");
    assert!(source.galley.text().contains('\t'));
    assert!(
        source
            .galley
            .job
            .sections
            .iter()
            .any(|section| section.format.color == Palette::for_context(&ctx).accent)
    );
    let right_arrow = ctx
        .read_response(egui::Id::new((
            "two-way-transfer",
            1usize,
            right_chunk,
            0usize,
        )))
        .unwrap()
        .rect;
    let right_editor = ctx.read_response(view.editor_id(1)).unwrap().rect;
    // The text anchor is 68 points after the shifted number/status gutter:
    // it clears the right-side 58 point transfer area and its 30 point arrow.
    assert!(right_editor.left() - right_arrow.right() >= 70.0);
    output.drop_without_applying_deltas();
}

#[test]
fn right_pane_wheel_updates_linked_readonly_scroll_in_the_same_frame() {
    let ctx = egui::Context::default();
    let mut view = view("left\n", "right\n");
    view.editing = false;
    view.diff = Some(readonly_comparison(120));
    view.diff_rows = (0..120).filter(|index| index % 3 == 0).collect();
    frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
    let right_pane = ctx
        .read_response(egui::Id::new(("interactive-readonly-pane", 0u64, 1usize)))
        .unwrap()
        .rect
        .center();
    frame(&mut view, &ctx, vec![egui::Event::PointerMoved(right_pane)])
        .drop_without_applying_deltas();
    frame(
        &mut view,
        &ctx,
        vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -8.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        }],
    )
    .drop_without_applying_deltas();
    assert!(
        view.scroll[1] > 0.0,
        "right pane should consume its wheel event; pane={right_pane:?}, scroll={:?}",
        view.scroll
    );
    assert_eq!(view.scroll[0], view.scroll[1]);
}
#[test]
fn merge_content_fills_each_pane_and_horizontal_bars_stay_at_the_bottom() {
    let wide_line = "long content ".repeat(100);
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for zoom in [1.0, 2.0] {
            for text in [
                String::new(),
                wide_line.clone(),
                format!("{wide_line}\n").repeat(100),
            ] {
                let ctx = egui::Context::default();
                set_theme(&ctx, theme);
                ctx.set_zoom_factor(zoom);
                let mut view = view(&text, &text);
                for _ in 0..3 {
                    frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                }
                for side in 0..2usize {
                    let track = ctx
                        .read_response(egui::Id::new(("merge-transfer-track", side)))
                        .unwrap()
                        .rect;
                    let editor = ctx
                        .read_response(egui::Id::new((
                            "two-way-edit",
                            view.side_generation[side],
                            side,
                        )))
                        .unwrap()
                        .rect;
                    assert!(
                        editor.height() >= track.height() - 12.0,
                        "content must fill even an empty pane: {theme:?}, {zoom}, {editor:?}, {track:?}"
                    );
                    if !text.is_empty() {
                        let point = egui::pos2(
                            if side == 0 {
                                track.left() - 100.0
                            } else {
                                track.right() + 100.0
                            },
                            track.bottom() - 8.0,
                        );
                        // Long files must keep their horizontal bar visible
                        // both at the start and after independent vertical scrolling.
                        for offset in [0.0, 120.0] {
                            if offset > 0.0 && text.lines().count() == 1 {
                                continue;
                            }
                            view.scroll[side] = offset;
                            frame(&mut view, &ctx, vec![egui::Event::PointerMoved(point)])
                                .drop_without_applying_deltas();
                            let output =
                                frame(&mut view, &ctx, vec![egui::Event::PointerMoved(point)]);
                            assert_eq!(
                                output.platform_output.cursor_icon,
                                egui::CursorIcon::PointingHand,
                                "horizontal bar should be at pane bottom: {theme:?}, {zoom}, side={side}, lines={}, offset={offset}, point={point:?}",
                                text.lines().count()
                            );
                            output.drop_without_applying_deltas();
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn merge_editors_fill_the_panes_without_lower_management_controls() {
    let text: String = (0..200)
        .map(|line| format!("fn item_{line}() {{}}\n"))
        .collect();
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for zoom in [1.0, 2.0] {
            let ctx = egui::Context::default();
            set_theme(&ctx, theme);
            ctx.set_zoom_factor(zoom);
            let mut view = view(&text, &text);
            for _ in 0..3 {
                frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
            }
            let output = frame(&mut view, &ctx, vec![]);
            for label in [
                "Selected lines",
                "Transfer →",
                "← Transfer",
                "Insert before line",
                "Replace selected target",
                "Create chunk",
                "Ignore range",
                "Manage ignored…",
            ] {
                assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
                        egui::Shape::Text(text) if text.galley.text() == label)));
            }
            let tracks: [Rect; 2] = std::array::from_fn(|side| {
                ctx.read_response(egui::Id::new(("merge-transfer-track", side)))
                    .unwrap()
                    .rect
            });
            let editors: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text().contains("fn item_0()") => {
                        Some(shape.clip_rect)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(editors.len(), 2);
            for rect in editors {
                let side = usize::from(rect.center().x > ctx.viewport_rect().center().x);
                assert!(
                    rect.height() >= tracks[side].height() - 12.0,
                    "editor should reclaim the lower controls area: editor={rect:?}, track={:?}",
                    tracks[side]
                );
            }
            output.drop_without_applying_deltas();
        }
    }
}

#[test]
fn merge_without_diff_keeps_preferences_and_history_across_modes() {
    let ctx = egui::Context::default();
    let mut view = view("left\n", "unrelated\n");
    view.merge_options = [false, true, false];
    view.caret[1] = 1;
    view.transfer(TwoWaySide::Left, 0..1);
    view.invalidate_diff();
    view.poll(&ctx);
    assert!(view.diff_job.is_none() && view.diff.is_none());
    let result = view.working_text(1).unwrap().to_owned();
    view.set_editing(false, [true, true, true]);
    std::thread::sleep(Duration::from_millis(155));
    view.poll(&ctx);
    assert!(view.diff_job.is_some() || view.diff.is_some());
    view.set_editing(true, [true, true, true]);
    view.poll(&ctx);
    assert_eq!(view.merge_options, [false, true, false]);
    assert_eq!(view.diff_options, [true, true, true]);
    assert!(view.diff_job.is_none() && view.diff.is_none());
    assert_eq!(view.working_text(1), Some(result.as_str()));
    assert!(view.workspace.as_mut().unwrap().undo());
    assert_eq!(view.working_text(1), Some("unrelated\n"));
    assert!(view.workspace.as_mut().unwrap().redo());
    assert_eq!(view.working_text(1), Some(result.as_str()));
}

#[test]
fn transfer_tracks_are_outside_content_and_follow_independent_source_scrolls() {
    let left: String = (0..35)
        .map(|i| format!("fn block_{i}() {{\n    left_{i}();\n}}\n"))
        .collect();
    let right: String = (0..35)
        .map(|i| format!("fn block_{i}() {{\n    right_{i}();\n}}\n"))
        .collect();
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for zoom in [1.0, 2.0] {
            for run_diff in [false, true] {
                let ctx = egui::Context::default();
                set_theme(&ctx, theme);
                ctx.set_zoom_factor(zoom);
                let mut view = view(&left, &right);
                if run_diff {
                    view.set_run_diff(true);
                    view.diff = compare_loaded_text(
                        [&left, &right],
                        &AtomicBool::new(false),
                        &FileViewOptions::default(),
                        [&[], &[]],
                    )
                    .unwrap();
                }
                let chunks = [
                    view.workspace.as_ref().unwrap().chunks(TwoWaySide::Left),
                    view.workspace.as_ref().unwrap().chunks(TwoWaySide::Right),
                ];
                for _ in 0..3 {
                    frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                }
                let output = frame(&mut view, &ctx, vec![]);
                let tracks: [Rect; 2] = std::array::from_fn(|side| {
                    ctx.read_response(egui::Id::new(("merge-transfer-track", side)))
                        .unwrap()
                        .rect
                });
                assert!(tracks[0].right() <= tracks[1].left());
                for side in 0..2 {
                    assert_eq!(tracks[side].width(), 38.0);
                    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.rect == tracks[side] && rect.fill == Palette::for_context(&ctx).background)));
                    let chunk = &chunks[side][0];
                    let line = chunk.range.start + usize::from(run_diff);
                    let arrow = ctx
                        .read_response(egui::Id::new(("two-way-transfer", side, chunk.id, line)))
                        .unwrap()
                        .rect;
                    let toggle = ctx
                        .read_response(egui::Id::new(("two-way-granularity", side, chunk.id)))
                        .unwrap()
                        .rect;
                    assert!(tracks[side].contains_rect(arrow));
                    if side == 0 {
                        assert!(toggle.right() + 5.0 <= tracks[side].left());
                    } else {
                        assert!(toggle.left() >= tracks[side].right() + 5.0);
                    }
                    let color = if side == 0 {
                        Palette::for_context(&ctx).left_only
                    } else {
                        Palette::for_context(&ctx).right_only
                    };
                    let head = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Path(path)
                                if path.stroke.color == egui::epaint::ColorMode::Solid(color)
                                    && path.points.iter().all(|point| arrow.contains(*point)) =>
                            {
                                Some((path, shape.clip_rect))
                            }
                            _ => None,
                        })
                        .expect("each track has a vector arrow");
                    assert!(
                        head.1.left() >= tracks[side].left()
                            && head.1.right() <= tracks[side].right()
                    );
                    assert_eq!(head.0.points[1].x > head.0.points[0].x, side == 0);
                }
                output.drop_without_applying_deltas();
                let first_left = chunks[0][0].range.start + usize::from(run_diff);
                let first_right = chunks[1][0].range.start + usize::from(run_diff);
                let left_id =
                    egui::Id::new(("two-way-transfer", 0usize, chunks[0][0].id, first_left));
                let right_id =
                    egui::Id::new(("two-way-transfer", 1usize, chunks[1][0].id, first_right));
                let left_y = ctx.read_response(left_id).unwrap().rect.top();
                let right_y = ctx.read_response(right_id).unwrap().rect.top();
                view.scroll[0] = 50.0;
                for _ in 0..3 {
                    frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
                }
                assert!(ctx.read_response(left_id).unwrap().rect.top() < left_y - 20.0);
                assert!((ctx.read_response(right_id).unwrap().rect.top() - right_y).abs() < 0.1);
                view.caret[0] = 0;
                click(&mut view, &ctx, right_id);
                let copied = if run_diff {
                    "    right_0();\n"
                } else {
                    "fn block_0() {\n    right_0();\n}\n"
                };
                assert_eq!(
                    view.working_text(0),
                    Some(format!("{copied}{left}").as_str())
                );
                assert_eq!(view.working_text(1), Some(right.as_str()));
            }
        }
    }
}

#[test]
fn actual_gutter_arrows_copy_chunks_and_lines_without_diff() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let ctx = egui::Context::default();
        set_theme(&ctx, theme);
        let mut view = view("fn source() {\n    one();\n    two();\n}\n", "target\n");
        view.caret[1] = 1;
        let output = frame(&mut view, &ctx, vec![]);
        let chunk = view.workspace.as_ref().unwrap().chunks(TwoWaySide::Left)[0].clone();
        let arrow = ctx
            .read_response(egui::Id::new((
                "two-way-transfer",
                0usize,
                chunk.id,
                chunk.range.start,
            )))
            .unwrap()
            .rect;
        let granularity = ctx
            .read_response(egui::Id::new(("two-way-granularity", 0usize, chunk.id)))
            .unwrap()
            .rect;
        assert!(arrow.width() >= 30.0);
        assert!(arrow.width() > granularity.width());
        let arrow_head = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Path(path)
                    if path.stroke.color
                        == egui::epaint::ColorMode::Solid(Palette::for_context(&ctx).left_only)
                        && path.points.iter().all(|point| arrow.contains(*point)) =>
                {
                    Some(path)
                }
                _ => None,
            })
            .expect("copy control should draw a vector arrow");
        assert_eq!(arrow_head.stroke.width, 2.0);
        assert!(arrow_head.points[1].x > arrow_head.points[0].x);
        output.drop_without_applying_deltas();
        for id in [
            egui::Id::new(("two-way-transfer", 0usize, chunk.id, chunk.range.start)),
            egui::Id::new(("two-way-granularity", 0usize, chunk.id)),
        ] {
            let position = ctx.read_response(id).unwrap().rect.center();
            let output = frame(&mut view, &ctx, vec![egui::Event::PointerMoved(position)]);
            assert_eq!(
                output.platform_output.cursor_icon,
                egui::CursorIcon::PointingHand
            );
            output.drop_without_applying_deltas();
        }
        click(
            &mut view,
            &ctx,
            egui::Id::new(("two-way-transfer", 0usize, chunk.id, chunk.range.start)),
        );
        assert_eq!(
            view.working_text(1),
            Some("target\nfn source() {\n    one();\n    two();\n}\n")
        );
        assert!(view.diff_job.is_none());
        assert_eq!(view.caret[1], 5);
        click(
            &mut view,
            &ctx,
            egui::Id::new(("two-way-granularity", 0usize, chunk.id)),
        );
        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        let first = ctx
            .read_response(egui::Id::new((
                "two-way-transfer",
                0usize,
                chunk.id,
                1usize,
            )))
            .unwrap()
            .rect;
        let second = ctx
            .read_response(egui::Id::new((
                "two-way-transfer",
                0usize,
                chunk.id,
                2usize,
            )))
            .unwrap()
            .rect;
        assert!(first.bottom() <= second.top() + 0.1, "line arrows overlap");
        click(
            &mut view,
            &ctx,
            egui::Id::new(("two-way-transfer", 0usize, chunk.id, 1usize)),
        );
        assert!(view.working_text(1).unwrap().ends_with("    one();\n"));
    }
}
#[test]
fn toolbar_actions_stay_visible_and_ordered_at_normal_and_high_zoom() {
    for zoom in [1.0, 2.0] {
        let ctx = egui::Context::default();
        ctx.set_zoom_factor(zoom);
        let mut view = view("left\n", "right\n");
        view.set_run_diff(true);
        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        let left = ctx
            .read_response(egui::Id::new("Accept All Left — replace Right with Left"))
            .unwrap()
            .rect;
        let undo = ctx.read_response(egui::Id::new("Undo")).unwrap().rect;
        let previous = ctx
            .read_response(egui::Id::new("Previous difference"))
            .unwrap()
            .rect;
        let next = ctx
            .read_response(egui::Id::new("Next difference"))
            .unwrap()
            .rect;
        assert!(left.right() < undo.left());
        let options = ctx
            .read_response(egui::Id::new("Ignore line endings"))
            .unwrap()
            .rect;
        assert!(options.right() < previous.left());
        assert!((options.center().y - previous.center().y).abs() < 0.5);
        assert!(previous.right() < next.left());
        assert!(
            next.right() <= ctx.viewport_rect().right() + 1.0,
            "next={next:?}, viewport={:?}",
            ctx.viewport_rect()
        );
        assert!(left.top() >= 0.0);
    }
}
#[test]
fn unsaved_indicator_sits_between_activity_and_save_with_guidance_on_left() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for zoom in [1.0, 2.0] {
            let ctx = egui::Context::default();
            set_theme(&ctx, theme);
            ctx.set_zoom_factor(zoom);
            let mut view = view("left\n", "right\n");
            view.workspace
                .as_mut()
                .unwrap()
                .set_text(TwoWaySide::Left, "changed\n");
            for _ in 0..3 {
                frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
            }
            let dirty = ctx
                .read_response(egui::Id::new("workspace-unsaved"))
                .unwrap()
                .rect;
            let save = ctx.read_response(egui::Id::new("Save Both")).unwrap().rect;
            let guidance = ctx
                .read_response(egui::Id::new("workspace-guidance"))
                .unwrap()
                .rect;
            let legend = ctx
                .read_response(egui::Id::new("workspace-legend"))
                .unwrap()
                .rect;
            assert!(guidance.right() <= legend.left());
            let status = ctx
                .read_response(egui::Id::new("workspace-status"))
                .unwrap()
                .rect;
            assert!(legend.right() <= status.left());
            assert!(status.right() <= dirty.left());
            assert!(dirty.right() < save.left());
            assert!((dirty.center().y - save.center().y).abs() < 1.0);
            let output = frame(&mut view, &ctx, vec![]);
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == "Unsaved Changes"
                        && text.galley.job.sections.iter().all(|section| section.format.color == Palette::for_context(&ctx).changed)
                        && shape.clip_rect.contains_rect(text.galley.rect.translate(text.pos.to_vec2())))));
            output.drop_without_applying_deltas();
            view.move_history(false);
            for _ in 0..3 {
                frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
            }
            assert!(
                ctx.read_response(egui::Id::new("workspace-unsaved"))
                    .is_none()
            );
        }
    }
}

#[test]
fn save_both_tracks_per_side_save_availability_and_only_previews_changed_files() {
    let ctx = egui::Context::default();
    set_theme(&ctx, egui::Theme::Dark);
    let mut view = view("left\n", "right\n");
    for side in [0usize, 1] {
        for _ in 0..2 {
            frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        }
        assert!(!view.can_save_side(0) && !view.can_save_side(1));
        assert!(
            !ctx.read_response(egui::Id::new("Save Both"))
                .unwrap()
                .enabled()
        );
        view.workspace
            .as_mut()
            .unwrap()
            .set_text(side_of(side), "changed\n");
        for _ in 0..2 {
            frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        }
        assert!(view.can_save_side(side) && !view.can_save_side(1 - side));
        assert!(
            ctx.read_response(egui::Id::new("Save Both"))
                .unwrap()
                .enabled()
        );
        click(&mut view, &ctx, egui::Id::new("Save Both"));
        assert_eq!(view.pending_save, Some(vec![side]));
        view.pending_save = None;
        view.move_history(false);
    }
    // A single loaded editable file still has an active Save Both action.
    let file = view.files.as_ref().unwrap()[0].clone();
    view.files = None;
    view.paths = None;
    view.side_sources = [Some("left.rs".into()), None];
    view.side_files = [Some(file), None];
    view.workspace
        .as_mut()
        .unwrap()
        .set_text(TwoWaySide::Left, "single changed\n");
    for _ in 0..2 {
        frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
    }
    assert!(view.can_save_side(0) && !view.can_save_side(1));
    assert!(
        ctx.read_response(egui::Id::new("Save Both"))
            .unwrap()
            .enabled()
    );
    click(&mut view, &ctx, egui::Id::new("Save Both"));
    assert_eq!(view.pending_save, Some(vec![0]));
}

#[test]
fn save_both_matches_toolbar_feedback_and_still_opens_a_preview() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let ctx = egui::Context::default();
        set_theme(&ctx, theme);
        let mut view = view("left\n", "right\n");
        view.workspace
            .as_mut()
            .unwrap()
            .set_text(TwoWaySide::Left, "edited left\n");
        view.workspace
            .as_mut()
            .unwrap()
            .set_text(TwoWaySide::Right, "edited right\n");
        let output = frame(&mut view, &ctx, vec![]);
        let button = ctx.read_response(egui::Id::new("Save Both")).unwrap().rect;
        let caption = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Save Both" => Some(text),
                _ => None,
            })
            .unwrap();
        assert_eq!(caption.galley.job.sections[0].format.font_id.size, 11.0);
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(rect) if button.contains_rect(rect.rect)
                    && rect.rect.width() == 14.0 && rect.rect.right() < caption.pos.x)));
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(rect) if rect.rect == button && rect.fill != Color32::TRANSPARENT)));
        output.drop_without_applying_deltas();
        let pos = button.center();
        let output = frame(
            &mut view,
            &ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        assert!(
            view.pending_save.is_none(),
            "press feedback must precede activation"
        );
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(rect) if rect.rect == button
                    && rect.fill == Palette::for_context(&ctx).accent.gamma_multiply(0.35))));
        output.drop_without_applying_deltas();
        frame(
            &mut view,
            &ctx,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        )
        .drop_without_applying_deltas();
        assert_eq!(view.pending_save, Some(vec![0, 1]));
        assert!(
            view.save_job.is_none(),
            "Save Both must preview before writing"
        );
        assert_eq!(view.working_text(0), Some("edited left\n"));
        assert_eq!(view.working_text(1), Some("edited right\n"));
        view.pending_save = None;
        view.set_external_save_busy(true);
        for _ in 0..2 {
            frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
        }
        assert!(
            !ctx.read_response(egui::Id::new("Save Both"))
                .unwrap()
                .enabled()
        );
        click(&mut view, &ctx, egui::Id::new("Save Both"));
        assert!(view.pending_save.is_none());
    }
}

#[test]
fn merge_toolbar_uses_search_after_new_without_a_global_back_button() {
    let ctx = egui::Context::default();
    let mut view = view("left\n", "right\n");
    view.from_folders = false;
    frame(&mut view, &ctx, vec![]).drop_without_applying_deltas();
    let new = ctx
        .read_response(egui::Id::new("New comparison"))
        .unwrap()
        .rect;
    let search = ctx
        .read_response(egui::Id::new("Find and replace"))
        .unwrap()
        .rect;
    assert!(new.right() <= search.left());
    assert!(
        ctx.read_response(egui::Id::new("Back to folders"))
            .is_none()
    );
}
#[test]
fn merge_toolbar_toggles_diff_controls_and_legends_without_resetting_edits() {
    let diff_buttons = [
        "Show only differences",
        "Ignore whitespace",
        "Ignore line endings",
        "Previous difference",
        "Next difference",
    ];
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for zoom in [1.0, 2.0] {
            let ctx = egui::Context::default();
            set_theme(&ctx, theme);
            ctx.set_zoom_factor(zoom);
            ctx.run_ui(egui::RawInput::default(), |_| {})
                .drop_without_applying_deltas();
            let mut view = view("left\n", "right\n");
            view.merge_options = [false, true, true];
            view.merge_options_initialized = true;
            view.diff_options = [true, false, true];
            view.set_run_diff(true);
            view.workspace
                .as_mut()
                .unwrap()
                .edit_bytes(TwoWaySide::Left, 0..0, "edited\n".into())
                .unwrap();
            view.caret = [1, 0];
            view.invalidate_diff();
            let undo = view.workspace.as_ref().unwrap().next_undo_tag();
            let mut output = frame(&mut view, &ctx, vec![]);
            output.textures_delta.clear();
            let legend_labels: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.text()),
                    _ => None,
                })
                .collect();
            assert!(
                legend_labels
                    .iter()
                    .any(|label| label.starts_with("Conflict"))
            );
            for icon in DIFF_LEGEND_ICONS {
                assert!(
                    legend_labels
                        .iter()
                        .any(|label| label.starts_with(icon.label()))
                );
            }
            let toolbar_undo = ctx.read_response(egui::Id::new("Undo")).unwrap().rect;
            assert!(toolbar_undo.bottom() < ctx.viewport_rect().bottom());
            for label in diff_buttons {
                assert!(
                    ctx.read_response(egui::Id::new(label)).is_some(),
                    "{label} missing"
                );
            }
            assert_eq!(view.working_text(0), Some("edited\nleft\n"));
            assert_eq!(view.workspace.as_ref().unwrap().next_undo_tag(), undo);
            assert_eq!(view.caret, [1, 0]);
            assert_eq!(view.merge_options, [false, true, true]);
            assert_eq!(view.diff_options, [true, false, true]);
            assert!(view.diff_job.is_none());
        }
    }
}
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
fn preview_for(
    view: &InteractiveView,
    replacements: &[(&str, std::ops::Range<usize>, &str)],
) -> versus::SearchPreview {
    let bases = view.export_working_files();
    let mut changes = Vec::new();
    for (path, range, replacement) in replacements {
        let file = &bases[&PathBuf::from(path)];
        let mut after = file.text.clone();
        after.replace_range(range.clone(), replacement);
        changes.push(SearchChange {
            path: PathBuf::from(path),
            before: file.text.clone(),
            after,
            hit_ids: vec![changes.len() as u64 + 1],
            ranges: vec![range.clone()],
            edits: vec![(range.clone(), (*replacement).into())],
        });
    }
    versus::SearchPreview {
        changes,
        bases,
        skipped: Vec::new(),
    }
}
fn refreshed_preview(view: &InteractiveView, updates: &[(&str, &str)]) -> versus::SearchPreview {
    let mut bases = view.export_working_files();
    let mut changes = Vec::new();
    for (path, text) in updates {
        let path = PathBuf::from(path);
        let base = bases.get_mut(&path).unwrap();
        base.text = (*text).into();
        base.fingerprint = versus::FileFingerprint::Bytes(text.as_bytes().to_vec());
        let start = text.find("match").unwrap();
        let range = start..start + 5;
        let mut after = (*text).to_owned();
        after.replace_range(range.clone(), "found");
        changes.push(SearchChange {
            path,
            before: (*text).into(),
            after,
            hit_ids: vec![changes.len() as u64 + 1],
            ranges: vec![range.clone()],
            edits: vec![(range, "found".into())],
        });
    }
    versus::SearchPreview {
        changes,
        bases,
        skipped: Vec::new(),
    }
}

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
