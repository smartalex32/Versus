//! Lazy metadata navigation and independent folder-pane painting.

use super::*;
use crate::app::visuals::icons::paint_icon;
use crate::app::visuals::status::format_size;
use crate::app::visuals::status::{paint_status_icon, state_bar_color, state_color, status_icon};
use crate::app::visuals::theme::{Palette, comparison_font, comparison_row_height};

pub(super) struct ListingJob {
    receiver: Receiver<Result<Vec<LazyDirectoryItem>, String>>,
}

impl FolderBrowser {
    /// Reveals one relative path by loading only its ancestor directories.
    /// No file contents or unrelated tree levels are read.
    pub(in crate::app) fn reveal_difference(&mut self, relative: PathBuf) {
        self.pending_reveal = [Some(relative.clone()), Some(relative.clone())];
        self.selected = [Some(relative.clone()), Some(relative.clone())];
        let mut ancestors = Vec::new();
        let mut cursor = relative.parent();
        while let Some(parent) = cursor {
            if parent.as_os_str().is_empty() {
                break;
            }
            ancestors.push(parent.to_path_buf());
            cursor = parent.parent();
        }
        for ancestor in ancestors.into_iter().rev() {
            for side in 0..2 {
                self.expanded[side].insert(ancestor.clone());
                self.load_side(side, ancestor.clone());
            }
        }
    }
    pub(super) fn load_side(&mut self, side: usize, relative: PathBuf) {
        if self.sources[side].is_none()
            || self.jobs[side].contains_key(&relative)
            || self.listings[side].contains_key(&relative)
        {
            return;
        }
        let root = self.sources[side].clone().expect("checked root");
        let rel = relative.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = versus::directory_entries_for_root(&root, &rel).map_err(|e| e.to_string());
            let _ = sender.send(result);
        });
        self.jobs[side].insert(relative, ListingJob { receiver });
    }
    pub(in crate::app) fn poll(&mut self, ctx: &egui::Context) {
        for side in 0..2 {
            let ready: Vec<_> = self.jobs[side]
                .iter()
                .filter_map(|(path, job)| match job.receiver.try_recv() {
                    Ok(result) => Some((path.clone(), result)),
                    Err(TryRecvError::Disconnected) => {
                        Some((path.clone(), Err("Directory worker stopped".into())))
                    }
                    _ => None,
                })
                .collect();
            for (path, result) in ready {
                self.jobs[side].remove(&path);
                match result {
                    Ok(entries) => {
                        self.listings[side].insert(path, entries);
                    }
                    Err(e) => self.error = Some(e),
                }
            }
        }
        // Search-state owns bounded streaming and cancellation. Navigation
        // only receives an error suitable for the shared activity area.
        if let Some(error) = self.search_state.poll() {
            self.error = Some(error);
        }
        if self.jobs.iter().any(|jobs| !jobs.is_empty())
            || self.search_state.is_searching()
            || self.save.is_some()
            || self.overlay_job.is_some()
            || !self.retired_overlay_jobs.is_empty()
            || self.overlay_not_before.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(40));
        }
    }
}

impl FolderBrowser {
    /// Renders one independently navigable source tree and returns a file
    /// opened from that side. Missing sources intentionally render nothing.
    pub(in crate::app) fn render_side(
        &mut self,
        ui: &mut egui::Ui,
        view: &mut InteractiveView,
        compared: Option<&FolderTree>,
        side: usize,
    ) -> Option<PathBuf> {
        let root = self.sources[side].clone()?;
        let palette = Palette::for_context(ui.ctx());
        let states = if view.run_diff_enabled() && self.paired_roots().is_some() {
            self.merged_folder_cache(compared).states.clone()
        } else {
            Arc::new(BTreeMap::new())
        };
        let mut rows = Vec::new();
        self.rows_for_side(side, std::path::Path::new(""), 0, &mut rows);
        if view.run_diff_enabled() && view.merge_options()[0] && self.paired_roots().is_some() {
            rows.retain(|(entry, _)| {
                states.get(&entry.relative_path) != Some(&DirectoryEntryState::Same)
            });
        }
        let reveal_index = self.pending_reveal[side].as_ref().and_then(|target| {
            rows.iter()
                .position(|(entry, _)| &entry.relative_path == target)
        });
        if let Some(index) = reveal_index {
            self.selected[side] = Some(rows[index].0.relative_path.clone());
            self.pending_reveal[side] = None;
        }
        let available = ui.available_height().max(80.0);
        let row_height = comparison_row_height(ui);
        let mut opened = None;
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("merge-folder-tree", side, self.source_generation[side]))
            .max_height(available - 5.0);
        if let Some(index) = reveal_index {
            scroll = scroll.vertical_scroll_offset(index.saturating_sub(1) as f32 * row_height);
        }
        scroll.show_rows(ui, row_height, rows.len(), |ui, range| {
            for index in range {
                let (entry, depth) = &rows[index];
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), row_height),
                    Sense::click(),
                );
                let directory = entry.side.kind == DirectoryEntryKind::Directory;
                let file = entry.side.kind == DirectoryEntryKind::File;
                let response = ui.interact(
                    rect,
                    ui.id().with(("merge-entry", side, &entry.relative_path)),
                    Sense::click(),
                );
                let response = if directory || file {
                    response.on_hover_cursor(egui::CursorIcon::PointingHand)
                } else {
                    response
                };
                let state = states
                    .get(&entry.relative_path)
                    .cloned()
                    .unwrap_or(DirectoryEntryState::Same);
                let selected = self.selected[side].as_ref() == Some(&entry.relative_path);
                let fill = if selected {
                    palette.accent.gamma_multiply(0.16)
                } else if response.hovered() {
                    palette.hover
                } else if index % 2 == 1 {
                    palette.alternate
                } else {
                    palette.panel
                };
                let painter = ui.painter().with_clip_rect(rect);
                painter.rect_filled(rect, 0, fill);
                let color = state_color(&state, palette);
                if state != DirectoryEntryState::Same {
                    painter.rect_filled(
                        Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
                        0,
                        state_bar_color(&state, palette, painter.ctx()),
                    );
                }
                let x = rect.left() + 8.0 + *depth as f32 * 14.0;
                let y = rect.center().y;
                let name_right = rect.right() - 98.0;
                let name_painter = painter.with_clip_rect(Rect::from_min_max(
                    rect.min,
                    egui::pos2(name_right.max(rect.left()), rect.bottom()),
                ));
                if directory {
                    name_painter.text(
                        egui::pos2(x, y),
                        Align2::LEFT_CENTER,
                        if self.expanded[side].contains(&entry.relative_path) {
                            "▾"
                        } else {
                            "▸"
                        },
                        FontId::monospace(15.0),
                        palette.muted,
                    );
                }
                paint_icon(
                    &name_painter,
                    egui::pos2(x + 19.0, y),
                    Some(&entry.side.kind),
                    color,
                );
                name_painter.text(
                    egui::pos2(x + 40.0, y),
                    Align2::LEFT_CENTER,
                    entry
                        .relative_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                    comparison_font(ui.ctx()),
                    color,
                );
                painter.text(
                    egui::pos2(rect.right() - 32.0, y),
                    Align2::RIGHT_CENTER,
                    format_size(entry.side.size),
                    FontId::monospace(10.0),
                    palette.muted,
                );
                if let Some(icon) = status_icon(&state) {
                    paint_status_icon(&painter, egui::pos2(rect.right() - 15.0, y), icon);
                }
                if response.clicked() {
                    if file {
                        self.selected[side] = Some(entry.relative_path.clone());
                    } else if directory {
                        if !self.expanded[side].insert(entry.relative_path.clone()) {
                            self.expanded[side].remove(&entry.relative_path);
                        } else {
                            self.load_side(side, entry.relative_path.clone());
                        }
                    }
                }
                if response.double_clicked() && file {
                    opened = Some(root.join(&entry.relative_path));
                }
            }
        });
        opened
    }
    fn rows_for_side(
        &self,
        side: usize,
        parent: &std::path::Path,
        depth: usize,
        output: &mut Vec<(LazyDirectoryItem, usize)>,
    ) {
        if let Some(entries) = self.listings[side].get(parent) {
            for entry in entries {
                output.push((entry.clone(), depth));
                if self.expanded[side].contains(&entry.relative_path) {
                    self.rows_for_side(side, &entry.relative_path, depth + 1, output);
                }
            }
        }
    }
    pub(in crate::app) fn render(
        &mut self,
        ui: &mut egui::Ui,
        view: &mut InteractiveView,
        compared: Option<&FolderTree>,
    ) -> Option<[PathBuf; 2]> {
        self.render_controls(ui, view);
        let mut opened = [None, None];
        ui.columns(2, |columns| {
            for side in 0..2 {
                opened[side] = self.render_side(&mut columns[side], view, compared, side);
            }
        });
        match opened {
            [Some(left), Some(right)] => Some([left, right]),
            [Some(left), None] => self.paired_roots().map(|roots| {
                let relative = left.strip_prefix(&roots[0]).unwrap_or(&left).to_path_buf();
                [left, roots[1].join(relative)]
            }),
            [None, Some(right)] => self.paired_roots().map(|roots| {
                let relative = right
                    .strip_prefix(&roots[1])
                    .unwrap_or(&right)
                    .to_path_buf();
                [roots[0].join(relative), right]
            }),
            _ => None,
        }
    }
}
