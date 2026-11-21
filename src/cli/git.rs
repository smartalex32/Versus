//! Explicit Git invocation parsing, separate from normal desktop launches.

use super::{Command, GitDiffRequest, GitMergeRequest};
use crate::selection::ComparisonMode;
use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

#[derive(Clone, Copy, Eq, PartialEq)]
enum GitCommand {
    Diff,
    Merge,
}

pub(super) fn parse(args: &[OsString]) -> Result<Option<Command>, String> {
    match command(args)? {
        Some(GitCommand::Diff) => parse_diff(args).map(Some),
        Some(GitCommand::Merge) => parse_merge(args).map(Some),
        None => Ok(None),
    }
}

fn command(args: &[OsString]) -> Result<Option<GitCommand>, String> {
    let mut command = None;
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == OsStr::new("--") {
            break;
        }
        if arg == OsStr::new("--git-diff") || arg == OsStr::new("--git-merge") {
            if command.replace(arg).is_some() {
                return Err("choose only one Git command: --git-diff or --git-merge".to_owned());
            }
            index += 1;
            continue;
        }
        // A named option's value may be a native path that begins with a dash.
        if ["--label", "--base", "--local", "--remote", "--output"]
            .iter()
            .any(|option| arg == OsStr::new(option))
        {
            index += 2;
            continue;
        }
        index += 1;
    }
    Ok(command.map(|command| {
        if command == OsStr::new("--git-diff") {
            GitCommand::Diff
        } else {
            GitCommand::Merge
        }
    }))
}

fn parse_diff(args: &[OsString]) -> Result<Command, String> {
    let mut mode = None;
    let mut label = None;
    let mut command = false;
    let mut wait = false;
    let mut separator = false;
    let mut paths = Vec::with_capacity(2);
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if !separator && arg == OsStr::new("--") {
            separator = true;
            index += 1;
        } else if !separator && arg == OsStr::new("--git-diff") {
            if command {
                return Err("--git-diff may be specified only once".to_owned());
            }
            command = true;
            index += 1;
        } else if !separator && arg == OsStr::new("--wait") {
            if wait {
                return Err("--wait may be specified only once".to_owned());
            }
            wait = true;
            index += 1;
        } else if !separator && (arg == OsStr::new("--diff") || arg == OsStr::new("--folder")) {
            let requested = if arg == OsStr::new("--diff") {
                ComparisonMode::File
            } else {
                ComparisonMode::Folder
            };
            if mode.replace(requested).is_some() {
                return Err("choose only one comparison mode: --diff or --folder".to_owned());
            }
            index += 1;
        } else if !separator && arg == OsStr::new("--label") {
            let value = required_value(args, index, "--label", "label")?;
            if label
                .replace(value.to_string_lossy().into_owned())
                .is_some()
            {
                return Err("--label may be specified only once".to_owned());
            }
            index += 2;
        } else if !separator && arg.as_os_str().to_string_lossy().starts_with('-') {
            return Err(format!(
                "unknown option for --git-diff: {} (use -- before a path beginning with -)",
                arg.to_string_lossy()
            ));
        } else if !separator {
            return Err("--git-diff requires -- before the two paths".to_owned());
        } else if arg.is_empty() {
            return Err("Git diff paths must not be empty".to_owned());
        } else {
            paths.push(PathBuf::from(arg));
            index += 1;
        }
    }
    if !command || !separator {
        return Err("--git-diff requires -- before the two paths".to_owned());
    }
    let paths = paths.try_into().map_err(|paths: Vec<PathBuf>| {
        format!(
            "--git-diff expects exactly two paths after --, received {}",
            paths.len()
        )
    })?;
    Ok(Command::GitDiff(GitDiffRequest { paths, mode, label }))
}

fn parse_merge(args: &[OsString]) -> Result<Command, String> {
    let mut command = false;
    let mut base = None;
    let mut no_base = false;
    let mut local = None;
    let mut remote = None;
    let mut output = None;
    let mut label = None;
    let mut wait = false;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == OsStr::new("--git-merge") {
            if command {
                return Err("--git-merge may be specified only once".to_owned());
            }
            command = true;
            index += 1;
        } else if arg == OsStr::new("--base") {
            let value = required_value(args, index, "--base", "base path")?;
            if no_base || base.replace(PathBuf::from(value)).is_some() {
                return Err("choose exactly one base option: --base or --no-base".to_owned());
            }
            index += 2;
        } else if arg == OsStr::new("--no-base") {
            if no_base || base.is_some() {
                return Err("choose exactly one base option: --base or --no-base".to_owned());
            }
            no_base = true;
            index += 1;
        } else if arg == OsStr::new("--local") {
            local = set_path(
                local,
                required_value(args, index, "--local", "local path")?,
                "--local",
            )?;
            index += 2;
        } else if arg == OsStr::new("--remote") {
            remote = set_path(
                remote,
                required_value(args, index, "--remote", "remote path")?,
                "--remote",
            )?;
            index += 2;
        } else if arg == OsStr::new("--output") {
            output = set_path(
                output,
                required_value(args, index, "--output", "output path")?,
                "--output",
            )?;
            index += 2;
        } else if arg == OsStr::new("--label") {
            let value = required_value(args, index, "--label", "label")?;
            if label
                .replace(value.to_string_lossy().into_owned())
                .is_some()
            {
                return Err("--label may be specified only once".to_owned());
            }
            index += 2;
        } else if arg == OsStr::new("--wait") {
            if wait {
                return Err("--wait may be specified only once".to_owned());
            }
            wait = true;
            index += 1;
        } else if arg.as_os_str().to_string_lossy().starts_with('-') {
            return Err(format!(
                "unknown option for --git-merge: {}",
                arg.to_string_lossy()
            ));
        } else {
            return Err(format!(
                "unexpected positional argument for --git-merge: {}",
                arg.to_string_lossy()
            ));
        }
    }
    if !command {
        return Err("--git-merge is required for a Git merge launch".to_owned());
    }
    if !no_base && base.is_none() {
        return Err("--git-merge requires exactly one of --base or --no-base".to_owned());
    }
    Ok(Command::GitMerge(GitMergeRequest {
        base,
        local: local.ok_or_else(|| "--git-merge requires --local <path>".to_owned())?,
        remote: remote.ok_or_else(|| "--git-merge requires --remote <path>".to_owned())?,
        output: output.ok_or_else(|| "--git-merge requires --output <path>".to_owned())?,
        label,
    }))
}

fn set_path(
    existing: Option<PathBuf>,
    value: &OsString,
    option: &str,
) -> Result<Option<PathBuf>, String> {
    if existing.is_some() {
        return Err(format!("{option} may be specified only once"));
    }
    Ok(Some(PathBuf::from(value)))
}

fn required_value<'a>(
    args: &'a [OsString],
    index: usize,
    option: &str,
    description: &str,
) -> Result<&'a OsString, String> {
    let value = args
        .get(index + 1)
        .ok_or_else(|| format!("{option} requires {description}"))?;
    if value.is_empty() {
        return Err(format!("{description} must not be empty"));
    }
    Ok(value)
}
