//! Recent comparison list presentation.

use super::*;
use crate::app::visuals::theme::Palette;

impl VersusApp {
    pub(super) fn recent_area(&mut self, ui: &mut egui::Ui, height: f32) {
        let palette = Palette::for_context(ui.ctx());
        let mut open = None;
        let entries: Vec<_> = self
            .session
            .recent
            .entries()
            .iter()
            .filter(|entry| entry.mode == self.session.comparison_mode)
            .collect();
        egui::ScrollArea::vertical()
            .id_salt((
                "recent-comparisons",
                self.session.comparison_mode.source_label(),
            ))
            .auto_shrink([false, false])
            .max_height(height)
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Recently compared")
                            .size(14.0)
                            .color(palette.muted),
                    );
                });
                ui.add_space(6.0);
                if let Some(message) = &self.session.recent_message {
                    ui.colored_label(palette.error, message);
                    ui.add_space(6.0);
                }
                if entries.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.label(
                            RichText::new(format!(
                                "No recent {} comparisons.",
                                self.session.comparison_mode.source_label()
                            ))
                            .color(palette.muted),
                        );
                    });
                }
                ui.spacing_mut().item_spacing.y = 0.0;
                for (index, entry) in entries.into_iter().enumerate() {
                    if recent_comparison_row(ui, entry, index, palette).clicked() {
                        open = Some(entry.clone());
                    }
                }
            });
        if let Some(entry) = open {
            self.session.open_recent(entry);
            ui.ctx().request_repaint();
        }
    }
}
pub(in crate::app) fn recent_comparison_row(
    ui: &mut egui::Ui,
    entry: &RecentComparison,
    index: usize,
    palette: Palette,
) -> egui::Response {
    let paths = entry
        .paths
        .each_ref()
        .map(|path| path.display().to_string());
    let font = egui::TextStyle::Body.resolve(ui.style());
    let separator_font = FontId::new(font.size + 2.0, font.family.clone());
    let separator = ui
        .painter()
        .layout_no_wrap("vs".into(), separator_font, palette.text);
    let padding = 8.0;
    let gap = 10.0;
    let available =
        (ui.available_width() - padding * 2.0 - gap * 2.0 - separator.size().x).max(1.0);
    let mut labels = paths.each_ref().map(|path| {
        ui.painter()
            .layout_no_wrap(path.clone(), font.clone(), palette.text)
    });
    if labels.iter().map(|label| label.size().x).sum::<f32>() > available {
        // Keep both paths readable at narrow widths instead of hiding one side.
        labels = paths.each_ref().map(|path| {
            ui.painter()
                .layout(path.clone(), font.clone(), palette.text, available / 2.0)
        });
    }
    let height = labels
        .iter()
        .map(|label| label.size().y)
        .fold(separator.size().y, f32::max)
        + padding * 2.0;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(
            ui.available_width(),
            height.max(ui.spacing().interact_size.y),
        ),
        Sense::hover(),
    );
    let response = ui.interact(
        rect,
        egui::Id::new(("recent-comparison", entry.mode.source_label(), index)),
        Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            format!("{} vs {}", paths[0], paths[1]),
        )
    });
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(
        rect,
        0,
        if response.hovered() || response.is_pointer_button_down_on() {
            palette.hover
        } else if index % 2 == 0 {
            palette.panel
        } else {
            palette.alternate
        },
    );
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            0,
            Stroke::new(1.0, palette.accent),
            egui::StrokeKind::Inside,
        );
    }
    let left = egui::pos2(
        rect.left() + padding,
        rect.center().y - labels[0].size().y / 2.0,
    );
    let versus = egui::pos2(
        left.x + labels[0].size().x + gap,
        rect.center().y - separator.size().y / 2.0,
    );
    let right = egui::pos2(
        versus.x + separator.size().x + gap,
        rect.center().y - labels[1].size().y / 2.0,
    );
    painter.galley(left, labels[0].clone(), palette.text);
    // Egui's default proportional font has no bold face. A small overprint gives
    // just the separator a heavier weight without adding a font dependency.
    painter.galley(versus, separator.clone(), palette.text);
    painter.galley(versus + egui::vec2(0.5, 0.0), separator, palette.text);
    painter.galley(right, labels[1].clone(), palette.text);
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!(
            "{} vs {}\nRecompare these sources",
            paths[0], paths[1]
        ))
}
