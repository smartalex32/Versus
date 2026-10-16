//! Session sources regression coverage.

use super::*;
use crate::app::visuals::theme::{apply_theme, set_theme};

#[test]
fn merge_source_paths_stay_on_one_line_and_keep_their_names_visible() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for kind in [SourceKind::Folder, SourceKind::File] {
            let ctx = egui::Context::default();
            set_theme(&ctx, theme);
            let mut app = VersusApp::default();
            app.session.comparison_mode = if kind == SourceKind::File {
                ComparisonMode::File
            } else {
                ComparisonMode::Folder
            };
            app.session.set_workspace_merge(true);
            app.session.source_kinds = [Some(kind); 2];
            let name = if kind == SourceKind::File {
                "model.rs"
            } else {
                "release"
            };
            app.session.paths = [
                engineering_path(&format!("{}project/{name}", "é𐐀目录/".repeat(60))),
                engineering_path(&format!("short/{name}")),
            ];
            let output = render(&mut app, &ctx, vec![]);
            let (shape, path) = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text().starts_with('…') => {
                        Some((shape, text))
                    }
                    _ => None,
                })
                .expect("long Merge paths should have a leading ellipsis");
            let suffix = PathBuf::from("project").join(name);
            assert!(path.galley.text().ends_with(suffix.to_str().unwrap()));
            assert_eq!(path.galley.rows.len(), 1);
            let bounds = Rect::from_min_size(path.pos, path.galley.size());
            assert!(app.source_header_rects[0].contains_rect(bounds));
            assert!(shape.clip_rect.contains_rect(bounds));
            let browse = ctx
                .read_response(egui::Id::new(format!(
                    "Browse left {}",
                    if kind == SourceKind::File {
                        "file"
                    } else {
                        "folder"
                    }
                )))
                .unwrap()
                .rect;
            assert!(bounds.right() < browse.left());
            let short = text_centers(&output, &app.session.paths[1])[0];
            assert!((bounds.center().y - short.y).abs() < 0.1);
            let height = app.source_header_rects[0].height();
            output.drop_without_applying_deltas();
            app.session.paths[0] = engineering_path(&format!("short/{name}"));
            render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
            assert_eq!(app.source_header_rects[0].height(), height);
        }
    }
}

#[test]
fn folder_worker_panic_becomes_a_status_error_and_releases_the_job() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = loaded_app();
    let sender = attach_job(&mut app);
    std::thread::spawn(move || {
        let result = crate::diagnostics::catch_worker(
            "Folder comparison",
            || -> Result<FolderTree, &'static str> {
                panic!("test comparison failure");
            },
        )
        .map_err(|message| CompareError {
            path: None,
            kind: versus::CompareErrorKind::Io,
            message,
        });
        sender.send(result).unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.session.job.is_some() {
        app.session.poll_comparison(&ctx);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        app.session
            .error
            .as_ref()
            .unwrap()
            .contains("worker panicked")
    );
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "1 error").len(), 1);
    assert!(
        app.session.tree.is_some(),
        "failure keeps the previous tree usable"
    );
    output.drop_without_applying_deltas();
}

#[test]
fn cancelled_completed_job_does_not_replace_last_comparison() {
    let mut app = loaded_app();
    let sender = attach_job(&mut app);
    sender.send(Ok(fixture())).unwrap();
    app.session
        .job
        .as_ref()
        .unwrap()
        .cancellation
        .store(true, Ordering::Relaxed);
    app.session.poll_comparison(&egui::Context::default());
    assert!(app.session.job.is_none());
    assert_eq!(
        app.session.roots.as_ref().unwrap(),
        &[PathBuf::from("left"), PathBuf::from("right")]
    );
    assert_eq!(app.session.message, "Comparison cancelled.");
}

#[test]
fn failed_worker_reports_error_and_keeps_last_comparison() {
    let mut app = loaded_app();
    drop(attach_job(&mut app));
    app.session.poll_comparison(&egui::Context::default());
    assert!(
        app.session
            .error
            .as_ref()
            .unwrap()
            .contains("worker stopped")
    );
    assert!(app.session.tree.is_some());
    assert_eq!(
        app.session.roots.as_ref().unwrap()[0],
        PathBuf::from("left")
    );
}

#[test]
fn command_line_files_open_direct_comparison_in_supplied_order_without_writes() {
    let sources = SourceFixture::new();
    let paths = [
        sources.path("left/model.txt"),
        sources.path("right/model.txt"),
    ];
    for mode in [None, Some(ComparisonMode::File)] {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut app = VersusApp::default();
        app.session.open_launch_request(crate::cli::LaunchRequest {
            paths: paths.clone(),
            mode,
        });
        settle_sources(&mut app, &ctx);
        assert_eq!(app.session.comparison_mode, ComparisonMode::File);
        let view = app.session.file_view.as_ref().unwrap();
        assert_eq!(view.paths, paths);
        assert_eq!(view.sources, paths.clone().map(Some));
        assert!(!view.from_folders);
        assert_eq!(view.counts[1], 1);
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(text_positions(&output, "old").len(), 1);
        assert_eq!(text_positions(&output, "new").len(), 1);
        assert!(
            ctx.read_response(egui::Id::new("Back to folders"))
                .is_none()
        );
        output.drop_without_applying_deltas();
    }
    assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "common\nold\n");
    assert_eq!(std::fs::read_to_string(&paths[1]).unwrap(), "common\nnew\n");
}

#[test]
fn command_line_folders_choose_folder_mode_and_reject_forced_file_mode() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let paths = [sources.path("left"), sources.path("right")];
    for mode in [None, Some(ComparisonMode::Folder)] {
        let mut app = VersusApp::default();
        app.session.open_launch_request(crate::cli::LaunchRequest {
            paths: paths.clone(),
            mode,
        });
        settle_sources(&mut app, &ctx);
        assert_eq!(app.session.comparison_mode, ComparisonMode::Folder);
        assert_eq!(app.session.roots, Some(paths.clone()));
        assert!(app.session.tree.is_some() && app.session.file_view.is_none());
    }
    for (mode, paths) in [
        (ComparisonMode::File, paths),
        (
            ComparisonMode::Folder,
            [
                sources.path("left/model.txt"),
                sources.path("right/model.txt"),
            ],
        ),
    ] {
        let mut app = VersusApp::default();
        app.session.open_launch_request(crate::cli::LaunchRequest {
            paths,
            mode: Some(mode),
        });
        settle_sources(&mut app, &ctx);
        assert!(app.session.error.as_ref().unwrap().contains("requires two"));
        assert!(app.session.tree.is_none() && app.session.file_view.is_none());
    }
}

#[test]
fn command_line_invalid_sources_show_errors_and_new_discards_pending_launch() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    for paths in [
        [sources.path("left/model.txt"), sources.path("missing")],
        [sources.path("left/model.txt"), sources.path("right")],
    ] {
        let mut app = VersusApp::default();
        app.session
            .open_launch_request(crate::cli::LaunchRequest { paths, mode: None });
        settle_sources(&mut app, &ctx);
        assert!(app.session.error.is_some() || app.session.mode() == SelectionMode::Incompatible);
        assert!(app.session.tree.is_none() && app.session.file_view.is_none());
    }
    let mut app = VersusApp::default();
    app.session.open_launch_request(crate::cli::LaunchRequest {
        paths: [
            sources.path("left/model.txt"),
            sources.path("right/model.txt"),
        ],
        mode: None,
    });
    app.session.new_comparison();
    settle_sources(&mut app, &ctx);
    assert!(app.session.paths.iter().all(String::is_empty));
    assert!(app.session.source_paths.iter().all(Option::is_none));
    assert!(app.session.launch_mode.is_none());
    assert!(app.session.tree.is_none() && app.session.file_view.is_none());
}

#[test]
fn git_null_device_inputs_show_added_and_deleted_lines_without_opening_devices() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    let mut nulls = vec![PathBuf::from("/dev/null")];
    if cfg!(windows) {
        nulls.extend([PathBuf::from("NUL"), PathBuf::from("nul")]);
    }
    for null in nulls {
        for empty_side in 0..2 {
            let mut paths = [
                sources.path("left/model.txt"),
                sources.path("right/model.txt"),
            ];
            paths[empty_side] = null.clone();
            let mut app = VersusApp::default();
            app.session.open_launch_request(crate::cli::LaunchRequest {
                paths,
                mode: Some(ComparisonMode::File),
            });
            settle_sources(&mut app, &ctx);
            let view = app.session.file_view.as_ref().unwrap();
            assert!(view.error.is_none());
            assert!(view.sources[empty_side].is_none());
            let expected = if empty_side == 0 {
                DirectoryEntryState::RightOnly
            } else {
                DirectoryEntryState::LeftOnly
            };
            let rows = &view.comparison.as_ref().unwrap().rows;
            assert_eq!(rows.len(), 2);
            assert!(rows.iter().all(|row| row.state == expected));
        }
    }
}

#[test]
fn command_line_native_paths_survive_display_conversion() {
    #[cfg(target_os = "linux")]
    use std::os::unix::ffi::OsStringExt;
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    // Linux permits non-UTF-8 filenames; macOS filesystems require Unicode.
    #[cfg(target_os = "linux")]
    let left_name = std::ffi::OsString::from_vec(b"left \xff.txt".to_vec());
    #[cfg(not(target_os = "linux"))]
    let left_name = std::ffi::OsString::from("left space é.txt");
    let left = sources.0.join(left_name);
    let right = sources.path("right space \u{6bd4}\u{8f03}.txt");
    std::fs::write(&left, "before\n").unwrap();
    std::fs::write(&right, "after\n").unwrap();
    let mut app = VersusApp::default();
    app.session.open_launch_request(crate::cli::LaunchRequest {
        paths: [left.clone(), right.clone()],
        mode: Some(ComparisonMode::File),
    });
    settle_sources(&mut app, &ctx);
    let view = app.session.file_view.as_ref().unwrap();
    assert_eq!(view.sources, [Some(left), Some(right)]);
    assert!(view.error.is_none());
    assert_eq!(view.counts[1], 1);
}

#[test]
fn selected_sources_switch_views_and_compare_only_matching_types() {
    let sources = SourceFixture::new();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    app.session.switch_comparison_mode(ComparisonMode::File);
    // Starting on the right must work just as starting on the left does.
    app.session.select_path(1, sources.path("right/model.txt"));
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.mode(), SelectionMode::File);
    let view = app.session.file_view.as_ref().unwrap();
    assert!(!view.from_folders && view.comparison.is_none() && view.job.is_none());
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "File Diff").len(), 1);
    assert!(
        ctx.read_response(egui::Id::new("Back to folders"))
            .is_none()
    );
    assert!(
        ctx.read_response(egui::Id::new("Browse left file"))
            .is_some()
    );
    assert!(
        ctx.read_response(egui::Id::new("Browse right file"))
            .is_some()
    );
    output.drop_without_applying_deltas();
    app.session.select_path(0, sources.path("left/model.txt"));
    settle_sources(&mut app, &ctx);
    assert!(app.session.file_view.as_ref().unwrap().comparison.is_some());
    assert_eq!(app.session.file_view.as_ref().unwrap().counts[1], 1);
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "old").len(), 1);
    assert_eq!(text_positions(&output, "new").len(), 1);
    assert!(
        ctx.read_response(egui::Id::new("Back to folders"))
            .is_none()
    );
    output.drop_without_applying_deltas();
    // Replacing the right file with a folder rejects the mixed pair.
    app.session.select_path(1, sources.path("right"));
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.mode(), SelectionMode::Incompatible);
    assert!(
        app.session.file_view.is_none() && app.session.tree.is_none() && app.session.job.is_none()
    );
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(
        text_positions(&output, "Incompatible source types").len(),
        1
    );
    assert!(text_positions(&output, &app.session.message).len() >= 1);
    assert!(
        app.session
            .message
            .contains("No comparison can be made between a file and a folder")
    );
    output.drop_without_applying_deltas();
    // Replacing the left side with a folder restores automatic comparison.
    app.session.select_path(0, sources.path("left"));
    settle_sources(&mut app, &ctx);
    assert_eq!(app.session.mode(), SelectionMode::Folder);
    assert!(app.session.file_view.is_none() && app.session.tree.is_some());
    assert_eq!(app.session.counts[1], 1);
}

#[test]
fn comparison_mode_buttons_stay_centered_with_icons_in_both_themes() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for (width, height) in [(900.0, 650.0), (1200.0, 800.0)] {
            for state in 0..4 {
                let ctx = egui::Context::default();
                set_theme(&ctx, theme);
                let mut app = if state == 0 {
                    VersusApp::default()
                } else {
                    loaded_app()
                };
                let _sender = (state == 2).then(|| attach_job(&mut app));
                if state == 3 {
                    app.session.comparison_mode = ComparisonMode::File;
                    let mut view = loaded_file_view();
                    view.from_folders = false;
                    app.session.file_view = Some(view);
                }
                let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(width, height));
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        ..Default::default()
                    },
                    |ui| app.render(ui),
                );
                let texture = app.logo_texture.as_ref().unwrap().1.id();
                let logo = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect) if rect.fill_texture_id() == texture => {
                            Some(rect.rect)
                        }
                        _ => None,
                    })
                    .unwrap();
                let folder = ctx
                    .read_response(egui::Id::new("Folder Diff"))
                    .unwrap()
                    .rect;
                let file = ctx.read_response(egui::Id::new("File Diff")).unwrap().rect;
                assert!(ctx.read_response(egui::Id::new("File Merge")).is_none());
                assert!((folder.union(file).center().x - screen.center().x).abs() < 0.5);
                assert!(folder.right() < file.left());
                assert!(file.right() < screen.right());
                assert!(logo.right() < folder.left());
                let theme_button = ctx
                    .read_response(egui::Id::new(if theme == egui::Theme::Dark {
                        "Switch to light mode"
                    } else {
                        "Switch to dark mode"
                    }))
                    .unwrap()
                    .rect;
                assert!(file.right() < theme_button.left());
                for (label, rect) in [("Folder Diff", folder), ("File Diff", file)] {
                    assert!(screen.contains_rect(rect));
                    let text = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) if text.galley.text() == label => Some(text),
                            _ => None,
                        })
                        .unwrap();
                    let bounds = Rect::from_min_size(text.pos, text.galley.size());
                    assert!((bounds.center().y - logo.center().y).abs() < 1.0);
                    assert!(rect.contains_rect(bounds));
                    let icon_center = egui::pos2(
                        rect.left() + ctx.style_of(theme).spacing.button_padding.x + 8.0,
                        rect.center().y,
                    );
                    let icon = Rect::from_center_size(icon_center, egui::vec2(16.0, 16.0));
                    assert!(icon.right() < bounds.left());
                    assert!(
                        output.shapes.iter().any(|shape| match &shape.shape {
                            egui::Shape::Rect(shape) if label == "Folder Diff" =>
                                icon.contains_rect(shape.rect)
                                    && shape.rect.width() == 13.0
                                    && shape.stroke.width > 0.0,
                            egui::Shape::Path(shape) if label == "File Diff" =>
                                shape.closed
                                    && shape.points.len() == 5
                                    && shape.points.iter().all(|point| icon.contains(*point)),
                            _ => false,
                        }),
                        "{label} must have a visible icon"
                    );
                }
                output.drop_without_applying_deltas();
            }
        }
    }
}

#[test]
fn diff_after_merge_keeps_source_errors_in_bottom_status() {
    let fixture = SourceFixture::new();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut app = VersusApp::default();
    app.session.set_workspace_merge(true);
    app.session.select_path(0, fixture.path("left/model.txt"));
    settle_merge_file(&mut app, &ctx, 0, "common\nold\n");
    app.session.set_workspace_merge(false);
    assert!(app.session.merge_file_open);
    app.session.source_errors[1] = Some("Cannot inspect peer source".into());
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "1 error").len(), 1);
    assert!(
        ctx.read_response(egui::Id::new("workspace-status"))
            .unwrap()
            .rect
            .top()
            >= app.pane_rects[0].bottom()
    );
    output.drop_without_applying_deltas();
}

#[test]
fn compare_icons_are_clickable_and_mode_buttons_support_keyboard_activation() {
    let ctx = egui::Context::default();
    let mut app = VersusApp::default();
    render(&mut app, &ctx, vec![]).drop_without_applying_deltas();
    let file = ctx.read_response(egui::Id::new("File Diff")).unwrap().rect;
    let icon = egui::pos2(file.left() + 12.0, file.center().y);
    let output = render(&mut app, &ctx, vec![egui::Event::PointerMoved(icon)]);
    assert_eq!(
        output.platform_output.cursor_icon,
        egui::CursorIcon::PointingHand
    );
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, icon);
    assert_eq!(app.session.comparison_mode, ComparisonMode::File);
    for (key, mode) in [
        (egui::Key::Enter, ComparisonMode::Folder),
        (egui::Key::Space, ComparisonMode::File),
    ] {
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new(mode.label())));
        render(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        )
        .drop_without_applying_deltas();
        assert_eq!(app.session.comparison_mode, mode);
    }
}

#[test]
fn new_comparison_button_clears_both_sides_and_preserves_theme() {
    let ctx = egui::Context::default();
    set_theme(&ctx, egui::Theme::Light);
    let mut app = VersusApp::default();
    app.session.paths = ["/left/model.step".into(), "/right/model.step".into()];
    app.session.source_kinds = [Some(SourceKind::File); 2];
    let mut view = loaded_file_view();
    view.from_folders = false;
    app.session.file_view = Some(view);
    let output = render(&mut app, &ctx, vec![]);
    let new = ctx
        .read_response(egui::Id::new("New comparison"))
        .unwrap()
        .rect;
    let legend = text_centers(&output, "Different  1")[0];
    assert!(new.center().y < app.pane_rects[0].top());
    assert!(legend.y > app.pane_rects[0].bottom());
    output.drop_without_applying_deltas();
    click(&mut app, &ctx, new.center());
    assert_eq!(app.session.paths, [String::new(), String::new()]);
    assert_eq!(app.session.mode(), SelectionMode::Empty);
    assert!(
        app.session.file_view.is_none()
            && app.session.tree.is_none()
            && app.session.roots.is_none()
    );
    assert_eq!(ctx.theme(), egui::Theme::Light);
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(text_positions(&output, "File Diff").len(), 1);
    assert_eq!(text_positions(&output, "Choose a folder").len(), 2);
    output.drop_without_applying_deltas();
}

#[test]
fn new_comparison_cancels_all_jobs_and_discards_late_results() {
    let mut app = loaded_app();
    let folder_sender = attach_job(&mut app);
    let folder_flag = app.session.job.as_ref().unwrap().cancellation.clone();
    let (file_sender, file_receiver) = mpsc::channel();
    let file_flag = Arc::new(AtomicBool::new(false));
    let mut view = loaded_file_view();
    view.job = Some(FileJob {
        receiver: file_receiver,
        cancellation: file_flag.clone(),
        progress: Arc::new(ComparisonProgress::default()),
        ignore_line_endings: true,
    });
    app.session.file_view = Some(view);
    let (source_sender, source_receiver) = mpsc::channel();
    let source_flag = Arc::new(AtomicBool::new(false));
    app.session.source_jobs[0] = Some(SourceJob {
        receiver: source_receiver,
        cancellation: source_flag.clone(),
    });
    app.session.source_errors[1] = Some("old error".into());
    app.session.paths = ["/old/left".into(), "/old/right".into()];
    app.session.source_kinds = [Some(SourceKind::Folder); 2];
    app.session.new_comparison();
    for flag in [folder_flag, file_flag, source_flag] {
        assert!(flag.load(Ordering::Relaxed));
    }
    assert!(folder_sender.send(Ok(fixture())).is_err());
    assert!(file_sender.send(Ok(None)).is_err());
    assert!(source_sender.send(Ok(SourceKind::File)).is_err());
    settle_sources(&mut app, &egui::Context::default());
    assert_eq!(app.session.mode(), SelectionMode::Empty);
    assert_eq!(app.session.source_kinds, [None; 2]);
    assert!(app.session.source_errors.iter().all(Option::is_none));
    assert!(app.session.error.is_none());
}

#[test]
fn replacing_a_source_discards_pending_classification_and_prior_results() {
    let sources = SourceFixture::new();
    let mut app = loaded_app();
    let (sender, receiver) = mpsc::channel();
    let cancellation = Arc::new(AtomicBool::new(false));
    app.session.source_jobs[0] = Some(SourceJob {
        receiver,
        cancellation: cancellation.clone(),
    });
    app.session.select_path(0, sources.path("left/model.txt"));
    assert!(cancellation.load(Ordering::Relaxed));
    assert!(sender.send(Ok(SourceKind::Folder)).is_err());
    assert!(app.session.tree.is_none() && app.session.roots.is_none());
    settle_sources(&mut app, &egui::Context::default());
    assert_eq!(app.session.source_kinds[0], Some(SourceKind::File));
    assert_eq!(app.session.mode(), SelectionMode::File);
}
