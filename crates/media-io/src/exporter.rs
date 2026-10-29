//! Run ffmpeg against a RenderPlan and report progress.

use crate::export_graph::RenderPlan;
use anyhow::{Context, Result};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Stdio;
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
    // Long commands blow past Windows' 32767-char CreateProcess limit
    // (OS error 206) on projects with dozens of inputs: move the
    // filtergraph into a script file. The guard deletes it on drop,
    // including when the spawn below fails.
    let mut args = plan.build_command(ffmpeg, output);
    let _script_guard = crate::filter_script::swap_filter_complex(&mut args)?;

    let mut child = crate::silent_cmd::silent_command(ffmpeg)
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
    let total_us = plan.total_duration_sec * 1_000_000.0;
    // Last non-progress stderr lines. ffmpeg prints the real reason for a
    // failure there ("Invalid argument", "No such filter", ...), and the
    // old message only said "ffmpeg exited exit code: 1".
    let mut tail: std::collections::VecDeque<String> = std::collections::VecDeque::new();

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
            let _ = events.send(ExportEvent::Log(line.clone()));
            push_tail(&mut tail, line);
        } else if !line.is_empty() && !is_progress_line(&line) {
            push_tail(&mut tail, line);
        }
    }

    let status = child.wait().context("wait ffmpeg")?;
    if status.success() {
        let _ = events.send(ExportEvent::Finished {
            output: output.to_path_buf(),
        });
        Ok(())
    } else {
        let detail = tail.iter().cloned().collect::<Vec<_>>().join(" | ");
        let msg = if detail.is_empty() {
            format!("ffmpeg exited {status}")
        } else {
            format!("ffmpeg exited {status}: {detail}")
        };
        for l in &tail {
            let _ = events.send(ExportEvent::Log(l.clone()));
        }
        // spawn_export reports this error as ExportEvent::Failed, so it
        // is not sent here too (that showed the toast twice).
        anyhow::bail!("{msg}")
    }
}

/// `-progress` key=value lines that carry no diagnostic value.
fn is_progress_line(line: &str) -> bool {
    const KEYS: [&str; 12] = [
        "frame=",
        "fps=",
        "stream_",
        "bitrate=",
        "total_size=",
        "out_time",
        "dup_frames=",
        "drop_frames=",
        "speed=",
        "progress=",
        "size=",
        "q=",
    ];
    KEYS.iter().any(|k| line.starts_with(k))
}

/// Keep only the last few lines so a long run cannot grow this.
fn push_tail(tail: &mut std::collections::VecDeque<String>, line: String) {
    const MAX_TAIL: usize = 6;
    if tail.len() == MAX_TAIL {
        tail.pop_front();
    }
    tail.push_back(line);
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
            // `{e:#}` keeps the whole cause chain: `{e}` alone showed
            // only "spawn ffmpeg at ..." and swallowed the OS error,
            // which is how the 32K command-line limit stayed hidden.
            let _ = tx.send(ExportEvent::Failed(format!("{e:#}")));
        }
    });
    rx
}

/// Open the folder in the OS file browser.
pub fn open_folder(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = crate::silent_cmd::silent_command("explorer")
            .arg(path)
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = crate::silent_cmd::silent_command("open").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = crate::silent_cmd::silent_command("xdg-open")
            .arg(path)
            .spawn();
    }
}

/// Open Explorer/Finder with the given file selected.
pub fn reveal_in_folder(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        // Explorer falls back to Documents when the /select target is
        // missing or relative, so resolve to something real first.
        let target = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        if target.is_file() {
            // NOTE: Explorer parses its own command line, so Rust's argv
            // quoting does not apply. The only form that survives paths
            // with spaces is /select,"<path>" (quotes around the path
            // only), passed literally via raw_arg. Both the glued whole-
            // quoted form and separate args fall back to Documents.
            {
                use std::os::windows::process::CommandExt;
                let mut cmd = crate::silent_cmd::silent_command("explorer");
                cmd.raw_arg(format!("/select,\"{}\"", target.display()));
                let _ = cmd.spawn();
            }
        } else if let Some(parent) = target.parent().filter(|p| p.is_dir()) {
            tracing::warn!("reveal: {} missing, opening parent", target.display());
            let _ = crate::silent_cmd::silent_command("explorer")
                .arg(parent)
                .spawn();
        } else {
            tracing::warn!("reveal: nothing to open for {}", target.display());
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = crate::silent_cmd::silent_command("open")
            .arg("-R")
            .arg(path)
            .spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Fallback: open the containing folder with xdg-open.
        if let Some(parent) = path.parent() {
            let _ = crate::silent_cmd::silent_command("xdg-open")
                .arg(parent)
                .spawn();
        }
    }
}
