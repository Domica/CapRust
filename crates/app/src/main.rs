// Hide console on Windows release builds.
// TEMP: keep console visible for debugging
// #![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use caprust_ui::CapRustApp;
use eframe::egui;
use tracing_subscriber::EnvFilter;

fn main() -> eframe::Result<()> {
    // Always log at INFO unless the user overrides via RUST_LOG.
    // Default: info. This ensures job/probe/thumbnail logs are visible
    // even when RUST_LOG is not set.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_writer(std::io::stderr)
        .init();

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
