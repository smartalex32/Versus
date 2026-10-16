//! Difference navigation and read-only presentation behavior.

use super::*;
use crate::app::visuals::theme::{
    Palette, ROW_HEIGHT, comparison_font, comparison_row_height, set_theme,
};

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
                    let row_height =
                        ROW_HEIGHT.max(ctx.fonts_mut(|fonts| fonts.row_height(&font)).ceil() + 8.0);
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
            ROW_HEIGHT.max(ctx.fonts_mut(|fonts| fonts.row_height(&font)).ceil() + 8.0);
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
        row_height = comparison_row_height(ui);
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
        row_height = comparison_row_height(ui);
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
