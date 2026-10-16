//! Shared icons visual vocabulary for Diff and Merge.

use crate::app::visuals::theme::{Palette, control_extent};
use crate::app::*;

#[derive(Clone, Copy)]
pub(in crate::app) enum ToolbarIcon {
    AcceptAllLeft,
    AcceptAllRight,
    Undo,
    Redo,
    ZoomIn,
    ZoomOut,
    Search,
    MatchCase,
    MatchWholeWord,
    UseRegularExpression,
    ReplaceNext,
    ReplaceAll,
    Save,
    New,
    Back,
    Refresh,
    Expand,
    Collapse,
    Browse,
    Cancel,
    Sun,
    Moon,
    Differences,
    Whitespace,
    LineEndings,
    Previous,
    Next,
}

pub(in crate::app) fn icon_button(
    ui: &mut egui::Ui,
    icon: ToolbarIcon,
    enabled: bool,
    label: &str,
) -> egui::Response {
    icon_button_state(ui, icon, enabled, false, label)
}

pub(in crate::app) fn icon_button_state(
    ui: &mut egui::Ui,
    icon: ToolbarIcon,
    enabled: bool,
    selected: bool,
    label: &str,
) -> egui::Response {
    icon_button_content(ui, icon, enabled, selected, label, None)
}

pub(in crate::app) fn captioned_icon_width(ui: &egui::Ui, label: &str) -> f32 {
    let text =
        ui.painter()
            .layout_no_wrap(label.into(), FontId::proportional(11.0), Color32::WHITE);
    control_extent(ui) + ui.spacing().item_spacing.x + text.size().x + ui.spacing().button_padding.x
}

pub(in crate::app) fn captioned_icon_button(
    ui: &mut egui::Ui,
    icon: ToolbarIcon,
    enabled: bool,
    label: &str,
) -> egui::Response {
    icon_button_content(ui, icon, enabled, false, label, Some(label))
}

pub(in crate::app) fn icon_button_content(
    ui: &mut egui::Ui,
    icon: ToolbarIcon,
    enabled: bool,
    selected: bool,
    label: &str,
    caption: Option<&str>,
) -> egui::Response {
    let palette = Palette::for_context(ui.ctx());
    ui.add_enabled_ui(enabled, |ui| {
        let extent = control_extent(ui);
        let width = caption.map_or(extent, |text| captioned_icon_width(ui, text));
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, extent), Sense::hover());
        let response = ui.interact(rect, egui::Id::new(label), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, label)
        });
        let color = if !enabled {
            palette.muted.gamma_multiply(0.4)
        } else if selected || response.hovered() || response.has_focus() {
            palette.accent
        } else {
            palette.text
        };
        if response.is_pointer_button_down_on() {
            ui.painter()
                .rect_filled(rect, 3, palette.accent.gamma_multiply(0.35));
        } else if response.hovered() || response.has_focus() {
            ui.painter().rect_filled(rect, 3, palette.border);
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                3,
                Stroke::new(2.0, palette.accent),
                egui::StrokeKind::Inside,
            );
        }
        if selected {
            ui.painter()
                .rect_filled(rect, 3, palette.accent.gamma_multiply(0.15));
            ui.painter().rect_stroke(
                rect,
                3,
                Stroke::new(1.0, palette.accent),
                egui::StrokeKind::Inside,
            );
        }
        let center = egui::pos2(rect.left() + extent / 2.0, rect.center().y);
        let point = |x, y| center + egui::vec2(x, y);
        let stroke = Stroke::new(1.4, color);
        match icon {
            ToolbarIcon::AcceptAllLeft | ToolbarIcon::AcceptAllRight => {
                let direction = if matches!(icon, ToolbarIcon::AcceptAllLeft) {
                    1.0
                } else {
                    -1.0
                };
                for y in [-3.0, 3.0] {
                    ui.painter().line_segment(
                        [point(-6.0 * direction, y), point(5.0 * direction, y)],
                        stroke,
                    );
                    ui.painter().line(
                        vec![
                            point(direction, y - 3.0),
                            point(5.0 * direction, y),
                            point(direction, y + 3.0),
                        ],
                        stroke,
                    );
                }
            }
            ToolbarIcon::Redo => {
                ui.painter().line(
                    vec![
                        point(6.0, -3.0),
                        point(-1.0, -3.0),
                        point(-5.0, 0.0),
                        point(-5.0, 4.0),
                        point(0.0, 6.0),
                    ],
                    stroke,
                );
                ui.painter().line(
                    vec![point(2.0, -7.0), point(6.0, -3.0), point(2.0, 1.0)],
                    stroke,
                );
            }
            ToolbarIcon::Undo => {
                ui.painter().line(
                    vec![
                        point(-6.0, -3.0),
                        point(1.0, -3.0),
                        point(5.0, 0.0),
                        point(5.0, 4.0),
                        point(0.0, 6.0),
                    ],
                    stroke,
                );
                ui.painter().line(
                    vec![point(-2.0, -7.0), point(-6.0, -3.0), point(-2.0, 1.0)],
                    stroke,
                );
            }
            ToolbarIcon::MatchCase
            | ToolbarIcon::MatchWholeWord
            | ToolbarIcon::UseRegularExpression
            | ToolbarIcon::ReplaceNext
            | ToolbarIcon::ReplaceAll => {
                let glyph = match icon {
                    ToolbarIcon::MatchCase => find_icons::FindIcon::MatchCase,
                    ToolbarIcon::MatchWholeWord => find_icons::FindIcon::MatchWholeWord,
                    ToolbarIcon::UseRegularExpression => find_icons::FindIcon::UseRegularExpression,
                    ToolbarIcon::ReplaceNext => find_icons::FindIcon::ReplaceNext,
                    ToolbarIcon::ReplaceAll => find_icons::FindIcon::ReplaceAll,
                    _ => unreachable!(),
                };
                find_icons::paint(ui, center, glyph, color);
            }
            ToolbarIcon::Save => {
                let rect = Rect::from_center_size(center, egui::vec2(14.0, 14.0));
                ui.painter()
                    .rect_stroke(rect, 1.0, stroke, egui::StrokeKind::Inside);
                ui.painter().rect_stroke(
                    Rect::from_min_max(
                        rect.min + egui::vec2(3.0, 0.0),
                        rect.min + egui::vec2(10.0, 5.0),
                    ),
                    0.0,
                    stroke,
                    egui::StrokeKind::Inside,
                );
                ui.painter().rect_stroke(
                    Rect::from_min_max(
                        rect.min + egui::vec2(3.0, 8.0),
                        rect.max - egui::vec2(3.0, 0.0),
                    ),
                    0.0,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
            ToolbarIcon::ZoomIn | ToolbarIcon::ZoomOut | ToolbarIcon::Search => {
                let lens = point(-2.0, -2.0);
                ui.painter().circle_stroke(lens, 5.5, stroke);
                ui.painter()
                    .line_segment([point(2.0, 2.0), point(7.0, 7.0)], stroke);
                if !matches!(icon, ToolbarIcon::Search) {
                    ui.painter()
                        .line_segment([point(-5.0, -2.0), point(1.0, -2.0)], stroke);
                }
                if matches!(icon, ToolbarIcon::ZoomIn) {
                    ui.painter()
                        .line_segment([point(-2.0, -5.0), point(-2.0, 1.0)], stroke);
                }
            }
            ToolbarIcon::Differences => {
                for y in [-4.0, 4.0] {
                    ui.painter()
                        .line_segment([point(-6.0, y), point(6.0, y)], stroke);
                }
                ui.painter()
                    .line_segment([point(3.0, -8.0), point(-3.0, 8.0)], stroke);
            }
            ToolbarIcon::Whitespace => {
                ui.painter().line(
                    vec![
                        point(-7.0, -2.0),
                        point(-7.0, 5.0),
                        point(7.0, 5.0),
                        point(7.0, -2.0),
                    ],
                    stroke,
                );
                ui.painter().circle_filled(point(0.0, -3.0), 1.3, color);
            }
            ToolbarIcon::LineEndings => {
                ui.painter().line(
                    vec![point(6.0, -6.0), point(6.0, 3.0), point(-6.0, 3.0)],
                    stroke,
                );
                ui.painter().line(
                    vec![point(-1.0, -2.0), point(-6.0, 3.0), point(-1.0, 8.0)],
                    stroke,
                );
            }
            ToolbarIcon::Previous | ToolbarIcon::Next => {
                let direction = if matches!(icon, ToolbarIcon::Next) {
                    1.0
                } else {
                    -1.0
                };
                ui.painter().line_segment(
                    [point(0.0, -7.0 * direction), point(0.0, 7.0 * direction)],
                    stroke,
                );
                ui.painter().line(
                    vec![
                        point(-5.0, 2.0 * direction),
                        point(0.0, 7.0 * direction),
                        point(5.0, 2.0 * direction),
                    ],
                    stroke,
                );
            }
            ToolbarIcon::New => {
                ui.painter().line_segment(
                    [center - egui::vec2(6.0, 0.0), center + egui::vec2(6.0, 0.0)],
                    stroke,
                );
                ui.painter().line_segment(
                    [center - egui::vec2(0.0, 6.0), center + egui::vec2(0.0, 6.0)],
                    stroke,
                );
            }
            ToolbarIcon::Back => {
                ui.painter()
                    .line_segment([point(6.0, 0.0), point(-6.0, 0.0)], stroke);
                ui.painter().line(
                    vec![point(-1.0, -5.0), point(-6.0, 0.0), point(-1.0, 5.0)],
                    stroke,
                );
            }
            ToolbarIcon::Refresh => {
                let arc: Vec<_> = (0..=24)
                    .map(|step| {
                        let angle = (50.0 + step as f32 * 270.0 / 24.0).to_radians();
                        point(angle.cos() * 6.0, angle.sin() * 6.0)
                    })
                    .collect();
                let end = *arc.last().unwrap();
                ui.painter().line(arc, stroke);
                ui.painter().line(
                    vec![
                        end + egui::vec2(-4.0, -0.5),
                        end,
                        end + egui::vec2(0.0, -4.0),
                    ],
                    stroke,
                );
            }
            ToolbarIcon::Expand | ToolbarIcon::Collapse => {
                let back = Rect::from_center_size(point(-2.0, 2.0), egui::vec2(12.0, 12.0));
                let front = Rect::from_center_size(point(1.0, -1.0), egui::vec2(12.0, 12.0));
                ui.painter().rect_stroke(
                    back,
                    2,
                    Stroke::new(1.0, color.gamma_multiply(0.65)),
                    egui::StrokeKind::Inside,
                );
                ui.painter().rect_filled(front, 2, palette.panel);
                ui.painter()
                    .rect_stroke(front, 2, stroke, egui::StrokeKind::Inside);
                let center = front.center();
                ui.painter().line_segment(
                    [center - egui::vec2(3.0, 0.0), center + egui::vec2(3.0, 0.0)],
                    stroke,
                );
                if matches!(icon, ToolbarIcon::Expand) {
                    ui.painter().line_segment(
                        [center - egui::vec2(0.0, 3.0), center + egui::vec2(0.0, 3.0)],
                        stroke,
                    );
                }
            }
            ToolbarIcon::Browse => paint_icon(
                ui.painter(),
                center,
                Some(&DirectoryEntryKind::Directory),
                color,
            ),
            ToolbarIcon::Sun => {
                ui.painter().circle_filled(center, 4.0, color);
                for index in 0..8 {
                    let angle = index as f32 * std::f32::consts::TAU / 8.0;
                    let direction = egui::vec2(angle.cos(), angle.sin());
                    ui.painter().line_segment(
                        [center + direction * 7.2, center + direction * 10.2],
                        Stroke::new(1.2, color),
                    );
                }
            }
            ToolbarIcon::Moon => {
                let arc: Vec<_> = (0..=24)
                    .map(|step| {
                        let angle = (60.0 + step as f32 * 240.0 / 24.0).to_radians();
                        point(angle.cos() * 7.0, angle.sin() * 7.0)
                    })
                    .chain((0..=16).map(|step| {
                        let angle = (-90.0 - step as f32 * 180.0 / 16.0).to_radians();
                        point(3.5 + angle.cos() * 5.0, angle.sin() * 6.0)
                    }))
                    .collect();
                ui.painter().add(egui::Shape::closed_line(arc, stroke));
            }
            ToolbarIcon::Cancel => {
                ui.painter()
                    .line_segment([point(-4.0, -4.0), point(4.0, 4.0)], stroke);
                ui.painter()
                    .line_segment([point(-4.0, 4.0), point(4.0, -4.0)], stroke);
            }
        }
        if let Some(caption) = caption {
            let text =
                ui.painter()
                    .layout_no_wrap(caption.into(), FontId::proportional(11.0), color);
            let position = egui::pos2(
                rect.left() + extent + ui.spacing().item_spacing.x,
                rect.center().y - text.size().y / 2.0,
            );
            ui.painter().galley(position, text, color);
        }
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(label)
    })
    .inner
}

pub(in crate::app) fn paint_icon(
    painter: &egui::Painter,
    center: Pos2,
    kind: Option<&DirectoryEntryKind>,
    color: Color32,
) {
    let rect = Rect::from_center_size(center, egui::vec2(13.0, 11.0));
    if kind == Some(&DirectoryEntryKind::Directory) {
        painter.rect_filled(
            Rect::from_min_size(rect.min - egui::vec2(0.0, 2.0), egui::vec2(6.0, 4.0)),
            1,
            color.gamma_multiply(0.6),
        );
        painter.rect_filled(rect, 2, color.gamma_multiply(0.35));
        painter.rect_stroke(rect, 2, Stroke::new(1.0, color), egui::StrokeKind::Inside);
    } else if kind == Some(&DirectoryEntryKind::Symlink) {
        painter.text(
            center,
            Align2::CENTER_CENTER,
            "@",
            FontId::monospace(13.0),
            color,
        );
    } else {
        painter.rect_stroke(
            rect.shrink2(egui::vec2(2.0, 0.0)),
            1,
            Stroke::new(1.0, color),
            egui::StrokeKind::Inside,
        );
    }
}
