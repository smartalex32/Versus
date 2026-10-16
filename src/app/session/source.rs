//! Source classification, comparison-mode selection, and source lifecycle.

//!

//! These methods own the transition from user-selected paths to a file, folder,

//! or merge workspace. Classification work only mutates the current job.

use super::*;

impl Session {
    pub(in crate::app) fn open_recent(&mut self, entry: RecentComparison) {
        self.sync_file_search_when_clean();
        if self
            .interactive
            .as_ref()
            .is_some_and(|view| view.any_dirty() || view.is_saving())
            || self
                .merge_folders
                .as_ref()
                .is_some_and(|browser| browser.any_dirty() || browser.is_saving())
            || self
                .merge_file_search
                .as_ref()
                .is_some_and(FolderBrowser::is_saving)
        {
            self.pending_merge_exit = Some(MergeExit::Recent(entry));
            return;
        }
        self.new_comparison();
        self.comparison_mode = entry.mode;
        for (side, path) in entry.paths.into_iter().enumerate() {
            self.select_path(side, path);
        }
        self.launch_mode = Some(Some(entry.mode));
    }

    pub(in crate::app) fn open_launch_request(&mut self, launch: crate::cli::LaunchRequest) {
        self.new_comparison();
        self.comparison_mode = launch.mode.unwrap_or(ComparisonMode::File);
        for (side, path) in launch.paths.into_iter().enumerate() {
            if launch.mode != Some(ComparisonMode::Folder) && empty_diff_side(&path) {
                // Git represents an added/deleted side with a null-device name.
                // Model it as absent; never open the device itself.
                self.paths[side] = path.to_string_lossy().into_owned();
                self.source_paths[side] = Some(path);
                self.source_kinds[side] = Some(SourceKind::File);
            } else {
                self.select_path(side, path);
            }
        }
        self.launch_mode = Some(launch.mode);
        self.start_selected_comparison();
    }

    pub(in crate::app) fn comparison_paths(&self) -> [PathBuf; 2] {
        std::array::from_fn(|side| {
            self.source_paths[side]
                .clone()
                .unwrap_or_else(|| absolute_path(&self.paths[side]))
        })
    }

    pub(in crate::app) fn invalidate(&mut self) {
        if let Some(browser) = &mut self.merge_folders {
            browser.invalidate_comparison_cache();
        }
        self.file_view = None;
        if let Some(job) = self.job.take() {
            job.cancellation.store(true, Ordering::Relaxed);
        }
        self.tree = None;
        self.roots = None;
        self.selected = None;
        self.tree_keyboard_focus = None;
        self.tree_focus_id = None;
        self.file_keyboard_focus_side = None;
        self.error = None;
        self.elapsed = None;
        self.counts = [0; 6];
        self.folder_differences.clear();
        self.tree_scroll_y = 0.0;
        self.message = "Source selection changed.".into();
        self.scroll_generation += 1;
    }

    pub(in crate::app) fn select_path(&mut self, side: usize, path: PathBuf) {
        self.invalidate();
        if self.merge_mode || (self.merge_file_open && self.interactive.is_some()) {
            self.merge_file_sides[side] = false;
            self.merge_file_open = self.merge_file_sides.iter().any(|open| *open);
            if let Some(view) = &mut self.interactive {
                view.open_side(side, None);
            }
        }
        self.drop_message = None;
        self.launch_mode = None;
        let path = absolute_native_path(path);
        self.paths[side] = path.to_string_lossy().into_owned();
        self.source_paths[side] = Some(path.clone());
        self.source_kinds[side] = None;
        self.source_errors[side] = None;
        self.source_jobs[side] = None;
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker_cancellation = cancellation.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            if !worker_cancellation.load(Ordering::Relaxed) {
                let _ = sender.send(selection::classify_path(&path));
            }
        });
        self.source_jobs[side] = Some(SourceJob {
            receiver,
            cancellation,
        });
        self.message = "Opening selected source…".into();
    }

    pub(in crate::app) fn poll_sources(&mut self, ctx: &egui::Context) {
        let mut changed = false;
        for side in 0..2 {
            let Some(job) = &self.source_jobs[side] else {
                continue;
            };
            let result = match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => {
                    Some(Err("The source worker stopped unexpectedly.".into()))
                }
                Err(TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.source_jobs[side] = None;
                match result {
                    Ok(kind) => self.source_kinds[side] = Some(kind),
                    Err(error) => self.source_errors[side] = Some(error),
                }
                changed = true;
            }
        }
        if changed {
            self.start_selected_comparison();
        }
        if self.source_jobs.iter().any(Option::is_some) {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    pub(in crate::app) fn mode(&self) -> SelectionMode {
        if self.file_view.is_some() {
            SelectionMode::File
        } else if self.tree.is_some() || self.job.is_some() {
            SelectionMode::Folder
        } else {
            selection::selection_mode(self.source_kinds)
        }
    }

    pub(in crate::app) fn switch_comparison_mode(&mut self, mode: ComparisonMode) {
        self.sync_file_search_when_clean();
        if self.merge_mode
            && mode == ComparisonMode::Folder
            && self.merge_folders.is_some()
            && self.source_kinds == [Some(SourceKind::Folder); 2]
        {
            self.merge_file_sides = [false; 2];
            self.merge_file_open = false;
            if let Some(view) = &mut self.interactive {
                view.open_sources([None, None]);
            }
            self.comparison_mode = mode;
            return;
        }
        if self.comparison_mode != mode
            && (self
                .interactive
                .as_ref()
                .is_some_and(|view| view.any_dirty() || view.is_saving())
                || self
                    .merge_folders
                    .as_ref()
                    .is_some_and(|browser| browser.any_dirty() || browser.is_saving())
                || self
                    .merge_file_search
                    .as_ref()
                    .is_some_and(FolderBrowser::is_saving))
        {
            self.pending_merge_exit = Some(MergeExit::Mode(mode));
            return;
        }
        if self.merge_file_open
            && mode == ComparisonMode::Folder
            && self.merge_folders.is_some()
            && self.source_kinds == [Some(SourceKind::Folder); 2]
        {
            self.merge_file_open = false;
            self.comparison_mode = mode;
            return;
        }
        if self.comparison_mode == mode {
            if mode == ComparisonMode::Folder
                && self
                    .file_view
                    .as_ref()
                    .is_some_and(|view| view.from_folders)
            {
                self.file_view = None;
            }
            return;
        }
        self.comparison_mode = mode;
        self.new_comparison();
    }

    pub(in crate::app) fn new_comparison(&mut self) {
        self.sync_file_search_when_clean();
        if self
            .interactive
            .as_ref()
            .is_some_and(InteractiveView::is_saving)
            || self
                .merge_folders
                .as_ref()
                .is_some_and(FolderBrowser::is_saving)
            || self
                .merge_file_search
                .as_ref()
                .is_some_and(FolderBrowser::is_saving)
            || self
                .interactive
                .as_ref()
                .is_some_and(InteractiveView::any_dirty)
            || self
                .merge_folders
                .as_ref()
                .is_some_and(FolderBrowser::any_dirty)
        {
            self.pending_merge_exit = Some(MergeExit::New);
            return;
        }
        self.interactive = self.merge_mode.then(InteractiveView::default);
        self.merge_folders = None;
        self.merge_file_search = None;
        self.merge_file_open = false;
        self.merge_file_sides = [false; 2];
        self.folder_diff_settings = None;
        self.invalidate();
        self.paths = Default::default();
        self.source_paths = [None, None];
        self.launch_mode = None;
        self.source_kinds = [None; 2];
        self.source_jobs = [None, None];
        self.source_errors = [None, None];
        self.drop_message = None;
        self.message = format!(
            "Choose a {} on each side to begin.",
            self.comparison_mode.source_label()
        );
    }

    pub(in crate::app) fn start_selected_comparison(&mut self) {
        self.error = self.source_errors.iter().flatten().next().cloned();
        if self.error.is_some() && !self.merge_mode {
            self.message = "Cannot open selected source. Choose another file or folder.".into();
            return;
        }
        if self.source_jobs.iter().any(Option::is_some) && !self.merge_mode {
            return;
        }
        if let Some(requested_mode) = self
            .launch_mode
            .take_if(|_| self.source_jobs.iter().all(Option::is_none))
        {
            if let Some(mode) = requested_mode {
                let expected = match mode {
                    ComparisonMode::File => SourceKind::File,
                    ComparisonMode::Folder => SourceKind::Folder,
                };
                if self.source_kinds.iter().any(|kind| *kind != Some(expected)) {
                    self.error = Some(format!(
                        "{} requires two {}. Choose matching sources.",
                        mode.label(),
                        match mode {
                            ComparisonMode::File => "regular files",
                            ComparisonMode::Folder => "folders",
                        }
                    ));
                    self.message = "Cannot compare these sources.".into();
                    return;
                }
            } else {
                self.comparison_mode = match selection::selection_mode(self.source_kinds) {
                    SelectionMode::Folder => ComparisonMode::Folder,
                    _ => ComparisonMode::File,
                };
            }
        }
        let retained_file_session = self.merge_file_open
            && self.interactive.is_some()
            && self.merge_folders.is_none()
            && selection::selection_mode(self.source_kinds) == SelectionMode::File;
        if self.merge_mode || retained_file_session {
            let recent_mode = match selection::selection_mode(self.source_kinds) {
                SelectionMode::File
                    if self
                        .source_kinds
                        .iter()
                        .all(|kind| *kind == Some(SourceKind::File)) =>
                {
                    Some(ComparisonMode::File)
                }
                SelectionMode::Folder
                    if self
                        .source_kinds
                        .iter()
                        .all(|kind| *kind == Some(SourceKind::Folder)) =>
                {
                    Some(ComparisonMode::Folder)
                }
                _ => None,
            };
            if let Some(mode) = recent_mode {
                self.record_recent(mode, self.comparison_paths());
            }
            self.open_interactive_sources();
            return;
        }
        if self.merge_file_open && self.merge_folders.is_none() {
            self.merge_file_open = false;
            self.merge_file_sides = [false; 2];
        }
        match selection::selection_mode(self.source_kinds) {
            SelectionMode::File => {
                let paths = self.comparison_paths();
                let sources = std::array::from_fn(|side| {
                    (self.source_kinds[side] == Some(SourceKind::File)
                        && !empty_diff_side(&paths[side]))
                    .then(|| paths[side].clone())
                });
                let ready = self
                    .source_kinds
                    .iter()
                    .all(|kind| *kind == Some(SourceKind::File));
                if ready && !paths.iter().any(|path| empty_diff_side(path)) {
                    self.record_recent(ComparisonMode::File, paths.clone());
                }
                self.file_view = Some(Self::create_file_view(
                    paths,
                    sources,
                    false,
                    false,
                    ready,
                    self.file_options(),
                ));
                self.message = "Choose a file on the other side to compare.".into();
            }
            SelectionMode::Folder => {
                if self
                    .source_kinds
                    .iter()
                    .all(|kind| *kind == Some(SourceKind::Folder))
                {
                    self.start_comparison();
                } else {
                    self.message = "Choose a folder on the other side to compare.".into();
                }
            }
            SelectionMode::Incompatible => {
                self.message = if self.source_kinds.contains(&Some(SourceKind::Other)) {
                    "No comparison can be made with this source type. Select regular files or folders."
                } else {
                    "No comparison can be made between a file and a folder. Select matching types."
                }.into();
            }
            SelectionMode::Empty => {}
        }
    }
}
