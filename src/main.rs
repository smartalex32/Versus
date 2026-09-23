mod app;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Versus")
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([900.0, 650.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Versus",
        options,
        Box::new(|cc| Ok(Box::new(app::VersusApp::new(cc)))),
    )
}
