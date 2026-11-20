//! Theme regression coverage.

use super::*;
use crate::app::visuals::theme::{apply_theme, set_theme};

#[test]
fn themed_buttons_use_pointer_cursor_without_overriding_disabled_or_text_fields() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        set_theme(&ctx, theme);
        let mut text = String::from("editable text");
        let mut rects = [Rect::NOTHING; 3];
        let mut frame = |position: Option<Pos2>| {
            ctx.run_ui(
                egui::RawInput {
                    events: position
                        .map(egui::Event::PointerMoved)
                        .into_iter()
                        .collect(),
                    ..Default::default()
                },
                |ui| {
                    rects[0] = ui.button("Save").rect;
                    rects[1] = ui.add_enabled(false, egui::Button::new("Unavailable")).rect;
                    rects[2] = ui.text_edit_singleline(&mut text).rect;
                },
            )
        };
        frame(None).drop_without_applying_deltas();
        // Widget responses describe the previous pass; capture settled rectangles.
        frame(None).drop_without_applying_deltas();
        drop(frame);
        for (index, expected) in [
            egui::CursorIcon::PointingHand,
            egui::CursorIcon::Default,
            egui::CursorIcon::Text,
        ]
        .into_iter()
        .enumerate()
        {
            let output = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::PointerMoved(rects[index].center())],
                    ..Default::default()
                },
                |ui| {
                    let _ = ui.button("Save");
                    ui.add_enabled(false, egui::Button::new("Unavailable"));
                    ui.text_edit_singleline(&mut text);
                },
            );
            assert_eq!(
                output.platform_output.cursor_icon, expected,
                "{theme:?}, {index}"
            );
            output.drop_without_applying_deltas();
        }
    }
}

#[test]
fn settings_previews_each_theme_in_both_views_without_losing_state() {
    for file_mode in [false, true] {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.session.tree.as_mut().unwrap().expand_all();
        app.session.selected = Some("assembly/model.step".into());
        if file_mode {
            app.session.file_view = Some(loaded_file_view());
        }
        let generation = app.session.scroll_generation;
        let output = render(&mut app, &ctx, vec![]);
        let settings = ctx.read_response(egui::Id::new("Settings")).unwrap().rect;
        assert!(settings.right() <= 1190.0 && settings.left() > 1100.0 && settings.bottom() < 45.0);
        assert!(text_positions(&output, "Settings").is_empty());
        output.drop_without_applying_deltas();
        click(&mut app, &ctx, settings.center());

        let mut backgrounds = std::collections::HashSet::new();
        let mut highlights = std::collections::HashSet::new();
        for (theme, expected_dark) in [
            (Theme::Dracula, true),
            (Theme::Nord, true),
            (Theme::SolarizedLight, false),
            (Theme::Light, false),
            (Theme::Dark, true),
        ] {
            let output = render(&mut app, &ctx, vec![]);
            let option = text_centers(&output, theme.label())[0];
            output.drop_without_applying_deltas();
            click(&mut app, &ctx, option);
            let output = render(&mut app, &ctx, vec![]);
            assert!(app.settings_open);
            assert_eq!(Theme::for_context(&ctx), theme);
            assert_eq!(ctx.theme() == egui::Theme::Dark, expected_dark);
            assert_eq!(app.logo_texture.as_ref().unwrap().0, expected_dark);
            assert_eq!(app.session.file_view.is_some(), file_mode);
            assert!(app.session.tree.as_ref().unwrap().is_expanded("assembly"));
            assert_eq!(
                app.session.selected.as_deref(),
                Some(std::path::Path::new("assembly/model.step"))
            );
            assert_eq!(app.session.scroll_generation, generation);
            let neutral = if file_mode { "unchanged" } else { "model.step" };
            let palette = Palette::for_context(&ctx);
            assert!(backgrounds.insert(palette.background));
            assert!(highlights.insert(palette.changed));
            assert!(output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text)
                    if text.galley.text() == neutral && text.fallback_color == palette.muted)
            }));
            assert_eq!(
                eframe::App::clear_color(&app, &ctx.global_style().visuals),
                palette.background.to_normalized_gamma_f32(),
            );
            if file_mode {
                assert!(output.shapes.iter().any(|shape| {
                    matches!(&shape.shape, egui::Shape::Text(text)
                        if text.galley.text() == "old value"
                            && text.galley.job.sections[0].format.color == palette.changed)
                }));
            }
            output.drop_without_applying_deltas();
        }

        let output = render(&mut app, &ctx, vec![]);
        let nord = text_centers(&output, "Nord")[0];
        let done = text_centers(&output, "Done")[0];
        output.drop_without_applying_deltas();
        click(&mut app, &ctx, nord);
        if file_mode {
            click(&mut app, &ctx, done);
        } else {
            render(
                &mut app,
                &ctx,
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            )
            .drop_without_applying_deltas();
        }
        assert!(!app.settings_open);
        assert_eq!(Theme::for_context(&ctx), Theme::Nord);
    }
}

#[test]
fn theme_preference_survives_relaunch_and_unknown_values_use_dark() {
    let mut storage = ZoomStorage::default();
    let mut launch = |stored: Option<&str>| {
        if let Some(stored) = stored {
            storage.set_string(THEME_STORAGE_KEY, stored.into());
        } else {
            storage.remove_string(THEME_STORAGE_KEY);
        }
        let ctx = egui::Context::default();
        let mut creation = eframe::CreationContext::_new_kittest(ctx.clone());
        creation.storage = Some(&storage);
        (VersusApp::new(&creation, None), ctx)
    };

    for (stored, expected) in [
        (None, Theme::Dark),
        (Some("unknown"), Theme::Dark),
        (Some("solarized-light"), Theme::SolarizedLight),
    ] {
        let (mut app, ctx) = launch(stored);
        assert_eq!(Theme::for_context(&ctx), expected);
        let mut saved = ZoomStorage::default();
        eframe::App::save(&mut app, &mut saved);
        let restarted = launch(saved.get_string(THEME_STORAGE_KEY).as_deref());
        assert_eq!(Theme::for_context(&restarted.1), expected);
    }
}

#[test]
fn settings_can_be_opened_and_changed_with_the_keyboard() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    let settings_button = egui::Id::new("Settings");
    ctx.memory_mut(|memory| memory.request_focus(settings_button));
    let press = |app: &mut VersusApp, key| {
        render(
            app,
            &ctx,
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        )
        .drop_without_applying_deltas();
    };

    press(&mut app, egui::Key::Enter);
    assert!(app.settings_open);
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    press(&mut app, egui::Key::Tab);
    press(&mut app, egui::Key::Space);
    assert_eq!(Theme::for_context(&ctx), Theme::Light);
    press(&mut app, egui::Key::Escape);
    assert!(!app.settings_open);
    assert!(ctx.memory(|memory| memory.has_focus(settings_button)));
}

#[test]
fn logo_texture_respects_transparency_of_stray_and_edge_pixels() {
    let ctx = egui::Context::default();
    let mut app = loaded_app();
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        set_theme(&ctx, theme);
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                ..Default::default()
            },
            |ui| app.render(ui),
        );
        let logo_id = app.logo_texture.as_ref().unwrap().1.id();
        let delta = &output.textures_delta.set.get(&logo_id).unwrap()[0];
        let egui::ImageData::Color(image) = &delta.image;
        let speck = image[(91, 74)];
        assert_eq!(speck.a(), 1);
        assert!(
            speck.r() <= 1 && speck.g() <= 1 && speck.b() <= 1,
            "Nearly transparent pixels must not become bright white"
        );
        for pixel in &image.pixels {
            assert!(
                pixel.r() <= pixel.a() && pixel.g() <= pixel.a() && pixel.b() <= pixel.a(),
                "Logo texture must use premultiplied alpha"
            );
        }
        output.drop_without_applying_deltas();
    }
}

#[test]
fn theme_changes_send_matching_icon_to_native_window() {
    let ctx = egui::Context::default();
    let mut app = loaded_app();
    for theme in [egui::Theme::Dark, egui::Theme::Light, egui::Theme::Dark] {
        set_theme(&ctx, theme);
        let output = render(&mut app, &ctx, vec![]);
        let expected = crate::logo::themed_icon(theme == egui::Theme::Dark);
        assert!(output.viewport_output.values().flat_map(|viewport| &viewport.commands).any(|command| {
            matches!(command,egui::ViewportCommand::Icon(Some(icon)) if icon.rgba == expected.rgba)
        }), "Native window must receive the matching theme icon");
        output.drop_without_applying_deltas();
    }
}
