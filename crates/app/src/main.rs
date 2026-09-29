// Hide console on Windows release builds.
// TEMP: keep console visible for debugging
// #![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use caprust_ui::CapRustApp;
use eframe::egui;
use tracing_subscriber::EnvFilter;

/// Decode the embedded PNG into an egui icon. Falls back to a
/// transparent 1x1 icon if decoding ever fails, so a corrupt asset
/// cannot stop the editor from starting.
fn load_icon() -> egui::IconData {
    const ICON_PNG: &[u8] = include_bytes!("../assets/icon-256.png");
    match image::load_from_memory(ICON_PNG) {
        Ok(img) => {
            let img = img.to_rgba8();
            let (w, h) = img.dimensions();
            egui::IconData {
                rgba: img.into_raw(),
                width: w,
                height: h,
            }
        }
        Err(e) => {
            eprintln!("main: failed to decode embedded icon: {e}");
            egui::IconData {
                rgba: vec![0, 0, 0, 0],
                width: 1,
                height: 1,
            }
        }
    }
}

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

    // Runtime icon: window title bar, taskbar, Alt+Tab switcher.
    // Embedded at compile time so a fresh binary is self-contained.
    let icon = load_icon();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("CapRust")
            .with_icon(icon),
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
