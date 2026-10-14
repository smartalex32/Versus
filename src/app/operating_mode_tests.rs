use super::*;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct SourceFixture {
    root: PathBuf,
}

impl SourceFixture {
    fn new() -> Self {
        static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "versus-operating-mode-{}-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("left")).unwrap();
        std::fs::create_dir_all(root.join("right")).unwrap();
        std::fs::write(root.join("left/one.txt"), "left one\n").unwrap();
        std::fs::write(root.join("right/one.txt"), "right one\n").unwrap();
        std::fs::write(root.join("left/new.txt"), "new left\n").unwrap();
        std::fs::write(root.join("right/other.txt"), "right other\n").unwrap();
        Self { root }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }
}

impl Drop for SourceFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn draw(app: &mut VersusApp, ctx: &egui::Context, input: egui::RawInput) -> egui::FullOutput {
    let mut output = ctx.run_ui(input, |ui| app.render(ui));
    output.textures_delta.clear();
    output
}

fn frame(app: &mut VersusApp, ctx: &egui::Context) -> egui::FullOutput {
    draw(
        app,
        ctx,
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            ..Default::default()
        },
    )
}

fn tick(app: &mut VersusApp, ctx: &egui::Context) {
    app.poll_sources(ctx);
    app.poll_comparison(ctx);
    app.poll_file_comparison(ctx);
    if let Some(view) = &mut app.interactive {
        view.poll(ctx);
    }
    if let Some(browser) = &mut app.merge_folders {
        browser.poll(ctx);
    }
    if let (Some(browser), Some(view)) = (&mut app.merge_folders, &mut app.interactive) {
        browser.poll_save(view, ctx);
    }
    app.sync_folder_merge_diff();
    frame(app, ctx).drop_without_applying_deltas();
}

fn settle_sources(app: &mut VersusApp, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while app.source_jobs.iter().any(Option::is_some) {
        tick(app, ctx);
        assert!(Instant::now() < deadline, "source classification timed out");
        thread::sleep(Duration::from_millis(1));
    }
    tick(app, ctx);
}

fn settle_interactive(app: &mut VersusApp, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while app.interactive.as_ref().is_some_and(|view| {
        view.current_paths().is_none() || view.pair_paths() != view.current_paths()
    }) {
        tick(app, ctx);
        assert!(Instant::now() < deadline, "working-copy load timed out");
        thread::sleep(Duration::from_millis(1));
    }
}

fn settle_interactive_diff(app: &mut VersusApp, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while app
        .interactive
        .as_ref()
        .unwrap()
        .loaded_pair_comparison()
        .is_none()
    {
        tick(app, ctx);
        assert!(
            Instant::now() < deadline,
            "working-copy comparison timed out"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

fn settle_folder_diff(app: &mut VersusApp, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while app.job.is_some() {
        tick(app, ctx);
        assert!(Instant::now() < deadline, "folder comparison timed out");
        thread::sleep(Duration::from_millis(1));
    }
}

#[derive(Debug)]
struct TestDrop(PathBuf);

impl egui::DroppedFile for TestDrop {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        panic!("merge drop must be classified from its path, not loaded on the UI thread")
    }
}

fn drop_on(app: &mut VersusApp, ctx: &egui::Context, side: usize, path: PathBuf) {
    frame(app, ctx).drop_without_applying_deltas();
    let position = app.pane_rects[side].center();
    draw(
        app,
        ctx,
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            events: vec![egui::Event::PointerMoved(position)],
            dropped_files: vec![Arc::new(TestDrop(path)) as egui::DroppedFileHandle],
            ..Default::default()
        },
    )
    .drop_without_applying_deltas();
}

#[test]
fn merge_selected_before_unrelated_files_never_starts_legacy_file_comparison() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.comparison_mode = ComparisonMode::File;
    app.set_workspace_merge(true);
    app.select_path(0, sources.path("left/one.txt"));
    app.select_path(1, sources.path("right/other.txt"));

    assert!(app.file_view.is_none());
    assert!(app.job.is_none());
    assert!(app.source_jobs.iter().any(Option::is_some));
    settle_sources(&mut app, &ctx);
    settle_interactive(&mut app, &ctx);

    let view = app.interactive.as_ref().unwrap();
    assert_eq!(
        view.current_paths(),
        Some(&[
            sources.path("left/one.txt"),
            sources.path("right/other.txt")
        ])
    );
    assert!(!view.run_diff_enabled());
    assert_eq!(view.merge_options(), [false; 3]);
    assert_eq!(
        [
            app.show_only_differences,
            app.ignore_whitespace,
            app.ignore_line_endings,
        ],
        [true; 3]
    );
    assert!(app.file_view.is_none() && app.job.is_none() && app.tree.is_none());
}

#[test]
fn picker_selection_is_not_overwritten_by_an_older_loaded_pair_event() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.comparison_mode = ComparisonMode::File;
    app.set_workspace_merge(true);
    let old = [sources.path("left/one.txt"), sources.path("right/one.txt")];
    app.paths = old.clone().map(|path| path.display().to_string());
    app.source_paths = old.clone().map(Some);
    app.source_kinds = [Some(SourceKind::File); 2];
    app.merge_file_open = true;
    app.merge_file_sides = [true; 2];
    app.interactive.as_mut().unwrap().open_pair(old);
    let deadline = Instant::now() + Duration::from_secs(3);
    while app.interactive.as_ref().unwrap().current_paths().is_none() {
        app.interactive.as_mut().unwrap().poll(&ctx);
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    let selected = sources.path("left/new.txt");
    app.select_path(0, selected.clone());
    frame(&mut app, &ctx).drop_without_applying_deltas();
    assert_eq!(app.source_paths[0], Some(selected.clone()));
    assert_eq!(app.source_kinds[0], None);
    settle_sources(&mut app, &ctx);
    settle_interactive(&mut app, &ctx);
    assert_eq!(
        app.interactive.as_ref().unwrap().current_paths().unwrap()[0],
        selected
    );
}

#[test]
fn merge_and_diff_toggles_keep_working_copies_and_mode_specific_preferences() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.comparison_mode = ComparisonMode::File;
    app.set_workspace_merge(true);
    app.select_path(0, sources.path("left/one.txt"));
    app.select_path(1, sources.path("right/one.txt"));
    settle_sources(&mut app, &ctx);
    settle_interactive(&mut app, &ctx);

    let right = sources.path("right/one.txt");
    let mut edited = app.interactive.as_ref().unwrap().export_working_files();
    edited.get_mut(&right).unwrap().text = "edited without alignment\n".into();
    app.interactive
        .as_mut()
        .unwrap()
        .ingest_working_files(&edited);
    assert!(app.interactive.as_ref().unwrap().any_dirty());

    app.set_workspace_merge(false);
    assert_eq!(
        app.interactive.as_ref().unwrap().working_text(1),
        Some("edited without alignment\n")
    );
    assert_eq!(
        [
            app.show_only_differences,
            app.ignore_whitespace,
            app.ignore_line_endings,
        ],
        [true; 3]
    );
    app.set_workspace_merge(true);
    let view = app.interactive.as_ref().unwrap();
    assert_eq!(view.working_text(1), Some("edited without alignment\n"));
    assert_eq!(view.merge_options(), [false; 3]);
    assert!(!view.run_diff_enabled());
    assert!(view.any_dirty());
}

#[test]
fn file_merge_to_diff_has_one_header_refresh_and_the_same_row_styles_as_new_file_diff() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let sources = SourceFixture::new();
        let ctx = egui::Context::default();
        set_theme(&ctx, theme);
        let mut app = VersusApp::default();
        app.comparison_mode = ComparisonMode::File;
        app.set_workspace_merge(true);
        app.select_path(0, sources.path("left/one.txt"));
        app.select_path(1, sources.path("right/one.txt"));
        settle_sources(&mut app, &ctx);
        settle_interactive(&mut app, &ctx);
        app.set_workspace_merge(false);
        settle_interactive_diff(&mut app, &ctx);
        for _ in 0..3 {
            frame(&mut app, &ctx).drop_without_applying_deltas();
        }
        let output = frame(&mut app, &ctx);
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            })
            .collect();
        assert_eq!(
            texts
                .iter()
                .filter(|text| text.ends_with("one.txt"))
                .count(),
            2,
            "one file-path header per side"
        );
        assert!(!texts.contains(&"LEFT (Incoming)") && !texts.contains(&"RIGHT (Current)"));
        let refresh = ctx
            .read_response(egui::Id::new("Refresh comparison"))
            .unwrap();
        assert!(refresh.enabled());
        let row_style = |output: &egui::FullOutput, label: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => Some(
                        text.galley
                            .job
                            .sections
                            .iter()
                            .map(|section| section.format.clone())
                            .collect::<Vec<_>>(),
                    ),
                    _ => None,
                })
                .expect("file row should be visible")
        };
        let switched_styles = [
            row_style(&output, "left one"),
            row_style(&output, "right one"),
        ];
        output.drop_without_applying_deltas();
        let native_ctx = egui::Context::default();
        set_theme(&native_ctx, theme);
        let mut native = VersusApp::default();
        native.comparison_mode = ComparisonMode::File;
        native.select_path(0, sources.path("left/one.txt"));
        native.select_path(1, sources.path("right/one.txt"));
        settle_sources(&mut native, &native_ctx);
        let deadline = Instant::now() + Duration::from_secs(3);
        while native.file_view.as_ref().unwrap().job.is_some() {
            tick(&mut native, &native_ctx);
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        let native_output = frame(&mut native, &native_ctx);
        assert_eq!(
            switched_styles,
            [
                row_style(&native_output, "left one"),
                row_style(&native_output, "right one")
            ]
        );
        native_output.drop_without_applying_deltas();
        // Browse must keep using the retained workspace after switching modes,
        // rather than opening a separate comparison hidden behind stale panes.
        app.select_path(1, sources.path("right/other.txt"));
        settle_sources(&mut app, &ctx);
        settle_interactive(&mut app, &ctx);
        settle_interactive_diff(&mut app, &ctx);
        assert!(!app.merge_mode);
        assert_eq!(
            app.interactive.as_ref().unwrap().working_text(1),
            Some("right other\n")
        );
        let changed = frame(&mut app, &ctx);
        assert!(changed.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "right other")));
        changed.drop_without_applying_deltas();
        let mut equal = app.interactive.as_ref().unwrap().export_working_files();
        equal
            .get_mut(&sources.path("right/other.txt"))
            .unwrap()
            .text = "left one\n".into();
        app.interactive
            .as_mut()
            .unwrap()
            .ingest_working_files(&equal);
        settle_interactive_diff(&mut app, &ctx);
        let equal_output = frame(&mut app, &ctx);
        assert_eq!(
            equal_output
                .shapes
                .iter()
                .filter(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "No differences"))
                .count(),
            1
        );
        equal_output.drop_without_applying_deltas();
    }
}

#[test]
fn transitioned_file_diff_refresh_compares_unsaved_text_and_merge_history_survives() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    set_theme(&ctx, egui::Theme::Dark);
    let mut app = VersusApp::default();
    app.comparison_mode = ComparisonMode::File;
    app.set_workspace_merge(true);
    app.select_path(0, sources.path("left/one.txt"));
    app.select_path(1, sources.path("right/one.txt"));
    settle_sources(&mut app, &ctx);
    settle_interactive(&mut app, &ctx);
    let right = sources.path("right/one.txt");
    let mut working = app.interactive.as_ref().unwrap().export_working_files();
    working.get_mut(&right).unwrap().text = "unsaved working text\n".into();
    app.interactive
        .as_mut()
        .unwrap()
        .ingest_working_files(&working);
    app.set_workspace_merge(false);
    settle_interactive_diff(&mut app, &ctx);
    for _ in 0..3 {
        frame(&mut app, &ctx).drop_without_applying_deltas();
    }
    let refresh = ctx
        .read_response(egui::Id::new("Refresh comparison"))
        .unwrap()
        .rect
        .center();
    for pressed in [true, false] {
        draw(
            &mut app,
            &ctx,
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                events: vec![
                    egui::Event::PointerMoved(refresh),
                    egui::Event::PointerButton {
                        pos: refresh,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            },
        )
        .drop_without_applying_deltas();
    }
    settle_interactive_diff(&mut app, &ctx);
    let output = frame(&mut app, &ctx);
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "unsaved working text")));
    output.drop_without_applying_deltas();
    assert_eq!(std::fs::read_to_string(&right).unwrap(), "right one\n");
    app.set_workspace_merge(true);
    assert_eq!(
        app.interactive.as_ref().unwrap().working_text(1),
        Some("unsaved working text\n")
    );
    assert!(!app.interactive.as_ref().unwrap().run_diff_enabled());
    for _ in 0..3 {
        frame(&mut app, &ctx).drop_without_applying_deltas();
    }
    let undo = ctx
        .read_response(egui::Id::new("Undo"))
        .unwrap()
        .rect
        .center();
    for pressed in [true, false] {
        draw(
            &mut app,
            &ctx,
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                events: vec![
                    egui::Event::PointerMoved(undo),
                    egui::Event::PointerButton {
                        pos: undo,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            },
        )
        .drop_without_applying_deltas();
    }
    assert_eq!(
        app.interactive.as_ref().unwrap().working_text(1),
        Some("right one\n")
    );
}

#[test]
fn merge_folder_navigation_is_metadata_only_until_run_diff_is_enabled() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.set_workspace_merge(true);
    app.select_path(0, sources.path("left"));
    app.select_path(1, sources.path("right"));
    settle_sources(&mut app, &ctx);
    tick(&mut app, &ctx);

    assert!(app.merge_folders.is_some());
    assert!(!app.merge_file_open);
    assert!(app.file_view.is_none());
    assert!(app.job.is_none());
    assert!(app.tree.is_none());
    let view = app.interactive.as_ref().unwrap();
    assert!(!view.run_diff_enabled());
    assert!(view.current_paths().is_none());
}

#[test]
fn switching_folder_modes_replaces_results_with_the_correct_ignore_preferences() {
    let sources = SourceFixture::new();
    std::fs::write(sources.path("left/one.txt"), "let a = 1;\n").unwrap();
    std::fs::write(sources.path("right/one.txt"), "let a=1;\n").unwrap();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.set_workspace_merge(true);
    app.select_path(0, sources.path("left"));
    app.select_path(1, sources.path("right"));
    settle_sources(&mut app, &ctx);
    app.interactive.as_mut().unwrap().set_run_diff(true);
    let state = |app: &VersusApp| {
        app.tree
            .as_ref()
            .unwrap()
            .all_rows()
            .into_iter()
            .find(|row| row.node.relative_path == PathBuf::from("one.txt"))
            .unwrap()
            .node
            .state
            .clone()
    };
    tick(&mut app, &ctx);
    settle_folder_diff(&mut app, &ctx);
    assert_eq!(state(&app), DirectoryEntryState::Different);
    app.set_workspace_merge(false);
    settle_folder_diff(&mut app, &ctx);
    assert_eq!(state(&app), DirectoryEntryState::Same);
    app.set_workspace_merge(true);
    tick(&mut app, &ctx);
    settle_folder_diff(&mut app, &ctx);
    assert_eq!(state(&app), DirectoryEntryState::Different);
    assert!(app.ignore_whitespace);
    assert_eq!(
        app.interactive.as_ref().unwrap().merge_options(),
        [false; 3]
    );
}

#[test]
fn returning_from_a_folder_merge_editor_to_diff_reuses_roots_and_starts_comparison() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.select_path(0, sources.path("left"));
    app.select_path(1, sources.path("right"));
    settle_sources(&mut app, &ctx);
    settle_folder_diff(&mut app, &ctx);
    assert!(app.tree.is_some());

    app.open_file(PathBuf::from("one.txt"), [true, true], false);
    app.set_workspace_merge(true);
    settle_interactive(&mut app, &ctx);
    assert!(app.merge_folders.is_some());
    assert_eq!(
        app.interactive.as_ref().unwrap().current_paths(),
        Some(&[sources.path("left/one.txt"), sources.path("right/one.txt")])
    );

    app.set_workspace_merge(false);
    let output = frame(&mut app, &ctx);
    let back = ctx
        .read_response(egui::Id::new("Back to folders"))
        .expect("folder merge editor should expose Back")
        .rect
        .center();
    output.drop_without_applying_deltas();
    for pressed in [true, false] {
        draw(
            &mut app,
            &ctx,
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                events: vec![
                    egui::Event::PointerMoved(back),
                    egui::Event::PointerButton {
                        pos: back,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
        )
        .drop_without_applying_deltas();
    }

    assert!(!app.merge_mode);
    assert!(!app.merge_file_open);
    assert!(app.merge_folders.is_some());
    assert!(
        app.job.is_some(),
        "Diff resumes a folder comparison after Back"
    );
}

#[test]
fn dropping_a_file_in_a_folder_merge_editor_replaces_only_that_editor_side() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    app.set_workspace_merge(true);
    app.select_path(0, sources.path("left"));
    app.select_path(1, sources.path("right"));
    settle_sources(&mut app, &ctx);
    let pair = [sources.path("left/one.txt"), sources.path("right/one.txt")];
    app.interactive.as_mut().unwrap().open_pair(pair.clone());
    app.merge_file_open = true;
    app.merge_file_sides = [true; 2];
    settle_interactive(&mut app, &ctx);

    drop_on(&mut app, &ctx, 0, sources.path("left/new.txt"));
    settle_interactive(&mut app, &ctx);
    assert_eq!(
        app.interactive.as_ref().unwrap().current_paths(),
        Some(&[sources.path("left/new.txt"), pair[1].clone()])
    );
    assert!(app.merge_folders.is_some());
    assert_eq!(app.source_paths[1], Some(sources.path("right")));
}
