#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use caprust_ui::CapRustApp;
use eframe::egui;

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("CapRust"),
        ..Default::default()
    };

    let result = eframe::run_native(
        "CapRust",
        options,
        Box::new(|cc| Ok(Box::new(CapRustApp::new(cc)))),
    );

    tracing::info!("Event loop exited (ok={})", result.is_ok());
    std::process::exit(0);
}
