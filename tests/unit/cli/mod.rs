use super::*;

fn arguments(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn request(command: Command) -> LaunchRequest {
    match command {
        Command::Open(Some(request)) => request,
        other => panic!("expected an open request, received {other:?}"),
    }
}

#[test]
fn opens_an_empty_workspace_without_arguments() {
    assert_eq!(parse(Vec::new()), Ok(Command::Open(None)));
}

#[test]
fn accepts_two_paths_in_the_supplied_left_to_right_order() {
    let launch = request(parse(arguments(&["left space/file.txt", "右/比較.txt"])).unwrap());

    assert_eq!(
        launch.paths,
        [
            PathBuf::from("left space/file.txt"),
            PathBuf::from("右/比較.txt")
        ]
    );
    assert_eq!(launch.mode, None);
}

#[test]
fn preserves_windows_drive_and_unc_path_spellings() {
    let launch = request(
        parse(arguments(&[
            r"C:\work\left file.txt",
            r"\\server\share\right file.txt",
        ]))
        .unwrap(),
    );

    assert_eq!(
        launch.paths,
        [
            PathBuf::from(r"C:\work\left file.txt"),
            PathBuf::from(r"\\server\share\right file.txt"),
        ]
    );
}

#[test]
fn accepts_explicit_mode_and_compatibility_wait_flag_in_any_order() {
    let file = request(parse(arguments(&["--wait", "--diff", "left", "right"])).unwrap());
    assert_eq!(file.mode, Some(ComparisonMode::File));

    let folder = request(parse(arguments(&["left", "--folder", "right", "--wait"])).unwrap());
    assert_eq!(folder.mode, Some(ComparisonMode::Folder));
}

#[test]
fn treats_dash_leading_paths_after_double_dash_as_paths() {
    let launch = request(parse(arguments(&["--folder", "--", "-left", "--right"])).unwrap());

    assert_eq!(
        launch.paths,
        [PathBuf::from("-left"), PathBuf::from("--right")]
    );
    assert_eq!(launch.mode, Some(ComparisonMode::Folder));

    let launch = request(parse(arguments(&["--", "--merge", "right"])).unwrap());
    assert_eq!(
        launch.paths,
        [PathBuf::from("--merge"), PathBuf::from("right")]
    );
}

#[test]
fn recognizes_standalone_help_and_version() {
    assert_eq!(parse(arguments(&["--help"])), Ok(Command::Help));
    assert_eq!(parse(arguments(&["-h"])), Ok(Command::Help));
    assert_eq!(parse(arguments(&["--version"])), Ok(Command::Version));
    assert_eq!(parse(arguments(&["-V"])), Ok(Command::Version));
    assert!(HELP.contains("read-only"));
    assert!(HELP.contains("blocking diff tool"));
    assert!(HELP.contains("--merge"));
}

#[test]
fn parses_a_three_way_merge_without_creating_a_window_request() {
    let request = match parse(arguments(&[
        "--merge",
        "base",
        "ours",
        "theirs",
        "--output",
        "merged",
        "--overwrite",
    ]))
    .unwrap()
    {
        Command::Merge(request) => request,
        other => panic!("expected a merge request, received {other:?}"),
    };
    assert_eq!(request.base, PathBuf::from("base"));
    assert_eq!(request.ours, PathBuf::from("ours"));
    assert_eq!(request.theirs, PathBuf::from("theirs"));
    assert_eq!(request.output, PathBuf::from("merged"));
    assert!(request.overwrite);
}

#[test]
fn merge_requires_three_inputs_and_an_output() {
    for args in [
        arguments(&["--merge", "base", "ours", "--output", "out"]),
        arguments(&["--merge", "base", "ours", "theirs"]),
        arguments(&["--merge", "base", "ours", "theirs", "--output"]),
        arguments(&[
            "--merge", "--diff", "base", "ours", "theirs", "--output", "out",
        ]),
    ] {
        assert!(parse(args).is_err());
    }
}

#[test]
fn rejects_invalid_switches_and_path_counts() {
    for args in [
        arguments(&["--unknown", "left", "right"]),
        arguments(&["--diff", "--diff", "left", "right"]),
        arguments(&["--diff", "--folder", "left", "right"]),
        arguments(&["--help", "left", "right"]),
        arguments(&["--version", "left", "right"]),
        arguments(&["left"]),
        arguments(&["left", "right", "extra"]),
        arguments(&["", "right"]),
    ] {
        assert!(parse(args).is_err());
    }
}

#[cfg(unix)]
#[test]
fn preserves_non_utf8_unix_paths() {
    use std::os::unix::ffi::OsStringExt;

    let left = OsString::from_vec(b"left-\xFF".to_vec());
    let right = OsString::from_vec(b"right-\xFE".to_vec());
    let launch = request(parse(vec![left.clone(), right.clone()]).unwrap());

    assert_eq!(launch.paths, [PathBuf::from(left), PathBuf::from(right)]);
}
