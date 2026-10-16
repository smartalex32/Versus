//! Shared footer visual vocabulary for Diff and Merge.

use crate::app::visuals::icons::captioned_icon_width;
use crate::app::visuals::status::{StatusIcon, paint_status_icon};
use crate::app::visuals::theme::{Palette, control_extent};
use crate::app::*;

pub(in crate::app) fn progress_text(snapshot: versus::ProgressSnapshot) -> String {
    let stage = match snapshot.stage {
        ProgressStage::Scanning => "Scanning folders",
        ProgressStage::Reading => "Reading files",
        ProgressStage::ComparingFiles => "Comparing files",
        ProgressStage::ComparingLines => "Comparing lines",
        ProgressStage::Highlighting => "Highlighting differences",
        ProgressStage::BuildingTree => "Finalizing folder tree",
        ProgressStage::Finished => "Preparing comparison",
    };
    if let Some(fraction) = snapshot.fraction() {
        format!("{stage}… {:.0}% complete", fraction * 100.0)
    } else if snapshot.stage == ProgressStage::Scanning {
        format!("{stage}… {} entries found", snapshot.completed)
    } else {
        format!("{stage}…")
    }
}

pub(in crate::app) const DIFF_LEGEND_ICONS: [StatusIcon; 5] = [
    StatusIcon::Different,
    StatusIcon::LeftOnly,
    StatusIcon::RightOnly,
    StatusIcon::TypeMismatch,
    StatusIcon::Error,
];

pub(in crate::app) fn legend_label(icon: StatusIcon, counts: Option<[usize; 6]>) -> String {
    let index = DIFF_LEGEND_ICONS
        .iter()
        .position(|candidate| *candidate == icon);
    match (counts, index) {
        (Some(counts), Some(index)) => format!("{}  {}", icon.label(), counts[index + 1]),
        _ => icon.label().into(),
    }
}

pub(in crate::app) fn legend_rows(
    ui: &egui::Ui,
    icons: &[StatusIcon],
    counts: Option<[usize; 6]>,
) -> Vec<Vec<(StatusIcon, String, f32)>> {
    legend_rows_in_width(ui, icons, counts, ui.available_width())
}

pub(in crate::app) fn legend_rows_in_width(
    ui: &egui::Ui,
    icons: &[StatusIcon],
    counts: Option<[usize; 6]>,
    available: f32,
) -> Vec<Vec<(StatusIcon, String, f32)>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut width = 0.0;
    let gap = ui.spacing().item_spacing.x;
    let available = available.max(1.0);
    for icon in icons {
        let label = legend_label(*icon, counts);
        let text_width = ui
            .painter()
            .layout_no_wrap(
                label.clone(),
                FontId::proportional(10.0),
                Palette::for_context(ui.ctx()).text,
            )
            .size()
            .x;
        let item_width = (14.0 + gap + text_width + 5.0).min(available);
        if !row.is_empty() && width + gap + item_width > available {
            rows.push(std::mem::take(&mut row));
            width = 0.0;
        }
        if !row.is_empty() {
            width += gap;
        }
        width += item_width;
        row.push((*icon, label, item_width));
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

pub(in crate::app) fn bottom_legend_height(
    ui: &egui::Ui,
    icons: &[StatusIcon],
    counts: Option<[usize; 6]>,
) -> f32 {
    let rows = legend_rows(ui, icons, counts).len();
    rows as f32 * control_extent(ui) + rows.saturating_sub(1) as f32 * ui.spacing().item_spacing.y
}

pub(in crate::app) fn bottom_legend(
    ui: &mut egui::Ui,
    icons: &[StatusIcon],
    counts: Option<[usize; 6]>,
) -> f32 {
    let rows = legend_rows(ui, icons, counts);
    let height = bottom_legend_height(ui, icons, counts);
    let (area, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let palette = Palette::for_context(ui.ctx());
    for (index, row) in rows.into_iter().enumerate() {
        let gap = ui.spacing().item_spacing.x;
        let width =
            row.iter().map(|item| item.2).sum::<f32>() + row.len().saturating_sub(1) as f32 * gap;
        let mut x = area.center().x - width / 2.0;
        let y = area.top() + index as f32 * (control_extent(ui) + ui.spacing().item_spacing.y);
        for (icon, label, width) in row {
            let rect = Rect::from_min_size(egui::pos2(x, y), egui::vec2(width, control_extent(ui)));
            let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
            paint_status_icon(
                &painter,
                egui::pos2(rect.left() + 7.0, rect.center().y),
                icon,
            );
            painter.text(
                egui::pos2(rect.left() + 14.0 + gap, rect.center().y),
                Align2::LEFT_CENTER,
                &label,
                FontId::proportional(10.0),
                icon.color(palette),
            );
            ui.interact(
                rect,
                egui::Id::new(("legend", icon.label())),
                Sense::hover(),
            )
            .on_hover_text(&label)
            .widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label.clone())
            });
            x += width + gap;
        }
    }
    height
}

pub(in crate::app) fn error_summary(ui: &mut egui::Ui, errors: &[String]) {
    let label = format!(
        "{} error{}",
        errors.len(),
        if errors.len() == 1 { "" } else { "s" }
    );
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let response = ui.add(
            egui::Label::new(
                RichText::new(&label)
                    .size(11.0)
                    .color(Palette::for_context(ui.ctx()).error),
            )
            .truncate(),
        );
        ui.interact(
            response.rect,
            egui::Id::new("workspace-errors"),
            Sense::hover(),
        )
        .on_hover_text(errors.join("\n\n"));
    });
}

pub(in crate::app) struct WorkspaceFooterLayout {
    pub(in crate::app) info: Rect,
    pub(in crate::app) legend: Rect,
    pub(in crate::app) status: Rect,
    pub(in crate::app) dirty: Option<Rect>,
    pub(in crate::app) save: Option<Rect>,
}

pub(in crate::app) fn footer_side_width(
    ui: &egui::Ui,
    width: f32,
    with_save: bool,
    with_dirty: bool,
) -> f32 {
    let dirty_width = ui
        .painter()
        .layout_no_wrap(
            "Unsaved Changes".into(),
            FontId::proportional(11.0),
            Palette::for_context(ui.ctx()).changed,
        )
        .size()
        .x;
    let save_and_dirty = with_dirty.then(|| {
        dirty_width
            + 40.0
            + with_save
                .then(|| captioned_icon_width(ui, "Save Both") + ui.spacing().item_spacing.x)
                .unwrap_or_default()
    });
    (width * 0.20)
        .max(save_and_dirty.unwrap_or(if with_save { 160.0 } else { 128.0 }))
        .min(260.0)
        .min(width / 2.0)
}

pub(in crate::app) fn workspace_footer_height(
    ui: &egui::Ui,
    with_save: bool,
    with_dirty: bool,
    icons: &[StatusIcon],
    counts: Option<[usize; 6]>,
) -> f32 {
    let width = ui.available_width();
    let legend_width = width - 2.0 * footer_side_width(ui, width, with_save, with_dirty);
    let rows = legend_rows_in_width(ui, icons, counts, legend_width)
        .len()
        .max(1);
    rows as f32 * control_extent(ui) + rows.saturating_sub(1) as f32 * ui.spacing().item_spacing.y
}

pub(in crate::app) fn workspace_footer_layout(
    ui: &mut egui::Ui,
    with_save: bool,
    with_dirty: bool,
    icons: &[StatusIcon],
    counts: Option<[usize; 6]>,
) -> WorkspaceFooterLayout {
    let height = workspace_footer_height(ui, with_save, with_dirty, icons, counts);
    let extent = control_extent(ui);
    let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    // Symmetric side areas keep the legend centered. At high zoom its entries
    // wrap inside this same footer, with guidance, activity and Save alongside.
    let side = footer_side_width(ui, row.width(), with_save, with_dirty);
    let line_top = row.center().y - extent / 2.0;
    let line_bottom = line_top + extent;
    let info = Rect::from_min_max(
        egui::pos2(row.left(), line_top),
        egui::pos2(row.left() + side, line_bottom),
    );
    let legend = Rect::from_min_max(
        egui::pos2(info.right(), row.top()),
        egui::pos2(row.right() - side, row.bottom()),
    );
    ui.interact(legend, egui::Id::new("workspace-legend"), Sense::hover());
    let save = with_save.then(|| {
        let width = captioned_icon_width(ui, "Save Both");
        Rect::from_min_max(
            egui::pos2(row.right() - width.min(side), line_top),
            egui::pos2(row.right(), line_bottom),
        )
    });
    let dirty = with_dirty.then(|| {
        let right = save.map_or(row.right(), |save| {
            save.left() - ui.spacing().item_spacing.x
        });
        let width = ui
            .painter()
            .layout_no_wrap(
                "Unsaved Changes".into(),
                FontId::proportional(11.0),
                Palette::for_context(ui.ctx()).changed,
            )
            .size()
            .x
            .min((right - legend.right() - ui.spacing().item_spacing.x).max(0.0));
        Rect::from_min_max(
            egui::pos2(right - width, line_top),
            egui::pos2(right, line_bottom),
        )
    });
    let status = Rect::from_min_max(
        egui::pos2(legend.right(), line_top),
        egui::pos2(
            dirty.or(save).map_or(row.right(), |dirty| {
                dirty.left() - ui.spacing().item_spacing.x
            }),
            line_bottom,
        ),
    );
    WorkspaceFooterLayout {
        info,
        legend,
        status,
        dirty,
        save,
    }
}

pub(in crate::app) fn footer_guidance(ui: &mut egui::Ui, text: &str) {
    let palette = Palette::for_context(ui.ctx());
    ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));
    ui.horizontal(|ui| {
        let available = ui.available_rect_before_wrap();
        ui.interact(
            available,
            egui::Id::new("workspace-guidance"),
            Sense::hover(),
        );
        ui.add(
            egui::Label::new(RichText::new(text).size(11.0).color(palette.muted))
                .truncate()
                .halign(egui::Align::LEFT),
        )
        .on_hover_text(text);
    });
}

pub(in crate::app) fn bottom_status_area(ui: &mut egui::Ui, draw: impl FnOnce(&mut egui::Ui)) {
    let (row, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), control_extent(ui)),
        Sense::hover(),
    );
    let width = row.width().min(420.0);
    let rect = Rect::from_min_max(egui::pos2(row.right() - width, row.top()), row.max);
    ui.interact(rect, egui::Id::new("workspace-status"), Sense::hover());
    let mut status_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    status_ui.set_clip_rect(status_ui.clip_rect().intersect(rect));
    draw(&mut status_ui);
}

pub(in crate::app) fn header_progress_label(
    ui: &mut egui::Ui,
    snapshot: versus::ProgressSnapshot,
    color: Color32,
) {
    let full = progress_text(snapshot);
    let width = ui
        .painter()
        .layout_no_wrap(full.clone(), FontId::proportional(11.0), color)
        .size()
        .x;
    let text = if width > ui.available_width() {
        snapshot
            .fraction()
            .map(|fraction| format!("{:.0}% complete", fraction * 100.0))
            .unwrap_or_else(|| full.clone())
    } else {
        full.clone()
    };
    ui.add_sized(
        ui.available_size(),
        egui::Label::new(RichText::new(text).size(11.0).color(color))
            .truncate()
            .halign(egui::Align::Center),
    )
    .on_hover_text(full);
}

pub(in crate::app) fn header_status_label(ui: &mut egui::Ui, text: &str, color: Color32) {
    ui.add_sized(
        ui.available_size(),
        egui::Label::new(RichText::new(text).size(11.0).color(color))
            .truncate()
            .halign(egui::Align::Center),
    )
    .on_hover_text(text);
}
