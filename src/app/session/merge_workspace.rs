//! Merge-workspace coordination and retained editable source lifecycle.

use super::*;

impl Session {
    pub(in crate::app) fn sync_file_search_when_clean(&mut self) {
        let Some(view) = self.interactive.as_ref().filter(|view| !view.any_dirty()) else {
            return;
        };
        if let Some(browser) = &mut self.merge_file_search {
            browser.sync_working_files(view);
        }
    }

    pub(in crate::app) fn poll_merge_save_owners(&mut self, ctx: &egui::Context) {
        if let (Some(browser), Some(view)) = (&mut self.merge_folders, &mut self.interactive) {
            browser.poll_save(view, ctx);
        }
        if let (Some(browser), Some(view)) = (&mut self.merge_file_search, &mut self.interactive) {
            browser.poll_save(view, ctx);
        }
        self.sync_file_search_when_clean();
        let external_save_busy = self
            .merge_folders
            .as_ref()
            .is_some_and(FolderBrowser::is_saving)
            || self
                .merge_file_search
                .as_ref()
                .is_some_and(FolderBrowser::is_saving);
        if let Some(view) = &mut self.interactive {
            view.set_external_save_busy(external_save_busy);
        }
    }

    pub(in crate::app) fn set_workspace_merge(&mut self, merge: bool) {
        if self.merge_mode == merge {
            return;
        }
        self.merge_mode = merge;
        // A folder result produced in the other mode has different comparison
        // preferences. Force its replacement when Merge is entered again.
        self.folder_diff_settings = None;
        if merge {
            if self.merge_folders.is_some() {
                self.tree = None;
                self.merge_folders
                    .as_mut()
                    .unwrap()
                    .invalidate_comparison_cache();
            }
            if let Some(job) = self.job.take() {
                job.cancellation.store(true, Ordering::Relaxed);
            }
            if let Some(view) = &mut self.file_view {
                if let Some(job) = view.job.take() {
                    job.cancellation.store(true, Ordering::Relaxed);
                }
            }
            self.interactive
                .get_or_insert_with(InteractiveView::default);
            if !self.merge_file_open && self.merge_folders.is_none() {
                if let Some(file) = &self.file_view {
                    if file.from_folders {
                        if let Some(roots) = &self.roots {
                            self.merge_folders = Some(FolderBrowser::new(roots.clone()));
                        }
                    }
                    let paths = file.paths.clone();
                    self.interactive.as_mut().unwrap().open_pair(paths);
                    self.merge_file_open = true;
                    self.merge_file_sides = [true; 2];
                } else {
                    self.open_interactive_sources();
                }
            }
        } else if !self.merge_file_open && self.merge_folders.is_some() && self.roots.is_some() {
            self.start_comparison();
        }
        let prefs = [
            self.show_only_differences,
            self.ignore_whitespace,
            self.ignore_line_endings,
        ];
        if let Some(view) = &mut self.interactive {
            view.set_editing(merge, prefs);
        }
    }

    pub(in crate::app) fn open_interactive_sources(&mut self) {
        self.sync_file_search_when_clean();
        let folder_sources: [Option<PathBuf>; 2] = std::array::from_fn(|side| {
            (self.source_kinds[side] == Some(SourceKind::Folder))
                .then(|| self.source_paths[side].clone())
                .flatten()
        });
        let previous_roots = self.merge_folders.as_ref().map(FolderBrowser::source_roots);
        let view = self
            .interactive
            .get_or_insert_with(InteractiveView::default);
        view.set_editing(
            self.merge_mode,
            [
                self.show_only_differences,
                self.ignore_whitespace,
                self.ignore_line_endings,
            ],
        );
        let current = view.current_sources();
        let files = std::array::from_fn(|side| match self.source_kinds[side] {
            Some(SourceKind::File) => self.source_paths[side].clone(),
            Some(SourceKind::Folder)
                if self.merge_file_sides[side]
                    && previous_roots
                        .as_ref()
                        .is_some_and(|roots| roots[side] == folder_sources[side]) =>
            {
                current[side].clone()
            }
            _ => None,
        });
        self.merge_file_sides = files.each_ref().map(Option::is_some);
        self.merge_file_open = self.merge_file_sides.iter().any(|open| *open);
        view.open_sources(files);
        if folder_sources.iter().any(Option::is_some) {
            if let Some(browser) = &mut self.merge_folders {
                browser.set_sources(folder_sources.clone());
            } else {
                self.merge_folders = Some(FolderBrowser::new_with_sources(folder_sources.clone()));
            }
            // A file-search save owns a receiver and must be allowed to drain
            // even if the workspace changes into a folder merge. Retire an idle
            // owner, but retain pending saves and errors for status reporting.
            if self.merge_file_search.as_ref().is_some_and(|browser| {
                !browser.is_saving() && !browser.any_dirty() && browser.error_details().is_empty()
            }) {
                self.merge_file_search = None;
            }
        } else {
            self.merge_folders = None;
            let sources = view.current_sources();
            if let Some(browser) = &mut self.merge_file_search {
                browser.set_search_sources(sources);
            }
        }
        self.roots = match folder_sources.clone() {
            [Some(left), Some(right)] => Some([left, right]),
            _ => None,
        };
        if previous_roots.as_ref() != Some(&folder_sources) {
            self.folder_diff_settings = None;
        }
    }

    pub(in crate::app) fn sync_folder_merge_diff(&mut self) {
        if !self.merge_mode || self.merge_folders.is_none() {
            return;
        }
        let Some(view) = &self.interactive else {
            return;
        };
        if !view.run_diff_enabled() || self.source_kinds != [Some(SourceKind::Folder); 2] {
            if let Some(job) = self.job.take() {
                job.cancellation.store(true, Ordering::Relaxed);
            }
            self.folder_diff_settings = None;
            if self.tree.take().is_some() {
                self.merge_folders
                    .as_mut()
                    .unwrap()
                    .invalidate_comparison_cache();
            }
            return;
        }
        let settings = view.merge_options();
        let compare_changed = self
            .folder_diff_settings
            .is_none_or(|previous| previous[1..] != settings[1..]);
        self.folder_diff_settings = Some(settings);
        if compare_changed {
            let old = [
                self.show_only_differences,
                self.ignore_whitespace,
                self.ignore_line_endings,
            ];
            self.ignore_whitespace = settings[1];
            self.ignore_line_endings = settings[2];
            self.start_comparison();
            self.show_only_differences = old[0];
            self.ignore_whitespace = old[1];
            self.ignore_line_endings = old[2];
        }
    }
    pub(in crate::app) fn sync_working_folder_diff(&mut self, ctx: &egui::Context) {
        if let (Some(browser), Some(view)) = (&mut self.merge_folders, &self.interactive) {
            browser.poll_comparisons(view, ctx, self.merge_mode);
        }
    }
}
