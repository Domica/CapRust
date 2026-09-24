//! Run ffmpeg against a RenderPlan and report progress.

use crate::export_graph::RenderPlan;
use anyhow::{Context, Result};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Sender};

#[derive(Debug, Clone)]
pub enum ExportEvent {
    Started,
    /// Progress 0.0..=1.0
    Progress(f32),
    Log(String),
    Finished {
        output: std::path::PathBuf,
    },
    Failed(String),
}

/// Spawn ffmpeg and stream progress events back through the given channel.
pub fn run_export(
    ffmpeg: &Path,
    plan: &RenderPlan,
    output: &Path,
    events: Sender<ExportEvent>,
) -> Result<()> {
    let args = plan.build_command(ffmpeg, output);

    let mut child = Command::new(ffmpeg)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn ffmpeg at {}", ffmpeg.display()))?;

    let stderr = child.stderr.take().context("ffmpeg stderr missing")?;
    let _ = events.send(ExportEvent::Started);

    // Parse `-progress pipe:2` output: lines like `out_time_ms=1234567`.
    let reader = BufReader::new(stderr);
    let total_us = (plan.total_duration_sec * 1_000_000.0) as f64;

    for line in reader.lines().map_while(Result::ok) {
        let line = line.trim().to_string();
        if line.starts_with("out_time_us=") || line.starts_with("out_time_ms=") {
            let val = line.split_once('=').map(|(_, v)| v).unwrap_or("").trim();
            if let Ok(us) = val.parse::<f64>() {
                let progress = if total_us > 0.0 {
                    (us / total_us).clamp(0.0, 1.0) as f32
                } else {
                    0.0
                };
                let _ = events.send(ExportEvent::Progress(progress));
            }
        } else if line.starts_with("error") || line.contains("Error") {
            let _ = events.send(ExportEvent::Log(line));
        }
    }

    let status = child.wait().context("wait ffmpeg")?;
    if status.success() {
        let _ = events.send(ExportEvent::Finished {
            output: output.to_path_buf(),
        });
        Ok(())
    } else {
        let _ = events.send(ExportEvent::Failed(format!("ffmpeg exited {status}")));
        anyhow::bail!("ffmpeg failed: {status}")
    }
}

/// Spawn the export on a background thread.
pub fn spawn_export(
    ffmpeg: std::path::PathBuf,
    plan: RenderPlan,
    output: std::path::PathBuf,
) -> std::sync::mpsc::Receiver<ExportEvent> {
    let (tx, rx) = channel::<ExportEvent>();
    std::thread::spawn(move || {
        if let Err(e) = run_export(&ffmpeg, &plan, &output, tx.clone()) {
            let _ = tx.send(ExportEvent::Failed(e.to_string()));
        }
    });
    rx
}

/// Open Explorer/Finder with the given file selected.
pub fn reveal_in_folder(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg("-R").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Fallback: open the containing folder with xdg-open.
        if let Some(parent) = path.parent() {
            let _ = Command::new("xdg-open").arg(parent).spawn();
        }
    }
}
