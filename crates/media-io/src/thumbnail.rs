//! Thumbnail extraction via ffmpeg subprocess.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// Extract a single frame at `at_sec` from `input` and save as JPEG to `out`.
/// Returns Ok even if the frame is black; caller decides whether to keep it.
pub fn extract_jpeg(
    ffmpeg: &Path,
    input: &Path,
    out: &Path,
    at_sec: f64,
    width: u32,
) -> Result<()> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }

    let status = Command::new(ffmpeg)
        .args(["-y", "-v", "error"])
        .args(["-ss", &format!("{at_sec}")])
        .arg("-i")
        .arg(input)
        .args(["-frames:v", "1", "-vf", &format!("scale={width}:-1")])
        .arg(out)
        .status()
        .with_context(|| format!("spawn ffmpeg thumbnail for {}", input.display()))?;

    if !status.success() {
        anyhow::bail!("ffmpeg thumbnail exited {} for {}", status, input.display());
    }
    Ok(())
}

/// Extract a JPEG at the "best-looking" moment:
/// 10% into the clip, but never earlier than 0.5s and never later than 5s.
pub fn best_frame_time(duration_ms: u64) -> f64 {
    if duration_ms == 0 {
        return 0.5;
    }
    let tenth = (duration_ms as f64) / 1000.0 * 0.10;
    tenth.clamp(0.5, 5.0)
}
