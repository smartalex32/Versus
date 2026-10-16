//! Toolbar and footer behavior, including save-preview presentation.

use super::*;
use crate::app::visuals::footer::DIFF_LEGEND_ICONS;
use crate::app::visuals::theme::{Palette, set_theme};

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
