//! Shared panels visual vocabulary for Diff and Merge.

use crate::app::visuals::icons::paint_icon;
use crate::app::visuals::theme::{Palette, comparison_font, comparison_row_height};
use crate::app::*;

pub(in crate::app) fn status_label(ui: &mut egui::Ui, text: &str, color: Color32) {
    ui.add(egui::Label::new(RichText::new(text).size(11.0).color(color)).truncate())
        .on_hover_text(text);
}

pub(in crate::app) fn comparison_mode_button(
    ui: &egui::Ui,
    rect: Rect,
    label: Arc<egui::Galley>,
    mode: ComparisonMode,
    selected: bool,
) -> egui::Response {
    let response = ui
        .interact(rect, egui::Id::new(mode.label()), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            ui.is_enabled(),
            selected,
            label.text(),
        )
    });
    let visuals = ui.style().interact_selectable(&response, selected);
    if selected
        || response.hovered()
        || response.has_focus()
        || response.is_pointer_button_down_on()
    {
        ui.painter().rect(
            rect,
            visuals.corner_radius,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
    }
    let icon_center = egui::pos2(
        rect.left() + ui.spacing().button_padding.x + 8.0,
        rect.center().y,
    );
    if mode == ComparisonMode::Folder {
        paint_icon(
            ui.painter(),
            icon_center,
            Some(&DirectoryEntryKind::Directory),
            visuals.fg_stroke.color,
        );
    } else {
        let point = |x, y| icon_center + egui::vec2(x, y);
        let stroke = Stroke::new(1.0, visuals.fg_stroke.color);
        ui.painter().add(egui::Shape::closed_line(
            vec![
                point(-5.0, -7.0),
                point(1.0, -7.0),
                point(5.0, -3.0),
                point(5.0, 7.0),
                point(-5.0, 7.0),
            ],
            stroke,
        ));
        ui.painter().line(
            vec![point(1.0, -7.0), point(1.0, -3.0), point(5.0, -3.0)],
            stroke,
        );
        for y in [1.0, 4.0] {
            ui.painter()
                .line_segment([point(-2.0, y), point(2.0, y)], stroke);
        }
    }
    let position = egui::pos2(icon_center.x + 14.0, rect.center().y - label.size().y / 2.0);
    ui.painter()
        .galley_with_override_text_color(position, label, visuals.fg_stroke.color);
    response
}

pub(in crate::app) fn workspace_mode_button(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.into(),
        FontId::proportional(11.0),
        Palette::for_context(ui.ctx()).text,
    );
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(galley.size().x + 12.0, 28.0), Sense::hover());
    let response = ui
        .interact(
            rect,
            egui::Id::new(("workspace-mode", label)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, label)
    });
    let visuals = ui.style().interact_selectable(&response, selected);
    if selected
        || response.hovered()
        || response.has_focus()
        || response.is_pointer_button_down_on()
    {
        ui.painter().rect(
            rect,
            visuals.corner_radius,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().galley_with_override_text_color(
        rect.center() - galley.size() * 0.5,
        galley,
        visuals.fg_stroke.color,
    );
    response
}

pub(in crate::app) fn comparison_panel(ui: &egui::Ui) -> egui::Frame {
    let palette = Palette::for_context(ui.ctx());
    egui::Frame::default()
        .fill(palette.panel)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(6)
}
pub(in crate::app) fn paint_panel_divider(ui: &mut egui::Ui) {
    let (divider, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment(
        [divider.left_center(), divider.right_center()],
        Stroke::new(1.0, Palette::for_context(ui.ctx()).border),
    );
}

pub(in crate::app) fn paint_side_label(
    painter: &egui::Painter,
    rect: Rect,
    side: usize,
    palette: Palette,
) {
    painter.text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        if side == 0 { "LEFT" } else { "RIGHT" },
        FontId::monospace(10.0),
        if side == 0 {
            palette.left_only
        } else {
            palette.right_only
        },
    );
}

pub(in crate::app) fn file_path_headers(ui: &mut egui::Ui, paths: &[PathBuf; 2]) {
    let palette = Palette::for_context(ui.ctx());
    let (header, _) = ui.allocate_exact_size(
        egui::vec2(
            ui.available_width(),
            38.0_f32.max(comparison_row_height(ui) + 14.0),
        ),
        Sense::hover(),
    );
    for (side, half) in split_rect(header).into_iter().enumerate() {
        let rect = half.shrink(7.0);
        let painter = ui.painter().with_clip_rect(rect);
        paint_side_label(&painter, rect, side, palette);
        let path = paths[side].display().to_string();
        let path_rect = Rect::from_min_max(rect.min + egui::vec2(40.0, 0.0), rect.max);
        let galley = left_elided_path(&painter, &path, path_rect.width(), palette.text);
        let position = egui::pos2(path_rect.left(), rect.center().y - galley.size().y / 2.0);
        painter.galley(position, galley, palette.text);
        ui.interact(path_rect, ui.id().with(("file-path", side)), Sense::hover())
            .on_hover_text(path);
    }
    ui.painter().line_segment(
        [header.center_top(), header.center_bottom()],
        Stroke::new(1.0, palette.border),
    );
}

pub(in crate::app) fn left_elided_path(
    painter: &egui::Painter,
    path: &str,
    width: f32,
    color: Color32,
) -> Arc<egui::Galley> {
    let layout = |text: String| painter.layout_no_wrap(text, comparison_font(painter.ctx()), color);
    let full = layout(path.into());
    if full.size().x <= width {
        return full;
    }
    if layout("…".into()).size().x > width {
        return layout(String::new());
    }
    let boundaries: Vec<_> = path
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(path.len()))
        .collect();
    let mut first = 0;
    let mut last = boundaries.len() - 1;
    while first < last {
        let middle = (first + last) / 2;
        if layout(format!("…{}", &path[boundaries[middle]..])).size().x <= width {
            last = middle;
        } else {
            first = middle + 1;
        }
    }
    layout(format!("…{}", &path[boundaries[first]..]))
}

pub(in crate::app) fn split_rect(rect: Rect) -> [Rect; 2] {
    [
        Rect::from_min_max(rect.min, rect.center_bottom()),
        Rect::from_min_max(rect.center_top(), rect.max),
    ]
}

pub(in crate::app) fn empty_display(ui: &mut egui::Ui, height: f32, title: &str, subtitle: &str) {
    let palette = Palette::for_context(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height.max(80.0)),
        Sense::hover(),
    );
    ui.painter().line_segment(
        [rect.center_top(), rect.center_bottom()],
        Stroke::new(1.0, palette.border.gamma_multiply(0.5)),
    );
    let painter = ui.painter().with_clip_rect(rect);
    {
        let chooser = title.starts_with("Choose ");
        for (side, half) in split_rect(rect).into_iter().enumerate() {
            paint_empty_drop_target(ui, half, side, chooser);
        }
        if chooser {
            return;
        }
    }
    let center = rect.center();
    painter.text(
        center - egui::vec2(0.0, 24.0),
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(21.0),
        palette.text,
    );
    painter.text(
        center + egui::vec2(0.0, 8.0),
        Align2::CENTER_CENTER,
        subtitle,
        FontId::proportional(11.0),
        palette.muted,
    );
}
