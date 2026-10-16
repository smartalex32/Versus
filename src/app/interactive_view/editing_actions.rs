//! Footer details, transfers, cursor operations, saves, and editor shortcuts.

use super::*;
use crate::app::visuals::footer::{
    DIFF_LEGEND_ICONS, bottom_legend, footer_guidance, workspace_footer_layout,
};
use crate::app::visuals::icons::{ToolbarIcon, captioned_icon_button};
use crate::app::visuals::status::StatusIcon;
use crate::app::visuals::theme::Palette;

impl InteractiveView {
    pub(super) fn footer(&mut self, ui: &mut egui::Ui) -> Rect {
        let icons = self.footer_legend_icons();
        let dirty = self.any_dirty();
        let row =
            workspace_footer_layout(ui, self.editing, dirty, &icons, self.footer_legend_counts());
        ui.scope_builder(egui::UiBuilder::new().max_rect(row.info), |ui| {
            footer_guidance(ui, self.guidance_message());
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(row.legend), |ui| {
            bottom_legend(ui, &icons, self.footer_legend_counts());
        });
        if let Some(dirty) = row.dirty {
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(dirty)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    let response = ui.add(
                        egui::Label::new(
                            RichText::new("Unsaved Changes")
                                .strong()
                                .size(11.0)
                                .color(Palette::for_context(ui.ctx()).changed),
                        )
                        .truncate(),
                    );
                    ui.interact(
                        response.rect,
                        egui::Id::new("workspace-unsaved"),
                        Sense::hover(),
                    )
                    .on_hover_text("Changes remain in memory until saved.");
                },
            );
        }
        if let Some(save) = row.save {
            let save_sides: Vec<_> = (0..2).filter(|side| self.can_save_side(*side)).collect();
            let retained_dirty = self.has_retained_dirty_documents();
            let enabled = !self.is_saving() && (!save_sides.is_empty() || retained_dirty);
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(save)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    if captioned_icon_button(ui, ToolbarIcon::Save, enabled, "Save Both").clicked()
                    {
                        if retained_dirty {
                            self.save_all_requested = true;
                        } else {
                            self.request_save(save_sides);
                        }
                    }
                },
            );
        }
        row.status
    }
    pub(super) fn footer_legend_icons(&self) -> Vec<StatusIcon> {
        let mut icons = DIFF_LEGEND_ICONS.to_vec();
        if self.editing {
            icons.push(StatusIcon::Conflict);
        }
        icons
    }
    pub(super) fn footer_legend_counts(&self) -> Option<[usize; 6]> {
        let Some(diff) = self.diff.as_ref() else {
            return self.presentation.pending_counts();
        };
        let mut counts = [0; 6];
        for row in &diff.rows {
            match row.state {
                DirectoryEntryState::Different => counts[1] += 1,
                DirectoryEntryState::LeftOnly => counts[2] += 1,
                DirectoryEntryState::RightOnly => counts[3] += 1,
                DirectoryEntryState::TypeMismatch => counts[4] += 1,
                DirectoryEntryState::Error(_) => counts[5] += 1,
                DirectoryEntryState::Same => {}
            }
        }
        Some(counts)
    }
    pub(super) fn active_file(&self, side: usize) -> Option<&WorkingFile> {
        self.side_sources[side]
            .as_ref()
            .and_then(|path| self.documents.get(&self.key_for(path)))
            .map(|retained| &retained.file)
            .or_else(|| {
                self.side_files[side]
                    .as_ref()
                    .or_else(|| self.files.as_ref().map(|files| &files[side]))
            })
    }
    pub(super) fn side_has_file(&self, side: usize) -> bool {
        self.active_file(side).is_some()
    }
    pub(super) fn has_actual_pair(&self) -> bool {
        self.files.is_some()
            || (self.side_sources.iter().all(Option::is_some)
                && self.side_files.iter().all(Option::is_some))
    }
    pub(super) fn transfer(&mut self, source: TwoWaySide, range: Range<usize>) {
        if !self.has_actual_pair() {
            return;
        }
        let dest = source.other();
        let side = side_index(dest);
        let at = self.caret[side].min(self.workspace.as_ref().unwrap().line_count(dest));
        let target = at..at;
        if self.filtered_merge_range_hidden(source, &range) {
            self.error = Some("That transfer crosses hidden unchanged lines. Turn off Show only differences to transfer the full range.".into());
            return;
        }
        let old_count = self.workspace.as_ref().unwrap().line_count(dest);
        match self.workspace.as_mut().unwrap().transfer(
            source,
            range,
            target.clone(),
            TransferPlacement::Insert,
        ) {
            Ok(()) => {
                let new_count = self.workspace.as_ref().unwrap().line_count(dest);
                let inserted = (new_count as isize - old_count as isize + target.len() as isize)
                    .max(0) as usize;
                self.caret[side] = target.start + inserted;
                self.activate_editor(side);
                self.error = None;
                self.status = "Transferred into the working copy. Save when ready.".into();
                self.invalidate_diff();
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    pub(super) fn filtered_merge_range_hidden(
        &self,
        side: TwoWaySide,
        range: &Range<usize>,
    ) -> bool {
        if range.is_empty() || !self.editing || !self.run_diff || !self.merge_options[0] {
            return false;
        }
        if self.diff.is_none() && !self.presentation.has_pending() {
            return false;
        }
        let decoration = self.merge_decoration(side_index(side));
        range.clone().any(|line| !decoration.is_changed(line))
    }

    pub(super) fn copy_all(&mut self, source: TwoWaySide) {
        if !self.has_actual_pair() {
            return;
        }
        let dest = source.other();
        let source_count = self.workspace.as_ref().unwrap().line_count(source);
        let dest_count = self.workspace.as_ref().unwrap().line_count(dest);
        match self.workspace.as_mut().unwrap().transfer(
            source,
            0..source_count,
            0..dest_count,
            TransferPlacement::Replace,
        ) {
            Ok(()) => {
                let count = self.workspace.as_ref().unwrap().line_count(dest);
                let side = side_index(dest);
                self.caret[side] = count;
                self.activate_editor(side);
                self.invalidate_diff();
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    /// Restore cursor coordinates through a full source view before the next
    /// frame projects them into the filtered editor.
    pub(super) fn restore_editor_byte_cursor(&mut self, side: usize, byte: usize) {
        let Some(ctx) = &self.editor_context else {
            return;
        };
        let Some(workspace) = &self.workspace else {
            return;
        };
        let updated = workspace.text(side_of(side));
        let displayed = updated.replace("\r\n", "\n").replace('\r', "\n");
        let position = original_byte_to_normalized_chars(updated, byte.min(updated.len()));
        let id = egui::Id::new(("two-way-edit", self.side_generation[side], side));
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(position),
            )));
        state.store(ctx, id);
        self.caret[side] = displayed
            .chars()
            .take(position)
            .filter(|c| *c == '\n')
            .count();
        self.presentation.set_full_editor(side, displayed);
    }
    pub(super) fn sync_editor_cursor(&mut self, side: usize) {
        let (Some(ctx), Some(workspace)) = (&self.editor_context, &self.workspace) else {
            return;
        };
        let text = workspace
            .text(side_of(side))
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        let at = self.caret[side];
        let offset = text
            .split_inclusive('\n')
            .take(at)
            .map(|line| line.chars().count())
            .sum::<usize>();
        let id = egui::Id::new(("two-way-edit", self.side_generation[side], side));
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(offset),
            )));
        state.clear_undoer();
        state.store(ctx, id);
        // These explicit targets are in the full source buffer. Record that
        // coordinate space so the next filtered render maps them only once.
        self.presentation.set_full_editor(side, text);
    }
    pub(super) fn activate_editor(&mut self, side: usize) {
        self.selected_side = side;
        self.sync_editor_cursor(side);
        if let Some(ctx) = &self.editor_context {
            ctx.memory_mut(|memory| {
                memory.request_focus(egui::Id::new((
                    "two-way-edit",
                    self.side_generation[side],
                    side,
                )))
            });
        }
    }
    pub(super) fn save_sides(&mut self, sides: Vec<usize>) {
        if self.is_saving() {
            return;
        }
        self.publish_active();
        let Some(workspace) = &self.workspace else {
            return;
        };
        let copies: BTreeMap<_, _> = sides
            .into_iter()
            .filter_map(|side| {
                let mut file = self.active_file(side)?.clone();
                file.text = workspace.text(side_of(side)).into();
                Some((file.identity_key(), file))
            })
            .collect();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let results = copies
                .into_values()
                .map(|mut file| {
                    save_working_file(&mut file)
                        .map(|()| file)
                        .map_err(|e| e.to_string())
                })
                .collect();
            let _ = sender.send(results);
        });
        self.save_job = Some(SaveJob { receiver });
        self.status = "Saving working copies…".into();
    }
    pub(super) fn poll_save(&mut self, ctx: &egui::Context) {
        let ready = self
            .save_job
            .as_ref()
            .and_then(|job| match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => Some(vec![Err("Save worker stopped".into())]),
                _ => None,
            });
        if let Some(results) = ready {
            self.save_job = None;
            let mut saved = BTreeMap::new();
            let mut errors = Vec::new();
            for result in results {
                match result {
                    Ok(file) => {
                        if let Some(path) = &file.path {
                            saved.insert(path.clone(), file);
                        }
                    }
                    Err(error) => errors.push(error),
                }
            }
            self.update_saved_files(&saved);
            self.status = format!("Saved {} working copies.", saved.len());
            self.error = (!errors.is_empty()).then(|| errors.join("\n"));
        }
        if self.save_job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(25));
        }
    }
    pub(super) fn request_save(&mut self, sides: Vec<usize>) {
        if self.is_saving() {
            return;
        }
        self.pending_save = Some(sides);
    }
    pub(super) fn save_preview(&mut self, ctx: &egui::Context) {
        let Some(sides) = self.pending_save.clone() else {
            return;
        };
        let mut confirm = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("interactive-save-preview")).show(ctx, |ui| {
            ui.heading("Save pending changes?");
            ui.label("These working copies will replace their original files after the external-change check.");
            for side in &sides {
                if let Some(path) = self.selected_source(*side)
                    .or_else(|| self.paths.as_ref().map(|paths| &paths[*side]))
                {
                    ui.label(path.display().to_string());
                }
            }
            if let Some(workspace)=&self.workspace{
                egui::ScrollArea::vertical().max_height(300.0).show(ui,|ui|{for side in &sides{
                    ui.label(if *side==0{"Left working copy"}else{"Right working copy"});
                    if let Some(file) = self.active_file(*side) {
                        ui.columns(2,|cols|{cols[0].label("Loaded file");cols[0].label(RichText::new(file.baseline_text().unwrap_or_default()).monospace());cols[1].label("Pending content");cols[1].label(RichText::new(workspace.text(side_of(*side))).monospace());});
                    }
                }});
            }
            ui.horizontal(|ui| { cancel = ui.button("Cancel").clicked(); confirm = ui.button("Save").clicked(); });
        });
        if cancel {
            self.pending_save = None;
        }
        if confirm {
            self.pending_save = None;
            self.save_sides(sides);
        }
    }
    pub(super) fn shortcuts(&mut self, ui: &mut egui::Ui) {
        if !self.editing {
            return;
        }
        // egui permits extra Shift for less-specific shortcuts, so consume
        // Redo before Undo or Command+Shift+Z would undo instead.
        if ui.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::Z,
            ) || i.consume_key(egui::Modifiers::CTRL, egui::Key::Y)
        }) {
            self.move_history(true);
        } else if ui.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)
                || i.consume_key(egui::Modifiers::CTRL, egui::Key::Z)
        }) {
            self.move_history(false);
        }
        if ui.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)
                || i.consume_key(egui::Modifiers::CTRL, egui::Key::S)
        }) {
            self.request_save(vec![self.selected_side])
        }
    }
}
