//! Read-only aligned file rows, search filtering, and difference navigation.

use super::*;
use crate::app::file_style;
use crate::app::visuals::panels::{comparison_panel, empty_display};
use crate::app::visuals::theme::{Palette, comparison_row_height};
use eframe::egui::{self, RichText};
use versus::{DirectoryEntryState, FileComparison, FileComparisonRow};

mod panes;

impl GitDiffView {
    pub(super) fn render_file(&mut self, ui: &mut egui::Ui, file: &mut FileScreen) -> bool {
        let palette = Palette::for_context(ui.ctx());
        let can_go_back = file.back.is_some();
        let mut back = false;
        let visible = visible_rows(&file.comparison, self.options, &self.search);
        ui.horizontal(|ui| {
            if can_go_back && ui.button("Back to files").clicked() {
                back = true;
            }
            let differences = visible
                .iter()
                .copied()
                .filter(|index| file.comparison.rows[*index].state != DirectoryEntryState::Same)
                .collect::<Vec<_>>();
            if ui
                .add_enabled(
                    !differences.is_empty(),
                    egui::Button::new("Previous difference"),
                )
                .clicked()
            {
                file.navigation = next_difference(file.navigation, &differences, false);
                file.pending_navigation = true;
            }
            if ui
                .add_enabled(
                    !differences.is_empty(),
                    egui::Button::new("Next difference"),
                )
                .clicked()
            {
                file.navigation = next_difference(file.navigation, &differences, true);
                file.pending_navigation = true;
            }
            ui.label(
                RichText::new(format!("{} displayed rows", visible.len())).color(palette.muted),
            );
        });
        comparison_panel(ui).show(ui, |ui| {
            source_headers(ui, &file.paths, self.request.label.as_deref());
            if let Some(message) = &file.comparison.message {
                ui.label(RichText::new(message).color(palette.muted));
                return;
            }
            if visible.is_empty() {
                empty_display(
                    ui,
                    ui.available_height(),
                    if self.search.is_empty() {
                        "No differences"
                    } else {
                        "No matching lines"
                    },
                    if self.search.is_empty() {
                        "The selected comparison options found no differences."
                    } else {
                        "Change the search text or comparison options."
                    },
                );
                return;
            }
            let max_line = file
                .comparison
                .rows
                .iter()
                .flat_map(|row| [row.left.as_ref(), row.right.as_ref()])
                .flatten()
                .map(|(number, _)| *number)
                .max()
                .unwrap_or(1);
            let gutter_extra = file_style::gutter_extra(max_line);
            let row_height = comparison_row_height(ui);
            if file.pending_navigation {
                file.scroll_y = navigation_offset(file.navigation, &visible, row_height)
                    .unwrap_or(file.scroll_y);
                file.pending_navigation = false;
            }
            file.paint_rows(
                ui,
                &visible,
                gutter_extra,
                self.options.ignore_line_endings,
                row_height,
            );
        });
        back
    }
}

pub(super) fn visible_rows(
    comparison: &FileComparison,
    options: Options,
    search: &str,
) -> Vec<usize> {
    comparison
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            (!options.differences_only || row.state != DirectoryEntryState::Same)
                && row_matches(row, search)
        })
        .map(|(index, _)| index)
        .collect()
}

fn row_matches(row: &FileComparisonRow, search: &str) -> bool {
    if search.is_empty() {
        return true;
    }
    let needle = search.to_lowercase();
    [row.left.as_ref(), row.right.as_ref()]
        .into_iter()
        .flatten()
        .any(|(_, text)| text.to_lowercase().contains(&needle))
}

pub(super) fn next_difference(
    current: Option<usize>,
    differences: &[usize],
    forward: bool,
) -> Option<usize> {
    let Some(current) = current else {
        return if forward {
            differences.first().copied()
        } else {
            differences.last().copied()
        };
    };
    if forward {
        differences
            .iter()
            .copied()
            .find(|index| *index > current)
            .or_else(|| differences.first().copied())
    } else {
        differences
            .iter()
            .copied()
            .rev()
            .find(|index| *index < current)
            .or_else(|| differences.last().copied())
    }
}

pub(super) fn navigation_offset(
    selected: Option<usize>,
    visible: &[usize],
    row_height: f32,
) -> Option<f32> {
    selected
        .and_then(|row| visible.iter().position(|item| *item == row))
        .map(|position| position as f32 * row_height)
}
