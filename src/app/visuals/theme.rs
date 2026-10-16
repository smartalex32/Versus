//! Shared theme visual vocabulary for Diff and Merge.

use crate::app::visuals::icons::{ToolbarIcon, icon_button};
use crate::app::*;

#[derive(Clone, Copy)]
pub(in crate::app) struct Palette {
    pub(in crate::app) background: Color32,
    pub(in crate::app) panel: Color32,
    pub(in crate::app) border: Color32,
    pub(in crate::app) muted: Color32,
    pub(in crate::app) text: Color32,
    pub(in crate::app) accent: Color32,
    pub(in crate::app) changed: Color32,
    pub(in crate::app) left_only: Color32,
    pub(in crate::app) right_only: Color32,
    pub(in crate::app) error: Color32,
    pub(in crate::app) alternate: Color32,
    pub(in crate::app) hover: Color32,
}

impl Palette {
    pub(in crate::app) fn for_context(ctx: &egui::Context) -> Self {
        Self::new(ctx.theme() == egui::Theme::Dark)
    }
    pub(in crate::app) fn new(dark: bool) -> Self {
        if dark {
            Self {
                background: Color32::from_rgb(16, 21, 29),
                panel: Color32::from_rgb(23, 30, 40),
                border: Color32::from_rgb(48, 59, 74),
                muted: Color32::from_rgb(145, 161, 181),
                text: Color32::from_rgb(221, 230, 240),
                accent: Color32::from_rgb(95, 163, 255),
                changed: Color32::from_rgb(244, 191, 98),
                left_only: Color32::from_rgb(94, 211, 221),
                right_only: Color32::from_rgb(189, 157, 255),
                error: Color32::from_rgb(255, 127, 137),
                alternate: Color32::from_rgb(26, 34, 45),
                hover: Color32::from_rgb(34, 44, 58),
            }
        } else {
            Self {
                background: Color32::from_rgb(244, 246, 249),
                panel: Color32::WHITE,
                border: Color32::from_rgb(207, 215, 225),
                muted: Color32::from_rgb(88, 101, 119),
                text: Color32::from_rgb(32, 43, 58),
                accent: Color32::from_rgb(26, 102, 185),
                changed: Color32::from_rgb(142, 86, 10),
                left_only: Color32::from_rgb(0, 112, 128),
                right_only: Color32::from_rgb(112, 65, 176),
                error: Color32::from_rgb(179, 38, 58),
                alternate: Color32::from_rgb(244, 247, 251),
                hover: Color32::from_rgb(230, 237, 246),
            }
        }
    }
}

pub(in crate::app) const ROW_HEIGHT: f32 = 22.0;
pub(in crate::app) const MIN_ZOOM: f32 = 0.2;
pub(in crate::app) const MAX_ZOOM: f32 = 5.0;

pub(in crate::app) fn restore_zoom(ctx: &egui::Context, storage: &dyn eframe::Storage) {
    if let Some(zoom) = storage
        .get_string("versus.zoom")
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|zoom| zoom.is_finite() && (MIN_ZOOM..=MAX_ZOOM).contains(zoom))
    {
        ctx.set_zoom_factor(zoom);
    }
}

pub(in crate::app) fn zoom_controls(ui: &mut egui::Ui) {
    let zoom_factor = ui.ctx().zoom_factor();
    let zoom = format!("Zoom {:.0}%", zoom_factor * 100.0);
    let shortcuts = [
        egui::gui_zoom::kb_shortcuts::ZOOM_IN,
        egui::gui_zoom::kb_shortcuts::ZOOM_OUT,
        egui::gui_zoom::kb_shortcuts::ZOOM_RESET,
    ]
    .map(|shortcut| ui.ctx().format_shortcut(&shortcut));
    if ui.add(egui::Button::new(RichText::new(zoom).color(Palette::for_context(ui.ctx()).muted)).frame(false))
        .on_hover_text(format!("{} to enlarge · {} to reduce · {} to reset\nCtrl + mouse wheel to zoom. Click to reset. Your zoom is saved for next time.", shortcuts[0], shortcuts[1], shortcuts[2]))
        .clicked() { ui.ctx().set_zoom_factor(1.0); }
    if icon_button(ui, ToolbarIcon::ZoomOut, zoom_factor > MIN_ZOOM, "Zoom out").clicked() {
        egui::gui_zoom::zoom_out(ui.ctx());
    }
    if icon_button(ui, ToolbarIcon::ZoomIn, zoom_factor < MAX_ZOOM, "Zoom in").clicked() {
        egui::gui_zoom::zoom_in(ui.ctx());
    }
}

pub(in crate::app) fn comparison_row_height(ui: &egui::Ui) -> f32 {
    ROW_HEIGHT.max(ui.text_style_height(&egui::TextStyle::Monospace).ceil() + 8.0)
}

pub(in crate::app) fn control_extent(ui: &egui::Ui) -> f32 {
    24.0_f32.max(ui.text_style_height(&egui::TextStyle::Button).ceil() + 10.0)
}

pub(in crate::app) fn comparison_font(ctx: &egui::Context) -> FontId {
    egui::TextStyle::Monospace.resolve(&ctx.style_of(ctx.theme()))
}

pub(in crate::app) fn apply_theme(ctx: &egui::Context) {
    // Support literal Ctrl on every platform as well as the platform command key.
    ctx.options_mut(|options| {
        options.input_options.zoom_modifier = egui::Modifiers::CTRL | egui::Modifiers::COMMAND
    });
    set_theme(ctx, egui::Theme::Dark);
}

pub(in crate::app) fn set_theme(ctx: &egui::Context, theme: egui::Theme) {
    let installed = egui::Id::new("versus-codicons-installed");
    if !ctx.data(|data| data.get_temp::<bool>(installed).unwrap_or(false)) {
        let mut fonts = egui::FontDefinitions::default();
        find_icons::register(&mut fonts);
        ctx.set_fonts(fonts);
        ctx.data_mut(|data| data.insert_temp(installed, true));
    }
    let palette = Palette::new(theme == egui::Theme::Dark);
    ctx.set_theme(theme);
    let mut visuals = if theme == egui::Theme::Dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.panel_fill = palette.background;
    visuals.window_fill = palette.panel;
    visuals.extreme_bg_color = palette.background;
    visuals.override_text_color = Some(palette.text);
    visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    visuals.selection.bg_fill = palette.accent.gamma_multiply(0.3);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.border);
    ctx.set_visuals(visuals);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(6.0, 3.0);
        style.spacing.interact_size.y = 24.0;
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(11.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(11.0));
        style
            .text_styles
            .insert(egui::TextStyle::Monospace, FontId::monospace(11.0));
    });
}
