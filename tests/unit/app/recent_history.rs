//! Recent history regression coverage.

use super::*;
use crate::app::visuals::theme::{Palette, apply_theme, set_theme};

#[test]
fn recent_pairs_reopen_and_are_kept_when_starting_over() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let paths = [
        sources.path("left/model.txt"),
        sources.path("right/model.txt"),
    ];
    let mut app = VersusApp::default();
    app.session.open_recent(RecentComparison {
        mode: ComparisonMode::File,
        paths: paths.clone(),
    });
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.comparison_paths(), paths);
    assert_eq!(app.session.recent.entries().len(), 1);
    app.session.new_comparison();
    assert_eq!(app.session.recent.entries().len(), 1);
}

#[test]
fn recent_pairs_reopen_from_startup_and_survive_new_comparison() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    for mode in [ComparisonMode::File, ComparisonMode::Folder] {
        let mut app = VersusApp::default();
        let paths = match mode {
            ComparisonMode::File => [
                sources.path("left/model.txt"),
                sources.path("right/model.txt"),
            ],
            ComparisonMode::Folder => [sources.path("left"), sources.path("right")],
        };
        app.session.open_recent(RecentComparison {
            mode,
            paths: paths.clone(),
        });
        settle_sources(&mut app, &ctx);
        assert!(app.session.error.is_none());
        assert_eq!(app.session.comparison_mode, mode);
        assert_eq!(
            app.session.recent.entries(),
            &[RecentComparison {
                mode,
                paths: paths.clone()
            }]
        );
        assert_eq!(app.session.tree.is_some(), mode == ComparisonMode::Folder);
        assert_eq!(
            app.session.file_view.is_some(),
            mode == ComparisonMode::File
        );
        app.session.new_comparison();
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(text_positions(&output, "Recently compared").len(), 1);
        let target = ctx
            .read_response(egui::Id::new(("recent-comparison", mode.source_label(), 0)))
            .unwrap()
            .rect
            .center();
        output.drop_without_applying_deltas();
        click(&mut app, &ctx, target);
        settle_sources(&mut app, &ctx);
        assert!(app.session.error.is_none());
        assert_eq!(app.session.comparison_paths(), paths);
        assert_eq!(app.session.recent.entries().len(), 1);
    }
}

#[test]
fn recent_list_filters_by_mode_and_uses_plain_alternating_rows() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        set_theme(&ctx, theme);
        let mut app = VersusApp::default();
        for (mode, name) in [
            (ComparisonMode::Folder, "folder-old"),
            (ComparisonMode::File, "file-old"),
            (ComparisonMode::Folder, "folder-new"),
            (ComparisonMode::File, "file-new"),
        ] {
            app.session.recent.record(
                mode,
                [
                    format!("/{name}/left").into(),
                    format!("/{name}/right").into(),
                ],
            );
        }
        for (mode, visible, hidden) in [
            (ComparisonMode::Folder, "folder", "file"),
            (ComparisonMode::File, "file", "folder"),
        ] {
            app.session.switch_comparison_mode(mode);
            let output = render(&mut app, &ctx, vec![]);
            let palette = Palette::for_context(&ctx);
            let mut positions = Vec::new();
            for (index, age, fill) in [(0, "new", palette.panel), (1, "old", palette.alternate)] {
                let left = text_positions(&output, &format!("/{visible}-{age}/left"));
                let right = text_positions(&output, &format!("/{visible}-{age}/right"));
                assert_eq!(left.len(), 1);
                assert_eq!(right.len(), 1);
                assert_eq!(left[0].y, right[0].y);
                let row = ctx
                    .read_response(egui::Id::new((
                        "recent-comparison",
                        mode.source_label(),
                        index,
                    )))
                    .unwrap()
                    .rect;
                assert!(left[0].x < row.center().x && right[0].x > left[0].x);
                assert!(output.shapes.iter().any(|shape| {
                    matches!(&shape.shape, egui::Shape::Rect(shape) if shape.rect == row && shape.fill == fill && shape.stroke == Stroke::NONE)
                }));
                assert!(text_positions(&output, &format!("/{hidden}-{age}/left")).is_empty());
                assert!(text_positions(&output, &format!("/{hidden}-{age}/right")).is_empty());
                positions.push(left[0]);
            }
            assert_eq!(positions[0].x, positions[1].x);
            assert!(positions[0].y < positions[1].y);
            output.drop_without_applying_deltas();
        }
    }
}

#[test]
fn recent_list_shows_an_empty_message_for_the_active_mode() {
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.session.recent.record(
        ComparisonMode::Folder,
        ["/left/folder".into(), "/right/folder".into()],
    );
    app.session.switch_comparison_mode(ComparisonMode::File);
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(
        text_positions(&output, "No recent file comparisons.").len(),
        1
    );
    assert!(
        text_positions(&output, "No recent file comparisons.")[0].x
            > text_positions(&output, "Recently compared")[0].x + 6.0
    );
    assert!(text_positions(&output, "/left/folder").is_empty());
    assert!(text_positions(&output, "/right/folder").is_empty());
    output.drop_without_applying_deltas();
}

#[test]
fn recent_missing_or_changed_sources_report_errors_and_keep_history() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    for (paths, expected) in [
        (
            [sources.path("missing.txt"), sources.path("right/model.txt")],
            "Cannot inspect",
        ),
        (
            [sources.path("left"), sources.path("right")],
            "requires two regular",
        ),
    ] {
        let mut app = VersusApp::default();
        let entry = RecentComparison {
            mode: ComparisonMode::File,
            paths,
        };
        app.session.recent.record(entry.mode, entry.paths.clone());
        app.session.open_recent(entry.clone());
        settle_sources(&mut app, &ctx);
        assert!(app.session.error.as_deref().unwrap().contains(expected));
        assert!(app.session.job.is_none() && app.session.file_view.is_none());
        assert_eq!(app.session.recent.entries(), &[entry]);
        app.session.new_comparison();
        assert_eq!(app.session.recent.entries().len(), 1);
    }
}

#[test]
fn history_worker_finishes_writes_on_close_and_merges_startup_history() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let history_path = sources.path("settings/recent-comparisons.v1");
    let first = [sources.path("left"), sources.path("right")];
    let second = [
        sources.path("left/model.txt"),
        sources.path("right/model.txt"),
    ];
    let mut saved = RecentComparisons::default();
    saved.record(ComparisonMode::Folder, first.clone());
    saved.save_to(&history_path).unwrap();
    let mut app = VersusApp::default();
    app.session.recent_store = Some(RecentStore::start(&ctx, Some(history_path.clone())));
    app.session
        .record_recent(ComparisonMode::File, second.clone());
    // Closing must drain the queued save, including when load was still pending.
    drop(app);
    let reopened = RecentComparisons::load_from(&history_path).unwrap();
    assert_eq!(
        reopened.entries(),
        &[
            RecentComparison {
                mode: ComparisonMode::File,
                paths: second.clone()
            },
            RecentComparison {
                mode: ComparisonMode::Folder,
                paths: first.clone()
            },
        ]
    );
    let mut app = VersusApp::default();
    app.session.recent_store = Some(RecentStore::start(&ctx, Some(history_path)));
    app.session
        .record_recent(ComparisonMode::Folder, first.clone());
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.session.recent.entries().len() != 2 {
        app.session.poll_recent(&ctx);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        app.session.recent.entries(),
        &[
            RecentComparison {
                mode: ComparisonMode::Folder,
                paths: first
            },
            RecentComparison {
                mode: ComparisonMode::File,
                paths: second
            },
        ]
    );
    assert!(app.session.recent_message.is_none());
}
