use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_versus"))
        .args(args)
        .output()
        .expect("Versus CLI should launch")
}

#[test]
fn help_and_version_exit_successfully_without_opening_a_window() {
    let help = run(&["--help"]);
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("Usage: versus"));
    assert!(help.contains("--diff"));
    assert!(help.contains("--folder"));
    assert!(help.contains("read-only"));

    let version = run(&["--version"]);
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap().trim(),
        concat!("Versus ", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn malformed_arguments_exit_with_usage_error_without_opening_a_window() {
    for args in [
        vec!["--diff", "only-one-file"],
        vec!["left", "right", "extra"],
        vec!["--unknown"],
        vec!["--diff", "--folder", "left", "right"],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(2), "args: {args:?}");
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("Versus:"));
        assert!(error.contains("Usage: versus"));
        assert!(output.stdout.is_empty());
    }
}

fn sandbox(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "versus-cli-{name}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn three_way_merge_combines_disjoint_changes_and_writes_no_markers() {
    let root = sandbox("disjoint");
    let base = root.join("base.txt");
    let ours = root.join("ours.txt");
    let theirs = root.join("theirs.txt");
    let output = root.join("output.txt");
    fs::write(&base, "one\ntwo\nthree\n").unwrap();
    fs::write(&ours, "one\nOURS\nthree\n").unwrap();
    fs::write(&theirs, "one\ntwo\nTHEIRS\n").unwrap();

    let result = run(&[
        "--merge",
        base.to_str().unwrap(),
        ours.to_str().unwrap(),
        theirs.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ]);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(fs::read_to_string(&output).unwrap(), "one\nOURS\nTHEIRS\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn three_way_merge_keeps_identical_edits_and_reports_conflicts() {
    let root = sandbox("conflicts");
    let base = root.join("base.txt");
    let ours = root.join("ours.txt");
    let theirs = root.join("theirs.txt");
    let output = root.join("output.txt");
    fs::write(&base, "same\nchange\n").unwrap();
    fs::write(&ours, "same\nOURS\n").unwrap();
    fs::write(&theirs, "same\nTHEIRS\n").unwrap();

    let result = run(&[
        "--merge",
        base.to_str().unwrap(),
        ours.to_str().unwrap(),
        theirs.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ]);
    assert_eq!(result.status.code(), Some(1));
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("conflicts")
    );
    assert_eq!(
        fs::read_to_string(&output).unwrap(),
        "same\n<<<<<<< OURS\nOURS\n=======\nTHEIRS\n>>>>>>> THEIRS\n"
    );

    let identical = root.join("identical.txt");
    fs::write(&ours, "same\nCHANGED\n").unwrap();
    fs::write(&theirs, "same\nCHANGED\n").unwrap();
    let result = run(&[
        "--merge",
        base.to_str().unwrap(),
        ours.to_str().unwrap(),
        theirs.to_str().unwrap(),
        "--output",
        identical.to_str().unwrap(),
    ]);
    assert!(result.status.success());
    assert_eq!(fs::read_to_string(identical).unwrap(), "same\nCHANGED\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn three_way_merge_refuses_to_replace_output_without_overwrite() {
    let root = sandbox("overwrite");
    let base = root.join("base.txt");
    let ours = root.join("ours.txt");
    let theirs = root.join("theirs.txt");
    let output = root.join("output.txt");
    fs::write(&base, "a\n").unwrap();
    fs::write(&ours, "b\n").unwrap();
    fs::write(&theirs, "b\n").unwrap();
    fs::write(&output, "keep\n").unwrap();

    let args = [
        "--merge",
        base.to_str().unwrap(),
        ours.to_str().unwrap(),
        theirs.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ];
    let refused = run(&args);
    assert_eq!(refused.status.code(), Some(2));
    assert_eq!(fs::read_to_string(&output).unwrap(), "keep\n");

    let mut overwrite_args = args.to_vec();
    overwrite_args.push("--overwrite");
    let saved = run(&overwrite_args);
    assert!(saved.status.success());
    assert_eq!(fs::read_to_string(&output).unwrap(), "b\n");
    fs::remove_dir_all(root).unwrap();
}
