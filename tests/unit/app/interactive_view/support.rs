//! Shared fixtures and interaction helpers for interactive workspace regression tests.

use super::*;
use crate::app::visuals::footer::{bottom_status_area, error_summary};
use versus::SearchChange;

impl InteractiveView {
    pub(crate) fn current_paths(&self) -> Option<&[PathBuf; 2]> {
        self.paths.as_ref()
    }

    pub(crate) fn working_text(&self, side: usize) -> Option<&str> {
        self.workspace
            .as_ref()
            .map(|workspace| workspace.text(side_of(side)))
    }

    pub(crate) fn editor_id(&self, side: usize) -> egui::Id {
        egui::Id::new(("two-way-edit", self.side_generation[side], side))
    }

    pub(crate) fn set_working_text_for_test(&mut self, side: usize, text: &str) {
        self.workspace
            .as_mut()
            .expect("loaded test file has a workspace")
            .set_text(side_of(side), text);
    }

    pub(crate) fn ingest_working_files(&mut self, files: &BTreeMap<PathBuf, WorkingFile>) {
        self.publish_active();
        for (path, file) in files {
            let key = file.identity_key();
            self.aliases.insert(path.clone(), key.clone());
            self.aliases.insert(key.clone(), key.clone());
            if let Some(retained) = self.documents.get_mut(&key) {
                if retained.document.text() != file.text {
                    let mut facade = TwoWayWorkspace::from_documents([
                        retained.document.clone(),
                        InteractiveDocument::new(""),
                    ]);
                    facade.set_text(TwoWaySide::Left, file.text.clone());
                    retained.document = facade.document(TwoWaySide::Left).clone();
                }
                retained.file = file.clone();
                if !file.is_dirty() {
                    retained.document.mark_saved_text(file.text.clone());
                }
            } else {
                self.documents.insert(key, retained_file(file.clone()));
            }
        }
        self.refresh_active();
        self.invalidate_diff();
    }

    pub(crate) fn render(&mut self, ui: &mut egui::Ui) {
        let (_, status) = self.render_with_sources(ui, |_| {});
        ui.scope_builder(egui::UiBuilder::new().max_rect(status), |ui| {
            let errors: Vec<_> = self
                .error_details()
                .map(|error| error.lines().map(str::to_owned).collect())
                .unwrap_or_default();
            bottom_status_area(ui, |ui| {
                if errors.is_empty() {
                    self.render_activity_status(ui);
                } else {
                    error_summary(ui, &errors);
                }
            });
        });
    }
}

pub(super) fn restore_line_endings(original: &str, edited: &str) -> String {
    let (range, replacement) = editor_byte_edit(original, edited, None);
    let mut text = original.to_owned();
    text.replace_range(range, &replacement);
    text
}

pub(super) fn view(left: &str, right: &str) -> InteractiveView {
    let paths = [PathBuf::from("left.rs"), PathBuf::from("right.rs")];
    let files = std::array::from_fn(|side| {
        let text = if side == 0 { left } else { right };
        WorkingFile {
            identity: None,
            path: Some(paths[side].clone()),
            text: text.into(),
            encoding: versus::TextEncoding::Utf8 { bom: false },
            fingerprint: versus::FileFingerprint::Bytes(text.as_bytes().to_vec()),
        }
    });
    InteractiveView {
        workspace: Some(TwoWayWorkspace::new(left, right)),
        files: Some(files),
        paths: Some(paths),
        editing: true,
        ..Default::default()
    }
}
pub(super) fn readonly_comparison(lines: usize) -> FileComparison {
    FileComparison {
        rows: (0..lines)
            .map(|index| versus::FileComparisonRow {
                left: Some((index + 1, format!("left\tline {index}"))),
                right: Some((index + 1, format!("right line {index}"))),
                state: if index % 3 == 0 {
                    DirectoryEntryState::Different
                } else {
                    DirectoryEntryState::Same
                },
                left_changed: Vec::new(),
                right_changed: Vec::new(),
                left_ending: Some(versus::DisplayLineEnding::Lf),
                right_ending: Some(versus::DisplayLineEnding::Lf),
            })
            .collect(),
        message: None,
    }
}
pub(super) fn partial_view(path: PathBuf, text: &str) -> InteractiveView {
    let file = WorkingFile {
        identity: None,
        path: Some(path.clone()),
        text: text.into(),
        encoding: versus::TextEncoding::Utf8 { bom: false },
        fingerprint: versus::FileFingerprint::Bytes(text.as_bytes().to_vec()),
    };
    let mut view = InteractiveView {
        editing: true,
        ..Default::default()
    };
    view.register_file(path.clone(), file.clone());
    view.side_sources[0] = Some(path);
    view.side_files[0] = Some(file);
    view.activate_side_workspace();
    view
}
pub(super) fn settle_save(view: &mut InteractiveView, ctx: &egui::Context) {
    for _ in 0..100 {
        view.poll_save(ctx);
        if !view.is_saving() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("save did not finish");
}
pub(super) fn frame(
    view: &mut InteractiveView,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(900.0 / ctx.zoom_factor(), 650.0 / ctx.zoom_factor()),
            )),
            events,
            ..Default::default()
        },
        |ui| view.render(ui),
    )
}
pub(super) fn click(view: &mut InteractiveView, ctx: &egui::Context, id: egui::Id) {
    click_frame(view, ctx, id).drop_without_applying_deltas();
}
pub(super) fn click_frame(
    view: &mut InteractiveView,
    ctx: &egui::Context,
    id: egui::Id,
) -> egui::FullOutput {
    let position = ctx.read_response(id).unwrap().rect.center();
    let pointer = |pressed| {
        vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    frame(view, ctx, pointer(true)).drop_without_applying_deltas();
    frame(view, ctx, pointer(false))
}
pub(super) fn preview_for(
    view: &InteractiveView,
    replacements: &[(&str, std::ops::Range<usize>, &str)],
) -> versus::SearchPreview {
    let bases = view.export_working_files();
    let mut changes = Vec::new();
    for (path, range, replacement) in replacements {
        let file = &bases[&PathBuf::from(path)];
        let mut after = file.text.clone();
        after.replace_range(range.clone(), replacement);
        changes.push(SearchChange {
            path: PathBuf::from(path),
            before: file.text.clone(),
            after,
            hit_ids: vec![changes.len() as u64 + 1],
            ranges: vec![range.clone()],
            edits: vec![(range.clone(), (*replacement).into())],
        });
    }
    versus::SearchPreview {
        changes,
        bases,
        skipped: Vec::new(),
    }
}
pub(super) fn refreshed_preview(
    view: &InteractiveView,
    updates: &[(&str, &str)],
) -> versus::SearchPreview {
    let mut bases = view.export_working_files();
    let mut changes = Vec::new();
    for (path, text) in updates {
        let path = PathBuf::from(path);
        let base = bases.get_mut(&path).unwrap();
        base.text = (*text).into();
        base.fingerprint = versus::FileFingerprint::Bytes(text.as_bytes().to_vec());
        let start = text.find("match").unwrap();
        let range = start..start + 5;
        let mut after = (*text).to_owned();
        after.replace_range(range.clone(), "found");
        changes.push(SearchChange {
            path,
            before: (*text).into(),
            after,
            hit_ids: vec![changes.len() as u64 + 1],
            ranges: vec![range.clone()],
            edits: vec![(range, "found".into())],
        });
    }
    versus::SearchPreview {
        changes,
        bases,
        skipped: Vec::new(),
    }
}
