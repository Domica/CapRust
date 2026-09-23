//! Frame decoder via ffmpeg subprocess.
//!
//! MVP approach: seek-based decode. For every request we spawn ffmpeg,
//! seek to the target source time, decode one frame as RGBA at the
//! requested target size, read from stdout.
//!
//! Later: replace with streaming decode + prefetch buffer for smooth 30 fps.

use anyhow::{Context, Result};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

/// Decode one frame as RGBA at the given source time.
/// Output size is exactly `target_w * target_h * 4` bytes.
pub fn decode_frame_rgba(
    ffmpeg: &Path,
    input: &Path,
    at_sec: f64,
    target_w: u32,
    target_h: u32,
) -> Result<Vec<u8>> {
    let w = target_w.max(2);
    let h = target_h.max(2);

    // Fast input seeking (-ss before -i) + skip audio/subtitle decoding
    // + fast scaler. On a 4K source, 640x270 RGBA goes from ~180ms to
    // ~40-60ms with these flags.
    let mut child = Command::new(ffmpeg)
        .args(["-v", "error"])
        .args(["-ss", &format!("{at_sec:.3}")])
        .arg("-i")
        .arg(input)
        .args(["-an", "-sn", "-dn"])
        .args(["-frames:v", "1"])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
        .args(["-sws_flags", "fast_bilinear"])
        .args(["-s", &format!("{w}x{h}")])
        .arg("-")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("spawn ffmpeg for {}", input.display()))?;

    let mut buf = Vec::with_capacity((w * h * 4) as usize);
    child
        .stdout
        .as_mut()
        .context("ffmpeg stdout missing")?
        .read_to_end(&mut buf)
        .context("read ffmpeg stdout")?;

    let status = child.wait().context("wait ffmpeg")?;
    if !status.success() {
        anyhow::bail!("ffmpeg exited {} for {}", status, input.display());
    }

    let expected = (w * h * 4) as usize;
    if buf.len() < expected {
        anyhow::bail!(
            "short frame from {}: got {} bytes, expected {}",
            input.display(),
            buf.len(),
            expected
        );
    }
    buf.truncate(expected);
    Ok(buf)
}

/// Compute the target decode size for a given project aspect.
/// Caps the long side at `max_side` for performance.
pub fn preview_size(project_w: u32, project_h: u32, max_side: u32) -> (u32, u32) {
    let pw = project_w.max(2) as f32;
    let ph = project_h.max(2) as f32;
    let scale = (max_side as f32 / pw).min(max_side as f32 / ph).min(1.0);
    let w = ((pw * scale) as u32).max(2) & !1; // even
    let h = ((ph * scale) as u32).max(2) & !1;
    (w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_size_downscales_16_9() {
        let (w, h) = preview_size(1920, 1080, 640);
        assert!(w <= 640);
        assert!(h <= 640);
        assert_eq!(w % 2, 0);
        assert_eq!(h % 2, 0);
    }

    #[test]
    fn preview_size_does_not_upscale() {
        let (w, h) = preview_size(320, 180, 640);
        assert_eq!((w, h), (320, 180));
    }
}
