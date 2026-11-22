// Hide the console window on Windows release builds. Debug builds keep
// it so `cargo run` still prints to the terminal.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod crash;

use caprust_ui::CapRustApp;
use eframe::egui;
use tracing_subscriber::EnvFilter;

#[cfg(any(feature = "gpu-cuda", feature = "gpu-directml"))]
use std::path::PathBuf;

#[cfg(any(feature = "gpu-cuda", feature = "gpu-directml"))]
use caprust_media_io::inference_backend::BackendType;

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

    // ---- GPU plugin mode ----
    //
    // When launched with `--gpu-plugin --model <path> --shape <json>`,
    // the binary enters a headless inference protocol mode instead of
    // opening the GUI. This is used by ExternalBackend in media-io to
    // run ONNX models on CUDA/DirectML GPUs without bundling ort in the
    // standard ~27 MB EXE.
    //
    // Protocol over stdin/stdout:
    //   LOAD <model_path> <shape_json>   -> OK   (model preloaded from --model arg)
    //   RUN <input_path> <output_path>   -> OK | ERR <msg>
    //   QUIT                             -> process exits
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--gpu-plugin") {
        return gpu_plugin_mode(&args);
    }

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
            .with_title(format!(
                "CapRust {} ({})",
                env!("CARGO_PKG_VERSION"),
                option_env!("CAPRUST_COMMIT").unwrap_or("unknown")
            ))
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

/// Headless GPU plugin mode.
///
/// Launched with `--gpu-plugin --model <path> --shape <json>`.
/// Loads the ONNX model via ort (CUDA/DirectML) and speaks a
/// line-based protocol on stdin/stdout for ExternalBackend in
/// caprust-media-io.
///
/// Protocol:
///   RUN <input_path> <output_path>   -> OK | ERR <msg>
///   QUIT                             -> process exits
///
/// (The model is preloaded from the --model argument; LOAD is answered
/// with OK immediately on startup.)
#[cfg(any(feature = "gpu-cuda", feature = "gpu-directml"))]
fn gpu_plugin_mode(args: &[String]) -> eframe::Result<()> {
    use caprust_media_io::inference_backend::InferenceSession;
    use std::io::{BufRead, Write};

    // ---- Parse command-line arguments ----
    let mut model_path: Option<PathBuf> = None;
    let mut shape: Option<[i64; 4]> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--model" => {
                i += 1;
                if i < args.len() {
                    model_path = Some(PathBuf::from(&args[i]));
                }
            }
            "--shape" => {
                i += 1;
                if i < args.len() {
                    shape = serde_json::from_str(&args[i]).ok();
                }
            }
            _ => {}
        }
        i += 1;
    }

    let model_path = match model_path {
        Some(p) => p,
        None => {
            eprintln!("GPU plugin: --model argument required");
            std::process::exit(1);
        }
    };

    let shape = match shape {
        Some(s) => s,
        None => {
            eprintln!("GPU plugin: --shape argument required");
            std::process::exit(1);
        }
    };

    // ---- Load model via ort (CUDA or DirectML) ----
    let backend = plugin_backend();
    let session = match InferenceSession::load(&model_path, shape, "GPU Plugin", backend) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "GPU plugin: failed to load model {}: {e}",
                model_path.display()
            );
            std::process::exit(1);
        }
    };

    tracing::info!(
        "GPU plugin: loaded {} on {:?} backend, shape {:?}",
        model_path.display(),
        backend,
        shape
    );

    // ---- Signal ready to parent process ----
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = writeln!(out, "OK");
    let _ = out.flush();

    // ---- Command loop ----
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        if line == "QUIT" {
            break;
        }

        if let Some(rest) = line.strip_prefix("RUN ") {
            let mut parts = rest.splitn(2, char::is_whitespace);
            let input_path = match parts.next() {
                Some(p) => p,
                None => {
                    let _ = writeln!(out, "ERR missing input path");
                    let _ = out.flush();
                    continue;
                }
            };
            let output_path = match parts.next() {
                Some(p) => p,
                None => {
                    let _ = writeln!(out, "ERR missing output path");
                    let _ = out.flush();
                    continue;
                }
            };

            // Read input binary (raw little-endian f32)
            let input_data = match std::fs::read(input_path) {
                Ok(d) => d,
                Err(e) => {
                    let _ = writeln!(out, "ERR cannot read input: {e}");
                    let _ = out.flush();
                    continue;
                }
            };

            if input_data.len() % 4 != 0 {
                let _ = writeln!(
                    out,
                    "ERR input size {} not a multiple of 4",
                    input_data.len()
                );
                let _ = out.flush();
                continue;
            }

            let input_f32: Vec<f32> = input_data
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();

            // Run inference
            match session.run(&input_f32) {
                Ok(outputs) => {
                    if outputs.is_empty() {
                        let _ = writeln!(out, "ERR model produced no outputs");
                    } else {
                        // Write first output tensor as raw little-endian f32
                        let output_bytes: Vec<u8> =
                            outputs[0].iter().flat_map(|&f| f.to_le_bytes()).collect();
                        match std::fs::write(output_path, &output_bytes) {
                            Ok(_) => {
                                let _ = writeln!(out, "OK");
                            }
                            Err(e) => {
                                let _ = writeln!(out, "ERR cannot write output: {e}");
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = writeln!(out, "ERR inference failed: {e}");
                }
            }
            let _ = out.flush();
        } else {
            let _ = writeln!(out, "ERR unknown command: {line}");
            let _ = out.flush();
        }
    }

    Ok(())
}

/// Stub for builds without GPU features — exits immediately.
#[cfg(not(any(feature = "gpu-cuda", feature = "gpu-directml")))]
fn gpu_plugin_mode(_args: &[String]) -> eframe::Result<()> {
    eprintln!("GPU plugin mode requires gpu-cuda or gpu-directml feature");
    std::process::exit(1);
}

/// Select the ort execution provider based on enabled features.
/// CUDA takes priority if both are enabled.
#[cfg(feature = "gpu-cuda")]
fn plugin_backend() -> BackendType {
    BackendType::Cuda
}

#[cfg(all(feature = "gpu-directml", not(feature = "gpu-cuda")))]
fn plugin_backend() -> BackendType {
    BackendType::DirectML
}
