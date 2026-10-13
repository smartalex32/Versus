//! Command-line parsing for launching Versus from Git and IDE diff tools.
//!
//! The parser deliberately performs no filesystem access. Callers can classify
//! the supplied paths asynchronously after the native window has started.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use crate::selection::ComparisonMode;

/// Command-line help for Git, IDE, and terminal launches.
pub(crate) const HELP: &str = "\
Usage: versus [--diff | --folder] [--wait] <left-path> <right-path>\n\
       versus --merge <base-path> <ours-path> <theirs-path> --output <path> [--overwrite]\n\
       versus\n\
       versus --help\n\
       versus --version\n\
\n\
With no arguments, open the empty workspace. With two paths, infer file or folder\n\
mode; --diff requires files and --folder requires folders. Use -- before paths\n\
that begin with a dash. Relative paths resolve from the current directory.\n\
\n\
Open a read-only file or folder comparison. Versus stays open until its window is\n\
closed, so Git and IDEs can use it directly as a blocking diff tool. --wait is\n\
accepted for compatibility and has no additional effect.\n\
\n\
--merge performs a conventional three-way text merge without opening the desktop\n\
window. It writes conflict markers and exits with status 1 when manual resolution\n\
is required. The output path must not already exist unless --overwrite is given.\n";

/// The two paths provided by an external diff-tool caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LaunchRequest {
    pub paths: [PathBuf; 2],
    pub mode: Option<ComparisonMode>,
}

/// Inputs for a conventional, non-interactive three-way merge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MergeRequest {
    pub base: PathBuf,
    pub ours: PathBuf,
    pub theirs: PathBuf,
    pub output: PathBuf,
    pub overwrite: bool,
}

/// A command requested through Versus's command-line interface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Command {
    /// Open the native comparison workspace, optionally with two sources.
    Open(Option<LaunchRequest>),
    /// Run a traditional three-way merge and exit without starting the GUI.
    Merge(MergeRequest),
    /// Print [`HELP`].
    Help,
    /// Print the application version.
    Version,
}

/// Parse command-line arguments after the program name.
///
/// Paths stay as [`OsString`] values until they become [`PathBuf`]s, preserving
/// platform-native paths including non-UTF-8 Unix names.
pub(crate) fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let args: Vec<OsString> = args.into_iter().collect();

    if args.is_empty() {
        return Ok(Command::Open(None));
    }

    if args
        .iter()
        .take_while(|arg| *arg != OsStr::new("--"))
        .any(|arg| arg == OsStr::new("--merge"))
    {
        return parse_merge(args);
    }

    if args.len() == 1 {
        if args[0] == OsStr::new("--help") || args[0] == OsStr::new("-h") {
            return Ok(Command::Help);
        }
        if args[0] == OsStr::new("--version") || args[0] == OsStr::new("-V") {
            return Ok(Command::Version);
        }
    }

    let mut mode = None;
    let mut paths = Vec::with_capacity(2);
    let mut parse_flags = true;

    for arg in args {
        if parse_flags && arg == OsStr::new("--") {
            parse_flags = false;
            continue;
        }

        if parse_flags && (arg == OsStr::new("--help") || arg == OsStr::new("-h")) {
            return Err("--help must be used by itself".to_owned());
        }
        if parse_flags && (arg == OsStr::new("--version") || arg == OsStr::new("-V")) {
            return Err("--version must be used by itself".to_owned());
        }

        if parse_flags && arg == OsStr::new("--wait") {
            continue;
        }

        if parse_flags && (arg == OsStr::new("--diff") || arg == OsStr::new("--folder")) {
            let requested_mode = if arg == OsStr::new("--diff") {
                ComparisonMode::File
            } else {
                ComparisonMode::Folder
            };
            if mode.replace(requested_mode).is_some() {
                return Err("choose only one comparison mode: --diff or --folder".to_owned());
            }
            continue;
        }

        if parse_flags && arg.as_os_str().to_string_lossy().starts_with('-') {
            return Err(format!(
                "unknown option: {} (use -- before a path beginning with -)",
                arg.to_string_lossy()
            ));
        }

        if arg.is_empty() {
            return Err("paths must not be empty".to_owned());
        }
        paths.push(PathBuf::from(arg));
    }

    let paths: [PathBuf; 2] = paths.try_into().map_err(|paths: Vec<PathBuf>| {
        format!("expected exactly two paths, received {}", paths.len())
    })?;

    Ok(Command::Open(Some(LaunchRequest { paths, mode })))
}

fn parse_merge(args: Vec<OsString>) -> Result<Command, String> {
    let mut inputs = Vec::with_capacity(3);
    let mut output = None;
    let mut merge = false;
    let mut overwrite = false;
    let mut parse_flags = true;
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        if parse_flags && arg == OsStr::new("--") {
            parse_flags = false;
            index += 1;
            continue;
        }
        if parse_flags && arg == OsStr::new("--merge") {
            if merge {
                return Err("--merge may be specified only once".to_owned());
            }
            merge = true;
            index += 1;
            continue;
        }
        if parse_flags && arg == OsStr::new("--overwrite") {
            overwrite = true;
            index += 1;
            continue;
        }
        if parse_flags && arg == OsStr::new("--output") {
            let value = args
                .get(index + 1)
                .ok_or_else(|| "--output requires a path".to_owned())?;
            if value.is_empty() {
                return Err("output path must not be empty".to_owned());
            }
            if output.replace(PathBuf::from(value)).is_some() {
                return Err("--output may be specified only once".to_owned());
            }
            index += 2;
            continue;
        }
        if parse_flags && arg.as_os_str().to_string_lossy().starts_with('-') {
            return Err(format!(
                "unknown option for --merge: {} (use -- before a path beginning with -)",
                arg.to_string_lossy()
            ));
        }
        if arg.is_empty() {
            return Err("merge paths must not be empty".to_owned());
        }
        inputs.push(PathBuf::from(arg));
        index += 1;
    }

    if !merge {
        return Err("--merge is required for a three-way merge".to_owned());
    }
    let [base, ours, theirs]: [PathBuf; 3] =
        inputs.try_into().map_err(|inputs: Vec<PathBuf>| {
            format!(
                "--merge expects exactly three input paths, received {}",
                inputs.len()
            )
        })?;
    let output = output.ok_or_else(|| "--merge requires --output <path>".to_owned())?;
    Ok(Command::Merge(MergeRequest {
        base,
        ours,
        theirs,
        output,
        overwrite,
    }))
}

#[cfg(test)]
mod tests {
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
}
