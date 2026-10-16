//! Shared app fixtures and capability test index.

use super::*;
use crate::app::session::FileView;
use crate::app::visuals::status::StatusIcon;
use crate::app::visuals::theme::ROW_HEIGHT;
use crate::recent::RecentComparisons;
use eframe::Storage;
use versus::ComparisonProgress;
use versus::FileComparison;
use versus::{DirectoryDiff, DirectoryEntry};

mod accessibility;
mod difference_navigation;
mod drag_drop;
mod file_diff;
mod folder_diff;
mod merge_save_search;
mod merge_workspace;
mod recent_history;
mod refresh;
mod session_sources_interactions;
mod session_sources_lifecycle;
mod theme;
mod workspace_chrome;
mod zoom_input;

fn engineering_path(relative: &str) -> String {
    // Windows needs a drive-qualified root to avoid resolving fixtures
    // relative to the CI checkout. Build components with native separators.
    let mut path = PathBuf::from(if cfg!(windows) {
        r"C:\engineering"
    } else {
        "/engineering"
    });
    path.extend(relative.split('/'));
    assert!(path.is_absolute());
    path.display().to_string()
}

fn fixture() -> FolderTree {
    FolderTree::from_diff(&DirectoryDiff {
        cancelled: false,
        entries: [
            ("assembly", DirectoryEntryKind::Directory),
            ("assembly/model.step", DirectoryEntryKind::File),
        ]
        .into_iter()
        .map(|(path, kind)| DirectoryEntry {
            relative_path: path.into(),
            left_size: Some(0),
            right_size: Some(0),
            left_exists: true,
            right_exists: true,
            left_kind: Some(kind.clone()),
            right_kind: Some(kind.clone()),
            kind,
            state: DirectoryEntryState::Same,
        })
        .collect(),
    })
}

// Interaction fixtures exercise the full list and whitespace toggle in a
// known starting state; production defaults are covered separately below.
fn unfiltered_app() -> VersusApp {
    let mut app = VersusApp::default();
    app.session.show_only_differences = false;
    app.session.ignore_whitespace = false;
    app
}

fn loaded_app() -> VersusApp {
    let mut app = unfiltered_app();
    app.session.tree = Some(fixture());
    app.session.roots = Some(["left".into(), "right".into()]);
    app
}

fn render(app: &mut VersusApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    // Keep the simulated physical window size fixed when the UI scale changes.
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(1200.0, 800.0) / ctx.zoom_factor(),
            )),
            events,
            ..Default::default()
        },
        |ui| app.render(ui),
    );
    output.textures_delta.clear();
    output
}

fn text_positions(output: &egui::FullOutput, value: &str) -> Vec<Pos2> {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == value => {
                Some(text.pos + text.galley.rect.min.to_vec2())
            }
            _ => None,
        })
        .collect()
}

fn text_centers(output: &egui::FullOutput, value: &str) -> Vec<Pos2> {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == value => {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            _ => None,
        })
        .collect()
}

fn click(app: &mut VersusApp, ctx: &egui::Context, position: Pos2) {
    for pressed in [true, false] {
        render(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        )
        .drop_without_applying_deltas();
    }
}

fn test_file_row(
    left: Option<(usize, String)>,
    right: Option<(usize, String)>,
    state: DirectoryEntryState,
) -> versus::FileComparisonRow {
    versus::FileComparisonRow {
        left_ending: left.as_ref().map(|_| versus::DisplayLineEnding::Lf),
        right_ending: right.as_ref().map(|_| versus::DisplayLineEnding::Lf),
        left_changed: Vec::new(),
        right_changed: Vec::new(),
        left,
        right,
        state,
    }
}

fn loaded_file_view() -> FileView {
    FileView {
        row_height: ROW_HEIGHT,
        from_folders: true,
        paths: ["/left/model.step".into(), "/right/model.step".into()],
        sources: [
            Some("/left/model.step".into()),
            Some("/right/model.step".into()),
        ],
        job: None,
        comparison: Some(FileComparison {
            rows: vec![
                test_file_row(
                    Some((1, "unchanged".into())),
                    Some((1, "unchanged".into())),
                    DirectoryEntryState::Same,
                ),
                test_file_row(
                    Some((2, "old value".into())),
                    Some((2, "new value".into())),
                    DirectoryEntryState::Different,
                ),
                test_file_row(
                    Some((3, "removed".into())),
                    None,
                    DirectoryEntryState::LeftOnly,
                ),
            ],
            message: None,
        }),
        error: None,
        error_icon: StatusIcon::Error,
        scroll_y: 0.0,
        content_widths: [300.0; 2],
        gutter_extra: 0.0,
        counts: [1, 1, 1, 0, 0, 0],
        visible_rows: vec![0, 1, 2],
        difference_rows: vec![1, 2],
        navigation_row: None,
        navigation_scroll_pending: false,
        ignore_line_endings: true,
    }
}

fn attach_job(app: &mut VersusApp) -> mpsc::Sender<Result<FolderTree, CompareError>> {
    let (sender, receiver) = mpsc::channel();
    app.session.job = Some(ComparisonJob {
        receiver,
        cancellation: Arc::new(AtomicBool::new(false)),
        roots: ["new-left".into(), "new-right".into()],
        started: Instant::now(),
        logged_progress: None,
        last_progress_log: Instant::now(),
        progress: Arc::new(ComparisonProgress::default()),
    });
    sender
}

struct SourceFixture(PathBuf);

impl SourceFixture {
    fn new() -> Self {
        static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "versus-source-ui-{unique}-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("left")).unwrap();
        std::fs::create_dir_all(root.join("right")).unwrap();
        std::fs::write(root.join("left/model.txt"), "common\nold\n").unwrap();
        std::fs::write(root.join("right/model.txt"), "common\nnew\n").unwrap();
        Self(root)
    }
    fn path(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }
}

impl Drop for SourceFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn settle_sources(app: &mut VersusApp, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.session.poll_sources(ctx);
        app.session.poll_comparison(ctx);
        app.session.poll_file_comparison(ctx);
        if app.session.source_jobs.iter().all(Option::is_none)
            && app.session.job.is_none()
            && app
                .session
                .file_view
                .as_ref()
                .is_none_or(|view| view.job.is_none())
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Comparison worker did not complete"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn settle_merge_file(app: &mut VersusApp, ctx: &egui::Context, side: usize, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.session.poll_sources(ctx);
        app.session.interactive.as_mut().unwrap().poll(ctx);
        if app.session.interactive.as_ref().unwrap().working_text(side) == Some(expected) {
            break;
        }
        assert!(Instant::now() < deadline, "Merge side did not load");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn click_action(app: &mut VersusApp, ctx: &egui::Context, label: &str) {
    click_action_frame(app, ctx, label).drop_without_applying_deltas();
}

fn click_action_frame(app: &mut VersusApp, ctx: &egui::Context, label: &str) -> egui::FullOutput {
    render(app, ctx, vec![]).drop_without_applying_deltas();
    let response = ctx.read_response(egui::Id::new(label)).unwrap();
    assert!(response.enabled(), "{label} should be enabled");
    let position = response.rect.center();
    let events = |pressed| {
        vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ]
    };
    render(app, ctx, events(true)).drop_without_applying_deltas();
    render(app, ctx, events(false))
}

#[derive(Debug)]
struct TestDrop(PathBuf);
impl egui::DroppedFile for TestDrop {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        panic!("Drop handlers must not read file contents on the UI thread")
    }
}

fn drop_sources(app: &mut VersusApp, ctx: &egui::Context, side: usize, paths: Vec<PathBuf>) {
    render(app, ctx, vec![]).drop_without_applying_deltas();
    let position = app.pane_rects[side].center();
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            events: vec![egui::Event::PointerMoved(position)],
            dropped_files: paths
                .into_iter()
                .map(|path| Arc::new(TestDrop(path)) as egui::DroppedFileHandle)
                .collect(),
            ..Default::default()
        },
        |ui| app.render(ui),
    )
    .drop_without_applying_deltas();
}

#[derive(Default)]
struct ZoomStorage(std::collections::HashMap<String, String>);
impl eframe::Storage for ZoomStorage {
    fn get_string(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
    fn set_string(&mut self, key: &str, value: String) {
        self.0.insert(key.into(), value);
    }
    fn remove_string(&mut self, key: &str) {
        self.0.remove(key);
    }
    fn flush(&mut self) {}
}
