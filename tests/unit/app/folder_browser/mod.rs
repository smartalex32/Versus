//! Shared fixtures and capability test modules for FolderBrowser.

use super::overlay::{build_merged_folder_cache, compare_overlay_files, overlay_signature};
use super::save::{pending_save_files, retain_saved_snapshot};
use super::search_state::SearchJob;
use super::*;
use std::fs;
use versus::{
    FileFingerprint, SearchEvent, SearchQuery, SearchResults, SearchScope, TextEncoding,
    load_working_file,
};

fn tree_entry(
    path: &str,
    kind: DirectoryEntryKind,
    state: DirectoryEntryState,
) -> versus::DirectoryEntry {
    let left_exists = !matches!(state, DirectoryEntryState::RightOnly);
    let right_exists = !matches!(state, DirectoryEntryState::LeftOnly);
    versus::DirectoryEntry {
        relative_path: PathBuf::from(path),
        left_exists,
        right_exists,
        left_kind: left_exists.then(|| kind.clone()),
        right_kind: right_exists.then(|| kind.clone()),
        left_size: None,
        right_size: None,
        kind,
        state,
    }
}

fn folder_tree(entries: Vec<versus::DirectoryEntry>) -> FolderTree {
    FolderTree::from_diff(&versus::DirectoryDiff {
        entries,
        cancelled: false,
    })
}

fn dialog_frame(
    browser: &mut FolderBrowser,
    view: &mut InteractiveView,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    dialog_frame_at(browser, view, ctx, events, None)
}
fn dialog_frame_at(
    browser: &mut FolderBrowser,
    view: &mut InteractiveView,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    time: Option<f64>,
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            events,
            time,
            ..Default::default()
        },
        |_| browser.search_window(ctx, view),
    )
}

fn streamed_file(side: usize, id: u64, text: &str) -> versus::SearchFileResults {
    let path = PathBuf::from(format!("side-{side}.txt"));
    let range = text.find("match").unwrap()..text.find("match").unwrap() + 5;
    versus::SearchFileResults {
        side,
        path: path.clone(),
        working_file: WorkingFile {
            identity: None,
            path: Some(path.clone()),
            text: text.into(),
            encoding: TextEncoding::Utf8 { bom: false },
            fingerprint: FileFingerprint::Bytes(text.as_bytes().to_vec()),
        },
        hits: vec![versus::SearchHit {
            id,
            side,
            path,
            range,
            line: 1,
            matched: "match".into(),
        }],
    }
}

fn click_dialog(
    browser: &mut FolderBrowser,
    view: &mut InteractiveView,
    ctx: &egui::Context,
    pos: Pos2,
) {
    for pressed in [true, false] {
        dialog_frame(
            browser,
            view,
            ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        )
        .drop_without_applying_deltas();
    }
}

mod cache;
mod modal;
mod navigation;
mod overlay;
mod save;
mod search;
mod search_lifecycle;
