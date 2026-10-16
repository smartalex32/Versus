//! Editable Merge pane painting and projected text edits.

use super::*;
use crate::app::visuals::status::state_color;
use crate::app::visuals::theme::{Palette, comparison_font, comparison_row_height};

impl InteractiveView {
    pub(super) fn editor(
        &mut self,
        ui: &mut egui::Ui,
        side: usize,
        height: f32,
        track: Option<Rect>,
    ) {
        let palette = Palette::for_context(ui.ctx());
        let source = side_of(side);
        let current = self.workspace.as_ref().unwrap().text(source).to_owned();
        let displayed = current.replace("\r\n", "\n").replace('\r', "\n");
        let displayed_lines: Vec<_> = displayed.lines().collect();
        let font = comparison_font(ui.ctx());
        let row_height = comparison_row_height(ui);
        let font_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
        let text_padding = ((row_height - font_height) / 2.0).max(0.0);
        let count = self.workspace.as_ref().unwrap().line_count(source);
        let chunks = self.workspace.as_ref().unwrap().chunks(source);
        let navigation_line = self.navigation_lines(ui.ctx())[side];
        let max_line_number =
            count.max(self.workspace.as_ref().unwrap().line_count(source.other()));
        let (changed, ending_differences) = self.merge_decoration(side).into_parts();
        let gutter_extra = super::file_style::gutter_extra(max_line_number);
        let decorated = self.diff.is_some() || self.presentation.has_pending();
        let filtering_same = self.run_diff && self.merge_options[0] && decorated;
        let projection = filtering_same.then(|| {
            merge_projection(
                &displayed,
                &MergeDecoration::from_parts(changed.clone(), ending_differences.clone()),
            )
        });
        if projection.as_ref().is_some_and(MergeProjection::is_empty) {
            self.pending_navigation_scroll[side] = false;
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("No differences with the current options.").color(palette.muted),
                );
            });
            return;
        }
        let visible_lines: Vec<usize> = projection
            .as_ref()
            .map(|projection| projection.source_lines().collect())
            .unwrap_or_else(|| (0..count.max(1)).collect());
        let mut next = projection
            .as_ref()
            .map(|projection| projection.text().to_owned())
            .unwrap_or_else(|| displayed.clone());
        let editor_id = egui::Id::new(("two-way-edit", self.side_generation[side], side));
        self.presentation.update_editor_cursor(
            side,
            ui.ctx(),
            editor_id,
            EditorProjection::new(next.clone(), visible_lines.clone()),
        );
        let previous_cursor = egui::TextEdit::load_state(ui.ctx(), editor_id)
            .and_then(|state| state.cursor.char_range())
            .map(|cursor| {
                let range = cursor.as_sorted_char_range();
                usize::from(range.start)..usize::from(range.end)
            });
        let syntax = self.side_sources[side]
            .as_ref()
            .or_else(|| self.paths.as_ref().map(|paths| &paths[side]))
            .and_then(|path| path.extension())
            .is_some_and(|ext| {
                matches!(
                    ext.to_str(),
                    Some(
                        "rs" | "py"
                            | "js"
                            | "ts"
                            | "tsx"
                            | "jsx"
                            | "c"
                            | "cpp"
                            | "h"
                            | "cs"
                            | "java"
                            | "go"
                            | "json"
                            | "toml"
                    )
                )
            });
        let merge_controls = self.editing && self.has_actual_pair();
        let text_height = (height - 5.0).max(60.0);
        let content_height = (visible_lines.len().max(1) as f32 * row_height).max(row_height);
        if std::mem::take(&mut self.pending_navigation_scroll[side]) {
            // TextEdit paints inside ScrollArea, before its final offset clamp.
            // Bound jumps using current row/viewport geometry so an EOF jump
            // cannot paint at an impossible offset and snap back next frame.
            // Extra TextEdit padding can increase the actual range; manual
            // scrolling still uses that full range without this navigation cap.
            self.scroll[side] = self.scroll[side].min((content_height - text_height).max(0.0));
        }
        let mut pending_transfer = None;
        let output = egui::ScrollArea::vertical()
            .id_salt(("two-way-scroll", self.side_generation[side], side))
            .auto_shrink([false, false])
            .max_height(text_height)
            .vertical_scroll_offset(self.scroll[side])
            .show(ui, |ui| {
                let number_width = super::file_style::TEXT_OFFSET - 4.0 + gutter_extra;
                let gutter_width = if merge_controls { 32.0 } else { 0.0 };
                let width = ui.available_width().max(180.0);
                let text_width = (width - number_width - gutter_width - 12.0).max(80.0);
                let origin = ui.cursor().min;
                let gutter_x = if side == 0 {
                    origin.x + width - gutter_width
                } else {
                    origin.x
                };
                let numbers_x = if side == 0 {
                    origin.x
                } else {
                    origin.x + gutter_width
                };
                let text_x = numbers_x + number_width + 4.0;
                let text_rect = Rect::from_min_size(
                    egui::pos2(text_x, origin.y + text_padding),
                    // Short/empty files still fill the viewport so their
                    // horizontal scrollbar sits at the bottom of the pane.
                    egui::vec2(text_width, content_height.max(text_height - text_padding)),
                );
                let mut highlight_slots = Vec::new();
                // The editable surface still owns its TextEdit/history, but
                // while comparison assistance is present its row decoration is
                // deliberately the same as File Diff.
                for (display_line, &line) in visible_lines.iter().enumerate() {
                    let state = changed
                        .get(&line)
                        .map(|(state, _)| state)
                        .unwrap_or(&DirectoryEntryState::Same);
                    let row_rect = Rect::from_min_size(
                        egui::pos2(numbers_x, origin.y + display_line as f32 * row_height),
                        egui::vec2(width - gutter_width, row_height),
                    );
                    super::file_style::paint_background(
                        ui.painter(),
                        row_rect,
                        state,
                        display_line % 2 == 1,
                        navigation_line == Some(line),
                    );
                    super::file_style::paint_gutter(
                        ui.painter(),
                        Rect::from_min_size(egui::pos2(numbers_x, row_rect.top()), row_rect.size()),
                        line + 1,
                        state,
                        gutter_extra,
                    );
                    if let Some((state, ranges)) = changed.get(&line) {
                        let slot = ui.painter().add(egui::Shape::Noop);
                        highlight_slots.push((
                            slot,
                            display_line,
                            displayed_lines
                                .get(line)
                                .copied()
                                .unwrap_or_default()
                                .to_owned(),
                            ranges.clone(),
                            state.clone(),
                        ));
                    }
                }
                let mut text_scroll_x = 0.0;
                let mut text_clip = text_rect.intersect(ui.clip_rect());
                ui.scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
                    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _: f32| {
                        let mut job = egui::text::LayoutJob::default();
                        job.wrap.max_width = f32::INFINITY;
                        for (display_line, line) in
                            buffer.as_str().split_inclusive('\n').enumerate()
                        {
                            let index = visible_lines
                                .get(display_line)
                                .copied()
                                .unwrap_or(display_line);
                            if let Some((state, ranges)) = changed.get(&index) {
                                super::file_style::append_text_without_inline_background(
                                    &mut job,
                                    line,
                                    state,
                                    ranges,
                                    palette,
                                    font.clone(),
                                    Some(row_height),
                                );
                            } else if decorated {
                                super::file_style::append_text(
                                    &mut job,
                                    line,
                                    &DirectoryEntryState::Same,
                                    &[],
                                    palette,
                                    font.clone(),
                                    Some(row_height),
                                );
                            } else {
                                append_code_line(
                                    &mut job,
                                    line,
                                    egui::TextFormat {
                                        font_id: font.clone(),
                                        color: palette.text,
                                        line_height: Some(row_height),
                                        ..Default::default()
                                    },
                                    syntax,
                                    palette,
                                );
                            }
                        }
                        if job.text.is_empty() {
                            job.append(
                                "",
                                0.0,
                                egui::TextFormat {
                                    font_id: font.clone(),
                                    ..Default::default()
                                },
                            );
                        }
                        ui.fonts_mut(|fonts| fonts.layout_job(job))
                    };
                    let horizontal = egui::ScrollArea::horizontal()
                        .id_salt(("interactive-horizontal", self.side_generation[side], side))
                        .auto_shrink([false, false])
                        .max_width(text_width)
                        .show(ui, |ui| {
                            // Keep egui's native caret margins without including
                            // the extra spacing used by comparison rows.
                            ui.visuals_mut().text_cursor.max_height = Some(font_height + 3.0);
                            egui::TextEdit::multiline(&mut next)
                                .id(editor_id)
                                .font(font.clone())
                                .code_editor()
                                .interactive(self.editing)
                                .desired_width(text_width)
                                .desired_rows(1)
                                .min_size(egui::vec2(text_width, ui.available_height()))
                                .frame(egui::Frame::NONE)
                                .layouter(&mut layouter)
                                .show(ui)
                        });
                    text_scroll_x = horizontal.state.offset.x;
                    text_clip = horizontal.inner_rect.intersect(ui.clip_rect());
                    let highlight_clip = Rect::from_min_max(
                        egui::pos2(text_clip.left(), text_clip.top() - text_padding),
                        text_clip.max,
                    )
                    .intersect(ui.clip_rect());
                    let highlight_painter = ui.painter().with_clip_rect(highlight_clip);
                    for (slot, display_line, text, ranges, state) in highlight_slots.drain(..) {
                        let shapes = super::file_style::inline_highlight_rects(
                            &highlight_painter,
                            egui::pos2(
                                text_x - text_scroll_x,
                                origin.y + display_line as f32 * row_height,
                            ),
                            &text,
                            &ranges,
                            font.clone(),
                            row_height,
                        )
                        .into_iter()
                        .map(|rect| {
                            egui::Shape::rect_filled(
                                rect,
                                0.0,
                                state_color(&state, palette).gamma_multiply(0.3),
                            )
                        })
                        .collect();
                        highlight_painter.set(slot, egui::Shape::Vec(shapes));
                    }
                    let edit = horizontal.inner;
                    if edit.response.has_focus() {
                        self.selected_side = side;
                        if let Some(cursor) = edit.cursor_range {
                            let position: usize = cursor.primary.index.into();
                            let display_line =
                                next.chars().take(position).filter(|c| *c == '\n').count();
                            self.caret[side] = visible_lines
                                .get(display_line)
                                .copied()
                                .unwrap_or_else(|| visible_lines.last().map_or(0, |line| line + 1));
                        }
                    }
                });
                for (display_line, &line) in visible_lines.iter().enumerate() {
                    let y = origin.y + display_line as f32 * row_height;
                    if let Some(ending) = ending_differences.get(&line) {
                        let text = displayed_lines.get(line).copied().unwrap_or_default();
                        let mut ending_job = egui::text::LayoutJob::default();
                        let state = changed
                            .get(&line)
                            .map(|(state, _)| state)
                            .unwrap_or(&DirectoryEntryState::Same);
                        let ranges = changed
                            .get(&line)
                            .map(|(_, ranges)| ranges.as_slice())
                            .unwrap_or(&[]);
                        super::file_style::append_text(
                            &mut ending_job,
                            text,
                            state,
                            ranges,
                            palette,
                            font.clone(),
                            Some(row_height),
                        );
                        let end_x =
                            text_x - text_scroll_x + ui.painter().layout_job(ending_job).size().x;
                        super::file_style::paint_ending(
                            &ui.painter().with_clip_rect(text_clip),
                            Rect::from_min_size(
                                egui::pos2(origin.x, y),
                                egui::vec2(width, row_height),
                            ),
                            end_x,
                            *ending,
                            state,
                        );
                    }
                }
                if merge_controls {
                    for chunk in &chunks {
                        let runs = visible_chunk_runs(&chunk.range, &visible_lines);
                        if runs.is_empty() {
                            continue;
                        }
                        let expanded = self.expanded[side].contains(&chunk.id);
                        for run in &runs {
                            let lines: Vec<_> = if expanded {
                                run.clone().collect()
                            } else {
                                vec![run.start]
                            };
                            for line in lines {
                                let display_line = visible_lines
                                    .iter()
                                    .position(|visible| *visible == line)
                                    .unwrap();
                                let y = origin.y + display_line as f32 * row_height;
                                let track = track.expect("loaded merge pair has transfer tracks");
                                let track_clip = Rect::from_min_max(
                                    egui::pos2(track.left(), track.top().max(ui.clip_rect().top())),
                                    egui::pos2(
                                        track.right(),
                                        track.bottom().min(ui.clip_rect().bottom()),
                                    ),
                                );
                                let rect = Rect::from_min_size(
                                    egui::pos2(track.center().x - 15.0, y),
                                    egui::vec2(30.0, row_height),
                                );
                                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                                    ui.set_clip_rect(track_clip);
                                    let tooltip = if expanded {
                                        format!("Copy line {} at the other pane’s cursor", line + 1)
                                    } else {
                                        format!(
                                            "Copy chunk {}–{} at the other pane’s cursor",
                                            run.start + 1,
                                            run.end
                                        )
                                    };
                                    let response = transfer_action(
                                        ui,
                                        rect,
                                        egui::Id::new(("two-way-transfer", side, chunk.id, line)),
                                        side,
                                        &tooltip,
                                    );
                                    if response.clicked() {
                                        pending_transfer = Some(if expanded {
                                            line..line + 1
                                        } else {
                                            run.clone()
                                        });
                                    }
                                });
                            }
                        }
                        let first_visible = runs[0].start;
                        let first_display = visible_lines
                            .iter()
                            .position(|line| *line == first_visible)
                            .unwrap();
                        let toggle = Rect::from_min_size(
                            egui::pos2(
                                gutter_x + 5.0,
                                origin.y + first_display as f32 * row_height,
                            ),
                            egui::vec2(21.0, row_height),
                        );
                        ui.scope_builder(egui::UiBuilder::new().max_rect(toggle), |ui| {
                            if inline_action(
                                ui,
                                toggle,
                                egui::Id::new(("two-way-granularity", side, chunk.id)),
                                if expanded { "−" } else { "+" },
                                if expanded {
                                    "Return to chunk controls"
                                } else {
                                    "Show an arrow for each line"
                                },
                            )
                            .clicked()
                            {
                                if expanded {
                                    self.expanded[side].retain(|id| *id != chunk.id);
                                } else {
                                    self.expanded[side].push(chunk.id);
                                }
                            }
                        });
                    }
                }
                ui.allocate_rect(
                    Rect::from_min_size(origin, egui::vec2(width, content_height)),
                    Sense::hover(),
                );
            });
        self.scroll[side] = output.state.offset.y;
        let base_text = projection
            .as_ref()
            .map(MergeProjection::text)
            .unwrap_or(displayed.as_str());
        if next != base_text {
            let edit = projection.as_ref().map_or_else(
                || Ok(editor_byte_edit(&current, &next, previous_cursor.clone())),
                |projection| {
                    projected_editor_edit(&current, projection, &next, previous_cursor.clone())
                },
            );
            match edit {
                Ok((range, replacement)) => {
                    let caret_byte = range.start + replacement.len();
                    if let Err(error) = self.workspace.as_mut().unwrap().edit_bytes(
                        source,
                        range.clone(),
                        replacement,
                    ) {
                        self.error = Some(error.to_string());
                    } else {
                        self.error = None;
                        // The TextEdit buffer was a compact projection. Its
                        // character offsets are no longer valid after this
                        // source mutation; restore it from the exact source
                        // byte position on the next full render.
                        self.restore_editor_byte_cursor(side, caret_byte);
                        self.invalidate_edited_diff(side, &current, range);
                    }
                }
                Err(error) => self.error = Some(error.into()),
            }
            // The gutter geometry was rendered for the previous text.
            pending_transfer = None;
        }
        if let Some(range) = pending_transfer {
            self.transfer(source, range);
        }
    }
}
