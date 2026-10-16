//! The five Codicons used by Folder Find and Replace.
//!
//! The embedded font is the unmodified `codicon.ttf` from the official
//! `@vscode/codicons` package, version `0.0.46-24`. The vector sources and
//! published glyph map are from Microsoft’s `vscode-codicons` repository at
//! commit `8853e0b1b0e2058b099fde515d5f1f478db2f8e1`.
//!
//! The font is licensed under CC BY 4.0. See `assets/codicons/LICENSE` and
//! `assets/codicons/NOTICE.md` for the complete attribution and license text.

use eframe::egui;
use std::sync::Arc;

/// Name of the dedicated egui font family used for the embedded Codicons.
pub(crate) const FONT_FAMILY: &str = "VersusCodicons";

const FONT_NAME: &str = "versus-codicons";
const CODICON_FONT: &[u8] = include_bytes!("../../assets/codicons/codicon.ttf");

/// Codicons used by the folder Find and Replace controls.
///
/// The Unicode values are taken from the official package's generated
/// `dist/codicon.css` for `@vscode/codicons@0.0.46-24`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FindIcon {
    /// `case-sensitive` (`U+EAB1`).
    MatchCase,
    /// `whole-word` (`U+EB7E`).
    MatchWholeWord,
    /// `regex` (`U+EB38`).
    UseRegularExpression,
    /// `replace` (`U+EB3D`).
    ReplaceNext,
    /// `replace-all` (`U+EB3C`).
    ReplaceAll,
}

impl FindIcon {
    fn glyph(self) -> char {
        match self {
            Self::MatchCase => '\u{EAB1}',
            Self::MatchWholeWord => '\u{EB7E}',
            Self::UseRegularExpression => '\u{EB38}',
            Self::ReplaceNext => '\u{EB3D}',
            Self::ReplaceAll => '\u{EB3C}',
        }
    }
}

/// Adds the Codicon face as its own font family while preserving existing faces.
///
/// Call this before passing `definitions` to [`egui::Context::set_fonts`].
/// The icon painter deliberately uses this named family, so application text
/// never falls back to the icon font.
pub(crate) fn register(definitions: &mut egui::FontDefinitions) {
    definitions
        .font_data
        .entry(FONT_NAME.to_owned())
        .or_insert_with(|| Arc::new(egui::FontData::from_static(CODICON_FONT)));
    definitions
        .families
        .entry(egui::FontFamily::Name(FONT_FAMILY.into()))
        .or_insert_with(|| vec![FONT_NAME.to_owned()]);
}

/// Paints an exact 16 px VS Code Codicon centered in an egui control.
///
/// [`register`] must have been called before the first frame that paints an icon.
pub(crate) fn paint(ui: &egui::Ui, center: egui::Pos2, icon: FindIcon, color: egui::Color32) {
    ui.painter().text(
        center,
        egui::Align2::CENTER_CENTER,
        icon.glyph(),
        egui::FontId::new(16.0, egui::FontFamily::Name(FONT_FAMILY.into())),
        color,
    );
}

#[cfg(test)]
mod tests;
