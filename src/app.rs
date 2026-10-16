//! Native app entry point: window lifecycle and capability composition.

use crate::app::visuals::theme::{Palette, apply_theme, restore_zoom};
use crate::recent::RecentComparison;
use crate::selection::{self, ComparisonMode, SelectionMode, SourceKind};
mod file_diff;
mod source_controls;
mod workspace;
use file_diff::paint_file_line;
mod drop_targets;
mod folder_diff;
mod leave_dialog;
mod recent_view;
mod workspace_status;
use drop_targets::paint_empty_drop_target;
mod visuals;
use crate::selection::{absolute_native_path, absolute_path, empty_diff_side};

mod file_style;
mod find_icons;
mod folder_browser;
mod interactive_view;
mod session;
use session::{MergeExit, RecentStore, Session};
#[cfg(test)]
#[path = "../tests/unit/app/operating_modes.rs"]
mod operating_mode_tests;
mod search_result_rows;
mod search_session;
#[cfg(test)]
#[path = "../tests/unit/app/mod.rs"]
mod tests;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use folder_browser::FolderBrowser;
use interactive_view::InteractiveView;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::{Duration, Instant},
};
use versus::{
    CompareError, DirectoryEntryKind, DirectoryEntryState, FolderTree, ProgressStage, TreeNode,
};

struct MergePaneHeaders {
    files: [Option<PathBuf>; 2],
    back: [bool; 2],
    save: [bool; 2],
    dirty: [bool; 2],
}

pub struct VersusApp {
    session: Session,
    logged_errors: Vec<String>,
    logo_texture: Option<(bool, egui::TextureHandle)>,
    preferences_context: Option<egui::Context>,
    pane_rects: [Rect; 2],
    source_header_rects: [Rect; 2],
    native_drag_position: Option<Pos2>,
    drop_hover_side: Option<usize>,
}
impl Default for VersusApp {
    fn default() -> Self {
        Self {
            session: Session::default(),
            logged_errors: Vec::new(),
            logo_texture: None,
            preferences_context: None,
            pane_rects: [Rect::NOTHING; 2],
            source_header_rects: [Rect::NOTHING; 2],
            native_drag_position: None,
            drop_hover_side: None,
        }
    }
}
impl VersusApp {
    pub(super) fn report_startup_error(&mut self, error: String) {
        self.session.error = Some(error);
    }
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        launch: Option<crate::cli::LaunchRequest>,
    ) -> Self {
        apply_theme(&cc.egui_ctx);
        if let Some(storage) = cc.storage {
            restore_zoom(&cc.egui_ctx, storage);
        }
        let mut app = Self::default();
        app.preferences_context = Some(cc.egui_ctx.clone());
        app.session.recent_store = Some(RecentStore::start(&cc.egui_ctx, None));
        if let Some(launch) = launch {
            app.session.open_launch_request(launch);
            cc.egui_ctx.request_repaint();
        }
        app
    }
}

impl eframe::App for VersusApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(ctx) = &self.preferences_context {
            storage.set_string("versus.zoom", ctx.zoom_factor().to_string());
        }
    }

    fn persist_egui_memory(&self) -> bool {
        // Only the UI scale is retained; compared paths/content remain session-local.
        false
    }
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        Palette::new(visuals.dark_mode)
            .background
            .to_normalized_gamma_f32()
    }
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if let Some(error) = crate::diagnostics::take_error() {
            self.session.error = Some(error);
        }
        self.session.poll(ctx);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        #[cfg(windows)]
        if ui.input(|input| {
            !input.raw.hovered_files.is_empty() || !input.raw.dropped_files.is_empty()
        }) {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            self.native_drag_position = _frame.window_handle().ok().and_then(|handle| {
                if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                    crate::native_drop::cursor_position(
                        handle.hwnd.get(),
                        ui.ctx().pixels_per_point(),
                    )
                } else {
                    None
                }
            });
        }
        self.render(ui);
    }
}
