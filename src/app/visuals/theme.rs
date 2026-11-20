//! Shared theme visual vocabulary for Diff and Merge.

use crate::app::visuals::icons::{ToolbarIcon, icon_button};
use crate::app::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::app) enum Theme {
    #[default]
    Dark,
    Light,
    Dracula,
    Nord,
    SolarizedLight,
}

pub(in crate::app) const THEME_STORAGE_KEY: &str = "versus.theme";

impl Theme {
    pub(in crate::app) const ALL: [Self; 5] = [
        Self::Dark,
        Self::Light,
        Self::Dracula,
        Self::Nord,
        Self::SolarizedLight,
    ];

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::Dracula => "Dracula",
            Self::Nord => "Nord",
            Self::SolarizedLight => "Solarized Light",
        }
    }

    pub(in crate::app) fn storage_id(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Dracula => "dracula",
            Self::Nord => "nord",
            Self::SolarizedLight => "solarized-light",
        }
    }

    pub(in crate::app) fn for_context(ctx: &egui::Context) -> Self {
        ctx.data(|data| data.get_temp(egui::Id::new(THEME_STORAGE_KEY)))
            .unwrap_or_else(|| ctx.theme().into())
    }

    fn egui_theme(self) -> egui::Theme {
        match self {
            Self::Light | Self::SolarizedLight => egui::Theme::Light,
            Self::Dark | Self::Dracula | Self::Nord => egui::Theme::Dark,
        }
    }

    fn palette(self) -> Palette {
        match self {
            Self::Dark => Palette::new(true),
            Self::Light => Palette::new(false),
            // Dracula Classic: https://draculatheme.com/spec
            Self::Dracula => Palette {
                background: rgb(0x282a36),
                panel: rgb(0x21222c),
                border: rgb(0x6272a4),
                muted: rgb(0xb2b8d4),
                text: rgb(0xf8f8f2),
                accent: rgb(0xbd93f9),
                changed: rgb(0xffb86c),
                left_only: rgb(0x8be9fd),
                right_only: rgb(0xff79c6),
                error: rgb(0xff7777),
                alternate: rgb(0x2e3040),
                hover: rgb(0x343746),
            },
            // Polar Night, Snow Storm, Frost and Aurora: https://www.nordtheme.com/docs/colors-and-palettes/
            Self::Nord => Palette {
                background: rgb(0x2e3440),
                panel: rgb(0x3b4252),
                border: rgb(0x4c566a),
                muted: rgb(0xbac7d9),
                text: rgb(0xeceff4),
                accent: rgb(0x92c7d7),
                changed: rgb(0xebcb8b),
                left_only: rgb(0xabc699),
                right_only: rgb(0xd9b6d2),
                error: rgb(0xf1a5ad),
                alternate: rgb(0x3e4658),
                hover: rgb(0x414a5b),
            },
            // Darkened Solarized accents keep small comparison text legible on cream.
            // https://ethanschoonover.com/solarized/
            Self::SolarizedLight => Palette {
                background: rgb(0xfdf6e3),
                panel: rgb(0xeee8d5),
                border: rgb(0x93a1a1),
                muted: rgb(0x4e6268),
                text: rgb(0x073642),
                accent: rgb(0x1c6094),
                changed: rgb(0x97400e),
                left_only: rgb(0x116c61),
                right_only: rgb(0x5257a5),
                error: rgb(0xb52932),
                alternate: rgb(0xf6efd9),
                hover: rgb(0xe4ddc9),
            },
        }
    }
}

impl From<egui::Theme> for Theme {
    fn from(theme: egui::Theme) -> Self {
        match theme {
            egui::Theme::Dark => Self::Dark,
            egui::Theme::Light => Self::Light,
        }
    }
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

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
        Theme::for_context(ctx).palette()
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

pub(in crate::app) fn restore_theme(ctx: &egui::Context, storage: &dyn eframe::Storage) {
    if let Some(theme) = storage.get_string(THEME_STORAGE_KEY).and_then(|id| {
        Theme::ALL
            .into_iter()
            .find(|theme| theme.storage_id() == id)
    }) {
        set_theme(ctx, theme);
    }
}

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
    set_theme(ctx, Theme::default());
}

pub(in crate::app) fn set_theme(ctx: &egui::Context, theme: impl Into<Theme>) {
    let installed = egui::Id::new("versus-codicons-installed");
    if !ctx.data(|data| data.get_temp::<bool>(installed).unwrap_or(false)) {
        let mut fonts = egui::FontDefinitions::default();
        find_icons::register(&mut fonts);
        ctx.set_fonts(fonts);
        ctx.data_mut(|data| data.insert_temp(installed, true));
    }
    let theme = theme.into();
    let palette = theme.palette();
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(THEME_STORAGE_KEY), theme));
    let base_theme = theme.egui_theme();
    ctx.set_theme(base_theme);
    let mut visuals = if base_theme == egui::Theme::Dark {
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
    if !matches!(theme, Theme::Dark | Theme::Light) {
        visuals.weak_text_color = Some(palette.muted);
        visuals.hyperlink_color = palette.accent;
        visuals.faint_bg_color = palette.alternate;
        visuals.code_bg_color = palette.alternate;
        visuals.warn_fg_color = palette.changed;
        visuals.error_fg_color = palette.error;
        visuals.window_stroke.color = palette.border;
        visuals.selection.stroke.color = palette.text;
        visuals.text_cursor.stroke.color = palette.accent;
        visuals.widgets.noninteractive.bg_fill = palette.panel;
        visuals.widgets.noninteractive.fg_stroke.color = palette.text;
        for (widget, fill, border) in [
            (
                &mut visuals.widgets.inactive,
                palette.alternate,
                palette.border,
            ),
            (&mut visuals.widgets.hovered, palette.hover, palette.accent),
            (&mut visuals.widgets.active, palette.hover, palette.accent),
            (&mut visuals.widgets.open, palette.hover, palette.accent),
        ] {
            widget.bg_fill = fill;
            widget.weak_bg_fill = fill;
            widget.bg_stroke = Stroke::new(1.0, border);
            widget.fg_stroke.color = palette.text;
        }
    }
    ctx.set_visuals_of(base_theme, visuals);
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
    ctx.request_repaint();
}
