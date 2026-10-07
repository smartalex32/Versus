use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::{Duration, Instant},
};
use versus::{
    CompareError, DirectoryCompareOptions, DirectoryEntryKind, DirectoryEntryState, FileComparison,
    FolderTree, TreeNode, load_file_comparison,
};

#[derive(Clone, Copy)]
struct Palette {
    background: Color32,
    panel: Color32,
    border: Color32,
    muted: Color32,
    text: Color32,
    accent: Color32,
    changed: Color32,
    left_only: Color32,
    right_only: Color32,
    error: Color32,
    alternate: Color32,
    hover: Color32,
}

impl Palette {
    fn for_context(ctx: &egui::Context) -> Self {
        Self::new(ctx.theme() == egui::Theme::Dark)
    }
    fn new(dark: bool) -> Self {
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

const ROW_HEIGHT: f32 = 22.0;

struct ComparisonJob {
    receiver: Receiver<Result<FolderTree, CompareError>>,
    cancellation: Arc<AtomicBool>,
    roots: [PathBuf; 2],
    started: Instant,
}

struct FileJob {
    receiver: Receiver<Result<Option<FileComparison>, CompareError>>,
    cancellation: Arc<AtomicBool>,
}

struct FileView {
    paths: [PathBuf; 2],
    sources: [Option<PathBuf>; 2],
    job: Option<FileJob>,
    comparison: Option<FileComparison>,
    error: Option<String>,
    error_icon: StatusIcon,
    scroll_y: f32,
    content_widths: [f32; 2],
    counts: [usize; 6],
}

impl Drop for FileView {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancellation.store(true, Ordering::Relaxed);
        }
    }
}

pub struct VersusApp {
    file_view: Option<FileView>,
    paths: [String; 2],
    roots: Option<[PathBuf; 2]>,
    tree: Option<FolderTree>,
    job: Option<ComparisonJob>,
    selected: Option<PathBuf>,
    message: String,
    error: Option<String>,
    elapsed: Option<Duration>,
    counts: [usize; 6],
    logo_texture: Option<(bool, egui::TextureHandle)>,
    scroll_generation: u64,
}

impl Default for VersusApp {
    fn default() -> Self {
        Self {
            file_view: None,
            paths: Default::default(),
            roots: None,
            tree: None,
            job: None,
            selected: None,
            message: "Choose two folders to begin.".into(),
            error: None,
            elapsed: None,
            counts: [0; 6],
            logo_texture: None,
            scroll_generation: 0,
        }
    }
}

impl VersusApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_theme(&cc.egui_ctx);
        Self::default()
    }

    fn invalidate(&mut self) {
        self.file_view = None;
        if let Some(job) = self.job.take() {
            job.cancellation.store(true, Ordering::Relaxed);
        }
        self.tree = None;
        self.roots = None;
        self.selected = None;
        self.error = None;
        self.elapsed = None;
        self.counts = [0; 6];
        self.message = "Folder selection changed. Compare to load the trees.".into();
        self.scroll_generation += 1;
    }

    fn start_comparison(&mut self) {
        if self.paths.iter().any(|path| path.trim().is_empty()) {
            return;
        }
        if let Some(job) = self.job.take() {
            job.cancellation.store(true, Ordering::Relaxed);
        }
        let roots = self.paths.clone().map(|path| absolute_path(&path));
        let worker_roots = roots.clone();
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker_cancellation = cancellation.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = versus::compare_directories(
                &worker_roots[0],
                &worker_roots[1],
                &DirectoryCompareOptions {
                    cancellation: Some(worker_cancellation),
                    ..Default::default()
                },
            )
            .map(|diff| FolderTree::from_diff(&diff));
            let _ = sender.send(result);
        });
        self.job = Some(ComparisonJob {
            receiver,
            cancellation,
            roots,
            started: Instant::now(),
        });
        self.error = None;
        self.message = "Scanning folders and comparing file contents…".into();
    }

    fn poll_comparison(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.job else {
            return;
        };
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
                self.message = "Comparison cancelled.".into();
                return;
            }
            match result {
                Ok(tree) => {
                    self.counts = count_entries(&tree);
                    self.tree = Some(tree);
                    self.roots = Some(job.roots);
                    self.elapsed = Some(job.started.elapsed());
                    self.selected = None;
                    self.scroll_generation += 1;
                    self.message = "Comparison complete.".into();
                }
                Err(error) => {
                    self.error = Some(error);
                    self.message =
                        "Comparison failed. Check the folder paths and access permissions.".into();
                }
            }
        } else {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    fn open_file(&mut self, relative_path: PathBuf, present: [bool; 2], type_mismatch: bool) {
        let Some(roots) = &self.roots else { return };
        let paths = roots.clone().map(|root| root.join(&relative_path));
        let sources = std::array::from_fn(|side| present[side].then(|| paths[side].clone()));
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker_cancellation = cancellation.clone();
        let worker_sources = sources.clone();
        let (sender, receiver) = mpsc::channel();
        if !type_mismatch {
            std::thread::spawn(move || {
                let result = load_file_comparison(&worker_sources, &worker_cancellation);
                let _ = sender.send(result);
            });
        }
        self.file_view = Some(FileView {
            paths,
            sources,
            job: (!type_mismatch).then_some(FileJob {
                receiver,
                cancellation,
            }),
            comparison: None,
            error: type_mismatch.then(|| "Entry types differ. Line comparison requires regular files; folders and symlink targets are not opened.".into()),
            error_icon: if type_mismatch { StatusIcon::TypeMismatch } else { StatusIcon::Error },
            scroll_y: 0.0,
            content_widths: [0.0; 2],
            counts: [0; 6],
        });
    }

    fn poll_file_comparison(&mut self, ctx: &egui::Context) {
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
            view.job = None;
            match result {
                Ok(Some(comparison)) => {
                    for row in &comparison.rows {
                        view.counts[state_index(&row.state)] += 1;
                        for (side, line) in [&row.left, &row.right].into_iter().enumerate() {
                            if let Some((_, text)) = line {
                                view.content_widths[side] = view.content_widths[side].max(
                                    96.0 + (text.len() + text.matches('\t').count() * 3) as f32
                                        * 8.0,
                                );
                            }
                        }
                    }
                    view.comparison = Some(comparison);
                }
                Ok(None) => view.error = Some("File comparison cancelled.".into()),
                Err(error) => view.error = Some(error),
            }
        } else {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    fn render(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let dark = ui.visuals().dark_mode;
        if self
            .logo_texture
            .as_ref()
            .is_none_or(|(theme, _)| *theme != dark)
        {
            let icon = crate::logo::themed_icon(dark);
            // PNG/window-icon bytes have straight alpha; egui textures need
            // premultiplied alpha, including nearly transparent edge pixels.
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            );
            self.logo_texture = Some((
                dark,
                ui.ctx()
                    .load_texture("versus-mark", image, egui::TextureOptions::LINEAR),
            ));
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Icon(Some(Arc::new(icon))));
        }
        egui::Frame::default()
            .fill(palette.background)
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                self.header(ui);
                ui.add_space(5.0);
                self.legend(ui);
                ui.add_space(5.0);
                if self.file_view.is_some() {
                    self.file_area(ui);
                    return;
                }
                if let Some(error) = &self.error {
                    egui::Frame::default()
                        .fill(palette.error.gamma_multiply(0.12))
                        .inner_margin(egui::Margin::same(6))
                        .show(ui, |ui| {
                            ui.colored_label(palette.error, error);
                        });
                    ui.add_space(4.0);
                }
                let available = ui.available_rect_before_wrap();
                let footer_height = ui.spacing().interact_size.y
                    + ui.text_style_height(&egui::TextStyle::Monospace).ceil()
                    + ui.spacing().item_spacing.y;
                let footer_rect = Rect::from_min_max(
                    egui::pos2(
                        available.left(),
                        (available.bottom() - footer_height).max(available.top()),
                    ),
                    available.max,
                );
                let tree_rect = Rect::from_min_max(
                    available.min,
                    egui::pos2(
                        available.right(),
                        (footer_rect.top() - 5.0).max(available.top()),
                    ),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(tree_rect), |ui| {
                    ui.set_clip_rect(ui.clip_rect().intersect(tree_rect));
                    self.tree_area(ui, (tree_rect.height() - 2.0).max(0.0));
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(footer_rect), |ui| {
                    ui.set_clip_rect(ui.clip_rect().intersect(footer_rect));
                    self.footer(ui);
                });
            });
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        ui.horizontal(|ui| {
            if let Some((_, logo)) = &self.logo_texture {
                ui.image((logo.id(), egui::vec2(28.0, 28.0)));
            }
            ui.label(RichText::new("Versus").size(18.0).strong());
            ui.label(
                RichText::new(if self.file_view.is_some() {
                    "/  FILE COMPARE"
                } else {
                    "/  FOLDER COMPARE"
                })
                .monospace()
                .size(10.0)
                .color(palette.muted),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dark = ui.visuals().dark_mode;
                if icon_button(
                    ui,
                    if dark {
                        ToolbarIcon::Sun
                    } else {
                        ToolbarIcon::Moon
                    },
                    true,
                    if dark {
                        "Switch to light mode"
                    } else {
                        "Switch to dark mode"
                    },
                )
                .clicked()
                {
                    set_theme(
                        ui.ctx(),
                        if dark {
                            egui::Theme::Light
                        } else {
                            egui::Theme::Dark
                        },
                    );
                    ui.ctx().request_repaint();
                }
                if self.file_view.is_none() {
                    self.controls(ui);
                }
            });
        });
    }

    fn folder_inputs(&mut self, ui: &mut egui::Ui) {
        self.folder_inputs_with_picker(ui, |side, current_path| {
            let mut dialog = rfd::FileDialog::new().set_title(if side == 0 {
                "Select left folder"
            } else {
                "Select right folder"
            });
            if !current_path.is_empty() {
                dialog = dialog.set_directory(current_path);
            }
            dialog.pick_folder()
        });
    }

    fn folder_inputs_with_picker(
        &mut self,
        ui: &mut egui::Ui,
        mut pick_folder: impl FnMut(usize, &str) -> Option<PathBuf>,
    ) {
        let palette = Palette::for_context(ui.ctx());
        let full_paths = self.paths.clone().map(|path| {
            if path.is_empty() {
                "Choose a folder".into()
            } else {
                absolute_path(&path).display().to_string()
            }
        });
        let width = ui.available_width();
        let path_width = (width / 2.0 - 84.0).max(40.0);
        let galleys = full_paths.clone().map(|path| {
            ui.painter()
                .layout(path, FontId::monospace(11.0), palette.text, path_width)
        });
        let height = galleys
            .iter()
            .map(|galley| galley.size().y)
            .fold(24.0, f32::max)
            + 14.0;
        let (header, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
        let mut picked = false;
        for (side, half) in split_rect(header).into_iter().enumerate() {
            let rect = half.shrink(7.0);
            let color = if side == 0 {
                palette.left_only
            } else {
                palette.right_only
            };
            let painter = ui.painter().with_clip_rect(rect);
            painter.text(
                rect.left_center(),
                Align2::LEFT_CENTER,
                if side == 0 { "LEFT" } else { "RIGHT" },
                FontId::monospace(10.0),
                color,
            );
            let galley = galleys[side].clone();
            let position = egui::pos2(rect.left() + 40.0, rect.center().y - galley.size().y / 2.0);
            let path_rect = Rect::from_min_size(
                egui::pos2(position.x, rect.top()),
                egui::vec2(path_width, rect.height()),
            );
            painter.galley(position, galley, palette.text);
            let path_response = ui
                .interact(
                    path_rect,
                    ui.id().with(("selected-folder", side)),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!(
                    "{}\nClick to browse for a folder",
                    full_paths[side]
                ));
            path_response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    if side == 0 {
                        "Browse left folder path"
                    } else {
                        "Browse right folder path"
                    },
                )
            });
            let browse_rect = Rect::from_center_size(
                rect.right_center() - egui::vec2(12.0, 0.0),
                egui::vec2(24.0, 24.0),
            );
            let mut browse_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(browse_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            let browse = icon_button(
                &mut browse_ui,
                ToolbarIcon::Browse,
                true,
                if side == 0 {
                    "Browse left folder"
                } else {
                    "Browse right folder"
                },
            );
            if browse.clicked() || path_response.clicked() {
                if let Some(path) = pick_folder(side, &self.paths[side]) {
                    self.paths[side] = path.to_string_lossy().into_owned();
                    picked = true;
                }
            }
        }
        if picked {
            self.invalidate();
            self.start_comparison();
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        if icon_button(
            ui,
            ToolbarIcon::Collapse,
            self.tree.is_some(),
            "Collapse all",
        )
        .clicked()
        {
            self.tree.as_mut().unwrap().collapse_all();
        }
        if icon_button(ui, ToolbarIcon::Expand, self.tree.is_some(), "Expand all").clicked() {
            self.tree.as_mut().unwrap().expand_all();
        }
        let ready = self.paths.iter().all(|path| !path.trim().is_empty());
        if icon_button(
            ui,
            ToolbarIcon::Refresh,
            ready && self.job.is_none(),
            if self.tree.is_some() {
                "Refresh comparison"
            } else {
                "Compare folders"
            },
        )
        .clicked()
        {
            self.start_comparison();
        }
        if let Some(job) = &self.job {
            let cancelling = job.cancellation.load(Ordering::Relaxed);
            if icon_button(
                ui,
                ToolbarIcon::Cancel,
                !cancelling,
                if cancelling {
                    "Cancelling…"
                } else {
                    "Cancel comparison"
                },
            )
            .clicked()
            {
                job.cancellation.store(true, Ordering::Relaxed);
            }
            ui.spinner();
        }
    }

    fn legend(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let back = self.file_view.is_some();
        let counts = if let Some(view) = &self.file_view {
            view.comparison.as_ref().map(|_| view.counts)
        } else {
            self.tree.as_ref().map(|_| self.counts)
        };
        ui.horizontal_wrapped(|ui| {
            if back && icon_button(ui, ToolbarIcon::Back, true, "Back to folders").clicked() {
                self.file_view = None;
            }
            for (index, icon) in [
                StatusIcon::Different,
                StatusIcon::LeftOnly,
                StatusIcon::RightOnly,
                StatusIcon::TypeMismatch,
                StatusIcon::Error,
            ]
            .into_iter()
            .enumerate()
            {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), Sense::hover());
                paint_status_icon(ui.painter(), rect.center(), icon);
                let label = counts.map_or_else(
                    || icon.label().into(),
                    |counts| format!("{}  {}", icon.label(), counts[index + 1]),
                );
                ui.label(RichText::new(label).size(10.0).color(icon.color(palette)));
                ui.add_space(5.0);
            }
        });
    }

    fn file_area(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        let view = self.file_view.as_mut().unwrap();
        egui::Frame::default()
            .fill(palette.panel)
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(6)
            .show(ui, |ui| {
                ui.columns(2, |columns| {
                    for (side, column) in columns.iter_mut().enumerate() {
                        column.horizontal(|ui| {
                            ui.label(
                                RichText::new(if side == 0 { "LEFT" } else { "RIGHT" })
                                    .monospace()
                                    .color(palette.muted),
                            );
                            ui.add(
                                egui::Label::new(
                                    RichText::new(view.paths[side].display().to_string())
                                        .monospace(),
                                )
                                .wrap(),
                            );
                        });
                        if view.sources[side].is_none() {
                            column.label(
                                RichText::new("Not present on this side").color(palette.muted),
                            );
                        }
                    }
                });
                ui.separator();
                if let Some(error) = &view.error {
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(14.0, 14.0), Sense::hover());
                        paint_status_icon(ui.painter(), rect.center(), view.error_icon);
                        ui.colored_label(palette.error, error);
                    });
                    return;
                }
                let Some(comparison) = &view.comparison else {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading file comparison…");
                    });
                    return;
                };
                if let Some(message) = &comparison.message {
                    ui.label(RichText::new(message).color(palette.muted));
                }
                if comparison.rows.is_empty() {
                    if comparison.message.is_none() {
                        ui.label(RichText::new("Both files are empty.").color(palette.muted));
                    }
                    return;
                }
                let height = (ui.available_height() - 2.0).max(0.0);
                let previous_y = view.scroll_y;
                let mut next_y = previous_y;
                // Process the pane under the pointer first, then paint both panes
                // using the resulting shared offset (scroll events apply during layout).
                let active_side =
                    usize::from(ui.input(|input| input.pointer.hover_pos()).is_some_and(
                        |position| position.x > ui.available_rect_before_wrap().center().x,
                    ));
                let mut viewports = [Rect::NOTHING; 2];
                let mut horizontal_offsets = [0.0; 2];
                ui.columns(2, |columns| {
                    for side in [active_side, 1 - active_side] {
                        let column = &mut columns[side];
                        column.spacing_mut().item_spacing = Vec2::ZERO;
                        let output = egui::ScrollArea::both()
                            .id_salt(("file-lines", &view.paths, side))
                            .scroll_bar_visibility(
                                egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                            )
                            .auto_shrink([false, false])
                            .max_height(height)
                            .vertical_scroll_offset(next_y)
                            .show_viewport(column, |ui, _| {
                                ui.set_min_size(egui::vec2(
                                    ui.available_width().max(view.content_widths[side]),
                                    ROW_HEIGHT * comparison.rows.len() as f32,
                                ));
                            });
                        next_y = output.state.offset.y;
                        viewports[side] = output.inner_rect;
                        horizontal_offsets[side] = output.state.offset.x;
                    }
                });
                for side in 0..2 {
                    let viewport = viewports[side];
                    let painter = ui.painter().with_clip_rect(viewport);
                    let first = (next_y / ROW_HEIGHT).floor() as usize;
                    let last = ((next_y + viewport.height()) / ROW_HEIGHT).ceil() as usize;
                    for index in first..last.min(comparison.rows.len()) {
                        let row = &comparison.rows[index];
                        let line = if side == 0 { &row.left } else { &row.right };
                        let rect = Rect::from_min_size(
                            egui::pos2(
                                viewport.left() - horizontal_offsets[side],
                                viewport.top() + index as f32 * ROW_HEIGHT - next_y,
                            ),
                            egui::vec2(viewport.width().max(view.content_widths[side]), ROW_HEIGHT),
                        );
                        paint_file_line(&painter, rect, line.as_ref(), &row.state, index % 2 == 1);
                    }
                }
                if (next_y - previous_y).abs() > 0.1 {
                    view.scroll_y = next_y;
                    ui.ctx().request_repaint();
                }
            });
    }

    fn tree_area(&mut self, ui: &mut egui::Ui, height: f32) {
        let palette = Palette::for_context(ui.ctx());
        let mut open = None;
        egui::Frame::default()
            .fill(palette.panel)
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(6)
            .show(ui, |ui| {
                ui.set_min_height(height);
                ui.set_max_height(height);
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let header_top = ui.cursor().top();
                self.folder_inputs(ui);
                let header_height = ui.cursor().top() - header_top;
                let (divider, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter().line_segment(
                    [divider.left_center(), divider.right_center()],
                    Stroke::new(1.0, palette.border),
                );
                ui.painter().line_segment(
                    [
                        egui::pos2(divider.center().x, header_top),
                        divider.center_bottom(),
                    ],
                    Stroke::new(1.0, palette.border),
                );
                let body_height = (height - header_height - 2.0).max(60.0);
                if let Some(tree) = &mut self.tree {
                    let rows = tree.visible_rows();
                    if rows.is_empty() {
                        empty_display(
                            ui,
                            body_height,
                            "Both folders are empty",
                            "There are no files or folders to compare.",
                        );
                        return;
                    }
                    let mut toggle = None;
                    let mut selected = None;
                    egui::ScrollArea::vertical()
                        .id_salt(("linked-trees", self.scroll_generation))
                        .auto_shrink([false, false])
                        .max_height(body_height)
                        .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                            for index in range {
                                let row = &rows[index];
                                let node = row.node;
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), ROW_HEIGHT),
                                    Sense::hover(),
                                );
                                let is_selected =
                                    self.selected.as_ref() == Some(&node.relative_path);
                                for (side, half) in split_rect(rect).into_iter().enumerate() {
                                    let entry = if side == 0 { &node.left } else { &node.right };
                                    let present = entry.exists;
                                    let response = ui.interact(
                                        half,
                                        ui.id().with((&node.relative_path, side)),
                                        Sense::click(),
                                    );
                                    let response = if present {
                                        response.on_hover_cursor(egui::CursorIcon::PointingHand)
                                    } else {
                                        response
                                    };
                                    response.widget_info(|| {
                                        egui::WidgetInfo::selected(
                                            egui::WidgetType::SelectableLabel,
                                            true,
                                            is_selected,
                                            format!(
                                                "{}: {} ({})",
                                                if side == 0 { "Left" } else { "Right" },
                                                node.relative_path.display(),
                                                state_label(&node.state)
                                            ),
                                        )
                                    });
                                    paint_row(
                                        ui,
                                        half,
                                        node,
                                        side,
                                        RowAppearance {
                                            depth: row.depth,
                                            expanded: tree.is_expanded(&node.relative_path),
                                            alternate: index % 2 == 1,
                                            selected: is_selected,
                                            hovered: response.hovered() || response.has_focus(),
                                        },
                                    );
                                    let expandable = node.is_expandable()
                                        && entry.kind == Some(DirectoryEntryKind::Directory);
                                    let keyboard_toggle = response.has_focus()
                                        && ui.input(|input| {
                                            input.key_pressed(egui::Key::Space)
                                                || input.key_pressed(egui::Key::Enter)
                                        });
                                    if (response.double_clicked() || keyboard_toggle)
                                        && entry.kind == Some(DirectoryEntryKind::File)
                                    {
                                        open = Some((
                                            node.relative_path.clone(),
                                            [node.left.exists, node.right.exists],
                                            [&node.left, &node.right].into_iter().any(|entry| {
                                                entry.exists
                                                    && entry.kind != Some(DirectoryEntryKind::File)
                                            }),
                                        ));
                                    }
                                    if response.clicked() || keyboard_toggle {
                                        response.request_focus();
                                        selected = Some(node.relative_path.clone());
                                        if expandable {
                                            toggle = Some(node.relative_path.clone());
                                        }
                                    }
                                    if response.has_focus() && expandable {
                                        let expanded = tree.is_expanded(&node.relative_path);
                                        if ui.input(|input| {
                                            (input.key_pressed(egui::Key::ArrowRight) && !expanded)
                                                || (input.key_pressed(egui::Key::ArrowLeft)
                                                    && expanded)
                                        }) {
                                            toggle = Some(node.relative_path.clone());
                                        }
                                    }
                                    response.on_hover_ui(|ui| {
                                        ui.monospace(node.relative_path.display().to_string());
                                        ui.label(if present {
                                            state_label(&node.state)
                                        } else {
                                            "Not present on this side"
                                        });
                                        if let Some(roots) = &self.roots {
                                            ui.monospace(
                                                roots[side]
                                                    .join(&node.relative_path)
                                                    .display()
                                                    .to_string(),
                                            );
                                        }
                                        if let DirectoryEntryState::Error(error) = &node.state {
                                            ui.colored_label(palette.error, error.to_string());
                                        }
                                    });
                                }
                                ui.painter().line_segment(
                                    [rect.center_top(), rect.center_bottom()],
                                    Stroke::new(1.0, palette.border),
                                );
                            }
                        });
                    if let Some(path) = toggle {
                        tree.toggle_expanded(path);
                    }
                    if let Some(path) = selected {
                        self.selected = Some(path);
                    }
                } else {
                    empty_display(
                        ui,
                        body_height,
                        if self.job.is_some() {
                            "Comparing folders…"
                        } else {
                            "Two folders. One clear view."
                        },
                        if self.job.is_some() {
                            "The trees will appear when the comparison finishes."
                        } else {
                            "Choose a folder on each side to see what matches and what changed."
                        },
                    );
                }
            });
        if let Some((path, present, type_mismatch)) = open {
            self.open_file(path, present, type_mismatch);
        }
    }

    fn footer(&self, ui: &mut egui::Ui) {
        let palette = Palette::for_context(ui.ctx());
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.message).size(11.0).color(palette.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(elapsed) = self.elapsed {
                    ui.label(
                        RichText::new(format!("{:.2}s", elapsed.as_secs_f64()))
                            .monospace()
                            .size(11.0)
                            .color(palette.muted),
                    );
                }
            });
        });
        if let Some(path) = &self.selected {
            ui.add(
                egui::Label::new(
                    RichText::new(path.display().to_string())
                        .monospace()
                        .size(11.0)
                        .color(palette.text),
                )
                .truncate(),
            );
        } else {
            ui.label(RichText::new("Click a folder to expand both trees.  •  Double-click a file to compare.  •  Symlinks are not traversed.").size(10.0).color(palette.muted));
        }
    }
}

impl Drop for VersusApp {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancellation.store(true, Ordering::Relaxed);
        }
    }
}

impl eframe::App for VersusApp {
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        Palette::new(visuals.dark_mode)
            .background
            .to_normalized_gamma_f32()
    }
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll_comparison(ctx);
        self.poll_file_comparison(ctx);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.render(ui);
    }
}

fn absolute_path(path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&path))
            .unwrap_or(path)
    }
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

fn paint_file_line(
    painter: &egui::Painter,
    rect: Rect,
    line: Option<&(usize, String)>,
    state: &DirectoryEntryState,
    alternate: bool,
) {
    let palette = Palette::for_context(painter.ctx());
    let painter = painter.with_clip_rect(rect);
    let color = state_color(state, palette);
    painter.rect_filled(
        rect,
        0,
        if alternate {
            palette.alternate
        } else {
            palette.panel
        },
    );
    if *state != DirectoryEntryState::Same {
        painter.rect_filled(
            Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
            0,
            color,
        );
    }
    let Some((number, text)) = line else {
        painter.text(
            rect.left_center() + egui::vec2(66.0, 0.0),
            Align2::LEFT_CENTER,
            "—",
            FontId::monospace(11.0),
            palette.muted,
        );
        return;
    };
    painter.text(
        rect.left_center() + egui::vec2(42.0, 0.0),
        Align2::RIGHT_CENTER,
        number.to_string(),
        FontId::monospace(10.0),
        palette.muted,
    );
    if let Some(icon) = status_icon(state) {
        paint_status_icon(&painter, rect.left_center() + egui::vec2(54.0, 0.0), icon);
    }
    painter.text(
        rect.left_center() + egui::vec2(68.0, 0.0),
        Align2::LEFT_CENTER,
        text.replace('\t', "    "),
        FontId::monospace(11.0),
        color,
    );
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

fn split_rect(rect: Rect) -> [Rect; 2] {
    [
        Rect::from_min_max(rect.min, rect.center_bottom()),
        Rect::from_min_max(rect.center_top(), rect.max),
    ]
}

#[derive(Clone, Copy)]
enum StatusIcon {
    Different,
    LeftOnly,
    RightOnly,
    TypeMismatch,
    Error,
}

impl StatusIcon {
    fn label(self) -> &'static str {
        match self {
            Self::Different => "Different",
            Self::LeftOnly => "Left only",
            Self::RightOnly => "Right only",
            Self::TypeMismatch => "Type mismatch",
            Self::Error => "Read error",
        }
    }
    fn color(self, palette: Palette) -> Color32 {
        match self {
            Self::Different => palette.changed,
            Self::LeftOnly => palette.left_only,
            Self::RightOnly => palette.right_only,
            Self::TypeMismatch | Self::Error => palette.error,
        }
    }
}

fn status_icon(state: &DirectoryEntryState) -> Option<StatusIcon> {
    match state {
        DirectoryEntryState::Same => None,
        DirectoryEntryState::Different => Some(StatusIcon::Different),
        DirectoryEntryState::LeftOnly => Some(StatusIcon::LeftOnly),
        DirectoryEntryState::RightOnly => Some(StatusIcon::RightOnly),
        DirectoryEntryState::TypeMismatch => Some(StatusIcon::TypeMismatch),
        DirectoryEntryState::Error(_) => Some(StatusIcon::Error),
    }
}

fn state_label(state: &DirectoryEntryState) -> &'static str {
    status_icon(state).map_or("Identical", StatusIcon::label)
}
fn state_color(state: &DirectoryEntryState, palette: Palette) -> Color32 {
    status_icon(state).map_or(palette.muted, |icon| icon.color(palette))
}

fn paint_status_icon(painter: &egui::Painter, center: Pos2, icon: StatusIcon) {
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
    }
}

#[derive(Clone, Copy)]
enum ToolbarIcon {
    Back,
    Refresh,
    Expand,
    Collapse,
    Browse,
    Cancel,
    Sun,
    Moon,
}

fn icon_button(ui: &mut egui::Ui, icon: ToolbarIcon, enabled: bool, label: &str) -> egui::Response {
    let palette = Palette::for_context(ui.ctx());
    ui.add_enabled_ui(enabled, |ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), Sense::hover());
        let response = ui.interact(rect, egui::Id::new(label), Sense::click());
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
        let color = if !enabled {
            palette.muted.gamma_multiply(0.4)
        } else if response.hovered() || response.has_focus() {
            palette.accent
        } else {
            palette.text
        };
        if response.hovered() || response.has_focus() {
            ui.painter().rect_filled(rect, 3, palette.border);
        }
        let center = rect.center();
        let point = |x, y| center + egui::vec2(x, y);
        let stroke = Stroke::new(1.4, color);
        match icon {
            ToolbarIcon::Back => {
                ui.painter()
                    .line_segment([point(6.0, 0.0), point(-6.0, 0.0)], stroke);
                ui.painter().line(
                    vec![point(-1.0, -5.0), point(-6.0, 0.0), point(-1.0, 5.0)],
                    stroke,
                );
            }
            ToolbarIcon::Refresh => {
                let arc: Vec<_> = (0..=24)
                    .map(|step| {
                        let angle = (50.0 + step as f32 * 270.0 / 24.0).to_radians();
                        point(angle.cos() * 6.0, angle.sin() * 6.0)
                    })
                    .collect();
                let end = *arc.last().unwrap();
                ui.painter().line(arc, stroke);
                ui.painter().line(
                    vec![
                        end + egui::vec2(-4.0, -0.5),
                        end,
                        end + egui::vec2(0.0, -4.0),
                    ],
                    stroke,
                );
            }
            ToolbarIcon::Expand | ToolbarIcon::Collapse => {
                let back = Rect::from_center_size(point(-2.0, 2.0), egui::vec2(12.0, 12.0));
                let front = Rect::from_center_size(point(1.0, -1.0), egui::vec2(12.0, 12.0));
                ui.painter().rect_stroke(
                    back,
                    2,
                    Stroke::new(1.0, color.gamma_multiply(0.65)),
                    egui::StrokeKind::Inside,
                );
                ui.painter().rect_filled(front, 2, palette.panel);
                ui.painter()
                    .rect_stroke(front, 2, stroke, egui::StrokeKind::Inside);
                let center = front.center();
                ui.painter().line_segment(
                    [center - egui::vec2(3.0, 0.0), center + egui::vec2(3.0, 0.0)],
                    stroke,
                );
                if matches!(icon, ToolbarIcon::Expand) {
                    ui.painter().line_segment(
                        [center - egui::vec2(0.0, 3.0), center + egui::vec2(0.0, 3.0)],
                        stroke,
                    );
                }
            }
            ToolbarIcon::Browse => paint_icon(
                ui.painter(),
                center,
                Some(&DirectoryEntryKind::Directory),
                color,
            ),
            ToolbarIcon::Sun => {
                ui.painter().circle_filled(center, 4.0, color);
                for index in 0..8 {
                    let angle = index as f32 * std::f32::consts::TAU / 8.0;
                    let direction = egui::vec2(angle.cos(), angle.sin());
                    ui.painter().line_segment(
                        [center + direction * 7.2, center + direction * 10.2],
                        Stroke::new(1.2, color),
                    );
                }
            }
            ToolbarIcon::Moon => {
                let arc: Vec<_> = (0..=24)
                    .map(|step| {
                        let angle = (60.0 + step as f32 * 240.0 / 24.0).to_radians();
                        point(angle.cos() * 7.0, angle.sin() * 7.0)
                    })
                    .chain((0..=16).map(|step| {
                        let angle = (-90.0 - step as f32 * 180.0 / 16.0).to_radians();
                        point(3.5 + angle.cos() * 5.0, angle.sin() * 6.0)
                    }))
                    .collect();
                ui.painter().add(egui::Shape::closed_line(arc, stroke));
            }
            ToolbarIcon::Cancel => {
                ui.painter()
                    .line_segment([point(-4.0, -4.0), point(4.0, 4.0)], stroke);
                ui.painter()
                    .line_segment([point(-4.0, 4.0), point(4.0, -4.0)], stroke);
            }
        }
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(label)
    })
    .inner
}

fn format_size(size: Option<u64>) -> String {
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

struct RowAppearance {
    depth: usize,
    expanded: bool,
    alternate: bool,
    selected: bool,
    hovered: bool,
}

fn paint_row(ui: &egui::Ui, rect: Rect, node: &TreeNode, side: usize, appearance: RowAppearance) {
    let palette = Palette::for_context(ui.ctx());
    let entry = if side == 0 { &node.left } else { &node.right };
    let kind = entry.kind.as_ref();
    let present = entry.exists;
    let state = &node.state;
    let RowAppearance {
        depth,
        expanded,
        alternate,
        selected,
        hovered,
    } = appearance;
    let painter = ui.painter().with_clip_rect(rect);
    let color = state_color(state, palette);
    let fill = if selected {
        palette.accent.gamma_multiply(0.16)
    } else if hovered {
        palette.hover
    } else if alternate {
        palette.alternate
    } else {
        palette.panel
    };
    painter.rect_filled(rect, 0, fill);
    if present && *state != DirectoryEntryState::Same {
        painter.rect_filled(
            Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
            0,
            color,
        );
    }
    let x = rect.left() + 8.0 + depth as f32 * 14.0;
    let y = rect.center().y;
    let name_right = rect.right() - 98.0;
    if !present {
        painter.text(
            egui::pos2(x + 36.0, y),
            Align2::LEFT_CENTER,
            "—",
            FontId::monospace(11.0),
            palette.muted.gamma_multiply(0.55),
        );
        return;
    }
    let name_painter = painter.with_clip_rect(Rect::from_min_max(
        rect.min,
        egui::pos2(name_right.max(rect.left()), rect.bottom()),
    ));
    if kind == Some(&DirectoryEntryKind::Directory) && node.is_expandable() {
        name_painter.text(
            egui::pos2(x, y),
            Align2::LEFT_CENTER,
            if expanded { "▾" } else { "▸" },
            FontId::monospace(15.0),
            palette.muted,
        );
    }
    paint_icon(&name_painter, egui::pos2(x + 19.0, y), kind, color);
    name_painter.text(
        egui::pos2(x + 40.0, y),
        Align2::LEFT_CENTER,
        node.name.to_string_lossy(),
        FontId::monospace(11.0),
        color,
    );
    painter.text(
        egui::pos2(rect.right() - 32.0, y),
        Align2::RIGHT_CENTER,
        format_size(entry.size),
        FontId::monospace(10.0),
        palette.muted,
    );
    if let Some(icon) = status_icon(state) {
        paint_status_icon(&painter, egui::pos2(rect.right() - 15.0, y), icon);
    }
}

fn paint_icon(
    painter: &egui::Painter,
    center: Pos2,
    kind: Option<&DirectoryEntryKind>,
    color: Color32,
) {
    let rect = Rect::from_center_size(center, egui::vec2(13.0, 11.0));
    if kind == Some(&DirectoryEntryKind::Directory) {
        painter.rect_filled(
            Rect::from_min_size(rect.min - egui::vec2(0.0, 2.0), egui::vec2(6.0, 4.0)),
            1,
            color.gamma_multiply(0.6),
        );
        painter.rect_filled(rect, 2, color.gamma_multiply(0.35));
        painter.rect_stroke(rect, 2, Stroke::new(1.0, color), egui::StrokeKind::Inside);
    } else if kind == Some(&DirectoryEntryKind::Symlink) {
        painter.text(
            center,
            Align2::CENTER_CENTER,
            "@",
            FontId::monospace(13.0),
            color,
        );
    } else {
        painter.rect_stroke(
            rect.shrink2(egui::vec2(2.0, 0.0)),
            1,
            Stroke::new(1.0, color),
            egui::StrokeKind::Inside,
        );
    }
}

fn empty_display(ui: &mut egui::Ui, height: f32, title: &str, subtitle: &str) {
    let palette = Palette::for_context(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height.max(80.0)),
        Sense::hover(),
    );
    ui.painter().line_segment(
        [rect.center_top(), rect.center_bottom()],
        Stroke::new(1.0, palette.border.gamma_multiply(0.5)),
    );
    let painter = ui.painter().with_clip_rect(rect);
    let center = rect.center();
    painter.text(
        center - egui::vec2(0.0, 24.0),
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(21.0),
        palette.text,
    );
    painter.text(
        center + egui::vec2(0.0, 8.0),
        Align2::CENTER_CENTER,
        subtitle,
        FontId::proportional(11.0),
        palette.muted,
    );
}

fn apply_theme(ctx: &egui::Context) {
    set_theme(ctx, egui::Theme::Dark);
}

fn set_theme(ctx: &egui::Context, theme: egui::Theme) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use versus::{DirectoryDiff, DirectoryEntry};

    fn fixture() -> FolderTree {
        FolderTree::from_diff(&DirectoryDiff {
            cancelled: false,
            entries: [
                ("assembly", DirectoryEntryKind::Directory),
                ("assembly/model.step", DirectoryEntryKind::File),
            ]
            .into_iter()
            .map(|(path, kind)| DirectoryEntry {
                relative_path: path.into(),
                left_size: Some(0),
                right_size: Some(0),
                left_exists: true,
                right_exists: true,
                left_kind: Some(kind.clone()),
                right_kind: Some(kind.clone()),
                kind,
                state: DirectoryEntryState::Same,
            })
            .collect(),
        })
    }

    fn loaded_app() -> VersusApp {
        let mut app = VersusApp::default();
        app.tree = Some(fixture());
        app.roots = Some(["left".into(), "right".into()]);
        app
    }

    fn render(
        app: &mut VersusApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                events,
                ..Default::default()
            },
            |ui| app.render(ui),
        );
        output.textures_delta.clear();
        output
    }

    fn text_positions(output: &egui::FullOutput, value: &str) -> Vec<Pos2> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == value => Some(text.pos),
                _ => None,
            })
            .collect()
    }

    fn text_centers(output: &egui::FullOutput, value: &str) -> Vec<Pos2> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == value => {
                    Some(text.pos + text.galley.size() / 2.0)
                }
                _ => None,
            })
            .collect()
    }

    fn click(app: &mut VersusApp, ctx: &egui::Context, position: Pos2) {
        for pressed in [true, false] {
            render(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            )
            .drop_without_applying_deltas();
        }
    }

    #[test]
    fn clicking_either_folder_pane_updates_both_rendered_trees() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        let output = render(&mut app, &ctx, vec![]);
        let folders = text_positions(&output, "assembly");
        assert_eq!(folders.len(), 2);
        output.drop_without_applying_deltas();
        click(&mut app, &ctx, folders[0] + egui::vec2(3.0, 5.0));
        assert!(app.tree.as_ref().unwrap().is_expanded("assembly"));
        let output = render(&mut app, &ctx, vec![]);
        let children = text_positions(&output, "model.step");
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].y, children[1].y);
        let folders = text_positions(&output, "assembly");
        output.drop_without_applying_deltas();
        click(&mut app, &ctx, folders[1] + egui::vec2(3.0, 5.0));
        assert!(!app.tree.as_ref().unwrap().is_expanded("assembly"));
        let output = render(&mut app, &ctx, vec![]);
        assert!(text_positions(&output, "model.step").is_empty());
        output.drop_without_applying_deltas();
    }

    #[test]
    fn one_sided_rows_render_an_aligned_placeholder() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.tree = Some(FolderTree::from_diff(&DirectoryDiff {
            cancelled: false,
            entries: vec![DirectoryEntry {
                relative_path: "left-only.step".into(),
                left_size: Some(1024),
                right_size: None,
                left_exists: true,
                right_exists: false,
                left_kind: Some(DirectoryEntryKind::File),
                right_kind: None,
                kind: DirectoryEntryKind::File,
                state: DirectoryEntryState::LeftOnly,
            }],
        }));
        let output = render(&mut app, &ctx, vec![]);
        let names = text_positions(&output, "left-only.step");
        let placeholders = text_positions(&output, "—");
        assert_eq!(names.len(), 1);
        assert_eq!(placeholders.len(), 1);
        assert_eq!(names[0].y, placeholders[0].y);
        assert!(placeholders[0].x > 600.0);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn empty_folder_has_no_disclosure_and_click_only_selects_it() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.tree = Some(FolderTree::from_diff(&DirectoryDiff {
            cancelled: false,
            entries: vec![DirectoryEntry {
                relative_path: "empty-folder".into(),
                left_size: Some(0),
                right_size: Some(0),
                left_exists: true,
                right_exists: true,
                left_kind: Some(DirectoryEntryKind::Directory),
                right_kind: Some(DirectoryEntryKind::Directory),
                kind: DirectoryEntryKind::Directory,
                state: DirectoryEntryState::Same,
            }],
        }));
        let output = render(&mut app, &ctx, vec![]);
        assert!(text_positions(&output, "▸").is_empty());
        let folders = text_positions(&output, "empty-folder");
        output.drop_without_applying_deltas();
        click(&mut app, &ctx, folders[0] + egui::vec2(3.0, 5.0));
        assert_eq!(
            app.selected.as_deref(),
            Some(std::path::Path::new("empty-folder"))
        );
        assert!(!app.tree.as_ref().unwrap().is_expanded("empty-folder"));
    }

    #[test]
    fn minimum_window_keeps_controls_and_footer_inside_the_viewport() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 650.0))),
                ..Default::default()
            },
            |ui| app.render(ui),
        );
        output.textures_delta.clear();
        let controls: Vec<_> = ["Collapse all", "Expand all", "Refresh comparison"]
            .into_iter()
            .map(|label| ctx.read_response(egui::Id::new(label)).unwrap().rect)
            .collect();
        assert!(
            controls
                .iter()
                .all(|rect| rect.right() <= 890.0 && rect.left() > 700.0 && rect.bottom() <= 45.0)
        );
        for text in ["Choose two folders to begin."] {
            let positions = text_positions(&output, text);
            assert!(!positions.is_empty(), "Missing {text}");
            assert!(
                positions.iter().all(|position| position.x >= 0.0
                    && position.x < 900.0
                    && position.y >= 0.0
                    && position.y < 630.0),
                "{text} outside viewport: {positions:?}"
            );
        }
        output.drop_without_applying_deltas();
    }

    #[test]
    fn clicked_item_footer_is_fully_visible_at_supported_window_sizes() {
        for (width, height) in [(900.0, 650.0), (1200.0, 800.0)] {
            let ctx = egui::Context::default();
            apply_theme(&ctx);
            let mut app = loaded_app();
            app.tree.as_mut().unwrap().expand_all();
            let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(width, height));
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| app.render(ui),
            );
            output.textures_delta.clear();
            let file = text_positions(&output, "model.step")[1];
            output.drop_without_applying_deltas();
            for pressed in [true, false] {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        events: vec![
                            egui::Event::PointerMoved(file + egui::vec2(3.0, 5.0)),
                            egui::Event::PointerButton {
                                pos: file + egui::vec2(3.0, 5.0),
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            },
                        ],
                        ..Default::default()
                    },
                    |ui| app.render(ui),
                );
                output.textures_delta.clear();
                output.drop_without_applying_deltas();
            }
            assert_eq!(
                app.selected.as_deref(),
                Some(std::path::Path::new("assembly/model.step"))
            );
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| app.render(ui),
            );
            output.textures_delta.clear();
            let (shape, text) = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "assembly/model.step" => {
                        Some((shape, text))
                    }
                    _ => None,
                })
                .expect("Clicked item should appear in footer");
            let bounds = Rect::from_min_size(text.pos, text.galley.size());
            assert!(bounds.top() > height - 55.0);
            assert!(
                screen.shrink(10.0).contains_rect(bounds),
                "Footer outside window: {bounds:?}"
            );
            assert!(
                shape.clip_rect.contains_rect(bounds),
                "Footer clipped: {bounds:?}"
            );
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn clicking_path_or_browse_opens_picker_for_the_correct_side() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.paths = [String::new(), "/engineering/release/designs".into()];
        let original_paths = app.paths.clone();
        let mut requests = Vec::new();
        {
            let mut render_inputs = |events| {
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            egui::vec2(1200.0, 800.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        app.folder_inputs_with_picker(ui, |side, path| {
                            requests.push((side, path.to_owned()));
                            None // Closing the dialog should preserve the current comparison.
                        })
                    },
                )
            };
            for (side, path_text) in [(0, "Choose a folder"), (1, "/engineering/release/designs")] {
                for path_area in [true, false] {
                    let output = render_inputs(vec![]);
                    let target = if path_area {
                        text_centers(&output, path_text)[0]
                    } else {
                        ctx.read_response(egui::Id::new(if side == 0 {
                            "Browse left folder"
                        } else {
                            "Browse right folder"
                        }))
                        .unwrap()
                        .rect
                        .center()
                    };
                    output.drop_without_applying_deltas();
                    let output = render_inputs(vec![egui::Event::PointerMoved(target)]);
                    assert_eq!(
                        output.platform_output.cursor_icon,
                        egui::CursorIcon::PointingHand
                    );
                    output.drop_without_applying_deltas();
                    for pressed in [true, false] {
                        render_inputs(vec![
                            egui::Event::PointerMoved(target),
                            egui::Event::PointerButton {
                                pos: target,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            },
                        ])
                        .drop_without_applying_deltas();
                    }
                }
            }
        }
        assert_eq!(
            requests,
            vec![
                (0, String::new()),
                (0, String::new()),
                (1, original_paths[1].clone()),
                (1, original_paths[1].clone())
            ]
        );
        assert_eq!(app.paths, original_paths);
        assert!(app.tree.is_some());
    }

    #[test]
    fn single_path_headers_center_side_labels_and_browse_buttons() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.paths = [
            format!("/engineering/{}release", "long-folder/".repeat(12)),
            "/engineering/release".into(),
        ];
        let output = render(&mut app, &ctx, vec![]);
        let mut centers = Vec::new();
        for (side, label) in ["LEFT", "RIGHT"].into_iter().enumerate() {
            let paths = text_centers(&output, &app.paths[side]);
            assert_eq!(paths.len(), 1, "Selected path must only appear once");
            let side_label = text_centers(&output, label)[0];
            let browse = ctx
                .read_response(egui::Id::new(if side == 0 {
                    "Browse left folder"
                } else {
                    "Browse right folder"
                }))
                .unwrap();
            assert_eq!(side_label.y, paths[0].y);
            assert_eq!(side_label.y, browse.rect.center().y);
            centers.push(side_label.y);
        }
        assert_eq!(centers[0], centers[1]);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn toolbar_icons_expand_collapse_and_refresh_the_comparison() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.paths = ["left".into(), "right".into()];
        render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
        let expand = ctx
            .read_response(egui::Id::new("Expand all"))
            .unwrap()
            .rect
            .center();
        click(&mut app, &ctx, expand);
        assert!(app.tree.as_ref().unwrap().is_expanded("assembly"));
        let collapse = ctx
            .read_response(egui::Id::new("Collapse all"))
            .unwrap()
            .rect
            .center();
        click(&mut app, &ctx, collapse);
        assert!(!app.tree.as_ref().unwrap().is_expanded("assembly"));
        let refresh = ctx
            .read_response(egui::Id::new("Refresh comparison"))
            .unwrap()
            .rect
            .center();
        click(&mut app, &ctx, refresh);
        assert!(app.job.is_some());
    }

    #[test]
    fn full_paths_follow_the_legend_and_precede_tree_rows() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.paths = [
            "/engineering/current/designs".into(),
            "/engineering/release/designs".into(),
        ];
        let output = render(&mut app, &ctx, vec![]);
        let legend = text_positions(&output, "Different  0")[0];
        let rows = text_positions(&output, "assembly");
        for path in &app.paths {
            let positions = text_positions(&output, path);
            assert!(!positions.is_empty());
            assert!(
                positions
                    .iter()
                    .all(|position| position.y > legend.y && position.y < rows[0].y)
            );
        }
        assert!(rows[0].y < 160.0, "Compact tree should begin near the top");
        output.drop_without_applying_deltas();
    }

    #[test]
    fn long_selected_path_wraps_without_truncation() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.paths[0] = format!("/engineering/{}/release", "long-folder-name/".repeat(12));
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 650.0))),
                ..Default::default()
            },
            |ui| app.render(ui),
        );
        output.textures_delta.clear();
        let wrapped = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text)
                    if text.galley.text() == app.paths[0] && text.galley.rows.len() > 1 =>
                {
                    Some(text)
                }
                _ => None,
            })
            .expect("Full selected path should wrap in the pane header");
        assert!(wrapped.pos.x + wrapped.galley.size().x < 450.0);
        assert!(text_positions(&output, "assembly")[0].y > wrapped.pos.y + wrapped.galley.size().y);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn hovering_files_and_folders_uses_the_pointer_cursor() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.tree.as_mut().unwrap().expand_all();
        let output = render(&mut app, &ctx, vec![]);
        let targets = [
            text_positions(&output, "assembly")[0],
            text_positions(&output, "model.step")[1],
        ];
        output.drop_without_applying_deltas();
        for position in targets {
            let output = render(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(position + egui::vec2(3.0, 5.0))],
            );
            assert_eq!(
                output.platform_output.cursor_icon,
                egui::CursorIcon::PointingHand
            );
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn row_shows_size_beside_status_icon_without_a_status_word() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        let output = render(&mut app, &ctx, vec![]);
        let names = text_centers(&output, "assembly");
        let sizes = text_centers(&output, "0 B");
        assert_eq!(sizes.len(), 2);
        assert!(
            sizes
                .iter()
                .zip(names)
                .all(|(size, name)| size.y == name.y && size.x > name.x)
        );
        assert!(text_positions(&output, "Identical").is_empty());
        output.drop_without_applying_deltas();
        assert_eq!(format_size(Some(1024)), "1.0 KiB");
        assert_eq!(format_size(Some(1024 * 1024)), "1.0 MiB");
        assert_eq!(format_size(None), "—");
    }

    fn loaded_file_view() -> FileView {
        FileView {
            paths: ["/left/model.step".into(), "/right/model.step".into()],
            sources: [
                Some("/left/model.step".into()),
                Some("/right/model.step".into()),
            ],
            job: None,
            comparison: Some(FileComparison {
                rows: vec![
                    versus::FileComparisonRow {
                        left: Some((1, "unchanged".into())),
                        right: Some((1, "unchanged".into())),
                        state: DirectoryEntryState::Same,
                    },
                    versus::FileComparisonRow {
                        left: Some((2, "old value".into())),
                        right: Some((2, "new value".into())),
                        state: DirectoryEntryState::Different,
                    },
                    versus::FileComparisonRow {
                        left: Some((3, "removed".into())),
                        right: None,
                        state: DirectoryEntryState::LeftOnly,
                    },
                ],
                message: None,
            }),
            error: None,
            error_icon: StatusIcon::Error,
            scroll_y: 0.0,
            content_widths: [300.0; 2],
            counts: [1, 1, 1, 0, 0, 0],
        }
    }

    #[test]
    fn double_clicking_file_on_either_side_opens_view_and_back_restores_tree() {
        for side in 0..2 {
            let ctx = egui::Context::default();
            apply_theme(&ctx);
            let mut app = loaded_app();
            app.tree.as_mut().unwrap().expand_all();
            let generation = app.scroll_generation;
            let output = render(&mut app, &ctx, vec![]);
            let target = text_positions(&output, "model.step")[side] + egui::vec2(3.0, 5.0);
            output.drop_without_applying_deltas();
            click(&mut app, &ctx, target);
            assert!(
                app.file_view.is_none(),
                "Single click should only select a file"
            );
            click(&mut app, &ctx, target);
            let view = app
                .file_view
                .as_ref()
                .expect("Double click should open file comparison");
            assert_eq!(
                view.paths,
                [
                    PathBuf::from("left/assembly/model.step"),
                    PathBuf::from("right/assembly/model.step")
                ]
            );
            let cancellation = view.job.as_ref().unwrap().cancellation.clone();
            let output = render(&mut app, &ctx, vec![]);
            let back = ctx
                .read_response(egui::Id::new("Back to folders"))
                .unwrap()
                .rect
                .center();
            assert!(text_positions(&output, "assembly").is_empty());
            output.drop_without_applying_deltas();
            click(&mut app, &ctx, back);
            assert!(app.file_view.is_none());
            assert!(cancellation.load(Ordering::Relaxed));
            assert!(app.tree.as_ref().unwrap().is_expanded("assembly"));
            assert_eq!(
                app.selected.as_deref(),
                Some(std::path::Path::new("assembly/model.step"))
            );
            assert_eq!(app.scroll_generation, generation);
            let output = render(&mut app, &ctx, vec![]);
            assert_eq!(text_positions(&output, "model.step").len(), 2);
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn file_lines_are_aligned_and_share_difference_legend() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.file_view = Some(loaded_file_view());
        let output = render(&mut app, &ctx, vec![]);
        let identical = text_centers(&output, "unchanged");
        assert_eq!(identical.len(), 2);
        assert_eq!(identical[0].y, identical[1].y);
        assert_eq!(
            text_centers(&output, "old value")[0].y,
            text_centers(&output, "new value")[0].y
        );
        assert_eq!(
            text_centers(&output, "removed")[0].y,
            text_centers(&output, "—")[0].y
        );
        for label in [
            "Different  1",
            "Left only  1",
            "Right only  0",
            "Type mismatch  0",
            "Read error  0",
        ] {
            assert_eq!(text_positions(&output, label).len(), 1);
        }
        assert!(text_positions(&output, "Identical  1").is_empty());
        for shape in &output.shapes {
            if let egui::Shape::Text(text) = &shape.shape {
                if text.galley.text() == "unchanged" {
                    assert_eq!(text.fallback_color, Palette::new(true).muted);
                }
            }
        }
        assert!(status_icon(&DirectoryEntryState::Same).is_none());
        assert_eq!(
            state_color(&DirectoryEntryState::Same, Palette::new(true)),
            Palette::new(true).muted
        );
        output.drop_without_applying_deltas();
    }

    #[test]
    fn scrolling_either_file_pane_keeps_line_rows_aligned() {
        for side in 0..2 {
            let ctx = egui::Context::default();
            apply_theme(&ctx);
            let mut app = loaded_app();
            let mut view = loaded_file_view();
            view.comparison.as_mut().unwrap().rows = (1..=200)
                .map(|number| versus::FileComparisonRow {
                    left: Some((number, format!("line {number}"))),
                    right: Some((number, format!("line {number}"))),
                    state: DirectoryEntryState::Same,
                })
                .collect();
            app.file_view = Some(view);
            render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
            let pointer = egui::pos2(if side == 0 { 300.0 } else { 900.0 }, 300.0);
            let output = render(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(pointer),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: egui::vec2(0.0, -180.0),
                        modifiers: Default::default(),
                    },
                ],
            );
            let assert_aligned = |output: &egui::FullOutput| {
                for number in 1..=200 {
                    let positions = text_positions(output, &format!("line {number}"));
                    if !positions.is_empty() {
                        assert_eq!(
                            positions.len(),
                            2,
                            "Line {number} should appear in both panes"
                        );
                        assert!(
                            (positions[0].y - positions[1].y).abs() < 0.1,
                            "Line {number} is misaligned: {positions:?}"
                        );
                    }
                }
            };
            assert_aligned(&output);
            output.drop_without_applying_deltas();
            for _ in 0..20 {
                let output = render(&mut app, &ctx, vec![]);
                assert_aligned(&output);
                output.drop_without_applying_deltas();
            }
            assert!(app.file_view.as_ref().unwrap().scroll_y > 0.0);
        }
    }

    #[test]
    fn back_button_sits_to_the_left_of_the_legend() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        app.file_view = Some(loaded_file_view());
        let output = render(&mut app, &ctx, vec![]);
        let back = ctx
            .read_response(egui::Id::new("Back to folders"))
            .unwrap()
            .rect
            .center();
        let legend = text_centers(&output, "Different  1")[0];
        let title = text_centers(&output, "Versus")[0];
        assert!(back.x < legend.x);
        assert!((back.y - legend.y).abs() < 1.0);
        assert!(back.y > title.y);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn theme_toggle_updates_both_views_without_losing_state() {
        for file_mode in [false, true] {
            let ctx = egui::Context::default();
            apply_theme(&ctx);
            let mut app = loaded_app();
            app.tree.as_mut().unwrap().expand_all();
            app.selected = Some("assembly/model.step".into());
            if file_mode {
                app.file_view = Some(loaded_file_view());
            }
            let generation = app.scroll_generation;
            for (action, expected_dark) in [
                ("Switch to light mode", false),
                ("Switch to dark mode", true),
            ] {
                let output = render(&mut app, &ctx, vec![]);
                let toggle = ctx.read_response(egui::Id::new(action)).unwrap().rect;
                assert!(
                    toggle.right() <= 1190.0 && toggle.left() > 1100.0 && toggle.bottom() < 45.0
                );
                assert!(
                    text_positions(&output, action).is_empty(),
                    "Theme action must use only an icon"
                );
                output.drop_without_applying_deltas();
                click(&mut app, &ctx, toggle.center());
                let output = render(&mut app, &ctx, vec![]);
                assert_eq!(ctx.theme() == egui::Theme::Dark, expected_dark);
                assert_eq!(app.logo_texture.as_ref().unwrap().0, expected_dark);
                assert_eq!(app.file_view.is_some(), file_mode);
                assert!(app.tree.as_ref().unwrap().is_expanded("assembly"));
                assert_eq!(
                    app.selected.as_deref(),
                    Some(std::path::Path::new("assembly/model.step"))
                );
                assert_eq!(app.scroll_generation, generation);
                let neutral = if file_mode { "unchanged" } else { "model.step" };
                let palette = Palette::new(expected_dark);
                for shape in &output.shapes {
                    if let egui::Shape::Text(text) = &shape.shape {
                        if text.galley.text() == neutral {
                            assert_eq!(text.fallback_color, palette.muted);
                        }
                    }
                }
                output.drop_without_applying_deltas();
            }
        }
    }

    #[test]
    fn logo_texture_respects_transparency_of_stray_and_edge_pixels() {
        let ctx = egui::Context::default();
        let mut app = loaded_app();
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            set_theme(&ctx, theme);
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                    ..Default::default()
                },
                |ui| app.render(ui),
            );
            let logo_id = app.logo_texture.as_ref().unwrap().1.id();
            let delta = &output.textures_delta.set.get(&logo_id).unwrap()[0];
            let egui::ImageData::Color(image) = &delta.image;
            let speck = image[(91, 74)];
            assert_eq!(speck.a(), 1);
            assert!(
                speck.r() <= 1 && speck.g() <= 1 && speck.b() <= 1,
                "Nearly transparent pixels must not become bright white"
            );
            for pixel in &image.pixels {
                assert!(
                    pixel.r() <= pixel.a() && pixel.g() <= pixel.a() && pixel.b() <= pixel.a(),
                    "Logo texture must use premultiplied alpha"
                );
            }
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn theme_changes_send_matching_icon_to_native_window() {
        let ctx = egui::Context::default();
        let mut app = loaded_app();
        for theme in [egui::Theme::Dark, egui::Theme::Light, egui::Theme::Dark] {
            set_theme(&ctx, theme);
            let output = render(&mut app, &ctx, vec![]);
            let expected = crate::logo::themed_icon(theme == egui::Theme::Dark);
            assert!(output.viewport_output.values().flat_map(|viewport| &viewport.commands).any(|command| {
                matches!(command,egui::ViewportCommand::Icon(Some(icon)) if icon.rgba == expected.rgba)
            }), "Native window must receive the matching theme icon");
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn file_type_mismatch_opens_explanation_without_loading_nonfile_side() {
        let mut app = loaded_app();
        app.open_file("assembly/model.step".into(), [true, true], true);
        let view = app.file_view.as_ref().unwrap();
        assert!(view.job.is_none());
        assert!(
            view.error
                .as_deref()
                .unwrap()
                .contains("Entry types differ")
        );
        assert!(matches!(view.error_icon, StatusIcon::TypeMismatch));
    }

    #[test]
    fn file_worker_errors_remain_in_file_view_with_back_available() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = loaded_app();
        let (sender, receiver) = mpsc::channel();
        let mut view = loaded_file_view();
        view.comparison = None;
        view.job = Some(FileJob {
            receiver,
            cancellation: Arc::new(AtomicBool::new(false)),
        });
        app.file_view = Some(view);
        sender
            .send(Err(CompareError {
                path: None,
                kind: versus::CompareErrorKind::InvalidPath,
                message: "file unavailable".into(),
            }))
            .unwrap();
        app.poll_file_comparison(&ctx);
        let output = render(&mut app, &ctx, vec![]);
        assert!(
            ctx.read_response(egui::Id::new("Back to folders"))
                .is_some()
        );
        assert!(
            app.file_view
                .as_ref()
                .unwrap()
                .error
                .as_deref()
                .unwrap()
                .contains("file unavailable")
        );
        assert!(app.tree.as_ref().is_some());
        output.drop_without_applying_deltas();
    }

    fn attach_job(app: &mut VersusApp) -> mpsc::Sender<Result<FolderTree, CompareError>> {
        let (sender, receiver) = mpsc::channel();
        app.job = Some(ComparisonJob {
            receiver,
            cancellation: Arc::new(AtomicBool::new(false)),
            roots: ["new-left".into(), "new-right".into()],
            started: Instant::now(),
        });
        sender
    }

    #[test]
    fn changing_paths_cancels_scan_and_clears_prior_results() {
        let mut app = loaded_app();
        let sender = attach_job(&mut app);
        let flag = app.job.as_ref().unwrap().cancellation.clone();
        app.invalidate();
        assert!(flag.load(Ordering::Relaxed));
        assert!(app.job.is_none() && app.tree.is_none() && app.roots.is_none());
        assert!(sender.send(Ok(fixture())).is_err());
    }

    #[test]
    fn cancelled_completed_job_does_not_replace_last_comparison() {
        let mut app = loaded_app();
        let sender = attach_job(&mut app);
        sender.send(Ok(fixture())).unwrap();
        app.job
            .as_ref()
            .unwrap()
            .cancellation
            .store(true, Ordering::Relaxed);
        app.poll_comparison(&egui::Context::default());
        assert!(app.job.is_none());
        assert_eq!(
            app.roots.as_ref().unwrap(),
            &[PathBuf::from("left"), PathBuf::from("right")]
        );
        assert_eq!(app.message, "Comparison cancelled.");
    }

    #[test]
    fn failed_worker_reports_error_and_keeps_last_comparison() {
        let mut app = loaded_app();
        drop(attach_job(&mut app));
        app.poll_comparison(&egui::Context::default());
        assert!(app.error.as_ref().unwrap().contains("worker stopped"));
        assert!(app.tree.is_some());
        assert_eq!(app.roots.as_ref().unwrap()[0], PathBuf::from("left"));
    }
}
