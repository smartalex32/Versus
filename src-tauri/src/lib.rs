use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::State;
use versus::{
    DiffHunk, DirectoryCompareOptions, DirectoryEntryKind, DirectoryEntryState, FileCompareOptions,
    FileDiffKind, HunkKind, LineEnding, SaveOptions, TextDiff,
};

#[derive(Default)]
struct DirectoryTask {
    current: Mutex<Option<Arc<AtomicBool>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryEntryDto {
    relative_path: String,
    kind: &'static str,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryResult {
    entries: Vec<DirectoryEntryDto>,
    cancelled: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HunkDto {
    left_start: usize,
    left_end: usize,
    right_start: usize,
    right_end: usize,
    kind: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TextDiffDto {
    left_lines: Vec<String>,
    right_lines: Vec<String>,
    hunks: Vec<HunkDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileResult {
    kind: &'static str,
    equal: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    left_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    right_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diff: Option<TextDiffDto>,
}

impl From<DiffHunk> for HunkDto {
    fn from(hunk: DiffHunk) -> Self {
        Self {
            left_start: hunk.left_range.start,
            left_end: hunk.left_range.end,
            right_start: hunk.right_range.start,
            right_end: hunk.right_range.end,
            kind: match hunk.kind {
                HunkKind::Added => "Added",
                HunkKind::Removed => "Removed",
                HunkKind::Changed => "Changed",
            },
        }
    }
}

impl From<TextDiff> for TextDiffDto {
    fn from(diff: TextDiff) -> Self {
        Self {
            left_lines: diff.left_lines,
            right_lines: diff.right_lines,
            hunks: diff.hunks.into_iter().map(Into::into).collect(),
        }
    }
}

fn file_options(ignore_whitespace: bool, ignore_line_endings: bool) -> FileCompareOptions {
    FileCompareOptions {
        ignore_whitespace,
        ignore_line_endings,
        ..Default::default()
    }
}

fn read_editable_text(path: &Path) -> Result<String, String> {
    let size = fs::metadata(path)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .len();
    if size > versus::DEFAULT_TEXT_SIZE_LIMIT {
        return Err(format!(
            "{}: file exceeds the text size limit",
            path.display()
        ));
    }
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if bytes.len() as u64 > versus::DEFAULT_TEXT_SIZE_LIMIT || bytes.contains(&0) {
        return Err(format!(
            "{}: file is not editable UTF-8 text",
            path.display()
        ));
    }
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    String::from_utf8(bytes.to_vec()).map_err(|error| format!("{}: {error}", path.display()))
}

fn directory_entry(entry: versus::DirectoryEntry) -> DirectoryEntryDto {
    let (state, error) = match entry.state {
        DirectoryEntryState::Same => ("Same", None),
        DirectoryEntryState::Different => ("Different", None),
        DirectoryEntryState::LeftOnly => ("LeftOnly", None),
        DirectoryEntryState::RightOnly => ("RightOnly", None),
        DirectoryEntryState::TypeMismatch => ("TypeMismatch", None),
        DirectoryEntryState::Error(error) => ("Error", Some(error.to_string())),
    };
    DirectoryEntryDto {
        relative_path: entry.relative_path.to_string_lossy().replace('\\', "/"),
        kind: match entry.kind {
            DirectoryEntryKind::File => "File",
            DirectoryEntryKind::Directory => "Directory",
            DirectoryEntryKind::Symlink => "Symlink",
            DirectoryEntryKind::Other => "Other",
        },
        state,
        error,
    }
}

#[tauri::command]
async fn pick_path(directory: bool) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dialog = rfd::FileDialog::new();
        let selected = if directory {
            dialog.pick_folder()
        } else {
            dialog.pick_file()
        };
        selected.map(|path| path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn compare_directories(
    left: String,
    right: String,
    state: State<'_, DirectoryTask>,
) -> Result<DirectoryResult, String> {
    let cancellation = Arc::new(AtomicBool::new(false));
    {
        let mut current = state.current.lock().map_err(|error| error.to_string())?;
        if let Some(previous) = current.replace(Arc::clone(&cancellation)) {
            previous.store(true, Ordering::Relaxed);
        }
    }
    let worker_token = Arc::clone(&cancellation);
    let result = tauri::async_runtime::spawn_blocking(move || {
        let options = DirectoryCompareOptions {
            cancellation: Some(worker_token),
            ..Default::default()
        };
        versus::compare_directories(
            PathBuf::from(left.trim()),
            PathBuf::from(right.trim()),
            &options,
        )
        .map_err(|error| error.to_string())
        .map(|diff| DirectoryResult {
            entries: diff.entries.into_iter().map(directory_entry).collect(),
            cancelled: diff.cancelled,
        })
    })
    .await
    .map_err(|error| error.to_string())?;
    let mut current = state.current.lock().map_err(|error| error.to_string())?;
    if current
        .as_ref()
        .is_some_and(|token| Arc::ptr_eq(token, &cancellation))
    {
        *current = None;
    }
    result
}

#[tauri::command]
fn cancel_directory_compare(state: State<'_, DirectoryTask>) -> Result<(), String> {
    if let Some(token) = state
        .current
        .lock()
        .map_err(|error| error.to_string())?
        .as_ref()
    {
        token.store(true, Ordering::Relaxed);
    }
    Ok(())
}

#[tauri::command]
async fn compare_files(
    left: String,
    right: String,
    ignore_whitespace: bool,
    ignore_line_endings: bool,
) -> Result<FileResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let left_path = PathBuf::from(left.trim());
        let right_path = PathBuf::from(right.trim());
        let options = file_options(ignore_whitespace, ignore_line_endings);
        let file = versus::compare_files(&left_path, &right_path, &options)
            .map_err(|error| error.to_string())?;
        let kind = match file.kind {
            FileDiffKind::Text => "Text",
            FileDiffKind::Binary => "Binary",
            FileDiffKind::TooLarge => "TooLarge",
        };
        let (left_text, right_text, diff, equal) = if file.kind == FileDiffKind::Text {
            let left_text = read_editable_text(&left_path)?;
            let right_text = read_editable_text(&right_path)?;
            let diff = versus::compare_texts(&left_text, &right_text, &options);
            let equal = diff.hunks.is_empty();
            (Some(left_text), Some(right_text), Some(diff.into()), equal)
        } else {
            (None, None, None, file.equal)
        };
        Ok(FileResult {
            kind,
            equal,
            left_text,
            right_text,
            diff,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn compare_texts(
    left_text: String,
    right_text: String,
    ignore_whitespace: bool,
    ignore_line_endings: bool,
) -> Result<TextDiffDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        versus::compare_texts(
            &left_text,
            &right_text,
            &file_options(ignore_whitespace, ignore_line_endings),
        )
        .into()
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn save_text(
    path: String,
    contents: String,
    line_ending: String,
    overwrite: bool,
) -> Result<(), String> {
    let ending = match line_ending.as_str() {
        "Lf" => LineEnding::Lf,
        "CrLf" => LineEnding::Crlf,
        _ => return Err("Unsupported line ending".into()),
    };
    tauri::async_runtime::spawn_blocking(move || {
        let target = PathBuf::from(path.trim());
        if target.as_os_str().is_empty() {
            return Err("Enter a destination path.".into());
        }
        versus::save_text_safely(
            &target,
            &contents,
            &SaveOptions {
                overwrite,
                line_ending: ending,
            },
        )
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

fn allows_offline_navigation(url: &tauri::Url) -> bool {
    match (url.scheme(), url.host_str()) {
        ("tauri", Some("localhost")) | ("http" | "https", Some("tauri.localhost")) => true,
        #[cfg(debug_assertions)]
        ("http", Some("127.0.0.1")) if url.port() == Some(1420) => true,
        _ => false,
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri::plugin::Builder::<tauri::Wry, ()>::new("offline-navigation")
                .on_navigation(|_, url| allows_offline_navigation(url))
                .build(),
        )
        .manage(DirectoryTask::default())
        .invoke_handler(tauri::generate_handler![
            pick_path,
            compare_directories,
            cancel_directory_compare,
            compare_files,
            compare_texts,
            save_text,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Versus");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_is_limited_to_the_local_app() {
        for value in ["tauri://localhost/", "http://tauri.localhost/"] {
            assert!(allows_offline_navigation(
                &tauri::Url::parse(value).unwrap()
            ));
        }
        assert!(!allows_offline_navigation(
            &tauri::Url::parse("https://example.com/").unwrap()
        ));
    }
}
