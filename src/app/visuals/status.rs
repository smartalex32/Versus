//! Shared status visual vocabulary for Diff and Merge.

use crate::app::visuals::theme::Palette;
use crate::app::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum StatusIcon {
    Different,
    LeftOnly,
    RightOnly,
    TypeMismatch,
    Error,
    Conflict,
}

impl StatusIcon {
    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Different => "Different",
            Self::LeftOnly => "Left only",
            Self::RightOnly => "Right only",
            Self::TypeMismatch => "Type mismatch",
            Self::Error => "Read error",
            Self::Conflict => "Conflict",
        }
    }
    pub(in crate::app) fn color(self, palette: Palette) -> Color32 {
        match self {
            Self::Different => palette.changed,
            Self::LeftOnly => palette.left_only,
            Self::RightOnly => palette.right_only,
            Self::TypeMismatch | Self::Error | Self::Conflict => palette.error,
        }
    }
}

pub(in crate::app) fn status_icon(state: &DirectoryEntryState) -> Option<StatusIcon> {
    match state {
        DirectoryEntryState::Same => None,
        DirectoryEntryState::Different => Some(StatusIcon::Different),
        DirectoryEntryState::LeftOnly => Some(StatusIcon::LeftOnly),
        DirectoryEntryState::RightOnly => Some(StatusIcon::RightOnly),
        DirectoryEntryState::TypeMismatch => Some(StatusIcon::TypeMismatch),
        DirectoryEntryState::Error(_) => Some(StatusIcon::Error),
    }
}

pub(in crate::app) fn state_label(state: &DirectoryEntryState) -> &'static str {
    status_icon(state).map_or("Identical", StatusIcon::label)
}

pub(in crate::app) fn side_name(side: usize) -> &'static str {
    if side == 0 { "Left" } else { "Right" }
}

pub(in crate::app) fn tree_row_accessible_label(node: &TreeNode, side: usize) -> String {
    let entry = if side == 0 { &node.left } else { &node.right };
    if !entry.exists {
        return format!(
            "{} side, {}: not present on this side; {}",
            side_name(side),
            node.relative_path.display(),
            state_label(&node.state)
        );
    }
    let kind = match entry.kind {
        Some(DirectoryEntryKind::Directory) => "folder",
        Some(DirectoryEntryKind::File) => "file",
        Some(DirectoryEntryKind::Symlink) => "symbolic link",
        Some(DirectoryEntryKind::Other) => "other item",
        None => "item",
    };
    format!(
        "{} {}: {}, {}, {}",
        side_name(side),
        kind,
        node.relative_path.display(),
        format_size(entry.size),
        state_label(&node.state),
    )
}

pub(in crate::app) fn file_line_accessible_label(
    side: usize,
    line: Option<&(usize, String)>,
    state: &DirectoryEntryState,
) -> String {
    match line {
        Some((number, text)) => {
            const MAX_BYTES: usize = 4096;
            let mut end = text.len().min(MAX_BYTES);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            let excerpt = if text.is_empty() {
                "blank"
            } else {
                &text[..end]
            };
            let omitted = if end < text.len() {
                format!(" … ({} bytes omitted)", text.len() - end)
            } else {
                String::new()
            };
            format!(
                "{} line {number}: {}, {excerpt}{omitted}",
                side_name(side),
                state_label(state)
            )
        }
        None => format!(
            "{}: no corresponding line, {}",
            side_name(side),
            state_label(state)
        ),
    }
}

pub(in crate::app) fn state_color(state: &DirectoryEntryState, palette: Palette) -> Color32 {
    status_icon(state).map_or(palette.muted, |icon| icon.color(palette))
}

pub(in crate::app) fn state_bar_color(
    state: &DirectoryEntryState,
    palette: Palette,
    ctx: &egui::Context,
) -> Color32 {
    let color = state_color(state, palette);
    if ctx.theme() != egui::Theme::Light {
        return color;
    }
    // Thin bars need saturated shades against the light background.
    match state {
        DirectoryEntryState::Different => Color32::from_rgb(236, 150, 0),
        DirectoryEntryState::LeftOnly => Color32::from_rgb(0, 164, 187),
        DirectoryEntryState::RightOnly => Color32::from_rgb(148, 76, 232),
        DirectoryEntryState::TypeMismatch | DirectoryEntryState::Error(_) => {
            Color32::from_rgb(235, 55, 82)
        }
        DirectoryEntryState::Same => color,
    }
}

pub(in crate::app) fn paint_status_icon(painter: &egui::Painter, center: Pos2, icon: StatusIcon) {
    let palette = Palette::for_context(painter.ctx());
    let stroke = Stroke::new(1.3, icon.color(palette));
    let point = |x, y| center + egui::vec2(x, y);
    match icon {
        StatusIcon::Different => {
            for y in [-2.0, 2.0] {
                painter.line_segment([point(-5.0, y), point(5.0, y)], stroke);
            }
            painter.line_segment([point(3.0, -6.0), point(-3.0, 6.0)], stroke);
        }
        StatusIcon::LeftOnly | StatusIcon::RightOnly => {
            let direction = if matches!(icon, StatusIcon::LeftOnly) {
                -1.0
            } else {
                1.0
            };
            painter.line_segment([point(-5.0, 0.0), point(5.0, 0.0)], stroke);
            painter.line(
                vec![
                    point(direction, -4.0),
                    point(direction * 5.0, 0.0),
                    point(direction, 4.0),
                ],
                stroke,
            );
        }
        StatusIcon::TypeMismatch => {
            painter.rect_stroke(
                Rect::from_center_size(center, egui::vec2(12.0, 12.0)),
                1,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.line_segment([point(-3.0, -3.0), point(3.0, 3.0)], stroke);
            painter.line_segment([point(-3.0, 3.0), point(3.0, -3.0)], stroke);
        }
        StatusIcon::Error => {
            painter.add(egui::Shape::closed_line(
                vec![point(0.0, -6.0), point(6.0, 5.0), point(-6.0, 5.0)],
                stroke,
            ));
            painter.line_segment([point(0.0, -2.0), point(0.0, 1.0)], stroke);
            painter.circle_filled(point(0.0, 3.0), 0.7, icon.color(palette));
        }
        StatusIcon::Conflict => {
            painter.rect_stroke(
                Rect::from_center_size(center, egui::vec2(12.0, 12.0)),
                2,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.line_segment([point(0.0, -3.0), point(0.0, 1.0)], stroke);
            painter.circle_filled(point(0.0, 3.0), 0.7, icon.color(palette));
        }
    }
}

pub(in crate::app) fn format_size(size: Option<u64>) -> String {
    let Some(bytes) = size else {
        return "—".into();
    };
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let units = ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let mut unit = 0;
    value /= 1024.0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", units[unit])
}
