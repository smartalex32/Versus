//! Difference filtering, refresh, and file/folder navigation.

use super::*;
use crate::app::visuals::status::StatusIcon;

impl Session {
    pub(in crate::app) fn recompare(&mut self) {
        if self.source_jobs.iter().any(Option::is_some) {
            return; // Pending source classification will use the latest options.
        }
        if self
            .source_kinds
            .iter()
            .all(|kind| *kind == Some(SourceKind::Folder))
        {
            self.start_comparison();
        }
        self.reload_file(false);
    }

    pub(in crate::app) fn refresh_file(&mut self) {
        self.reload_file(true);
    }

    pub(in crate::app) fn reload_file(&mut self, refresh_types: bool) {
        if self.source_jobs.iter().any(Option::is_some) {
            return;
        }
        let options = self.file_options();
        if let Some(view) = &mut self.file_view {
            if refresh_types {
                view.error_icon = StatusIcon::Error;
            }
            let ready = view.from_folders
                || self
                    .source_kinds
                    .iter()
                    .all(|kind| *kind == Some(SourceKind::File));
            let mut refreshed = Self::create_file_view(
                view.paths.clone(),
                view.sources.clone(),
                view.from_folders,
                matches!(view.error_icon, StatusIcon::TypeMismatch),
                ready,
                options,
            );
            if let Some(job) = view.job.take() {
                job.cancellation.store(true, Ordering::Relaxed);
            }
            view.job = refreshed.job.take();
            if !matches!(view.error_icon, StatusIcon::TypeMismatch) {
                view.error = None;
            }
        }
    }

    pub(in crate::app) fn jump_difference(&mut self, forward: bool) {
        if let Some(view) = &mut self.file_view {
            let top = (view.scroll_y / view.row_height).floor() as usize;
            let anchor = view
                .navigation_row
                .or_else(|| view.visible_rows.get(top).copied());
            let target = if forward {
                view.difference_rows.iter().copied().find(|row| {
                    anchor.is_none_or(|anchor| {
                        if view.navigation_row.is_some() {
                            *row > anchor
                        } else {
                            *row >= anchor
                        }
                    })
                })
            } else {
                view.difference_rows
                    .iter()
                    .rev()
                    .copied()
                    .find(|row| anchor.is_some_and(|anchor| *row < anchor))
            };
            if let Some(target) = target {
                if let Some(index) = view.visible_rows.iter().position(|row| *row == target) {
                    view.scroll_y = index.saturating_sub(1) as f32 * view.row_height;
                    view.navigation_row = Some(target);
                    view.navigation_scroll_pending = true;
                }
            }
            return;
        }
        let Some(tree) = &mut self.tree else { return };
        let all = tree.all_rows();
        let anchor = self
            .selected
            .as_ref()
            .and_then(|path| all.iter().position(|row| &row.node.relative_path == path))
            .or_else(|| {
                let visible: Vec<_> = tree
                    .visible_rows()
                    .into_iter()
                    .filter(|row| {
                        !self.show_only_differences || row.node.state != DirectoryEntryState::Same
                    })
                    .collect();
                let top = (self.tree_scroll_y / self.row_height).floor() as usize;
                visible.get(top).and_then(|row| {
                    all.iter()
                        .position(|item| item.node.relative_path == row.node.relative_path)
                })
            });
        let selected = self.selected.is_some();
        let target = if forward {
            all.iter().enumerate().find(|(index, row)| {
                is_file_difference(row.node)
                    && anchor.is_none_or(|anchor| {
                        if selected {
                            *index > anchor
                        } else {
                            *index >= anchor
                        }
                    })
            })
        } else {
            all.iter().enumerate().rev().find(|(index, row)| {
                is_file_difference(row.node) && anchor.is_some_and(|anchor| *index < anchor)
            })
        }
        .map(|(_, row)| row.node.relative_path.clone());
        if let Some(path) = target {
            tree.expand_parents(&path);
            let visible: Vec<_> = tree
                .visible_rows()
                .into_iter()
                .filter(|row| {
                    !self.show_only_differences || row.node.state != DirectoryEntryState::Same
                })
                .collect();
            if let Some(index) = visible
                .iter()
                .position(|row| row.node.relative_path == path)
            {
                self.tree_scroll_y = index.saturating_sub(1) as f32 * self.row_height;
            }
            self.selected = Some(path);
        }
    }

    pub(in crate::app) fn set_differences_only(&mut self, enabled: bool) {
        let previous: Vec<PathBuf> = self
            .tree
            .as_ref()
            .map(|tree| {
                tree.visible_rows()
                    .into_iter()
                    .filter(|row| {
                        !self.show_only_differences || row.node.state != DirectoryEntryState::Same
                    })
                    .map(|row| row.node.relative_path.clone())
                    .collect()
            })
            .unwrap_or_default();
        let top = (self.tree_scroll_y / self.row_height).floor() as usize;
        let fraction = self.tree_scroll_y % self.row_height;
        self.show_only_differences = enabled;
        if let Some(view) = &mut self.file_view {
            update_visible_file_rows(view, enabled);
        }
        if let Some(tree) = &self.tree {
            let next: Vec<_> = tree
                .visible_rows()
                .into_iter()
                .filter(|row| !enabled || row.node.state != DirectoryEntryState::Same)
                .map(|row| &row.node.relative_path)
                .collect();
            let positions: std::collections::HashMap<_, _> = next
                .iter()
                .enumerate()
                .map(|(index, path)| (*path, index))
                .collect();
            // Keep the top entry, or its closest following/preceding survivor.
            let anchor = previous
                .iter()
                .skip(top)
                .chain(previous.iter().take(top).rev())
                .find_map(|path| positions.get(path).copied());
            self.tree_scroll_y =
                anchor.map_or(0.0, |index| index as f32 * self.row_height + fraction);
        }
    }

    pub(in crate::app) fn navigation_status(&self) -> String {
        let (current, total) = if let Some(view) = &self.file_view {
            (
                view.navigation_row.and_then(|row| {
                    view.difference_rows
                        .iter()
                        .position(|candidate| *candidate == row)
                }),
                view.difference_rows.len(),
            )
        } else {
            (
                self.selected.as_ref().and_then(|path| {
                    self.folder_differences
                        .iter()
                        .position(|candidate| candidate == path)
                }),
                self.folder_differences.len(),
            )
        };
        current.map_or_else(
            || format!("{total} changes"),
            |index| format!("Change {} of {total}", index + 1),
        )
    }

    pub(in crate::app) fn comparison_counts(&self) -> Option<[usize; 6]> {
        if let Some(view) = &self.file_view {
            view.comparison.as_ref().map(|_| view.counts)
        } else {
            self.tree.as_ref().map(|_| self.counts)
        }
    }
}

pub(in crate::app) fn file_line_anchor(
    view: &FileView,
    index: Option<usize>,
) -> [Option<usize>; 2] {
    index
        .and_then(|index| view.comparison.as_ref()?.rows.get(index))
        .map(|row| {
            [
                row.left.as_ref().map(|line| line.0),
                row.right.as_ref().map(|line| line.0),
            ]
        })
        .unwrap_or([None; 2])
}

pub(in crate::app) fn find_file_line(
    comparison: &FileComparison,
    anchor: [Option<usize>; 2],
) -> Option<usize> {
    let side = if anchor[0].is_some() { 0 } else { 1 };
    let number = anchor[side]?;
    comparison.rows.iter().position(|row| {
        let line = if side == 0 { &row.left } else { &row.right };
        line.as_ref().is_some_and(|line| line.0 == number)
    })
}

pub(in crate::app) fn update_visible_file_rows(view: &mut FileView, differences_only: bool) {
    let fraction = view.scroll_y % view.row_height;
    let anchor = view
        .visible_rows
        .get((view.scroll_y / view.row_height).floor() as usize)
        .copied();
    let Some(comparison) = &view.comparison else {
        return;
    };
    view.difference_rows = comparison
        .rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| (row.state != DirectoryEntryState::Same).then_some(index))
        .collect();
    view.visible_rows = if differences_only {
        view.difference_rows.clone()
    } else {
        (0..comparison.rows.len()).collect()
    };
    let top = anchor
        .and_then(|anchor| view.visible_rows.iter().position(|index| *index >= anchor))
        .unwrap_or_else(|| view.visible_rows.len().saturating_sub(1));
    view.scroll_y = if view.visible_rows.is_empty() {
        0.0
    } else {
        top as f32 * view.row_height + fraction
    };
}
