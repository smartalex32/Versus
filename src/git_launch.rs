//! Blocking native-window lifetime and exit status for explicit Git invocations.

use crate::app::{GitApp, GitLaunch};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

pub(crate) fn run(launch: GitLaunch) -> std::process::ExitCode {
    let (guard, logging_error) = match crate::diagnostics::initialize() {
        Ok(guard) => (Some(guard), None),
        Err(error) => (None, Some(error)),
    };
    // Merge requires explicit save success; closing a read-only Diff advances Git.
    let completion = Arc::new(AtomicU8::new(launch.initial_exit_code()));
    let status = completion.clone();
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_title(match &launch {
            GitLaunch::Diff(_) => "Versus — Git Diff",
            GitLaunch::Merge(_) => "Versus — Git Merge",
        })
        .with_icon(crate::logo::themed_icon(true))
        .with_inner_size([1400.0, 850.0])
        .with_min_inner_size([900.0, 650.0]);
    #[cfg(target_os = "linux")]
    let viewport =
        if std::env::var_os("VERSUS_X11_COMPAT").as_deref() == Some(std::ffi::OsStr::new("1")) {
            viewport
                .with_maximized(true)
                .with_decorations(false)
                .with_min_inner_size([1.0, 1.0])
        } else {
            viewport
        };
    let result = eframe::run_native(
        "Versus",
        eframe::NativeOptions {
            viewport,
            event_loop_builder: crate::drop_capable_event_loop(),
            persist_window: false,
            ..Default::default()
        },
        Box::new(move |cc| Ok(Box::new(GitApp::new(cc, launch, status, logging_error)))),
    );
    if let Err(error) = result {
        crate::diagnostics::error(format!("Git window failed: {error}"));
        crate::attach_parent_console();
        eprintln!("Versus: {error}");
        return std::process::ExitCode::from(2);
    }
    crate::diagnostics::info("Git window closed");
    drop(guard);
    std::process::ExitCode::from(completion.load(Ordering::Acquire))
}
