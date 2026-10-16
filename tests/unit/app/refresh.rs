//! Refresh regression coverage.

use super::*;
use crate::app::visuals::theme::{ROW_HEIGHT, apply_theme, set_theme};

#[test]
fn toolbar_icons_expand_collapse_and_refresh_the_comparison() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    app.session.paths = ["left".into(), "right".into()];
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    let expand = ctx
        .read_response(egui::Id::new("Expand all"))
        .unwrap()
        .rect
        .center();
    click(&mut app, &ctx, expand);
    assert!(app.session.tree.as_ref().unwrap().is_expanded("assembly"));
    let collapse = ctx
        .read_response(egui::Id::new("Collapse all"))
        .unwrap()
        .rect
        .center();
    click(&mut app, &ctx, collapse);
    assert!(!app.session.tree.as_ref().unwrap().is_expanded("assembly"));
    let refresh = ctx
        .read_response(egui::Id::new("Refresh comparison"))
        .unwrap()
        .rect
        .center();
    click(&mut app, &ctx, refresh);
    assert!(app.session.job.is_some());
}

#[test]
fn merge_folder_search_and_refresh_are_icon_actions_in_the_expected_rows() {
    let fixture = SourceFixture::new();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    app.session.set_workspace_merge(true);
    app.session.select_path(0, fixture.path("left"));
    settle_sources(&mut app, &ctx);
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    let new = ctx
        .read_response(egui::Id::new("New comparison"))
        .unwrap()
        .rect;
    let search = ctx
        .read_response(egui::Id::new("Find and replace"))
        .unwrap()
        .rect;
    assert!(search.left() > new.right());
    assert!((search.center().y - new.center().y).abs() < 0.5);
    assert!(
        ctx.read_response(egui::Id::new("Refresh comparison"))
            .is_none()
    );
    click_action(&mut app, &ctx, "Find and replace");
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "Find and Replace").len(), 1);
    output.drop_without_applying_deltas();
    app.session.interactive.as_mut().unwrap().set_run_diff(true);
    let output = render(&mut app, &ctx, vec![]);
    let refresh = ctx
        .read_response(egui::Id::new("Refresh comparison"))
        .unwrap()
        .rect;
    let run = text_positions(&output, "Run Diff")[0];
    assert!(refresh.left() > run.x);
    assert!(refresh.top() < app.pane_rects[0].top());
    output.drop_without_applying_deltas();
}

#[test]
fn ignore_refresh_keeps_file_rows_and_viewport_until_results_are_ready() {
    let sources = SourceFixture::new();
    let left: String = (1..=200)
        .map(|number| format!("line {number}: value\n"))
        .collect();
    let right: String = (1..=200)
        .map(|number| match number {
            81 => "line 81: changed\n".into(),
            100 => "line 100:  value\n".into(),
            _ => format!("line {number}: value\n"),
        })
        .collect();
    std::fs::write(sources.path("left/model.txt"), left).unwrap();
    std::fs::write(sources.path("right/model.txt"), right).unwrap();
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        set_theme(&ctx, theme);
        let mut app = unfiltered_app();
        app.session.open_launch_request(crate::cli::LaunchRequest {
            paths: [
                sources.path("left/model.txt"),
                sources.path("right/model.txt"),
            ],
            mode: None,
        });
        settle_sources(&mut app, &ctx);
        let view = app.session.file_view.as_mut().unwrap();
        view.scroll_y = 80.0 * ROW_HEIGHT + 7.0;
        view.navigation_row = Some(80);
        let before = render(&mut app, &ctx, vec![]);
        let positions = text_positions(&before, "line 81: value");
        assert_eq!(positions.len(), 1);
        let panes = app.pane_rects;
        before.drop_without_applying_deltas();
        let updating = click_action_frame(&mut app, &ctx, "Ignore whitespace");
        assert_eq!(positions, text_positions(&updating, "line 81: value"));
        assert_eq!(panes, app.pane_rects);
        assert!(text_positions(&updating, "Loading file comparison…").is_empty());
        updating.drop_without_applying_deltas();
        let view = app.session.file_view.as_ref().unwrap();
        assert!(view.job.is_some() && view.comparison.is_some());
        assert_eq!(view.navigation_row, Some(80));
        assert_eq!(view.counts[1], 2);
        settle_sources(&mut app, &ctx);
        let after = render(&mut app, &ctx, vec![]);
        assert_eq!(positions, text_positions(&after, "line 81: value"));
        assert_eq!(panes, app.pane_rects);
        let view = app.session.file_view.as_ref().unwrap();
        assert_eq!(view.counts.iter().sum::<usize>(), 200);
        assert_eq!(view.counts[1], 1);
        assert_eq!(view.navigation_row, Some(80));
        assert_eq!(view.scroll_y, 80.0 * ROW_HEIGHT + 7.0);
        after.drop_without_applying_deltas();
    }
}

#[test]
fn ignore_refresh_keeps_folder_rows_expansion_selection_and_scroll() {
    let sources = SourceFixture::new();
    for side in ["left", "right"] {
        std::fs::create_dir_all(sources.path(&format!("{side}/z_group"))).unwrap();
        std::fs::write(sources.path(&format!("{side}/z_group/detail.txt")), side).unwrap();
        for number in 0..80 {
            let text = if side == "right" && number == 20 {
                "same  value\n"
            } else {
                "same value\n"
            };
            std::fs::write(sources.path(&format!("{side}/a_{number:03}.txt")), text).unwrap();
        }
    }
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = unfiltered_app();
    app.session.open_launch_request(crate::cli::LaunchRequest {
        paths: [sources.path("left"), sources.path("right")],
        mode: None,
    });
    settle_sources(&mut app, &ctx);
    app.session.tree.as_mut().unwrap().expand_all();
    app.session.selected = Some("z_group/detail.txt".into());
    app.session.tree_scroll_y = 20.0 * ROW_HEIGHT + 7.0;
    let before = render(&mut app, &ctx, vec![]);
    let positions = text_positions(&before, "a_020.txt");
    assert_eq!(positions.len(), 2);
    let panes = app.pane_rects;
    let generation = app.session.scroll_generation;
    before.drop_without_applying_deltas();
    let updating = click_action_frame(&mut app, &ctx, "Ignore whitespace");
    assert_eq!(positions, text_positions(&updating, "a_020.txt"));
    assert_eq!(panes, app.pane_rects);
    assert!(app.session.tree.is_some() && app.session.job.is_some());
    updating.drop_without_applying_deltas();
    settle_sources(&mut app, &ctx);
    let after = render(&mut app, &ctx, vec![]);
    assert_eq!(positions, text_positions(&after, "a_020.txt"));
    assert_eq!(panes, app.pane_rects);
    assert_eq!(app.session.scroll_generation, generation);
    assert_eq!(
        app.session.selected.as_deref(),
        Some(std::path::Path::new("z_group/detail.txt"))
    );
    assert!(app.session.tree.as_ref().unwrap().is_expanded("z_group"));
    assert_eq!(app.session.tree_scroll_y, 20.0 * ROW_HEIGHT + 7.0);
    assert_eq!(app.session.counts[1], 3); // model.txt, z_group and its detail.txt
    after.drop_without_applying_deltas();
}

#[test]
fn file_refresh_preserves_source_line_anchors_when_alignment_changes() {
    let ctx = egui::Context::default();
    let mut app = loaded_app();
    let mut view = loaded_file_view();
    view.scroll_y = ROW_HEIGHT + 5.0;
    view.navigation_row = Some(1);
    let mut replacement = view.comparison.as_ref().unwrap().clone();
    replacement.rows.insert(
        1,
        test_file_row(
            None,
            Some((99, "inserted".into())),
            DirectoryEntryState::RightOnly,
        ),
    );
    let (sender, receiver) = mpsc::channel();
    view.job = Some(FileJob {
        receiver,
        cancellation: Arc::new(AtomicBool::new(false)),
        progress: Arc::new(ComparisonProgress::default()),
        ignore_line_endings: false,
    });
    let sources = view.sources.clone();
    app.session.file_view = Some(view);
    sender
        .send(Ok(Some(LoadedFile {
            comparison: replacement,
            sources,
        })))
        .unwrap();
    app.session.poll_file_comparison(&ctx);
    let view = app.session.file_view.as_ref().unwrap();
    assert_eq!(view.scroll_y, 2.0 * ROW_HEIGHT + 5.0);
    assert_eq!(view.navigation_row, Some(2));
    assert_eq!(view.counts, [1, 1, 1, 1, 0, 0]);
    assert!(!view.ignore_line_endings);
}

#[test]
fn failed_or_cancelled_file_refresh_keeps_the_last_result_visible() {
    for cancelled in [false, true] {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        let mut view = loaded_file_view();
        let before = view.comparison.clone();
        let (sender, receiver) = mpsc::channel();
        view.job = Some(FileJob {
            receiver,
            cancellation: Arc::new(AtomicBool::new(cancelled)),
            progress: Arc::new(ComparisonProgress::default()),
            ignore_line_endings: false,
        });
        let sources = view.sources.clone();
        app.session.file_view = Some(view);
        if cancelled {
            sender
                .send(Ok(Some(LoadedFile {
                    comparison: FileComparison {
                        rows: vec![],
                        message: None,
                    },
                    sources,
                })))
                .unwrap();
        } else {
            sender
                .send(Err(CompareError {
                    path: None,
                    kind: versus::CompareErrorKind::InvalidPath,
                    message: "file unavailable".into(),
                }))
                .unwrap();
        }
        app.session.poll_file_comparison(&ctx);
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(text_positions(&output, "old value").len(), 1);
        let view = app.session.file_view.as_ref().unwrap();
        assert_eq!(view.comparison, before);
        assert!(view.error.is_some() && view.job.is_none());
        assert!(view.ignore_line_endings);
        output.drop_without_applying_deltas();
    }
}

#[test]
fn ignore_buttons_recompare_file_and_folder_results_and_show_ending_changes() {
    let sources = SourceFixture::new();
    std::fs::write(sources.path("left/model.txt"), "let value = 1;\r\n").unwrap();
    std::fs::write(sources.path("right/model.txt"), "letvalue=1;\n").unwrap();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = unfiltered_app();
    app.session.open_launch_request(crate::cli::LaunchRequest {
        paths: [sources.path("left"), sources.path("right")],
        mode: None,
    });
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.counts[1], 1);
    app.session
        .open_file("model.txt".into(), [true, true], false);
    settle_sources(&mut app, &ctx);
    click_action(&mut app, &ctx, "Ignore whitespace");
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.counts[1], 0);
    assert_eq!(app.session.file_view.as_ref().unwrap().counts[1], 0);
    click_action(&mut app, &ctx, "Show only differences");
    let output = render(&mut app, &ctx, vec![]);
    assert!(!text_positions(&output, "No differences with the current options.").is_empty());
    output.drop_without_applying_deltas();
    click_action(&mut app, &ctx, "Ignore line endings");
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.counts[1], 1);
    assert_eq!(app.session.file_view.as_ref().unwrap().counts[1], 1);
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "CRLF").len(), 1);
    assert_eq!(text_positions(&output, "LF").len(), 1);
    output.drop_without_applying_deltas();
    click_action(&mut app, &ctx, "Back to folders");
    assert!(app.session.file_view.is_none());
    assert_eq!(app.session.counts[1], 1);
}

#[test]
fn refreshing_a_folder_drilldown_rechecks_newly_created_sides() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.session.select_path(0, sources.path("left"));
    app.session.select_path(1, sources.path("right"));
    settle_sources(&mut app, &ctx);
    std::fs::remove_file(sources.path("right/model.txt")).unwrap();
    app.session
        .open_file("model.txt".into(), [true, false], false);
    settle_sources(&mut app, &ctx);
    std::fs::write(sources.path("right/model.txt"), "common\nold\n").unwrap();
    app.session.refresh_file();
    settle_sources(&mut app, &ctx);
    assert!(
        app.session
            .file_view
            .as_ref()
            .unwrap()
            .sources
            .iter()
            .all(Option::is_some)
    );
    assert_eq!(app.session.file_view.as_ref().unwrap().counts[0], 2);
}

#[cfg(unix)]
#[test]
fn refreshing_a_folder_drilldown_does_not_follow_a_symlink() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.session.select_path(0, sources.path("left"));
    app.session.select_path(1, sources.path("right"));
    settle_sources(&mut app, &ctx);
    app.session
        .open_file("model.txt".into(), [true, true], false);
    settle_sources(&mut app, &ctx);
    std::fs::remove_file(sources.path("left/model.txt")).unwrap();
    std::os::unix::fs::symlink(
        sources.path("right/model.txt"),
        sources.path("left/model.txt"),
    )
    .unwrap();
    app.session.refresh_file();
    settle_sources(&mut app, &ctx);
    assert!(
        app.session
            .file_view
            .as_ref()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("non-symlink")
    );
}

#[test]
fn folder_file_refresh_reports_inspection_errors_instead_of_an_empty_side() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.session.select_path(0, sources.path("left"));
    app.session.select_path(1, sources.path("right"));
    settle_sources(&mut app, &ctx);
    app.session
        .open_file("model.txt".into(), [true, true], false);
    settle_sources(&mut app, &ctx);
    // A child below a regular file is NotADirectory on Unix but NotFound on
    // Windows. An embedded NUL produces a genuine inspection error on both.
    let invalid_path = sources.path("right/invalid\0path.txt");
    let inspection_error = std::fs::symlink_metadata(&invalid_path).unwrap_err();
    assert_eq!(inspection_error.kind(), std::io::ErrorKind::InvalidInput);
    let previous = app.session.file_view.as_ref().unwrap().comparison.clone();
    app.session.file_view.as_mut().unwrap().paths[1] = invalid_path;
    app.session.refresh_file();
    settle_sources(&mut app, &ctx);
    assert!(app.session.file_view.as_ref().unwrap().error.is_some());
    assert_eq!(app.session.file_view.as_ref().unwrap().comparison, previous);
}

#[test]
fn refresh_button_reloads_direct_files_and_folder_drilldown() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    for from_folders in [false, true] {
        std::fs::write(sources.path("right/model.txt"), "common\nnew\n").unwrap();
        let mut app = VersusApp::default();
        if from_folders {
            app.session.select_path(0, sources.path("left"));
            app.session.select_path(1, sources.path("right"));
            settle_sources(&mut app, &ctx);
            app.session.selected = Some("model.txt".into());
            app.session
                .open_file("model.txt".into(), [true, true], false);
        } else {
            app.session.comparison_mode = ComparisonMode::File;
            app.session.select_path(0, sources.path("left/model.txt"));
            app.session.select_path(1, sources.path("right/model.txt"));
        }
        settle_sources(&mut app, &ctx);
        let paths = app.session.file_view.as_ref().unwrap().paths.clone();
        assert_eq!(app.session.file_view.as_ref().unwrap().counts[1], 1);
        std::fs::write(sources.path("right/model.txt"), "common\nold\n").unwrap();
        render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
        let refresh = ctx
            .read_response(egui::Id::new("Refresh comparison"))
            .unwrap()
            .rect
            .center();
        click(&mut app, &ctx, refresh);
        assert!(app.session.file_view.as_ref().unwrap().job.is_some());
        assert!(app.session.job.is_none());
        settle_sources(&mut app, &ctx);
        let view = app.session.file_view.as_ref().unwrap();
        assert_eq!(view.paths, paths);
        assert_eq!(view.from_folders, from_folders);
        assert_eq!(view.counts[1], 0);
        assert_eq!(view.counts[0], 2);
        if from_folders {
            app.session.file_view = None;
            assert_eq!(
                app.session.selected.as_deref(),
                Some(std::path::Path::new("model.txt"))
            );
            assert!(app.session.tree.is_some());
        }
    }
}

#[test]
fn folder_file_refresh_discovers_created_and_removed_sides_and_rejects_symlinks() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.session.select_path(0, sources.path("left"));
    app.session.select_path(1, sources.path("right"));
    settle_sources(&mut app, &ctx);
    std::fs::remove_file(sources.path("right/model.txt")).unwrap();
    app.session
        .open_file("model.txt".into(), [true, false], false);
    settle_sources(&mut app, &ctx);
    std::fs::write(sources.path("right/model.txt"), "common\nold\n").unwrap();
    app.session.refresh_file();
    settle_sources(&mut app, &ctx);
    assert!(
        app.session
            .file_view
            .as_ref()
            .unwrap()
            .sources
            .iter()
            .all(Option::is_some)
    );
    assert_eq!(app.session.file_view.as_ref().unwrap().counts[0], 2);
    std::fs::remove_file(sources.path("left/model.txt")).unwrap();
    app.session.refresh_file();
    settle_sources(&mut app, &ctx);
    assert!(app.session.file_view.as_ref().unwrap().sources[0].is_none());
    assert_eq!(app.session.file_view.as_ref().unwrap().counts[3], 2);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            sources.path("right/model.txt"),
            sources.path("left/model.txt"),
        )
        .unwrap();
        app.session.refresh_file();
        settle_sources(&mut app, &ctx);
        assert!(
            app.session
                .file_view
                .as_ref()
                .unwrap()
                .error
                .as_deref()
                .unwrap()
                .contains("non-symlink")
        );
    }
}
