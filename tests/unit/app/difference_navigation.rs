//! Difference navigation regression coverage.

use super::*;
use crate::app::session::update_visible_file_rows;
use crate::app::visuals::theme::{ROW_HEIGHT, apply_theme, comparison_font, set_theme};

#[test]
fn filtered_merge_typing_after_hidden_lines_preserves_source_text_and_undo() {
    let fixture = SourceFixture::new();
    let left = "prefix\noldfirst\nmiddle\noldsecond\nsuffix\n";
    let right = "prefix\nnewfirst\nmiddle\nnewsecond\nsuffix\n";
    std::fs::write(fixture.path("left/model.txt"), left).unwrap();
    std::fs::write(fixture.path("right/model.txt"), right).unwrap();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    app.session.comparison_mode = ComparisonMode::File;
    app.session.set_workspace_merge(true);
    app.session.select_path(0, fixture.path("left/model.txt"));
    app.session.select_path(1, fixture.path("right/model.txt"));
    settle_merge_file(&mut app, &ctx, 0, left);
    settle_merge_file(&mut app, &ctx, 1, right);
    app.session.interactive.as_mut().unwrap().set_run_diff(true);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app
        .session
        .interactive
        .as_ref()
        .unwrap()
        .loaded_pair_comparison()
        .is_none()
    {
        app.session.interactive.as_mut().unwrap().poll(&ctx);
        assert!(Instant::now() < deadline, "Merge comparison did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
    for _ in 0..3 {
        render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    }
    let output = render(&mut app, &ctx, vec![]);
    let position = text_positions(&output, "oldfirst\noldsecond\n")[0];
    let font = comparison_font(&ctx);
    let row_height = ROW_HEIGHT.max(ctx.fonts_mut(|fonts| fonts.row_height(&font)).ceil() + 8.0);
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, position + egui::vec2(1.0, row_height + 6.0));
    render(&mut app, &ctx, vec![egui::Event::Text("X".into())]).drop_without_applying_deltas();
    assert_eq!(
        app.session.interactive.as_ref().unwrap().working_text(0),
        Some("prefix\noldfirst\nmiddle\nXoldsecond\nsuffix\n")
    );
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    )
    .drop_without_applying_deltas();
    render(
        &mut app,
        &ctx,
        vec![
            egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::Text("Y".into()),
        ],
    )
    .drop_without_applying_deltas();
    assert_eq!(
        app.session.interactive.as_ref().unwrap().working_text(0),
        Some("prefix\noldfirst\nmiddle\nX\nYoldsecond\nsuffix\n")
    );
    for _ in 0..3 {
        click_action(&mut app, &ctx, "Undo");
    }
    assert_eq!(
        app.session.interactive.as_ref().unwrap().working_text(0),
        Some(left)
    );
    assert_eq!(
        app.session.interactive.as_ref().unwrap().working_text(1),
        Some(right)
    );
    assert_eq!(
        std::fs::read_to_string(fixture.path("left/model.txt")).unwrap(),
        left
    );
}

#[test]
fn differences_filter_hides_equal_lines_and_preserves_original_numbers() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    app.session.file_view = Some(loaded_file_view());
    click_action(&mut app, &ctx, "Show only differences");
    let output = render(&mut app, &ctx, vec![]);
    assert!(text_positions(&output, "unchanged").is_empty());
    assert!(text_positions(&output, "1").is_empty());
    assert_eq!(text_positions(&output, "2").len(), 2);
    assert_eq!(text_positions(&output, "3").len(), 1);
    let left = text_positions(&output, "old value")[0];
    let right = text_positions(&output, "new value")[0];
    assert!((left.y - right.y).abs() < 0.1);
    output.drop_without_applying_deltas();
    click_action(&mut app, &ctx, "Show only differences");
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "unchanged").len(), 2);
    output.drop_without_applying_deltas();
}

#[test]
fn navigation_scrolls_both_file_panes_and_advances_when_rows_fit_the_window() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    let mut view = loaded_file_view();
    view.comparison.as_mut().unwrap().rows = (1..=200)
        .map(|number| {
            let changed = [1, 20, 80, 150, 200].contains(&number);
            test_file_row(
                Some((number, format!("left line {number}"))),
                Some((number, format!("right line {number}"))),
                if changed {
                    DirectoryEntryState::Different
                } else {
                    DirectoryEntryState::Same
                },
            )
        })
        .collect();
    update_visible_file_rows(&mut view, false);
    app.session.file_view = Some(view);
    for (action, number) in [
        ("Next difference", 1),
        ("Next difference", 20),
        ("Next difference", 80),
        ("Next difference", 150),
        ("Next difference", 200),
        ("Previous difference", 150),
        ("Previous difference", 80),
        ("Previous difference", 20),
    ] {
        let first = click_action_frame(&mut app, &ctx, action);
        let first_positions = text_positions(&first, &format!("left line {number}"));
        assert_eq!(first_positions.len(), 1);
        first.drop_without_applying_deltas();
        let output = render(&mut app, &ctx, vec![]);
        let left = text_positions(&output, &format!("left line {number}"))[0];
        let right = text_positions(&output, &format!("right line {number}"))[0];
        assert!((left.y - right.y).abs() < 0.1);
        // Keep the preceding line visible, or clamp to the file start.
        if number > 1 {
            assert_eq!(
                text_positions(&output, &format!("left line {}", number - 1)).len(),
                1
            );
        } else {
            assert_eq!(app.session.file_view.as_ref().unwrap().scroll_y, 0.0);
        }
        assert_eq!(
            first_positions,
            vec![left],
            "Jump should settle on its first frame"
        );
        assert_eq!(
            app.session.file_view.as_ref().unwrap().navigation_row,
            Some(number - 1)
        );
        output.drop_without_applying_deltas();
    }
    app.session.file_view = Some(loaded_file_view());
    for (action, index) in [
        ("Next difference", 1),
        ("Next difference", 2),
        ("Previous difference", 1),
    ] {
        click_action(&mut app, &ctx, action);
        assert_eq!(
            app.session.file_view.as_ref().unwrap().navigation_row,
            Some(index)
        );
    }
}

#[test]
fn folder_navigation_reveals_collapsed_parents_in_both_panes() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut entries = Vec::new();
    for index in 0..100 {
        entries.push(DirectoryEntry {
            relative_path: format!("a_common_{index:03}").into(),
            left_exists: true,
            right_exists: true,
            left_kind: Some(DirectoryEntryKind::Directory),
            right_kind: Some(DirectoryEntryKind::Directory),
            left_size: Some(0),
            right_size: Some(0),
            kind: DirectoryEntryKind::Directory,
            state: DirectoryEntryState::Same,
        });
    }
    for (path, state) in [
        ("z_changed/deep/changed.txt", DirectoryEntryState::Different),
        ("z_changed/deep/removed.txt", DirectoryEntryState::LeftOnly),
        ("z_changed/deep/same.txt", DirectoryEntryState::Same),
        ("z_other/added.txt", DirectoryEntryState::RightOnly),
    ] {
        let left_exists = state != DirectoryEntryState::RightOnly;
        let right_exists = state != DirectoryEntryState::LeftOnly;
        entries.push(DirectoryEntry {
            relative_path: path.into(),
            left_exists,
            right_exists,
            left_kind: left_exists.then_some(DirectoryEntryKind::File),
            right_kind: right_exists.then_some(DirectoryEntryKind::File),
            left_size: left_exists.then_some(1),
            right_size: right_exists.then_some(1),
            kind: DirectoryEntryKind::File,
            state,
        });
    }
    let tree = FolderTree::from_diff(&DirectoryDiff {
        entries,
        cancelled: false,
    });
    let mut app = unfiltered_app();
    let sender = attach_job(&mut app);
    sender.send(Ok(tree)).unwrap();
    app.session.poll_comparison(&ctx);
    let first = click_action_frame(&mut app, &ctx, "Next difference");
    let first_positions = text_positions(&first, "changed.txt");
    assert_eq!(first_positions.len(), 2);
    first.drop_without_applying_deltas();
    assert_eq!(
        app.session.selected.as_deref(),
        Some(std::path::Path::new("z_changed/deep/changed.txt"))
    );
    assert!(app.session.tree.as_ref().unwrap().is_expanded("z_changed"));
    assert!(
        app.session
            .tree
            .as_ref()
            .unwrap()
            .is_expanded("z_changed/deep")
    );
    let output = render(&mut app, &ctx, vec![]);
    let rows = text_positions(&output, "changed.txt");
    assert_eq!(rows.len(), 2);
    assert_eq!(
        first_positions, rows,
        "Jump should settle on its first frame"
    );
    assert!((rows[0].y - rows[1].y).abs() < 0.1);
    output.drop_without_applying_deltas();
    click_action(&mut app, &ctx, "Show only differences");
    let output = render(&mut app, &ctx, vec![]);
    assert!(text_positions(&output, "a_common_000").is_empty());
    assert_eq!(text_positions(&output, "changed.txt").len(), 2);
    output.drop_without_applying_deltas();
    for path in ["z_changed/deep/removed.txt", "z_other/added.txt"] {
        click_action(&mut app, &ctx, "Next difference");
        assert_eq!(
            app.session.selected.as_deref(),
            Some(std::path::Path::new(path))
        );
    }
    assert!(app.session.tree.as_ref().unwrap().is_expanded("z_other"));
    click_action(&mut app, &ctx, "Next difference");
    assert_eq!(
        app.session.selected.as_deref(),
        Some(std::path::Path::new("z_other/added.txt"))
    );
    click_action(&mut app, &ctx, "Previous difference");
    assert_eq!(
        app.session.selected.as_deref(),
        Some(std::path::Path::new("z_changed/deep/removed.txt"))
    );
    click_action(&mut app, &ctx, "Previous difference");
    assert_eq!(
        app.session.selected.as_deref(),
        Some(std::path::Path::new("z_changed/deep/changed.txt"))
    );
}

#[test]
fn folder_navigation_is_disabled_when_only_folders_differ() {
    let ctx = egui::Context::default();
    let tree = FolderTree::from_diff(&DirectoryDiff {
        entries: vec![DirectoryEntry {
            relative_path: "empty-left-only".into(),
            left_exists: true,
            right_exists: false,
            left_kind: Some(DirectoryEntryKind::Directory),
            right_kind: None,
            left_size: Some(0),
            right_size: None,
            kind: DirectoryEntryKind::Directory,
            state: DirectoryEntryState::LeftOnly,
        }],
        cancelled: false,
    });
    let mut app = VersusApp::default();
    let sender = attach_job(&mut app);
    sender.send(Ok(tree)).unwrap();
    app.session.poll_comparison(&ctx);
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    for label in ["Previous difference", "Next difference"] {
        assert!(!ctx.read_response(egui::Id::new(label)).unwrap().enabled());
    }
    assert!(app.session.folder_differences.is_empty());
}

#[test]
fn default_options_filter_equal_lines_and_ignore_whitespace_and_endings() {
    let sources = SourceFixture::new();
    std::fs::write(
        sources.path("left/model.txt"),
        "common\nspacing value\nold\n",
    )
    .unwrap();
    std::fs::write(
        sources.path("right/model.txt"),
        "common\r\nspacing  value\r\nnew\r\n",
    )
    .unwrap();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    assert!(
        app.session.show_only_differences
            && app.session.ignore_whitespace
            && app.session.ignore_line_endings
    );
    app.session.open_launch_request(crate::cli::LaunchRequest {
        paths: [
            sources.path("left/model.txt"),
            sources.path("right/model.txt"),
        ],
        mode: None,
    });
    settle_sources(&mut app, &ctx);
    let view = app.session.file_view.as_ref().unwrap();
    assert_eq!(view.counts[1], 1);
    assert_eq!(view.visible_rows, vec![2]);
    let output = render(&mut app, &ctx, vec![]);
    assert!(text_positions(&output, "common").is_empty());
    assert!(text_positions(&output, "spacing value").is_empty());
    assert_eq!(text_positions(&output, "old").len(), 1);
    assert_eq!(text_positions(&output, "new").len(), 1);
    output.drop_without_applying_deltas();
}

#[test]
fn line_ending_ignore_defaults_on_in_both_comparison_views() {
    let sources = SourceFixture::new();
    std::fs::write(sources.path("left/model.txt"), "same\r\nnext\r\n").unwrap();
    std::fs::write(sources.path("right/model.txt"), "same\nnext\n").unwrap();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    assert!(app.session.ignore_line_endings);
    app.session.open_launch_request(crate::cli::LaunchRequest {
        paths: [sources.path("left"), sources.path("right")],
        mode: None,
    });
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.counts[1], 0);
    app.session
        .open_file("model.txt".into(), [true, true], false);
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.file_view.as_ref().unwrap().counts[1], 0);
    click_action(&mut app, &ctx, "Ignore line endings");
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.counts[1], 1);
    assert_eq!(app.session.file_view.as_ref().unwrap().counts[1], 2);
    click_action(&mut app, &ctx, "Ignore line endings");
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.counts[1], 0);
    assert_eq!(app.session.file_view.as_ref().unwrap().counts[1], 0);
}

#[test]
fn inserted_line_and_commented_peers_align_with_original_numbers_in_both_panes() {
    let sources = SourceFixture::new();
    let prefix: String = (1..50)
        .map(|line| format!("unchanged line {line}\n"))
        .collect();
    std::fs::write(
        sources.path("left/model.txt"),
        format!("{prefix}extra_call();\n    first_action();\n    second_action();\ntail\n"),
    )
    .unwrap();
    std::fs::write(
        sources.path("right/model.txt"),
        format!("{prefix}    // first_action();\n    // second_action();\ntail\n"),
    )
    .unwrap();
    let ctx = egui::Context::default();
    let mut app = unfiltered_app();
    app.session.open_launch_request(crate::cli::LaunchRequest {
        paths: [
            sources.path("left/model.txt"),
            sources.path("right/model.txt"),
        ],
        mode: Some(ComparisonMode::File),
    });
    settle_sources(&mut app, &ctx);
    let comparison = app
        .session
        .file_view
        .as_ref()
        .unwrap()
        .comparison
        .as_ref()
        .unwrap();
    assert_eq!(comparison.rows[49].left.as_ref().unwrap().0, 50);
    assert!(comparison.rows[49].right.is_none());
    for (index, left_number, right_number) in [(50, 51, 50), (51, 52, 51)] {
        let row = &comparison.rows[index];
        assert_eq!(row.left.as_ref().unwrap().0, left_number);
        assert_eq!(row.right.as_ref().unwrap().0, right_number);
        assert_eq!(row.state, DirectoryEntryState::Different);
    }
    click_action(&mut app, &ctx, "Next difference");
    for differences_only in [false, true] {
        if differences_only {
            click_action(&mut app, &ctx, "Show only differences");
        }
        let mut output = render(&mut app, &ctx, vec![]);
        output.textures_delta.clear();
        for (left, right) in [
            ("    first_action();", "    // first_action();"),
            ("    second_action();", "    // second_action();"),
        ] {
            let left_position = text_positions(&output, left)[0];
            let right_position = text_positions(&output, right)[0];
            assert!((left_position.y - right_position.y).abs() < 0.1);
            let highlighted: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == right => Some(&text.galley),
                    _ => None,
                })
                .flat_map(|galley| {
                    galley.job.sections.iter().filter_map(|section| {
                        (section.format.background != Color32::TRANSPARENT).then(|| {
                            galley.job.text[section.byte_range.start.0..section.byte_range.end.0]
                                .to_owned()
                        })
                    })
                })
                .collect();
            assert_eq!(highlighted.concat(), "// ");
        }
        output.drop_without_applying_deltas();
    }
}

#[test]
fn changed_text_is_highlighted_while_common_text_stays_unhighlighted() {
    let sources = SourceFixture::new();
    std::fs::write(sources.path("left/model.txt"), "old stable old\n").unwrap();
    std::fs::write(sources.path("right/model.txt"), "new stable new\n").unwrap();
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        set_theme(&ctx, theme);
        let mut app = VersusApp::default();
        app.session.open_launch_request(crate::cli::LaunchRequest {
            paths: [
                sources.path("left/model.txt"),
                sources.path("right/model.txt"),
            ],
            mode: None,
        });
        settle_sources(&mut app, &ctx);
        let output = render(&mut app, &ctx, vec![]);
        for expected in ["old stable old", "new stable new"] {
            let galley = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == expected => Some(&text.galley),
                    _ => None,
                })
                .unwrap();
            let highlighted: Vec<_> = galley
                .job
                .sections
                .iter()
                .filter(|section| section.format.background != Color32::TRANSPARENT)
                .map(|section| {
                    &galley.job.text[section.byte_range.start.0..section.byte_range.end.0]
                })
                .collect();
            assert_eq!(
                highlighted,
                if expected.starts_with("old") {
                    vec!["old", "old"]
                } else {
                    vec!["new", "new"]
                }
            );
        }
        output.drop_without_applying_deltas();
    }
}

#[test]
fn filter_preserves_folder_anchor_and_nearest_survivor() {
    let mut app = unfiltered_app();
    app.session.row_height = 34.0;
    app.session.tree = Some(FolderTree::from_diff(&DirectoryDiff {
        cancelled: false,
        entries: (0..100)
            .map(|index| DirectoryEntry {
                relative_path: format!("entry-{index:03}.txt").into(),
                left_size: Some(1),
                right_size: Some(1),
                left_exists: true,
                right_exists: true,
                left_kind: Some(DirectoryEntryKind::File),
                right_kind: Some(DirectoryEntryKind::File),
                kind: DirectoryEntryKind::File,
                state: if index % 2 == 0 {
                    DirectoryEntryState::Different
                } else {
                    DirectoryEntryState::Same
                },
            })
            .collect(),
    }));
    app.session.selected = Some("entry-040.txt".into());
    app.session.tree_scroll_y = 40.0 * app.session.row_height + 7.0;
    app.session.set_differences_only(true);
    assert_eq!(
        app.session.tree_scroll_y,
        20.0 * app.session.row_height + 7.0
    );
    assert_eq!(
        app.session.selected.as_deref(),
        Some(std::path::Path::new("entry-040.txt"))
    );
    app.session.set_differences_only(false);
    assert_eq!(
        app.session.tree_scroll_y,
        40.0 * app.session.row_height + 7.0
    );
    app.session.tree_scroll_y = 41.0 * app.session.row_height + 7.0;
    app.session.set_differences_only(true);
    assert_eq!(
        app.session.tree_scroll_y,
        21.0 * app.session.row_height + 7.0
    );
    app.session.set_differences_only(false);
    app.session.tree_scroll_y = 99.0 * app.session.row_height;
    app.session.set_differences_only(true);
    assert_eq!(app.session.tree_scroll_y, 49.0 * app.session.row_height);
}

#[test]
fn navigation_status_reports_current_change_in_both_views() {
    let mut app = loaded_app();
    app.session.folder_differences = vec!["a.txt".into(), "b.txt".into(), "c.txt".into()];
    app.session.selected = Some("b.txt".into());
    assert_eq!(app.session.navigation_status(), "Change 2 of 3");
    app.session.file_view = Some(loaded_file_view());
    let view = app.session.file_view.as_mut().unwrap();
    view.navigation_row = Some(view.difference_rows[0]);
    let count = view.difference_rows.len();
    assert_eq!(
        app.session.navigation_status(),
        format!("Change 1 of {count}")
    );
}
