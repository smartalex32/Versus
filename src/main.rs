#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod cli;
mod logo;
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
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Versus — Compare")
            .with_icon(logo::themed_icon(true))
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([900.0, 650.0]),
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
