//! Native drag/drop pane targeting and empty-source affordances.

use super::*;
use crate::app::visuals::status::side_name;
use crate::app::visuals::theme::Palette;

impl VersusApp {
    pub(super) fn drag_and_drop(&mut self, ui: &mut egui::Ui) {
        let (hovering, position, dropped) = native_drop_frame(ui, self.native_drag_position.take());
        let side =
            position.and_then(|pos| self.pane_rects.iter().position(|rect| rect.contains(pos)));
        if hovering {
            // OLE drags can suppress ordinary mouse motion. Keep polling the
            // native cursor so the highlight follows it even without events.
            if cfg!(windows) {
                ui.ctx().request_repaint_after(Duration::from_millis(33));
            }
            if position.is_some() {
                self.drop_hover_side = side;
            }
            if let Some(side) = side.or(self.drop_hover_side) {
                paint_native_drop_highlight(
                    ui.ctx(),
                    self.pane_rects[side],
                    &format!(
                        "Drop a file or folder on {}",
                        if side == 0 { "LEFT" } else { "RIGHT" }
                    ),
                );
            }
        }
        if !dropped.is_empty() {
            let target = if position.is_some() {
                side
            } else {
                self.drop_hover_side
            };
            if dropped.len() == 1 && !dropped[0].path().as_os_str().is_empty() && target.is_some() {
                let side = target.unwrap();
                let path = dropped[0].path().to_path_buf();
                if self.session.merge_file_open && !self.session.merge_mode {
                    if let Some(view) = &mut self.session.interactive {
                        if let Some(paths) = view.pair_paths() {
                            let mut paths = paths.clone();
                            paths[side] = absolute_native_path(path);
                            view.open_pair(paths);
                            ui.ctx().request_repaint();
                            return;
                        }
                    }
                }
                // A drop in a drilled-down file view retains the other displayed
                // file, rather than accidentally pairing it with the folder root.
                if let Some(view) = self
                    .session
                    .file_view
                    .as_ref()
                    .filter(|view| view.from_folders)
                {
                    let peer = view.sources[1 - side].clone();
                    self.session.new_comparison();
                    if let Some(peer) = peer {
                        self.session.select_path(1 - side, peer);
                    }
                }
                self.session.select_path(side, path);
                self.session.launch_mode = Some(None);
            } else {
                self.session.drop_message = Some(if dropped.len() != 1 {
                    "Drop one file or folder at a time.".into()
                } else if dropped[0].path().as_os_str().is_empty() {
                    "This drag did not include a file or folder path. Use Browse to select it."
                        .into()
                } else {
                    "Drop into the large LEFT or RIGHT view below the path bars.".into()
                });
            }
            ui.ctx().request_repaint();
        }
        if !hovering {
            self.drop_hover_side = None;
        }
    }
}
// Shared by compare and merge so native targeting and feedback stay consistent.
pub(in crate::app) fn native_drop_frame(
    ui: &egui::Ui,
    native_position: Option<Pos2>,
) -> (bool, Option<Pos2>, Vec<egui::DroppedFileHandle>) {
    let (hovering, frame_position, retained_position) = ui.input(|input| {
        (
            !input.raw.hovered_files.is_empty(),
            input.raw.events.iter().rev().find_map(|event| match event {
                egui::Event::PointerMoved(position) => Some(*position),
                _ => None,
            }),
            input.pointer.interact_pos(),
        )
    });
    let position = frame_position.or(native_position).or(retained_position);
    let dropped = ui
        .ctx()
        .input_mut(|input| std::mem::take(&mut input.raw.dropped_files));
    (hovering, position, dropped)
}

pub(in crate::app) fn paint_drop_outline(painter: &egui::Painter, target: Rect, color: Color32) {
    let corners = [
        target.left_top(),
        target.right_top(),
        target.right_bottom(),
        target.left_bottom(),
        target.left_top(),
    ];
    painter.extend(egui::Shape::dashed_line(
        &corners,
        Stroke::new(1.0, color),
        6.0,
        4.0,
    ));
}

pub(in crate::app) fn paint_native_drop_highlight(ctx: &egui::Context, pane: Rect, message: &str) {
    let rect = pane.shrink(3.0);
    let palette = Palette::for_context(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("native-drop-highlight"),
    ));
    painter.rect_filled(rect, 4, palette.accent.gamma_multiply(0.1));
    painter.rect_stroke(
        rect,
        4,
        Stroke::new(3.0, palette.accent),
        egui::StrokeKind::Inside,
    );
    let label =
        painter.layout_no_wrap(message.to_owned(), FontId::proportional(16.0), palette.text);
    let label_rect = Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + 64.0),
        label.size() + egui::vec2(24.0, 16.0),
    );
    painter.rect_filled(label_rect, 4, palette.panel);
    painter.galley(
        label_rect.center() - label.size() * 0.5,
        label,
        palette.text,
    );
}

pub(in crate::app) fn paint_empty_drop_target(
    ui: &mut egui::Ui,
    rect: Rect,
    side: usize,
    chooser: bool,
) {
    let palette = Palette::for_context(ui.ctx());
    let painter = ui.painter().with_clip_rect(rect);
    let target = rect.shrink(8.0);
    paint_drop_outline(&painter, target, palette.border);
    ui.interact(
        target,
        ui.id().with(("empty-drop-target", side)),
        Sense::hover(),
    )
    .widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Label,
            true,
            format!(
                "{}: Drop a file or folder here, or use Browse above.",
                side_name(side)
            ),
        )
    });
    painter.text(
        target.center() - egui::vec2(0.0, if chooser { 12.0 } else { -24.0 }),
        Align2::CENTER_CENTER,
        "Drop a file or folder here",
        FontId::proportional(14.0),
        palette.muted,
    );
    painter.text(
        target.center() + egui::vec2(0.0, if chooser { 12.0 } else { 48.0 }),
        Align2::CENTER_CENTER,
        "or use Browse above",
        FontId::proportional(11.0),
        palette.muted,
    );
}
