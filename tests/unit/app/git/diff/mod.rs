use crate::app::git::diff::Options;
use crate::app::git::diff::file::{navigation_offset, next_difference, visible_rows};
use crate::app::git::diff::loading::source_presence;
use std::path::PathBuf;
use versus::{DirectoryEntryState, FileComparison};

fn row(state: DirectoryEntryState, left: &str, right: &str) -> versus::FileComparisonRow {
    versus::FileComparisonRow {
        left: Some((1, left.into())),
        right: Some((1, right.into())),
        state,
        left_changed: vec![],
        right_changed: vec![],
        left_ending: None,
        right_ending: None,
    }
}

#[test]
fn visible_rows_respect_difference_filter_and_search() {
    let comparison = FileComparison {
        rows: vec![
            row(DirectoryEntryState::Same, "same", "same"),
            row(DirectoryEntryState::Different, "alpha", "beta"),
            row(DirectoryEntryState::LeftOnly, "needle", ""),
        ],
        message: None,
    };
    let options = Options::default();
    assert_eq!(visible_rows(&comparison, options, ""), vec![1, 2]);
    assert_eq!(visible_rows(&comparison, options, "needle"), vec![2]);
    assert!(visible_rows(&comparison, options, "same").is_empty());
}

#[test]
fn difference_navigation_wraps_in_each_direction() {
    assert_eq!(next_difference(Some(4), &[2, 6], true), Some(6));
    assert_eq!(next_difference(Some(6), &[2, 6], true), Some(2));
    assert_eq!(next_difference(Some(2), &[2, 6], false), Some(6));
    assert_eq!(next_difference(None, &[2, 6], true), Some(2));
}

#[test]
fn null_input_becomes_an_empty_file_side() {
    assert_eq!(
        source_presence(&[PathBuf::from("/dev/null"), PathBuf::from("right.txt")]),
        [None, Some(())]
    );
}

#[test]
fn navigation_offset_targets_an_offscreen_visible_row() {
    let visible: Vec<_> = (0..200).collect();
    assert_eq!(navigation_offset(Some(150), &visible, 22.0), Some(3300.0));
}

#[test]
fn directory_search_finds_files_inside_collapsed_directories() {
    use crate::app::git::{diff::folder::matching_rows, tests::GitFixture};
    let fixture = GitFixture::new("nested-search");
    let left = fixture.path("left");
    let right = fixture.path("right");
    for path in [&left, &right] {
        std::fs::create_dir_all(path.join("src")).unwrap();
    }
    std::fs::write(left.join("src/nested.rs"), "before\n").unwrap();
    std::fs::write(right.join("src/nested.rs"), "after\n").unwrap();
    let diff =
        versus::compare_directories(&left, &right, &versus::DirectoryCompareOptions::default())
            .unwrap();
    let tree = versus::FolderTree::from_diff(&diff);
    assert!(
        tree.visible_rows()
            .iter()
            .all(|row| row.node.name != "nested.rs")
    );
    let matches = matching_rows(&tree, Options::default(), "src/nested.rs");
    assert_eq!(matches.len(), 1);
    assert_eq!(
        matches[0].node.relative_path,
        PathBuf::from("src/nested.rs")
    );
}

#[test]
fn added_file_null_side_loads_and_renders_read_only_content() {
    use super::*;
    use crate::app::git::tests::GitFixture;
    let fixture = GitFixture::new("null-render");
    let right = fixture.write("added.txt", "new content\n");
    let ctx = egui::Context::default();
    let mut view = GitDiffView::new(
        crate::cli::GitDiffRequest {
            paths: [PathBuf::from("/dev/null"), right],
            mode: None,
            label: Some("added.txt".into()),
        },
        &ctx,
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while matches!(view.screen, Screen::Loading) {
        view.poll(&ctx);
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    let Screen::File(file) = &view.screen else {
        panic!("null input failed to load");
    };
    assert!(
        file.comparison
            .rows
            .iter()
            .any(|row| row.state == DirectoryEntryState::RightOnly)
    );
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| view.render(ui));
    assert!(!output.shapes.is_empty());
    output.textures_delta.clear();
    assert_eq!(
        std::fs::read_to_string(fixture.path("added.txt")).unwrap(),
        "new content\n"
    );
}

#[test]
fn long_file_lines_scroll_horizontally_without_moving_the_other_pane() {
    use super::*;
    let ctx = egui::Context::default();
    crate::app::visuals::theme::apply_theme(&ctx);
    let long = format!("long-left-{}", "x".repeat(300));
    let paths = [PathBuf::from("left.txt"), PathBuf::from("right.txt")];
    let mut view = GitDiffView {
        request: crate::cli::GitDiffRequest {
            paths: paths.clone(),
            mode: None,
            label: None,
        },
        options: Options::default(),
        job: None,
        search: String::new(),
        screen: Screen::File(FileScreen {
            paths,
            comparison: FileComparison {
                rows: vec![row(DirectoryEntryState::Different, &long, "short-right")],
                message: None,
            },
            navigation: None,
            back: None,
            sources: [true; 2],
            scroll_y: 0.0,
            pending_navigation: false,
            content_columns: None,
        }),
    };
    let raw = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1200.0, 800.0),
        )),
        ..Default::default()
    };
    let locate = |output: &egui::FullOutput, text: &str| {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(item) if item.galley.text() == text => {
                    Some((item.pos, shape.clip_rect))
                }
                _ => None,
            })
            .unwrap()
    };
    let mut before = ctx.run_ui(raw(), |ui| view.render(ui));
    let (left_before, clip) = locate(&before, &long);
    let (right_before, _) = locate(&before, "short-right");
    before.textures_delta.clear();
    let mut input = raw();
    input.events = vec![
        egui::Event::PointerMoved(clip.center()),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(-150.0, 0.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        },
    ];
    let mut after = ctx.run_ui(input, |ui| view.render(ui));
    let (left_after, _) = locate(&after, &long);
    let (right_after, _) = locate(&after, "short-right");
    after.textures_delta.clear();
    assert!(left_after.x < left_before.x);
    assert_eq!(right_after, right_before);
}
