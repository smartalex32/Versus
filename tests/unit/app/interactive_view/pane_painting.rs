//! Editable pane layout, scroll behavior, and transfer-track painting.

use super::*;
use crate::app::visuals::theme::{Palette, set_theme};

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
