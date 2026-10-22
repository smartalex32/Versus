//! Background classification and bounded comparison jobs for Git Diff.

use super::*;
use crate::selection::ComparisonMode;
use std::sync::{Arc, atomic::AtomicBool, mpsc};
use versus::{
    DirectoryCompareOptions, FileComparison, FileViewOptions, FolderTree, compare_directories,
    load_file_comparison_with_options,
};

pub(super) fn spawn_initial_job(
    paths: [PathBuf; 2],
    mode: Option<ComparisonMode>,
    options: Options,
) -> Job {
    spawn(paths, mode, options, Target::Initial)
}

pub(super) fn spawn_folder_file_job(
    paths: [PathBuf; 2],
    exists: [bool; 2],
    options: Options,
    previous: FolderScreen,
) -> Job {
    let cancellation = Arc::new(AtomicBool::new(false));
    let worker_cancellation = cancellation.clone();
    let worker_paths = paths.clone();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(
            load_file(
                &worker_paths,
                exists.map(|present| present.then_some(())),
                options,
                &worker_cancellation,
            )
            .map(Loaded::File),
        );
    });
    Job {
        receiver,
        cancellation,
        target: Some(Target::FolderFile {
            previous,
            paths,
            sources: exists,
        }),
    }
}

fn spawn(
    paths: [PathBuf; 2],
    mode: Option<ComparisonMode>,
    options: Options,
    target: Target,
) -> Job {
    let cancellation = Arc::new(AtomicBool::new(false));
    let worker_cancellation = cancellation.clone();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = match mode {
            Some(ComparisonMode::File) => load_file(
                &paths,
                source_presence(&paths),
                options,
                &worker_cancellation,
            )
            .map(Loaded::File),
            Some(ComparisonMode::Folder) => {
                load_folder(&paths, options, &worker_cancellation).map(Loaded::Folder)
            }
            None => match classify(&paths) {
                Ok(ComparisonMode::File) => load_file(
                    &paths,
                    source_presence(&paths),
                    options,
                    &worker_cancellation,
                )
                .map(Loaded::File),
                Ok(ComparisonMode::Folder) => {
                    load_folder(&paths, options, &worker_cancellation).map(Loaded::Folder)
                }
                Err(error) => Err(error),
            },
        };
        let _ = sender.send(result);
    });
    Job {
        receiver,
        cancellation,
        target: Some(target),
    }
}

fn classify(paths: &[PathBuf; 2]) -> Result<ComparisonMode, String> {
    let kinds = paths.clone().map(|path| {
        if crate::selection::empty_diff_side(&path) {
            return Ok(None);
        }
        std::fs::symlink_metadata(&path)
            .map_err(|error| format!("Cannot inspect {}: {error}", path.display()))
            .and_then(|metadata| {
                if metadata.file_type().is_symlink() {
                    Err(format!(
                        "Git Diff does not follow symbolic links: {}",
                        path.display()
                    ))
                } else if metadata.is_file() {
                    Ok(Some(ComparisonMode::File))
                } else if metadata.is_dir() {
                    Ok(Some(ComparisonMode::Folder))
                } else {
                    Err(format!(
                        "Git Diff supports files and folders only: {}",
                        path.display()
                    ))
                }
            })
    });
    match (&kinds[0], &kinds[1]) {
        (Ok(Some(left)), Ok(Some(right))) if left == right => Ok(*left),
        (Ok(None), Ok(Some(ComparisonMode::File))) | (Ok(Some(ComparisonMode::File)), Ok(None)) => {
            Ok(ComparisonMode::File)
        }
        (Ok(None), Ok(None)) => Err("Git Diff requires at least one file or folder.".into()),
        (Ok(_), Ok(_)) => Err("Git Diff cannot compare a file with a folder.".into()),
        (Err(error), _) | (_, Err(error)) => Err(error.clone()),
    }
}

pub(super) fn source_presence(paths: &[PathBuf; 2]) -> [Option<()>; 2] {
    std::array::from_fn(|side| (!crate::selection::empty_diff_side(&paths[side])).then_some(()))
}

fn load_file(
    paths: &[PathBuf; 2],
    exists: [Option<()>; 2],
    options: Options,
    cancellation: &AtomicBool,
) -> Result<FileComparison, String> {
    let sources = std::array::from_fn(|side| exists[side].map(|()| paths[side].clone()));
    load_file_comparison_with_options(
        &sources,
        cancellation,
        &FileViewOptions {
            ignore_whitespace: options.ignore_whitespace,
            ignore_line_endings: options.ignore_line_endings,
            progress: None,
        },
    )
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "Comparison cancelled.".into())
}

fn load_folder(
    paths: &[PathBuf; 2],
    options: Options,
    cancellation: &Arc<AtomicBool>,
) -> Result<FolderTree, String> {
    for path in paths {
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| format!("Cannot inspect {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Git Diff does not follow symbolic links: {}",
                path.display()
            ));
        }
        if !metadata.is_dir() {
            return Err(format!("Git Diff requires a folder: {}", path.display()));
        }
    }
    compare_directories(
        &paths[0],
        &paths[1],
        &DirectoryCompareOptions {
            cancellation: Some(cancellation.clone()),
            ignore_whitespace: options.ignore_whitespace,
            ignore_line_endings: options.ignore_line_endings,
            ..Default::default()
        },
    )
    .map(|diff| FolderTree::from_diff(&diff))
    .map_err(|error| error.to_string())
}
