//! Continuous recording: DXGI frames piped into ffmpeg stdin.
//!
//! PR 3 scope: fixed-duration recording to H.264 mp4. UI wiring
//! (Start/Stop buttons, monitor picker) is PR 4.
//!
//! Frame timing model:
//!   * The desktop only produces a new frame when something
//!     changed. On a static screen DXGI times out.
//!   * We want a constant-fps output. So the loop runs at
//!     `1/fps` ticks and, on a timeout, re-feeds the last
//!     frame we saw. That keeps the ffmpeg input stream honest
//!     and avoids "bursty" video on a mostly-static desktop.

use anyhow::{anyhow, Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::capture::{DuplicationSession, Frame};

#[derive(Debug, Clone)]
pub struct RecordStats {
    pub frames_written: usize,
    pub elapsed: Duration,
    pub output_path: PathBuf,
}

/// Shared counters the caller (UI) can poll without blocking.
#[derive(Default)]
pub struct RecordHandle {
    pub frames_written: Arc<AtomicUsize>,
    pub stop: Arc<AtomicBool>,
}

/// Record the given monitor for `duration` at target `fps` and
/// encode to H.264 mp4 with `ffmpeg`.
///
/// `ffmpeg_path` must point at an ffmpeg binary; the caller is
/// responsible for the resolution (see DIRECTIVES 10.7).
pub fn record_to_file(
    ffmpeg_path: &Path,
    monitor_index: usize,
    output_path: &Path,
    duration: Duration,
    fps: u32,
    handle: RecordHandle,
) -> Result<RecordStats> {
    if fps == 0 {
        anyhow::bail!("fps must be > 0");
    }
    let session = DuplicationSession::open(monitor_index)?;
    let w = session.width();
    let h = session.height();

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }

    // Raw BGRA on stdin, encoded to yuv420p H.264.
    let mut child = Command::new(ffmpeg_path)
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "warning",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "bgra",
            "-s",
            &format!("{w}x{h}"),
            "-r",
            &fps.to_string(),
            "-i",
            "-",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "23",
            "-pix_fmt",
            "yuv420p",
            output_path.to_string_lossy().as_ref(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow!("spawn ffmpeg: {e}"))?;

    // Reader thread for stderr so ffmpeg never blocks on a full
    // pipe buffer. Every line goes to tracing so a failed run is
    // diagnosable from the console.
    if let Some(mut err) = child.stderr.take() {
        std::thread::spawn(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(&mut err);
            for line in reader.lines().map_while(std::result::Result::ok) {
                if !line.trim().is_empty() {
                    tracing::warn!("ffmpeg: {line}");
                }
            }
        });
    }

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("ffmpeg stdin missing"))?;

    let frame_interval = Duration::from_secs_f64(1.0 / fps as f64);
    let started = Instant::now();
    let deadline = started + duration;
    let mut next_tick = started;
    let mut last_frame: Option<Frame> = None;
    let mut written: usize = 0;

    while Instant::now() < deadline {
        if handle.stop.load(Ordering::Relaxed) {
            break;
        }
        // Wait until the next tick. This is what keeps the output
        // constant-fps even when DXGI updates irregularly.
        let now = Instant::now();
        if now < next_tick {
            std::thread::sleep(next_tick - now);
        }
        next_tick += frame_interval;

        // Ask for a frame with a timeout at most the tick size.
        let timeout_ms = (frame_interval.as_millis() as u32).max(1);
        match session.next_frame(timeout_ms) {
            Ok(Some(f)) => last_frame = Some(f),
            Ok(None) => {
                // Static desktop: repeat the previous frame. On the
                // very first tick there is nothing to repeat, so
                // skip until DXGI hands us at least one.
                if last_frame.is_none() {
                    continue;
                }
            }
            Err(e) => {
                let _ = child.kill();
                return Err(e);
            }
        }

        let Some(frame) = &last_frame else { continue };
        stdin
            .write_all(&frame.pixels)
            .map_err(|e| anyhow!("write frame to ffmpeg: {e}"))?;
        written += 1;
        handle.frames_written.store(written, Ordering::Relaxed);
    }

    // Close stdin so ffmpeg sees EOF and finalizes the container.
    drop(stdin);
    let status = child.wait().context("wait ffmpeg")?;
    if !status.success() {
        anyhow::bail!("ffmpeg exited {status}");
    }

    Ok(RecordStats {
        frames_written: written,
        elapsed: started.elapsed(),
        output_path: output_path.to_path_buf(),
    })
}
