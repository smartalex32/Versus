use std::process::Command;

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
