//! Command-line parsing for launching Versus from Git and IDE diff tools.
//!
//! The parser deliberately performs no filesystem access. Callers can classify
//! the supplied paths asynchronously after the native window has started.

mod git;

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use crate::selection::ComparisonMode;

/// Command-line help for Git, IDE, and terminal launches.
pub(crate) const HELP: &str = "\
Usage: versus [--diff | --folder] [--wait] <left-path> <right-path>\n\
       versus --merge <base-path> <ours-path> <theirs-path> --output <path> [--overwrite]\n\
       versus --git-diff [--diff | --folder] [--label <text>] [--wait] -- <left-path> <right-path>\n\
       versus --git-merge (--base <path> | --no-base) --local <path> --remote <path> --output <path> [--label <text>] [--wait]\n\
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
is required. The output path must not already exist unless --overwrite is given.\n\
\n\
--git-diff opens a focused, read-only Git comparison with fixed sources.\n\
--git-merge opens a focused, blocking Git merge window. Save and Resolve returns\n\
success only after the Result is written; cancelling returns failure to Git.\n";

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

/// Inputs supplied by Git when it launches a focused diff view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GitDiffRequest {
    pub paths: [PathBuf; 2],
    pub mode: Option<ComparisonMode>,
    pub label: Option<String>,
}

/// Inputs supplied by Git when it launches a focused merge view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GitMergeRequest {
    pub base: Option<PathBuf>,
    pub local: PathBuf,
    pub remote: PathBuf,
    pub output: PathBuf,
    pub label: Option<String>,
}

/// A command requested through Versus's command-line interface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Command {
    /// Open the native comparison workspace, optionally with two sources.
    Open(Option<LaunchRequest>),
    /// Run a traditional three-way merge and exit without starting the GUI.
    Merge(MergeRequest),
    /// Open a focused, read-only comparison supplied by Git.
    GitDiff(GitDiffRequest),
    /// Open a focused, interactive three-way merge supplied by Git.
    GitMerge(GitMergeRequest),
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

    if let Some(command) = git::parse(&args)? {
        return Ok(command);
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
#[path = "../tests/unit/cli/mod.rs"]
mod tests;
