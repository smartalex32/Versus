#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod cli;
mod logo;
#[cfg(windows)]
mod native_drop;
mod selection;

fn main() -> std::process::ExitCode {
    let launch = match cli::parse(std::env::args_os().skip(1)) {
        Ok(cli::Command::Open(launch)) => launch,
        Ok(cli::Command::Help) => {
            attach_parent_console();
            print!("{}", cli::HELP);
            return std::process::ExitCode::SUCCESS;
        }
        Ok(cli::Command::Version) => {
            attach_parent_console();
            println!("Versus {}", env!("CARGO_PKG_VERSION"));
            return std::process::ExitCode::SUCCESS;
        }
        Err(error) => {
            attach_parent_console();
            eprintln!("Versus: {error}\n\n{}", cli::HELP);
            return std::process::ExitCode::from(2);
        }
    };
    match run(launch) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            attach_parent_console();
            eprintln!("Versus: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(launch: Option<cli::LaunchRequest>) -> eframe::Result {
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_title("Versus — Compare")
        .with_icon(logo::themed_icon(true))
        .with_inner_size([1200.0, 800.0])
        .with_min_inner_size([900.0, 650.0]);
    #[cfg(target_os = "linux")]
    let viewport =
        if std::env::var_os("VERSUS_X11_COMPAT").as_deref() == Some(std::ffi::OsStr::new("1")) {
            // The private display has its own window manager. Let it resize this
            // window with the outer Xephyr window, without a second title bar.
            viewport.with_maximized(true).with_decorations(false)
        } else {
            viewport
        };
    let options = eframe::NativeOptions {
        viewport,
        event_loop_builder: drop_capable_event_loop(),
        // Retain app zoom without introducing window-position persistence.
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "Versus",
        options,
        // Keep this process alive for the window's lifetime. Git may remove
        // temporary inputs as soon as its diff-tool child exits.
        Box::new(move |cc| Ok(Box::new(app::VersusApp::new(cc, launch)))),
    )
}

fn drop_capable_event_loop() -> Option<eframe::EventLoopBuilderHook> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_some_and(|display| !display.is_empty()) {
        // Winit's native Wayland backend does not emit file-drop events. Prefer
        // X11/XWayland when available, retaining Wayland on systems without X11.
        return Some(Box::new(|builder| {
            use winit::platform::x11::EventLoopBuilderExtX11 as _;
            builder.with_x11();
        }));
    }
    None
}

fn attach_parent_console() {
    #[cfg(target_os = "windows")]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn AttachConsole(process_id: u32) -> i32;
        }
        // SAFETY: ATTACH_PARENT_PROCESS is a documented constant. The function
        // takes no pointers; failure (no parent console/already attached) is fine.
        unsafe {
            let _ = AttachConsole(u32::MAX);
        }
    }
}
