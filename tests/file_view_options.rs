use std::{
    fs,
    ops::Range,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_SANDBOX: AtomicU64 = AtomicU64::new(0);
use versus::{
    ComparisonProgress, DirectoryEntryState, DisplayLineEnding, FileViewOptions, ProgressStage,
    load_file_comparison_with_options,
};

fn sandbox(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "versus-file-view-options-{name}-{}-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_SANDBOX.fetch_add(1, Ordering::Relaxed),
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn paths(left: PathBuf, right: PathBuf) -> [Option<PathBuf>; 2] {
    [Some(left), Some(right)]
}

fn compare(left: &str, right: &str, options: FileViewOptions) -> Vec<versus::FileComparisonRow> {
    let root = sandbox("compare");
    let left_path = root.join("left.txt");
    let right_path = root.join("right.txt");
    fs::write(&left_path, left).unwrap();
    fs::write(&right_path, right).unwrap();
    let comparison = load_file_comparison_with_options(
        &paths(left_path, right_path),
        &AtomicBool::new(false),
        &options,
    )
    .unwrap()
    .unwrap();
    let _ = fs::remove_dir_all(root);
    comparison.rows
}

#[test]
fn whitespace_and_line_ending_options_are_independent() {
    let left = "alpha beta\r\nsame\n";
    let right = "alpha\tbeta\nsame\r\n";
    let strict = compare(
        left,
        right,
        FileViewOptions {
            ignore_line_endings: false,
            ..Default::default()
        },
    );
    assert!(
        strict
            .iter()
            .all(|row| row.state == DirectoryEntryState::Different)
    );

    let whitespace = compare(
        left,
        right,
        FileViewOptions {
            ignore_whitespace: true,
            ignore_line_endings: false,
            progress: None,
        },
    );
    assert!(
        whitespace
            .iter()
            .all(|row| row.state == DirectoryEntryState::Different)
    );

    let endings = compare(
        left,
        right,
        FileViewOptions {
            ignore_line_endings: true,
            ..Default::default()
        },
    );
    assert_eq!(
        endings.iter().map(|row| &row.state).collect::<Vec<_>>(),
        vec![&DirectoryEntryState::Different, &DirectoryEntryState::Same]
    );

    let both = compare(
        left,
        right,
        FileViewOptions {
            ignore_whitespace: true,
            ignore_line_endings: true,
            progress: None,
        },
    );
    assert!(
        both.iter()
            .all(|row| row.state == DirectoryEntryState::Same)
    );
}

#[test]
fn final_line_ending_is_a_strict_difference_and_is_retained() {
    let rows = compare(
        "same\n",
        "same",
        FileViewOptions {
            ignore_line_endings: false,
            ..Default::default()
        },
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, DirectoryEntryState::Different);
    assert_eq!(rows[0].left_ending, Some(DisplayLineEnding::Lf));
    assert_eq!(rows[0].right_ending, Some(DisplayLineEnding::None));
}

#[test]
fn highlights_unicode_boundaries_and_one_sided_lines() {
    let changed = compare("café\n", "caff\n", FileViewOptions::default());
    assert_eq!(changed[0].left_changed, vec![Range { start: 3, end: 5 }]);
    assert_eq!(changed[0].right_changed, vec![Range { start: 3, end: 4 }]);

    let inserted = compare("base\n", "base\nadded\n", FileViewOptions::default());
    assert_eq!(inserted[1].state, DirectoryEntryState::RightOnly);
    assert_eq!(inserted[1].right_changed, vec![0..5]);
    assert!(inserted[1].left_changed.is_empty());

    let deleted = compare("base\nremoved\n", "base\n", FileViewOptions::default());
    assert_eq!(deleted[1].state, DirectoryEntryState::LeftOnly);
    assert_eq!(deleted[1].left_changed, vec![0..7]);
    assert!(deleted[1].right_changed.is_empty());
}

#[test]
fn highlights_each_disjoint_edit_without_covering_stable_text() {
    let rows = compare(
        "old stable old\n",
        "new stable new\n",
        FileViewOptions::default(),
    );
    assert_eq!(rows[0].left_changed, vec![0..3, 11..14]);
    assert_eq!(rows[0].right_changed, vec![0..3, 11..14]);
}

#[test]
fn ignored_whitespace_is_not_highlighted_around_real_edits() {
    let rows = compare(
        "old  stable\told\n",
        "new stable new\n",
        FileViewOptions {
            ignore_whitespace: true,
            ..Default::default()
        },
    );
    assert_eq!(rows[0].left_changed, vec![0..3, 12..15]);
    assert_eq!(rows[0].right_changed, vec![0..3, 11..14]);
}

#[test]
fn uses_full_line_highlights_for_large_changed_lines_and_finishes_progress() {
    let large_left = format!("{}a\n", "x".repeat(16 * 1024));
    let large_right = format!("{}b\n", "x".repeat(16 * 1024));
    let progress = Arc::new(ComparisonProgress::default());
    let rows = compare(
        &large_left,
        &large_right,
        FileViewOptions {
            progress: Some(progress.clone()),
            ..Default::default()
        },
    );
    assert_eq!(rows[0].left_changed, vec![0..large_left.len() - 1]);
    assert_eq!(rows[0].right_changed, vec![0..large_right.len() - 1]);
    let snapshot = progress.snapshot();
    assert_eq!(snapshot.stage, ProgressStage::Finished);
    assert_eq!(snapshot.total, Some(0));
    let ending_only = compare(
        &format!("{}\r\n", "x ".repeat(16 * 1024)),
        &format!("{}\n", "x".repeat(16 * 1024)),
        FileViewOptions {
            ignore_whitespace: true,
            ignore_line_endings: false,
            progress: None,
        },
    );
    assert_eq!(ending_only[0].state, DirectoryEntryState::Different);
    assert!(ending_only[0].left_changed.is_empty());
    assert!(ending_only[0].right_changed.is_empty());
}

#[test]
fn cancellation_returns_no_comparison_before_reading() {
    let root = sandbox("cancelled");
    let left = root.join("left.txt");
    let right = root.join("right.txt");
    fs::write(&left, "left\n").unwrap();
    fs::write(&right, "right\n").unwrap();
    let cancelled = AtomicBool::new(true);
    assert_eq!(
        load_file_comparison_with_options(
            &paths(left, right),
            &cancelled,
            &FileViewOptions::default()
        )
        .unwrap(),
        None
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn aligns_insertions_before_related_changed_lines_without_losing_line_numbers() {
    let prefix = (1..=49)
        .map(|line| format!("shared {line}\n"))
        .collect::<String>();
    let left = format!("{prefix}inserted on the left\nlet alpha = 1;\nlet beta = 2;\ntail\n");
    let right = format!("{prefix}// let alpha = 1;\n// let beta = 2;\ntail\n");
    let rows = compare(&left, &right, FileViewOptions::default());

    assert_eq!(rows.len(), 53);
    assert_eq!(rows[49].state, DirectoryEntryState::LeftOnly);
    assert_eq!(rows[49].left.as_ref().map(|line| line.0), Some(50));
    assert_eq!(rows[49].right, None);
    assert_eq!(rows[50].state, DirectoryEntryState::Different);
    assert_eq!(rows[50].left.as_ref().map(|line| line.0), Some(51));
    assert_eq!(rows[50].right.as_ref().map(|line| line.0), Some(50));
    assert_eq!(rows[51].state, DirectoryEntryState::Different);
    assert_eq!(rows[51].left.as_ref().map(|line| line.0), Some(52));
    assert_eq!(rows[51].right.as_ref().map(|line| line.0), Some(51));
    assert_eq!(rows[52].state, DirectoryEntryState::Same);
    assert_eq!(rows[52].left.as_ref().map(|line| line.0), Some(53));
    assert_eq!(rows[52].right.as_ref().map(|line| line.0), Some(52));

    // The inserted line is one-sided, and the matched replacements retain
    // prefix-only highlights instead of treating the whole lines as unrelated.
    assert_eq!(rows[49].left_changed, vec![0..20]);
    assert_eq!(rows[50].left_changed, Vec::<Range<usize>>::new());
    assert_eq!(rows[50].right_changed, vec![0..3]);
}

#[test]
fn aligns_shifted_changes_in_both_directions_and_at_end_of_file() {
    let left = "anchor\n// let café = true;\n// repeat\n";
    let right = "anchor\ninserted\nlet café = true;\nrepeat\n";
    let rows = compare(left, right, FileViewOptions::default());
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[1].state, DirectoryEntryState::RightOnly);
    assert_eq!(rows[1].right.as_ref().map(|line| line.0), Some(2));
    assert_eq!(rows[2].state, DirectoryEntryState::Different);
    assert_eq!(rows[2].left.as_ref().map(|line| line.0), Some(2));
    assert_eq!(rows[2].right.as_ref().map(|line| line.0), Some(3));
    assert_eq!(rows[3].state, DirectoryEntryState::Different);
    assert_eq!(rows[3].left.as_ref().map(|line| line.0), Some(3));
    assert_eq!(rows[3].right.as_ref().map(|line| line.0), Some(4));

    let whitespace = compare(
        "anchor\nadded\nvalue    with spaces\n",
        "anchor\n// value with spaces\n",
        FileViewOptions {
            ignore_whitespace: true,
            ..Default::default()
        },
    );
    assert_eq!(whitespace.len(), 3);
    assert_eq!(whitespace[1].state, DirectoryEntryState::LeftOnly);
    assert_eq!(whitespace[2].state, DirectoryEntryState::Different);
    assert_eq!(whitespace[2].left.as_ref().map(|line| line.0), Some(3));
    assert_eq!(whitespace[2].right.as_ref().map(|line| line.0), Some(2));
}

#[test]
fn aligns_wrapped_comments_using_shared_interior_text() {
    let left = "anchor\ninserted\nlet alpha = 1;\nlet beta = 2;\ntail\n";
    let right = "anchor\n/* let alpha = 1; */\n/* let beta = 2; */\ntail\n";
    let rows = compare(left, right, FileViewOptions::default());
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[1].state, DirectoryEntryState::LeftOnly);
    assert_eq!(rows[1].left.as_ref().map(|line| line.0), Some(2));
    assert_eq!(rows[2].state, DirectoryEntryState::Different);
    assert_eq!(rows[2].left.as_ref().map(|line| line.0), Some(3));
    assert_eq!(rows[2].right.as_ref().map(|line| line.0), Some(2));
    assert_eq!(rows[3].state, DirectoryEntryState::Different);
    assert_eq!(rows[3].left.as_ref().map(|line| line.0), Some(4));
    assert_eq!(rows[3].right.as_ref().map(|line| line.0), Some(3));
    assert_eq!(rows[2].left_changed, Vec::<Range<usize>>::new());
    assert_eq!(rows[2].right_changed, vec![0..3, 17..20]);

    let inverse = compare(right, left, FileViewOptions::default());
    assert_eq!(inverse.len(), 5);
    assert_eq!(inverse[1].state, DirectoryEntryState::RightOnly);
    assert_eq!(inverse[1].right.as_ref().map(|line| line.0), Some(2));
    assert_eq!(inverse[2].state, DirectoryEntryState::Different);
    assert_eq!(inverse[2].left.as_ref().map(|line| line.0), Some(2));
    assert_eq!(inverse[2].right.as_ref().map(|line| line.0), Some(3));
    assert_eq!(inverse[3].state, DirectoryEntryState::Different);
    assert_eq!(inverse[3].left.as_ref().map(|line| line.0), Some(3));
    assert_eq!(inverse[3].right.as_ref().map(|line| line.0), Some(4));
    assert_eq!(inverse[2].left_changed, vec![0..3, 17..20]);
    assert_eq!(inverse[2].right_changed, Vec::<Range<usize>>::new());
}

#[test]
fn aligns_the_exact_original_before_a_near_duplicate_insertion() {
    let left = "anchor\nαction(2);\nαction(1);\ntail\n";
    for right_line in ["// αction(1);", "/* αction(1); */"] {
        let right = format!("anchor\n{right_line}\ntail\n");
        let rows = compare(left, &right, FileViewOptions::default());
        assert_eq!(rows.len(), 4, "{right_line}");
        assert_eq!(rows[1].state, DirectoryEntryState::LeftOnly, "{right_line}");
        assert_eq!(rows[1].left.as_ref().map(|line| line.0), Some(2));
        assert_eq!(
            rows[2].state,
            DirectoryEntryState::Different,
            "{right_line}"
        );
        assert_eq!(rows[2].left.as_ref().map(|line| line.0), Some(3));
        assert_eq!(rows[2].right.as_ref().map(|line| line.0), Some(2));
        assert_eq!(rows[3].state, DirectoryEntryState::Same, "{right_line}");
    }
}
