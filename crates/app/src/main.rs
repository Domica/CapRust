// Hide the console window on Windows release builds. Debug builds keep
// it so `cargo run` still prints to the terminal.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod crash;

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
    // Install the crash handler first so a panic anywhere below
    // (tracing init, icon decode, eframe event loop) leaves a
    // report in %APPDATA%/CapRust/crashes/.
    crash::install();

    // ---- Logging ----
    //
    // Release builds ship without a console window. To keep the log
    // reachable after the fact, every run writes INFO+ to
    // %APPDATA%/CapRust/logs/caprust.log.YYYY-MM-DD. Files older than
    // LOG_RETENTION_DAYS are removed at startup.
    //
    // Debug builds (cargo run) AND release builds with CAPRUST_DEBUG=1
    // also write to stderr, so developers see live output in a terminal.
    let logs_dir = logs_dir();
    let _ = std::fs::create_dir_all(&logs_dir);
    prune_old_logs(&logs_dir, LOG_RETENTION_DAYS);

    // Best-effort removal of orphaned preview temp files (crash
    // leftovers). Pattern-guarded to our own names only; the audio
    // cache (-cache-) lives in the project dir, never here.
    {
        let dir = std::env::temp_dir();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let ours = name.starts_with("caprust-audio-")
                    && name.ends_with(".pcm")
                    && !name.contains("-cache-")
                    || name.starts_with("caprust-last-preview-");
                if ours {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }

    let file_appender = tracing_appender::rolling::daily(&logs_dir, "caprust.log");
    let (non_blocking, log_guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    use tracing_subscriber::prelude::*;
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true);

    let want_stderr = cfg!(debug_assertions)
        || std::env::var("CAPRUST_DEBUG")
            .map(|v| v == "1")
            .unwrap_or(false);
    let stderr_layer = want_stderr.then(|| {
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(true)
    });

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(stderr_layer)
        .init();

    // Keep the non-blocking writer guard alive until the process exits.
    // Dropping it earlier would silently stop file logging.
    let _log_guard = log_guard;

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

/// Days to keep rotated log files. Files older than this are
/// deleted at startup so a long-lived install does not accumulate
/// hundreds of tiny files.
const LOG_RETENTION_DAYS: u64 = 7;

/// Where the log files live: %APPDATA%/CapRust/logs on Windows,
/// ~/.caprust/logs on Unix-like. Mirrors update_checker::cache_path.
fn logs_dir() -> std::path::PathBuf {
    let base = std::env::var("APPDATA")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    std::path::Path::new(&base).join("CapRust").join("logs")
}

/// Delete log files older than `days`. Best-effort: any error is
/// ignored, a missing dir is a no-op.
fn prune_old_logs(dir: &std::path::Path, days: u64) {
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(days * 24 * 60 * 60);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if !name.starts_with("caprust.log") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if modified < cutoff {
            let _ = std::fs::remove_file(&path);
        }
    }
}
