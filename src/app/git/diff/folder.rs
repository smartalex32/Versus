//! Git directory-diff list and file drill-in.

use super::*;
use crate::app::visuals::panels::{comparison_panel, empty_display};
use crate::app::visuals::theme::{Palette, ROW_HEIGHT};
use eframe::egui::{self, RichText};
use versus::{DirectoryEntryKind, DirectoryEntryState};

impl GitDiffView {
    pub(super) fn render_folder(
        &mut self,
        ui: &mut egui::Ui,
        folder: &mut FolderScreen,
    ) -> Option<([PathBuf; 2], [bool; 2])> {
        let palette = Palette::for_context(ui.ctx());
        let mut open = None;
        let mut toggle = None;
        comparison_panel(ui).show(ui, |ui| {
            source_headers(ui, &self.request.paths, self.request.label.as_deref());
            if let Some(error) = &folder.error {
                ui.colored_label(Palette::for_context(ui.ctx()).error, error);
            }
            let rows: Vec<_> = matching_rows(&folder.tree, self.options, &self.search)
                .into_iter()
                .map(|row| {
                    (
                        row.node.relative_path.clone(),
                        row.node.name.clone(),
                        row.node.left.clone(),
                        row.node.right.clone(),
                        row.node.state.clone(),
                        row.node.is_expandable(),
                        row.depth,
                    )
                })
                .collect();
            if rows.is_empty() {
                empty_display(
                    ui,
                    ui.available_height(),
                    if self.search.is_empty() {
                        "No differences"
                    } else {
                        "No matching files"
                    },
                    "Change the search text or comparison options.",
                );
                return;
            }
            egui::ScrollArea::vertical()
                .id_salt("git-diff-folder")
                .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                    for index in range {
                        let (path, name, left, right, state, expandable, depth) = &rows[index];
                        let selected = folder.selected.as_ref() == Some(path);
                        let label = format!(
                            "{}{}{}",
                            "  ".repeat(*depth),
                            if *expandable && folder.tree.is_expanded(path) {
                                "▾ "
                            } else if *expandable {
                                "▸ "
                            } else {
                                "  "
                            },
                            name.to_string_lossy()
                        );
                        let response = ui.add(
                            egui::Button::selectable(
                                selected,
                                RichText::new(label).color(if *expandable {
                                    palette.text
                                } else {
                                    palette.muted
                                }),
                            )
                            .frame(false),
                        );
                        if response.clicked() {
                            if *expandable {
                                toggle = Some(path.clone());
                            } else if left.kind == Some(DirectoryEntryKind::File)
                                || right.kind == Some(DirectoryEntryKind::File)
                            {
                                open = Some((path.clone(), [left.exists, right.exists]));
                            }
                            folder.selected = Some(path.clone());
                        }
                        response.on_hover_text(format!(
                            "{} — {}",
                            path.display(),
                            state_label(state)
                        ));
                    }
                });
        });
        if let Some(path) = toggle {
            folder.tree.toggle_expanded(path);
        }
        open.map(|(path, exists)| {
            (
                self.request.paths.clone().map(|root| root.join(&path)),
                exists,
            )
        })
    }
}

pub(super) fn matching_rows<'a>(
    tree: &'a versus::FolderTree,
    options: Options,
    search: &str,
) -> Vec<versus::VisibleTreeRow<'a>> {
    let rows = if search.is_empty() {
        tree.visible_rows()
    } else {
        tree.all_rows()
    };
    rows.into_iter()
        .filter(|row| {
            (!options.differences_only || row.node.state != DirectoryEntryState::Same)
                && path_matches(&row.node.relative_path, search)
        })
        .collect()
}

fn path_matches(path: &std::path::Path, search: &str) -> bool {
    search.is_empty()
        || path
            .to_string_lossy()
            .to_lowercase()
            .contains(&search.to_lowercase())
}
