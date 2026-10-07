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
    CompareError, DirectoryCompareOptions, DirectoryEntryKind, DirectoryEntryState, FolderTree,
    TreeNode,
};

const BACKGROUND: Color32 = Color32::from_rgb(16, 21, 29);
const PANEL: Color32 = Color32::from_rgb(23, 30, 40);
const BORDER: Color32 = Color32::from_rgb(48, 59, 74);
const MUTED: Color32 = Color32::from_rgb(145, 161, 181);
const TEXT: Color32 = Color32::from_rgb(221, 230, 240);
const ACCENT: Color32 = Color32::from_rgb(95, 163, 255);
const CHANGED: Color32 = Color32::from_rgb(244, 191, 98);
const LEFT_ONLY: Color32 = Color32::from_rgb(94, 211, 221);
const RIGHT_ONLY: Color32 = Color32::from_rgb(189, 157, 255);
const ERROR: Color32 = Color32::from_rgb(255, 127, 137);
const SAME: Color32 = Color32::from_rgb(133, 187, 157);
const ROW_HEIGHT: f32 = 22.0;

struct ComparisonJob {
    receiver: Receiver<Result<FolderTree, CompareError>>,
    cancellation: Arc<AtomicBool>,
    roots: [PathBuf; 2],
    started: Instant,
}

pub struct VersusApp {
    paths: [String; 2],
    roots: Option<[PathBuf; 2]>,
    tree: Option<FolderTree>,
    job: Option<ComparisonJob>,
    selected: Option<PathBuf>,
    message: String,
    error: Option<String>,
    elapsed: Option<Duration>,
    counts: [usize; 6],
    logo_texture: Option<egui::TextureHandle>,
    scroll_generation: u64,
}

impl Default for VersusApp {
    fn default() -> Self {
        Self {
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

    fn render(&mut self, ui: &mut egui::Ui) {
        if self.logo_texture.is_none() {
            let image: egui::ColorImage = (&crate::logo::icon_data()).into();
            self.logo_texture = Some(ui.ctx().load_texture(
                "versus-mark",
                image,
                egui::TextureOptions::LINEAR,
            ));
        }
        egui::Frame::default()
            .fill(BACKGROUND)
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                self.header(ui);
                ui.add_space(5.0);
                self.legend(ui);
                ui.add_space(5.0);
                if let Some(error) = &self.error {
                    egui::Frame::default()
                        .fill(ERROR.gamma_multiply(0.12))
                        .inner_margin(egui::Margin::same(6))
                        .show(ui, |ui| {
                            ui.colored_label(ERROR, error);
                        });
                    ui.add_space(4.0);
                }
                let height = (ui.available_height() - 36.0).max(120.0);
                self.tree_area(ui, height);
                ui.add_space(5.0);
                self.footer(ui);
            });
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(logo) = &self.logo_texture {
                ui.image((logo.id(), egui::vec2(28.0, 28.0)));
            }
            ui.label(RichText::new("Versus").size(18.0).strong());
            ui.label(
                RichText::new("/  FOLDER COMPARE")
                    .monospace()
                    .size(10.0)
                    .color(MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.controls(ui)
            });
        });
    }

    fn folder_inputs(&mut self, ui: &mut egui::Ui) {
        let mut edited = false;
        let mut picked = false;
        ui.columns(2, |columns| {
            for (side, column) in columns.iter_mut().enumerate() {
                let color = if side == 0 { LEFT_ONLY } else { RIGHT_ONLY };
                egui::Frame::default()
                    .inner_margin(egui::Margin::same(7))
                    .show(column, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(if side == 0 { "LEFT" } else { "RIGHT" })
                                    .monospace()
                                    .size(10.0)
                                    .color(color),
                            );
                            let width = (ui.available_width() - 30.0).max(80.0);
                            let response = ui.add_sized(
                                [width, 24.0],
                                egui::TextEdit::singleline(&mut self.paths[side])
                                    .font(egui::TextStyle::Monospace)
                                    .hint_text("Enter or paste a folder path")
                                    .id_salt(("folder-path", side)),
                            );
                            edited |= response.changed();
                            if response.lost_focus()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter))
                            {
                                picked = true;
                            }
                            if icon_button(
                                ui,
                                ToolbarIcon::Browse,
                                true,
                                if side == 0 {
                                    "Browse left folder"
                                } else {
                                    "Browse right folder"
                                },
                            )
                            .clicked()
                            {
                                let mut dialog = rfd::FileDialog::new().set_title(if side == 0 {
                                    "Select left folder"
                                } else {
                                    "Select right folder"
                                });
                                if !self.paths[side].is_empty() {
                                    dialog = dialog.set_directory(&self.paths[side]);
                                }
                                if let Some(path) = dialog.pick_folder() {
                                    self.paths[side] = path.to_string_lossy().into_owned();
                                    edited = true;
                                    picked = true;
                                }
                            }
                        });
                        let full_path = if self.paths[side].is_empty() {
                            if side == 0 {
                                "Choose a left folder".into()
                            } else {
                                "Choose a right folder".into()
                            }
                        } else {
                            absolute_path(&self.paths[side]).display().to_string()
                        };
                        ui.add(
                            egui::Label::new(
                                RichText::new(full_path).monospace().size(10.0).color(MUTED),
                            )
                            .wrap(),
                        );
                    });
            }
        });
        if edited {
            self.invalidate();
        }
        if picked {
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

    fn legend(&self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for (index, icon) in [
                StatusIcon::Same,
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
                let label = if self.tree.is_some() {
                    format!("{}  {}", icon.label(), self.counts[index])
                } else {
                    icon.label().into()
                };
                ui.label(RichText::new(label).size(10.0).color(icon.color()));
                ui.add_space(5.0);
            }
        });
    }

    fn tree_area(&mut self, ui: &mut egui::Ui, height: f32) {
        egui::Frame::default()
            .fill(PANEL)
            .stroke(Stroke::new(1.0, BORDER))
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
                    Stroke::new(1.0, BORDER),
                );
                ui.painter().line_segment(
                    [
                        egui::pos2(divider.center().x, header_top),
                        divider.center_bottom(),
                    ],
                    Stroke::new(1.0, BORDER),
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
                                            ui.colored_label(ERROR, error.to_string());
                                        }
                                    });
                                }
                                ui.painter().line_segment(
                                    [rect.center_top(), rect.center_bottom()],
                                    Stroke::new(1.0, BORDER),
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
    }

    fn footer(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.message).size(11.0).color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(elapsed) = self.elapsed {
                    ui.label(
                        RichText::new(format!("{:.2}s", elapsed.as_secs_f64()))
                            .monospace()
                            .size(11.0)
                            .color(MUTED),
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
                        .color(TEXT),
                )
                .truncate(),
            );
        } else {
            ui.label(RichText::new("Click a folder on either side to expand both trees.  •  Symlinks are not traversed.").size(10.0).color(MUTED));
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
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        BACKGROUND.to_normalized_gamma_f32()
    }
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll_comparison(ctx);
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

fn count_entries(tree: &FolderTree) -> [usize; 6] {
    let mut counts = [0; 6];
    let mut nodes: Vec<_> = tree.root().children.iter().collect();
    while let Some(node) = nodes.pop() {
        let index = match node.state {
            DirectoryEntryState::Same => 0,
            DirectoryEntryState::Different => 1,
            DirectoryEntryState::LeftOnly => 2,
            DirectoryEntryState::RightOnly => 3,
            DirectoryEntryState::TypeMismatch => 4,
            DirectoryEntryState::Error(_) => 5,
        };
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
    Same,
    Different,
    LeftOnly,
    RightOnly,
    TypeMismatch,
    Error,
}

impl StatusIcon {
    fn label(self) -> &'static str {
        match self {
            Self::Same => "Identical",
            Self::Different => "Different",
            Self::LeftOnly => "Left only",
            Self::RightOnly => "Right only",
            Self::TypeMismatch => "Type mismatch",
            Self::Error => "Read error",
        }
    }
    fn color(self) -> Color32 {
        match self {
            Self::Same => SAME,
            Self::Different => CHANGED,
            Self::LeftOnly => LEFT_ONLY,
            Self::RightOnly => RIGHT_ONLY,
            Self::TypeMismatch | Self::Error => ERROR,
        }
    }
}

fn status_icon(state: &DirectoryEntryState) -> StatusIcon {
    match state {
        DirectoryEntryState::Same => StatusIcon::Same,
        DirectoryEntryState::Different => StatusIcon::Different,
        DirectoryEntryState::LeftOnly => StatusIcon::LeftOnly,
        DirectoryEntryState::RightOnly => StatusIcon::RightOnly,
        DirectoryEntryState::TypeMismatch => StatusIcon::TypeMismatch,
        DirectoryEntryState::Error(_) => StatusIcon::Error,
    }
}

fn state_label(state: &DirectoryEntryState) -> &'static str {
    status_icon(state).label()
}
fn state_color(state: &DirectoryEntryState) -> Color32 {
    status_icon(state).color()
}

fn paint_status_icon(painter: &egui::Painter, center: Pos2, icon: StatusIcon) {
    let stroke = Stroke::new(1.3, icon.color());
    let point = |x, y| center + egui::vec2(x, y);
    match icon {
        StatusIcon::Same => {
            painter.line(
                vec![point(-5.0, 0.0), point(-1.0, 4.0), point(5.0, -4.0)],
                stroke,
            );
        }
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
            painter.circle_filled(point(0.0, 3.0), 0.7, icon.color());
        }
    }
}

#[derive(Clone, Copy)]
enum ToolbarIcon {
    Refresh,
    Expand,
    Collapse,
    Browse,
    Cancel,
}

fn icon_button(ui: &mut egui::Ui, icon: ToolbarIcon, enabled: bool, label: &str) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), Sense::hover());
        let response = ui.interact(rect, egui::Id::new(label), Sense::click());
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
        let color = if !enabled {
            MUTED.gamma_multiply(0.4)
        } else if response.hovered() || response.has_focus() {
            ACCENT
        } else {
            TEXT
        };
        if response.hovered() || response.has_focus() {
            ui.painter().rect_filled(rect, 3, BORDER);
        }
        let center = rect.center();
        let point = |x, y| center + egui::vec2(x, y);
        let stroke = Stroke::new(1.4, color);
        match icon {
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
                let direction = if matches!(icon, ToolbarIcon::Expand) {
                    1.0
                } else {
                    -1.0
                };
                for y in [-3.0, 3.0] {
                    ui.painter().line(
                        vec![
                            point(-5.0, y - direction * 2.0),
                            point(0.0, y + direction * 2.0),
                            point(5.0, y - direction * 2.0),
                        ],
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
    let color = state_color(state);
    let fill = if selected {
        ACCENT.gamma_multiply(0.16)
    } else if hovered {
        Color32::from_rgb(34, 44, 58)
    } else if alternate {
        Color32::from_rgb(26, 34, 45)
    } else {
        PANEL
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
            MUTED.gamma_multiply(0.55),
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
            MUTED,
        );
    }
    paint_icon(&name_painter, egui::pos2(x + 19.0, y), kind, color);
    name_painter.text(
        egui::pos2(x + 40.0, y),
        Align2::LEFT_CENTER,
        node.name.to_string_lossy(),
        FontId::monospace(11.0),
        if *state == DirectoryEntryState::Same {
            TEXT
        } else {
            color
        },
    );
    painter.text(
        egui::pos2(rect.right() - 32.0, y),
        Align2::RIGHT_CENTER,
        format_size(entry.size),
        FontId::monospace(10.0),
        MUTED,
    );
    paint_status_icon(
        &painter,
        egui::pos2(rect.right() - 15.0, y),
        status_icon(state),
    );
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
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height.max(80.0)),
        Sense::hover(),
    );
    ui.painter().line_segment(
        [rect.center_top(), rect.center_bottom()],
        Stroke::new(1.0, BORDER.gamma_multiply(0.5)),
    );
    let painter = ui.painter().with_clip_rect(rect);
    let center = rect.center();
    painter.text(
        center - egui::vec2(0.0, 24.0),
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(21.0),
        TEXT,
    );
    painter.text(
        center + egui::vec2(0.0, 8.0),
        Align2::CENTER_CENTER,
        subtitle,
        FontId::proportional(11.0),
        MUTED,
    );
}

fn apply_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BACKGROUND;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = BACKGROUND;
    visuals.override_text_color = Some(TEXT);
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.3);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
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
        let legend = text_positions(&output, "Identical  0")[0];
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
