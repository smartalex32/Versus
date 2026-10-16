//! Aligned folder tree presentation, input handling and row painting.

use super::*;
use crate::app::visuals::icons::{ToolbarIcon, icon_button, paint_icon};
use crate::app::visuals::panels::{comparison_panel, empty_display, split_rect};
use crate::app::visuals::status::format_size;
use crate::app::visuals::status::{
    paint_status_icon, state_bar_color, state_color, state_label, status_icon,
    tree_row_accessible_label,
};
use crate::app::visuals::theme::{Palette, comparison_font};

impl VersusApp {
    pub(super) fn controls(&mut self, ui: &mut egui::Ui) {
        if icon_button(
            ui,
            ToolbarIcon::Collapse,
            self.session.tree.is_some(),
            "Collapse all",
        )
        .clicked()
        {
            self.session.tree.as_mut().unwrap().collapse_all();
        }
        if icon_button(
            ui,
            ToolbarIcon::Expand,
            self.session.tree.is_some(),
            "Expand all",
        )
        .clicked()
        {
            self.session.tree.as_mut().unwrap().expand_all();
        }
        let ready = self
            .session
            .paths
            .iter()
            .all(|path| !path.trim().is_empty())
            && self.session.source_jobs.iter().all(Option::is_none)
            && self.session.source_errors.iter().all(Option::is_none);
        if icon_button(
            ui,
            ToolbarIcon::Refresh,
            ready && self.session.job.is_none(),
            if self.session.tree.is_some() {
                "Refresh comparison"
            } else {
                "Compare folders"
            },
        )
        .clicked()
        {
            self.session.start_comparison();
        }
        if let Some(job) = &self.session.job {
            let cancelling = job.cancellation.load(Ordering::Relaxed);
            if icon_button(
                ui,
                ToolbarIcon::Cancel,
                !cancelling,
                if cancelling {
                    "Cancelling…"
                } else {
                    "Cancel comparison"
                },
            )
            .clicked()
            {
                job.cancellation.store(true, Ordering::Relaxed);
            }
            ui.spinner();
        }
    }

    pub(super) fn tree_area(&mut self, ui: &mut egui::Ui, height: f32) {
        let palette = Palette::for_context(ui.ctx());
        let row_height = self.session.row_height;
        let mut open = None;
        comparison_panel(ui).show(ui, |ui| {
            ui.set_min_height(height);
            ui.set_max_height(height);
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let header_top = ui.cursor().top();
            self.folder_inputs(ui);
            let header_height = ui.cursor().top() - header_top;
            let (divider, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter().line_segment(
                [divider.left_center(), divider.right_center()],
                Stroke::new(1.0, palette.border),
            );
            let body_height = (height - header_height - 2.0).max(60.0);
            if let Some(tree) = &mut self.session.tree {
                let rows: Vec<_> = tree
                    .visible_rows()
                    .into_iter()
                    .filter(|row| {
                        !self.session.show_only_differences
                            || row.node.state != DirectoryEntryState::Same
                    })
                    .collect();
                if rows.is_empty() {
                    empty_display(
                        ui,
                        body_height,
                        if self.session.show_only_differences {
                            "No differences"
                        } else {
                            "Both folders are empty"
                        },
                        if self.session.show_only_differences {
                            "No files or folders differ with the current options."
                        } else {
                            "There are no files or folders to compare."
                        },
                    );
                    return;
                }
                let mut toggle = None;
                let mut selected = None;
                let mut tree_keyboard_focus = self.session.tree_keyboard_focus.clone();
                // A target is only retained between the Arrow-key frame and
                // the frame where virtual scrolling renders that row. Keep
                // its side beside the path so focus does not jump panes.
                let pending_side_id = ui.id().with("tree-keyboard-focus-side");
                let mut pending_side =
                    ui.memory(|memory| memory.data.get_temp::<usize>(pending_side_id));
                let mut rendered_tree_focus = None;
                self.session.tree_focus_id = None;
                // show_rows paints before ScrollArea clamps its offset. Clamp
                // first so a jump near the end is correct on its first frame.
                self.session.tree_scroll_y = self
                    .session
                    .tree_scroll_y
                    .clamp(0.0, (rows.len() as f32 * row_height - body_height).max(0.0));
                let output = egui::ScrollArea::vertical()
                    .id_salt(("linked-trees", self.session.scroll_generation))
                    .auto_shrink([false, false])
                    .max_height(body_height)
                    .vertical_scroll_offset(self.session.tree_scroll_y)
                    .show_rows(ui, row_height, rows.len(), |ui, range| {
                        for index in range {
                            let row = &rows[index];
                            let node = row.node;
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), row_height),
                                Sense::hover(),
                            );
                            let is_selected =
                                self.session.selected.as_ref() == Some(&node.relative_path);
                            for (side, half) in split_rect(rect).into_iter().enumerate() {
                                let entry = if side == 0 { &node.left } else { &node.right };
                                let present = entry.exists;
                                let response = ui.interact(
                                    half,
                                    ui.id().with((&node.relative_path, side)),
                                    Sense::click(),
                                );
                                let response = if present {
                                    response.on_hover_cursor(egui::CursorIcon::PointingHand)
                                } else {
                                    response
                                };
                                if tree_keyboard_focus.as_ref() == Some(&node.relative_path)
                                    && pending_side == Some(side)
                                {
                                    response.request_focus();
                                    tree_keyboard_focus = None;
                                    pending_side = None;
                                    ui.memory_mut(|memory| {
                                        memory.data.remove::<usize>(pending_side_id);
                                    });
                                }
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::WidgetType::SelectableLabel,
                                        true,
                                        is_selected,
                                        if node.is_expandable() {
                                            format!(
                                                "{}, {}",
                                                tree_row_accessible_label(node, side),
                                                if tree.is_expanded(&node.relative_path) {
                                                    "expanded"
                                                } else {
                                                    "collapsed"
                                                }
                                            )
                                        } else {
                                            tree_row_accessible_label(node, side)
                                        },
                                    )
                                });
                                paint_row(
                                    ui,
                                    half,
                                    node,
                                    side,
                                    RowAppearance {
                                        depth: row.depth,
                                        expanded: tree.is_expanded(&node.relative_path),
                                        alternate: index % 2 == 1,
                                        selected: is_selected,
                                        hovered: response.hovered() || response.has_focus(),
                                    },
                                );
                                if response.has_focus() {
                                    self.session.tree_focus_id = Some(response.id);
                                    ui.memory_mut(|memory| {
                                        memory.set_focus_lock_filter(
                                            response.id,
                                            egui::EventFilter {
                                                horizontal_arrows: true,
                                                vertical_arrows: true,
                                                ..Default::default()
                                            },
                                        )
                                    });
                                    rendered_tree_focus = Some((node.relative_path.clone(), side));
                                    ui.painter().rect_stroke(
                                        half.shrink(1.0),
                                        3.0,
                                        Stroke::new(2.0, palette.accent),
                                        egui::StrokeKind::Inside,
                                    );
                                }
                                let expandable = node.is_expandable()
                                    && entry.kind == Some(DirectoryEntryKind::Directory);
                                let keyboard_toggle = response.has_focus()
                                    && ui.input(|input| {
                                        input.key_pressed(egui::Key::Space)
                                            || input.key_pressed(egui::Key::Enter)
                                    });
                                if (response.double_clicked() || keyboard_toggle)
                                    && entry.kind == Some(DirectoryEntryKind::File)
                                {
                                    open = Some((
                                        node.relative_path.clone(),
                                        [node.left.exists, node.right.exists],
                                        [&node.left, &node.right].into_iter().any(|entry| {
                                            entry.exists
                                                && entry.kind != Some(DirectoryEntryKind::File)
                                        }),
                                    ));
                                }
                                if response.clicked() || keyboard_toggle {
                                    response.request_focus();
                                    self.session.tree_focus_id = Some(response.id);
                                    selected = Some(node.relative_path.clone());
                                    if expandable {
                                        toggle = Some(node.relative_path.clone());
                                    }
                                }
                                if response.has_focus() && expandable {
                                    let expanded = tree.is_expanded(&node.relative_path);
                                    if ui.input(|input| {
                                        (input.key_pressed(egui::Key::ArrowRight) && !expanded)
                                            || (input.key_pressed(egui::Key::ArrowLeft) && expanded)
                                    }) {
                                        toggle = Some(node.relative_path.clone());
                                    }
                                }
                                response.on_hover_ui(|ui| {
                                    ui.monospace(node.relative_path.display().to_string());
                                    ui.label(if present {
                                        state_label(&node.state)
                                    } else {
                                        "Not present on this side"
                                    });
                                    if let Some(roots) = &self.session.roots {
                                        ui.monospace(
                                            roots[side]
                                                .join(&node.relative_path)
                                                .display()
                                                .to_string(),
                                        );
                                    }
                                    if let DirectoryEntryState::Error(error) = &node.state {
                                        ui.colored_label(palette.error, error.to_string());
                                    }
                                });
                            }
                            ui.painter().line_segment(
                                [rect.center_top(), rect.center_bottom()],
                                Stroke::new(1.0, palette.border),
                            );
                        }
                    });
                self.session.tree_scroll_y = output.state.offset.y;
                if let Some((current_path, side)) = rendered_tree_focus {
                    let step = ui.input(|input| {
                        if input.key_pressed(egui::Key::ArrowDown) {
                            1_i32
                        } else if input.key_pressed(egui::Key::ArrowUp) {
                            -1
                        } else {
                            0
                        }
                    });
                    if step != 0 {
                        let current = rows
                            .iter()
                            .position(|row| row.node.relative_path == current_path)
                            .unwrap_or(0);
                        let next = (current as i32 + step)
                            .clamp(0, rows.len().saturating_sub(1) as i32)
                            as usize;
                        let path = rows[next].node.relative_path.clone();
                        tree_keyboard_focus = Some(path.clone());
                        ui.memory_mut(|memory| {
                            memory.data.insert_temp(pending_side_id, side);
                        });
                        selected = Some(path);
                        let target_top = next as f32 * row_height;
                        let target_bottom = target_top + row_height;
                        let viewport_height = output.inner_rect.height();
                        self.session.tree_scroll_y = if target_top < self.session.tree_scroll_y {
                            target_top
                        } else if target_bottom > self.session.tree_scroll_y + viewport_height {
                            target_bottom - viewport_height
                        } else {
                            self.session.tree_scroll_y
                        }
                        .clamp(0.0, (rows.len() as f32 * row_height - body_height).max(0.0));
                        ui.ctx().request_repaint();
                    }
                }
                if let Some(path) = toggle {
                    tree.toggle_expanded(path);
                }
                if let Some(path) = selected {
                    self.session.selected = Some(path);
                }
                self.session.tree_keyboard_focus = tree_keyboard_focus;
            } else if self.session.paths.iter().all(String::is_empty)
                && (!self.session.recent.entries().is_empty()
                    || self.session.recent_message.is_some())
            {
                self.recent_area(ui, body_height);
            } else {
                let title = if self.session.job.is_some() {
                    "Comparing folders…"
                } else if self.session.source_jobs.iter().any(Option::is_some) {
                    "Opening selected source…"
                } else if self.session.mode() == SelectionMode::Incompatible {
                    "Incompatible source types"
                } else if self.session.error.is_some() {
                    "Cannot open selected source"
                } else if self.session.mode() == SelectionMode::Folder {
                    "Choose another folder"
                } else {
                    match self.session.comparison_mode {
                        ComparisonMode::Folder => "Choose folders to compare",
                        ComparisonMode::File => "Choose files to compare",
                    }
                };
                empty_display(ui, body_height, title, "");
            }
        });
        if let Some((path, present, type_mismatch)) = open {
            self.session.open_file(path, present, type_mismatch);
        }
    }
}
pub(in crate::app) struct RowAppearance {
    pub(in crate::app) depth: usize,
    pub(in crate::app) expanded: bool,
    pub(in crate::app) alternate: bool,
    pub(in crate::app) selected: bool,
    pub(in crate::app) hovered: bool,
}

pub(in crate::app) fn paint_row(
    ui: &egui::Ui,
    rect: Rect,
    node: &TreeNode,
    side: usize,
    appearance: RowAppearance,
) {
    let palette = Palette::for_context(ui.ctx());
    let entry = if side == 0 { &node.left } else { &node.right };
    let kind = entry.kind.as_ref();
    let present = entry.exists;
    let state = &node.state;
    let RowAppearance {
        depth,
        expanded,
        alternate,
        selected,
        hovered,
    } = appearance;
    let painter = ui.painter().with_clip_rect(rect);
    let color = state_color(state, palette);
    let fill = if selected {
        palette.accent.gamma_multiply(0.16)
    } else if hovered {
        palette.hover
    } else if alternate {
        palette.alternate
    } else {
        palette.panel
    };
    painter.rect_filled(rect, 0, fill);
    if present && *state != DirectoryEntryState::Same {
        painter.rect_filled(
            Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
            0,
            state_bar_color(state, palette, painter.ctx()),
        );
    }
    let x = rect.left() + 8.0 + depth as f32 * 14.0;
    let y = rect.center().y;
    let name_right = rect.right() - 98.0;
    if !present {
        painter.text(
            egui::pos2(x + 36.0, y),
            Align2::LEFT_CENTER,
            "—",
            comparison_font(ui.ctx()),
            palette.muted.gamma_multiply(0.55),
        );
        return;
    }
    let name_painter = painter.with_clip_rect(Rect::from_min_max(
        rect.min,
        egui::pos2(name_right.max(rect.left()), rect.bottom()),
    ));
    if kind == Some(&DirectoryEntryKind::Directory) && node.is_expandable() {
        name_painter.text(
            egui::pos2(x, y),
            Align2::LEFT_CENTER,
            if expanded { "▾" } else { "▸" },
            FontId::monospace(15.0),
            palette.muted,
        );
    }
    paint_icon(&name_painter, egui::pos2(x + 19.0, y), kind, color);
    name_painter.text(
        egui::pos2(x + 40.0, y),
        Align2::LEFT_CENTER,
        node.name.to_string_lossy(),
        comparison_font(ui.ctx()),
        color,
    );
    painter.text(
        egui::pos2(rect.right() - 32.0, y),
        Align2::RIGHT_CENTER,
        format_size(entry.size),
        FontId::monospace(10.0),
        palette.muted,
    );
    if let Some(icon) = status_icon(state) {
        paint_status_icon(&painter, egui::pos2(rect.right() - 15.0, y), icon);
    }
}
