//! Accessibility regression coverage.

use super::*;
use crate::app::visuals::theme::apply_theme;

#[test]
fn arrow_keys_move_tree_selection_through_visible_rows() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    let output = render(&mut app, &ctx, vec![]);
    let assembly = text_positions(&output, "assembly")[0];
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, assembly + egui::vec2(3.0, 4.0));
    assert_eq!(
        app.session.selected.as_deref(),
        Some(std::path::Path::new("assembly"))
    );
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::ArrowDown,
            physical_key: Some(egui::Key::ArrowDown),
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    )
    .drop_without_applying_deltas();
    assert_eq!(
        app.session.selected.as_deref(),
        Some(std::path::Path::new("assembly/model.step"))
    );
    assert_eq!(
        app.session.tree_keyboard_focus.as_deref(),
        Some(std::path::Path::new("assembly/model.step"))
    );
}

#[test]
fn tree_tab_navigation_escapes_both_panes_in_either_direction() {
    for side in 0..2 {
        for backwards in [false, true] {
            let ctx = egui::Context::default();
            apply_theme(&ctx);
            let mut app = loaded_app();
            let output = render(&mut app, &ctx, vec![]);
            let assembly = text_positions(&output, "assembly")[side];
            output.drop_without_applying_deltas();
            click(&mut app, &ctx, assembly + egui::vec2(3.0, 4.0));
            let focused_before_tab = ctx.memory(|memory| memory.focused()).unwrap();
            render(
                &mut app,
                &ctx,
                vec![egui::Event::Key {
                    key: egui::Key::Tab,
                    physical_key: Some(egui::Key::Tab),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers {
                        shift: backwards,
                        ..Default::default()
                    },
                }],
            )
            .drop_without_applying_deltas();
            render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
            assert_ne!(
                ctx.memory(|memory| memory.focused()),
                Some(focused_before_tab)
            );
            assert_eq!(
                ctx.memory(|memory| memory.focused()),
                Some(egui::Id::new(if backwards {
                    "Browse right folder"
                } else {
                    "Folder Diff"
                }))
            );
            assert!(app.session.tree_keyboard_focus.is_none());
        }
    }
}

#[test]
fn keyboard_tree_navigation_keeps_a_long_list_visible_without_resetting_scroll() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = unfiltered_app();
    app.session.tree = Some(FolderTree::from_diff(&DirectoryDiff {
        cancelled: false,
        entries: (0..80)
            .map(|index| DirectoryEntry {
                relative_path: format!("item-{index:03}.txt").into(),
                left_size: Some(1),
                right_size: Some(1),
                left_exists: true,
                right_exists: true,
                left_kind: Some(DirectoryEntryKind::File),
                right_kind: Some(DirectoryEntryKind::File),
                kind: DirectoryEntryKind::File,
                state: DirectoryEntryState::Same,
            })
            .collect(),
    }));
    let output = render(&mut app, &ctx, vec![]);
    let first = text_positions(&output, "item-000.txt")[0];
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, first + egui::vec2(3.0, 4.0));
    for _ in 0..50 {
        render(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::ArrowDown,
                physical_key: Some(egui::Key::ArrowDown),
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        )
        .drop_without_applying_deltas();
        render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    }
    assert_eq!(
        app.session.selected.as_deref(),
        Some(std::path::Path::new("item-050.txt"))
    );
    assert!(app.session.tree_scroll_y > 0.0);
}

#[test]
fn focused_file_pane_scrolls_with_arrow_keys() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = unfiltered_app();
    let mut view = loaded_file_view();
    let rows: Vec<_> = (1..=80)
        .map(|number| {
            test_file_row(
                Some((number, format!("left line {number}"))),
                Some((number, format!("right line {number}"))),
                DirectoryEntryState::Same,
            )
        })
        .collect();
    view.comparison.as_mut().unwrap().rows = rows;
    view.visible_rows = (0..80).collect();
    app.session.file_view = Some(view);
    let output = render(&mut app, &ctx, vec![]);
    let line = text_positions(&output, "left line 1")[0];
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, line + egui::vec2(8.0, 3.0));
    assert_eq!(app.session.file_keyboard_focus_side, Some(0));
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::ArrowDown,
            physical_key: Some(egui::Key::ArrowDown),
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    )
    .drop_without_applying_deltas();
    assert_eq!(
        app.session.file_view.as_ref().unwrap().scroll_y,
        app.session.row_height
    );
    let focused = ctx.memory(|memory| memory.focused());
    assert!(focused.is_some());
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    assert_eq!(ctx.memory(|memory| memory.focused()), focused);
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::ArrowDown,
            physical_key: Some(egui::Key::ArrowDown),
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    )
    .drop_without_applying_deltas();
    assert_eq!(
        app.session.file_view.as_ref().unwrap().scroll_y,
        2.0 * app.session.row_height
    );
}

#[test]
fn accesskit_exposes_empty_targets_and_file_lines_with_side_number_and_status() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    ctx.enable_accesskit();
    let mut app = VersusApp::default();
    let output = render(&mut app, &ctx, vec![]);
    let labels: Vec<_> = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .filter_map(|(_, node)| node.label().or_else(|| node.value()))
        .collect();
    for side in ["Left", "Right"] {
        assert!(
            labels
                .iter()
                .any(|label| label.starts_with(side) && label.contains("Drop a file or folder"))
        );
    }
    output.drop_without_applying_deltas();
    app.session.file_view = Some(loaded_file_view());
    let output = render(&mut app, &ctx, vec![]);
    let labels: Vec<_> = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .filter_map(|(_, node)| node.label().or_else(|| node.value()))
        .collect();
    assert!(
        labels
            .iter()
            .any(|label| label.contains("Left line 2") && label.contains("Different")),
        "{labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|label| label.contains("Right file comparison pane") && label.contains("right"))
    );
    output.drop_without_applying_deltas();
}

#[test]
fn long_unicode_file_lines_have_bounded_accessibility_updates() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    ctx.enable_accesskit();
    let mut app = unfiltered_app();
    let mut view = loaded_file_view();
    let text = "工程🦀".repeat(20_000);
    view.comparison.as_mut().unwrap().rows = vec![test_file_row(
        Some((1, text.clone())),
        Some((1, text)),
        DirectoryEntryState::Different,
    )];
    view.visible_rows = vec![0];
    app.session.file_view = Some(view);
    let output = render(&mut app, &ctx, vec![]);
    let labels: Vec<_> = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .filter_map(|(_, node)| node.value())
        .filter(|label| label.contains("line 1:"))
        .collect();
    assert_eq!(labels.len(), 2);
    for label in labels {
        assert!(label.len() < 4200);
        assert!(label.contains("bytes omitted"));
        assert!(label.contains("工程🦀"));
    }
    output.drop_without_applying_deltas();
}
