use super::*;
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};

pub(super) struct GitFixture(PathBuf);

impl GitFixture {
    pub(super) fn new(name: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "versus-git-ui-{name}-{unique}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    pub(super) fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    pub(super) fn write(&self, name: &str, text: impl AsRef<[u8]>) -> PathBuf {
        let path = self.path(name);
        fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for GitFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn diff_request(paths: [PathBuf; 2]) -> GitDiffRequest {
    GitDiffRequest {
        paths,
        mode: Some(crate::selection::ComparisonMode::File),
        label: Some("test comparison".into()),
    }
}

pub(super) fn merge_request(fixture: &GitFixture) -> GitMergeRequest {
    GitMergeRequest {
        base: Some(fixture.path("base.txt")),
        local: fixture.path("local.txt"),
        remote: fixture.path("remote.txt"),
        output: fixture.path("result.txt"),
        label: Some("test merge".into()),
    }
}

fn draw(app: &mut GitApp, ctx: &egui::Context) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            ..Default::default()
        },
        |ui| app.render(ui),
    )
}

fn click(app: &mut GitApp, ctx: &egui::Context, label: &str) {
    // Modal placement is established in the first frame and its controls are
    // available to hit testing on the following frame.
    draw(app, ctx).drop_without_applying_deltas();
    let output = draw(app, ctx);
    let position = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("{label} was not rendered"));
    output.drop_without_applying_deltas();
    for pressed in [true, false] {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            },
            |ui| app.render(ui),
        )
        .drop_without_applying_deltas();
    }
}

fn settle_diff(app: &mut GitApp, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.poll(ctx);
        if matches!(&app.view, GitView::Diff(view) if view.exit_code() == 0) {
            return;
        }
        assert!(Instant::now() < deadline, "Git worker did not settle");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn git_diff_starts_polls_and_renders_without_an_ordinary_workspace_session() {
    let fixture = GitFixture::new("diff-app");
    let left = fixture.write("left.txt", "before\n");
    let right = fixture.write("right.txt", "after\n");
    let ctx = egui::Context::default();
    let completion = Arc::new(AtomicU8::new(1));
    let mut app = GitApp::start(
        &ctx,
        GitLaunch::Diff(diff_request([left, right])),
        completion.clone(),
        None,
    );

    assert!(matches!(&app.view, GitView::Diff(_)));
    settle_diff(&mut app, &ctx);
    let output = draw(&mut app, &ctx);
    assert!(!output.shapes.is_empty());
    output.drop_without_applying_deltas();
    app.poll(&ctx);
    assert_eq!(completion.load(Ordering::Acquire), 0);
}

#[test]
fn closing_git_diff_while_loading_or_reloading_advances_git() {
    let fixture = GitFixture::new("diff-close-loading");
    let paths = [
        fixture.write("left.txt", "before\n"),
        fixture.write("right.txt", "after\n"),
    ];
    let ctx = egui::Context::default();
    let completion = Arc::new(AtomicU8::new(1));
    let mut app = GitApp::start(
        &ctx,
        GitLaunch::Diff(diff_request(paths)),
        completion.clone(),
        None,
    );
    assert_eq!(completion.load(Ordering::Acquire), 0);
    app.poll(&ctx);
    assert_eq!(completion.load(Ordering::Acquire), 0);
    assert!(!app.close_prompt);
}

#[test]
fn git_merge_close_prompt_keeps_editing_or_returns_failure_without_creating_a_normal_session() {
    let fixture = GitFixture::new("close-prompt");
    fixture.write("base.txt", "base\n");
    fixture.write("local.txt", "current\n");
    fixture.write("remote.txt", "incoming\n");
    fixture.write("result.txt", "partial\n");
    let ctx = egui::Context::default();
    let completion = Arc::new(AtomicU8::new(1));
    let mut app = GitApp::start(
        &ctx,
        GitLaunch::Merge(merge_request(&fixture)),
        completion.clone(),
        None,
    );

    assert!(matches!(&app.view, GitView::Merge(_)));
    app.close_prompt = true;
    click(&mut app, &ctx, "Keep editing");
    assert!(!app.close_prompt);
    assert!(!app.closing);
    assert_eq!(completion.load(Ordering::Acquire), 1);

    app.close_prompt = true;
    click(&mut app, &ctx, "Cancel merge");
    assert!(!app.close_prompt);
    assert!(app.closing);
    assert_eq!(completion.load(Ordering::Acquire), 1);
}
