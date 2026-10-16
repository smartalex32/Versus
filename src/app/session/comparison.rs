//! File and folder comparison workers and ordered result publication.

//!

//! The workers do filesystem work; `Session` publishes their results only after

//! checking that the job still belongs to the active comparison.

use super::*;
use crate::app::visuals::status::StatusIcon;
use crate::app::visuals::theme::{ROW_HEIGHT, comparison_font};
use versus::{DirectoryEntryKind, TreeNode};

impl Session {
    pub(in crate::app) fn start_comparison(&mut self) {
        if self.paths.iter().any(|path| path.trim().is_empty()) {
            return;
        }
        if let Some(job) = self.job.take() {
            job.cancellation.store(true, Ordering::Relaxed);
        }
        let roots = self.comparison_paths();
        self.record_recent(ComparisonMode::Folder, roots.clone());
        let worker_roots = roots.clone();
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker_cancellation = cancellation.clone();
        let (sender, receiver) = mpsc::channel();
        let progress = Arc::new(ComparisonProgress::default());
        let worker_progress = progress.clone();
        let ignore_whitespace = self.ignore_whitespace;
        let ignore_line_endings = self.ignore_line_endings;
        crate::diagnostics::info(format!(
            "Folder comparison started: left={} right={} ignore_whitespace={ignore_whitespace} ignore_line_endings={ignore_line_endings}",
            roots[0].display(),
            roots[1].display()
        ));
        std::thread::spawn(move || {
            let result = crate::diagnostics::catch_worker("Folder comparison", || {
                let diff = versus::compare_directories(
                    &worker_roots[0],
                    &worker_roots[1],
                    &DirectoryCompareOptions {
                        cancellation: Some(worker_cancellation.clone()),
                        ignore_whitespace,
                        ignore_line_endings,
                        progress: Some(worker_progress.clone()),
                        ..Default::default()
                    },
                )?;
                crate::diagnostics::info(format!(
                    "Folder content comparison finished: {} entries; cancelled={}; building tree",
                    diff.entries.len(),
                    diff.cancelled
                ));
                worker_progress.begin(ProgressStage::BuildingTree, None);
                if worker_cancellation.load(Ordering::Relaxed) {
                    return Ok(FolderTree::from_diff(&versus::DirectoryDiff {
                        entries: vec![],
                        cancelled: true,
                    }));
                }
                let tree = FolderTree::from_diff(&diff);
                worker_progress.begin(ProgressStage::Finished, Some(diff.entries.len() as u64));
                worker_progress.advance(diff.entries.len() as u64);
                Ok::<_, CompareError>(tree)
            })
            .map_err(|message| CompareError {
                path: None,
                kind: versus::CompareErrorKind::Io,
                message,
            });
            let _ = sender.send(result);
        });
        self.job = Some(ComparisonJob {
            receiver,
            cancellation,
            roots,
            started: Instant::now(),
            progress,
            logged_progress: None,
            last_progress_log: Instant::now(),
        });
        self.error = None;
        self.message = "Scanning folders and comparing file contents…".into();
    }

    pub(in crate::app) fn poll_comparison(&mut self, ctx: &egui::Context) {
        let Some(job) = &mut self.job else {
            return;
        };
        let snapshot = job.progress.snapshot();
        let state = (snapshot.stage, snapshot.completed, snapshot.total);
        if job.logged_progress != Some(state)
            && (job
                .logged_progress
                .is_none_or(|last| last.0 != snapshot.stage)
                || job.last_progress_log.elapsed() >= Duration::from_secs(1))
        {
            crate::diagnostics::info(format!(
                "Folder comparison progress: {:?} {}/{}",
                snapshot.stage,
                snapshot.completed,
                snapshot
                    .total
                    .map_or_else(|| "unknown".into(), |total| total.to_string())
            ));
            job.logged_progress = Some(state);
            job.last_progress_log = Instant::now();
        }
        let result = match job.receiver.try_recv() {
            Ok(result) => Some(result.map_err(|error| error.to_string())),
            Err(TryRecvError::Disconnected) => Some(Err(
                "The comparison worker stopped unexpectedly. Try comparing again.".into(),
            )),
            Err(TryRecvError::Empty) => None,
        };
        if let Some(result) = result {
            let job = self.job.take().unwrap();
            if job.cancellation.load(Ordering::Relaxed) {
                crate::diagnostics::info("Folder comparison cancelled");
                self.message = "Comparison cancelled.".into();
                return;
            }
            match result {
                Ok(mut tree) => {
                    crate::diagnostics::info(format!(
                        "Folder comparison result received after {:.3}s",
                        job.started.elapsed().as_secs_f64()
                    ));
                    let same_roots = self.roots.as_ref() == Some(&job.roots);
                    if same_roots {
                        let anchor = self.tree.as_ref().and_then(|previous| {
                            previous
                                .visible_rows()
                                .into_iter()
                                .filter(|row| {
                                    !self.show_only_differences
                                        || row.node.state != DirectoryEntryState::Same
                                })
                                .nth((self.tree_scroll_y / self.row_height).floor() as usize)
                                .map(|row| row.node.relative_path.clone())
                        });
                        if let Some(previous) = &self.tree {
                            let expanded: Vec<_> = tree
                                .all_rows()
                                .into_iter()
                                .filter(|row| previous.is_expanded(&row.node.relative_path))
                                .map(|row| row.node.relative_path.clone())
                                .collect();
                            for path in expanded {
                                tree.set_expanded(path, true);
                            }
                        }
                        if self.selected.as_ref().is_some_and(|path| {
                            !tree
                                .all_rows()
                                .iter()
                                .any(|row| &row.node.relative_path == path)
                        }) {
                            self.selected = None;
                        }
                        if let Some(top) = anchor.and_then(|path| {
                            tree.visible_rows()
                                .into_iter()
                                .filter(|row| {
                                    !self.show_only_differences
                                        || row.node.state != DirectoryEntryState::Same
                                })
                                .position(|row| row.node.relative_path == path)
                        }) {
                            self.tree_scroll_y =
                                top as f32 * self.row_height + self.tree_scroll_y % self.row_height;
                        }
                    } else {
                        self.selected = None;
                        self.tree_scroll_y = 0.0;
                        self.scroll_generation += 1;
                    }
                    self.folder_differences = tree
                        .all_rows()
                        .into_iter()
                        .filter(|row| is_file_difference(row.node))
                        .map(|row| row.node.relative_path.clone())
                        .collect();
                    self.counts = count_entries(&tree);
                    self.tree = Some(tree);
                    if let Some(browser) = &mut self.merge_folders {
                        browser.invalidate_comparison_cache();
                    }
                    self.roots = Some(job.roots);
                    self.elapsed = Some(job.started.elapsed());
                    self.message = "Comparison complete.".into();
                }
                Err(error) => {
                    crate::diagnostics::error(format!("Folder comparison failed: {error}"));
                    self.error = Some(error);
                    self.message =
                        "Comparison failed. Check the folder paths and access permissions.".into();
                }
            }
        } else {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    pub(in crate::app) fn open_file(
        &mut self,
        relative_path: PathBuf,
        present: [bool; 2],
        type_mismatch: bool,
    ) {
        let Some(roots) = &self.roots else { return };
        let paths = roots.clone().map(|root| root.join(&relative_path));
        let sources = std::array::from_fn(|side| present[side].then(|| paths[side].clone()));
        self.file_view = Some(Self::create_file_view(
            paths,
            sources,
            true,
            type_mismatch,
            true,
            self.file_options(),
        ));
    }

    pub(in crate::app) fn create_file_view(
        paths: [PathBuf; 2],
        sources: [Option<PathBuf>; 2],
        from_folders: bool,
        type_mismatch: bool,
        ready: bool,
        mut options: FileViewOptions,
    ) -> FileView {
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker_cancellation = cancellation.clone();
        let worker_sources = sources.clone();
        let worker_paths = paths.clone();
        let (sender, receiver) = mpsc::channel();
        let progress = Arc::new(ComparisonProgress::default());
        options.progress = Some(progress.clone());
        let ignore_line_endings = options.ignore_line_endings;
        if ready && !type_mismatch {
            std::thread::spawn(move || {
                let sources = if from_folders {
                    std::array::from_fn(|side| {
                        // Only a missing path represents an empty side. Keep other
                        // inspection failures visible through the guarded loader.
                        (!std::fs::symlink_metadata(&worker_paths[side])
                            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound))
                        .then(|| worker_paths[side].clone())
                    })
                } else {
                    worker_sources
                };
                let result =
                    load_file_comparison_with_options(&sources, &worker_cancellation, &options)
                        .map(|comparison| {
                            comparison.map(|comparison| LoadedFile {
                                comparison,
                                sources,
                            })
                        });
                let _ = sender.send(result);
            });
        }
        FileView {
            row_height: ROW_HEIGHT,
            from_folders,
            paths,
            sources,
            job: (ready && !type_mismatch).then_some(FileJob {
                receiver,
                cancellation,
                progress,
                ignore_line_endings,
            }),
            comparison: None,
            error: type_mismatch.then(|| "Entry types differ. Line comparison requires regular files; folders and symlink targets are not opened.".into()),
            error_icon: if type_mismatch { StatusIcon::TypeMismatch } else { StatusIcon::Error },
            scroll_y: 0.0,
            content_widths: [0.0; 2],
            gutter_extra: 0.0,
            counts: [0; 6],
            visible_rows: Vec::new(),
            difference_rows: Vec::new(),
            navigation_row: None,
            navigation_scroll_pending: false,
            ignore_line_endings,
        }
    }

    pub(in crate::app) fn poll_file_comparison(&mut self, ctx: &egui::Context) {
        let Some(view) = &mut self.file_view else {
            return;
        };
        let Some(job) = &view.job else { return };
        let result = match job.receiver.try_recv() {
            Ok(result) => Some(result.map_err(|error| error.to_string())),
            Err(TryRecvError::Disconnected) => Some(Err(
                "The file comparison worker stopped unexpectedly.".into(),
            )),
            Err(TryRecvError::Empty) => None,
        };
        if let Some(result) = result {
            let job = view.job.take().unwrap();
            if job.cancellation.load(Ordering::Relaxed) {
                view.error = Some("File comparison cancelled.".into());
                return;
            }
            match result {
                Ok(Some(loaded)) => {
                    let comparison = loaded.comparison;
                    view.sources = loaded.sources;
                    // Ignore rules can change row alignment. Restore the viewport
                    // and navigation by source line number, rather than row index.
                    let anchor = file_line_anchor(
                        view,
                        view.visible_rows
                            .get((view.scroll_y / view.row_height).floor() as usize)
                            .copied(),
                    );
                    let navigation = file_line_anchor(view, view.navigation_row);
                    let fraction = view.scroll_y % view.row_height;
                    view.counts = [0; 6];
                    view.content_widths = [0.0; 2];
                    let font_width = comparison_font(ctx).size;
                    let mut max_line_number = 1;
                    for row in &comparison.rows {
                        view.counts[state_index(&row.state)] += 1;
                        for (side, line) in [&row.left, &row.right].into_iter().enumerate() {
                            if let Some((number, text)) = line {
                                max_line_number = max_line_number.max(*number);
                                view.content_widths[side] = view.content_widths[side].max(
                                    (file_style::TEXT_OFFSET + 28.0)
                                        + (text.len() + text.matches('\t').count() * 3) as f32
                                            * font_width,
                                );
                            }
                        }
                    }
                    view.gutter_extra = file_style::gutter_extra(max_line_number);
                    for width in &mut view.content_widths {
                        if *width > 0.0 {
                            *width += view.gutter_extra;
                        }
                    }
                    view.comparison = Some(comparison);
                    view.ignore_line_endings = job.ignore_line_endings;
                    view.error = None;
                    update_visible_file_rows(view, self.show_only_differences);
                    let comparison = view.comparison.as_ref().unwrap();
                    if let Some(row) = find_file_line(comparison, anchor) {
                        let top = view
                            .visible_rows
                            .iter()
                            .position(|index| *index >= row)
                            .unwrap_or_else(|| view.visible_rows.len().saturating_sub(1));
                        view.scroll_y = if view.visible_rows.is_empty() {
                            0.0
                        } else {
                            top as f32 * view.row_height + fraction
                        };
                    }
                    view.navigation_row = find_file_line(comparison, navigation)
                        .filter(|index| view.difference_rows.contains(index));
                    view.navigation_scroll_pending = true;
                }
                Ok(None) => view.error = Some("File comparison cancelled.".into()),
                Err(error) => view.error = Some(error),
            }
        } else {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    pub(in crate::app) fn file_options(&self) -> FileViewOptions {
        FileViewOptions {
            ignore_whitespace: self.ignore_whitespace,
            ignore_line_endings: self.ignore_line_endings,
            progress: None,
        }
    }
}

pub(in crate::app) fn is_file_difference(node: &TreeNode) -> bool {
    node.state != DirectoryEntryState::Same
        && (node.left.kind == Some(DirectoryEntryKind::File)
            || node.right.kind == Some(DirectoryEntryKind::File))
}

fn count_entries(tree: &FolderTree) -> [usize; 6] {
    let mut counts = [0; 6];
    let mut nodes: Vec<_> = tree.root().children.iter().collect();
    while let Some(node) = nodes.pop() {
        let index = state_index(&node.state);
        counts[index] += 1;
        nodes.extend(&node.children);
    }
    counts
}

fn state_index(state: &DirectoryEntryState) -> usize {
    match state {
        DirectoryEntryState::Same => 0,
        DirectoryEntryState::Different => 1,
        DirectoryEntryState::LeftOnly => 2,
        DirectoryEntryState::RightOnly => 3,
        DirectoryEntryState::TypeMismatch => 4,
        DirectoryEntryState::Error(_) => 5,
    }
}
