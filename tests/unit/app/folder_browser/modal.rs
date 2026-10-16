//! Folder browser modal capability regressions.

use super::*;
use crate::app::visuals::theme::{Palette, set_theme};

#[test]
fn find_modal_has_centered_half_width_layout_and_a_styled_results_placeholder() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let ctx = egui::Context::default();
        set_theme(&ctx, theme);
        // Inspect the final colors rather than the Area's fade-in opacity.
        ctx.all_styles_mut(|style| style.animation_time = 0.0);
        let mut browser = FolderBrowser::new_with_sources([None, None]);
        let mut view = InteractiveView::default();
        browser.open_search();
        // Area sizing is invisible on its first pass; responses retain the
        // previous pass, so settle the anchored layout before inspecting it.
        dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
        dialog_frame(&mut browser, &mut view, &ctx, vec![]).drop_without_applying_deltas();
        let output = dialog_frame(&mut browser, &mut view, &ctx, vec![]);
        let modal = ctx
            .memory(|memory| memory.area_rect(egui::Id::new("folder-find-replace-modal")))
            .unwrap();
        assert!(modal.center().distance(ctx.content_rect().center()) < 1.0);
        assert!(
            (modal.width() - 600.0).abs() < 10.0 && modal.height() < 450.0,
            "{modal:?}"
        );
        let find = ctx
            .read_response(egui::Id::new(("folder-find-pattern", "frame")))
            .unwrap()
            .rect;
        let replacement = ctx
            .read_response(egui::Id::new(("folder-replacement-text", "frame")))
            .unwrap()
            .rect;
        assert!(find.width() > modal.width() / 2.0 - 30.0 && find.height() > 32.0);
        let results = ctx
            .read_response(egui::Id::new("folder-search-results"))
            .unwrap()
            .rect;
        assert!(results.left() >= modal.center().x && find.right() < results.left());
        assert!(modal.contains_rect(results));
        assert!(
            ctx.read_response(egui::Id::new("Find")).is_none(),
            "the Find action was removed"
        );
        assert!(replacement.height() > 32.0);
        for label in ["Match Case", "Match Whole Word", "Use Regular Expression"] {
            let control = ctx.read_response(egui::Id::new(label)).unwrap().rect;
            assert!(find.contains_rect(control));
            assert!(
                control.left()
                    > ctx
                        .read_response(egui::Id::new("folder-find-pattern"))
                        .unwrap()
                        .rect
                        .right()
            );
        }
        let next = ctx
            .read_response(egui::Id::new("Replace Next"))
            .unwrap()
            .rect;
        let all = ctx
            .read_response(egui::Id::new("Replace All"))
            .unwrap()
            .rect;
        assert!(next.left() > replacement.right() && all.left() > next.right());
        assert!(
            (next.center().y - replacement.center().y).abs() < 1.0,
            "next={next:?}, replacement={replacement:?}"
        );
        let palette = Palette::for_context(&ctx);
        assert!(results.width() > modal.width() / 2.0 - 40.0);
        assert!(results.height() > 300.0);
        assert!(
            output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(frame) if frame.fill == palette.panel
                    && frame.rect.contains_rect(results)
                    && (frame.rect.width() - results.width() - 18.0).abs() < 1.0)),
            "the painted panel must fill the right column even without results"
        );
        let left_view = ctx
            .read_response(egui::Id::new(("folder-search-view", 0usize)))
            .unwrap()
            .rect;
        let right_view = ctx
            .read_response(egui::Id::new(("folder-search-view", 1usize)))
            .unwrap()
            .rect;
        assert!((left_view.width() - right_view.width()).abs() < 1.0);
        assert!((left_view.left() - (results.left() - 9.0)).abs() < 1.0);
        assert!((right_view.right() - (results.right() + 9.0)).abs() < 1.0);
        for label in ["LEFT (0)", "RIGHT (0)"] {
            assert!(
                output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == label
                        && text.fallback_color == palette.text
                        && text.galley.rect.translate(text.pos.to_vec2()).center().y < results.top()
                        && text.pos.x > modal.center().x)),
                "result view toggle must be neutral"
            );
        }
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text() == "Search results will appear here"
                    && text.galley.job.sections[0].format.font_id == FontId::proportional(14.0)
                    && text.fallback_color == palette.muted
                    && results.contains_rect(text.galley.rect.translate(text.pos.to_vec2()))
                    && text.galley.rect.translate(text.pos.to_vec2()).center().distance(results.center()) < 1.0)));
        for (label, color) in [("LEFT", palette.left_only), ("RIGHT", palette.right_only)] {
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == label && text.fallback_color == color)),
                    "{label} expected={color:?}, actual={:?}", output.shapes.iter().filter_map(|shape|
                        if let egui::Shape::Text(text) = &shape.shape { (text.galley.text() == label).then_some(text.fallback_color) } else { None }).collect::<Vec<_>>());
        }
        for placeholder in ["Find", "Replace"] {
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == placeholder
                        && text.galley.job.sections[0].format.font_id.size == 14.0
                        && text.galley.job.sections[0].format.italics
                        && text.galley.job.sections[0].format.color == palette.muted.gamma_multiply(0.8))));
        }
        output.drop_without_applying_deltas();
        for label in ["Match Case", "Match Whole Word", "Use Regular Expression"] {
            let pos = ctx
                .read_response(egui::Id::new(label))
                .unwrap()
                .rect
                .center();
            for pressed in [true, false] {
                dialog_frame(
                    &mut browser,
                    &mut view,
                    &ctx,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                )
                .drop_without_applying_deltas();
            }
        }
        assert!(
            browser.search_state.query.case_sensitive
                && browser.search_state.query.whole_word
                && browser.search_state.query.regex
        );
        assert!(
            browser.search_state.open,
            "clicking controls inside the field must keep the modal open"
        );
        let outside = egui::pos2(modal.left() - 10.0, modal.center().y);
        for pressed in [true, false] {
            dialog_frame(
                &mut browser,
                &mut view,
                &ctx,
                vec![
                    egui::Event::PointerMoved(outside),
                    egui::Event::PointerButton {
                        pos: outside,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            )
            .drop_without_applying_deltas();
        }
        assert!(!browser.search_state.open);
    }
}

#[test]
fn find_modal_columns_and_controls_fit_at_normal_and_high_zoom() {
    for zoom in [1.0, 2.0] {
        let ctx = egui::Context::default();
        set_theme(&ctx, egui::Theme::Light);
        ctx.set_zoom_factor(zoom);
        let mut browser = FolderBrowser::new_search([None, None]);
        let mut view = InteractiveView::default();
        browser.open_search();
        for _ in 0..6 {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        Pos2::ZERO,
                        egui::vec2(900.0, 800.0) / zoom,
                    )),
                    ..Default::default()
                },
                |_| browser.search_window(&ctx, &mut view),
            )
            .drop_without_applying_deltas();
        }
        let modal = ctx
            .memory(|memory| memory.area_rect(egui::Id::new("folder-find-replace-modal")))
            .unwrap();
        assert!(
            ctx.content_rect().contains_rect(modal),
            "zoom={zoom}: {modal:?}"
        );
        let results = ctx
            .read_response(egui::Id::new("folder-search-results"))
            .unwrap()
            .rect;
        for label in [
            "Match Case",
            "Match Whole Word",
            "Use Regular Expression",
            "Replace Next",
            "Replace All",
        ] {
            let control = ctx.read_response(egui::Id::new(label)).unwrap().rect;
            assert!(
                modal.contains_rect(control) && control.right() < results.left(),
                "zoom={zoom}: {label}"
            );
        }
        let replace = ctx
            .read_response(egui::Id::new(("folder-replacement-text", "frame")))
            .unwrap()
            .rect;
        let both = ctx
            .read_response(egui::Id::new(("folder-search-scope", "BOTH")))
            .unwrap()
            .rect;
        assert!(
            both.top() > replace.bottom(),
            "scope must not overlap Replace"
        );
        let actions: Vec<_> = [
            "Undo replacement batch",
            "Redo replacement batch",
            "Refresh search",
        ]
        .into_iter()
        .map(|label| ctx.read_response(egui::Id::new(label)).unwrap().rect)
        .collect();
        assert!(
            actions.iter().all(|rect| modal.contains_rect(*rect)),
            "zoom={zoom} modal={modal:?} actions={actions:?}"
        );
        assert!(
            actions
                .windows(2)
                .all(|pair| pair[1].left() > pair[0].right())
        );
        if zoom == 1.0 {
            assert!(actions[0].left() > both.right());
            assert!((actions[0].center().y - both.center().y).abs() < 1.0);
        }
    }
}
