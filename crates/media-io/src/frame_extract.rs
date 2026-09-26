//! Extract downscaled RGB frames from a video file for analysis
//! (auto-reframe, future beat detection, ...).
//!
//! Runs ffmpeg as a subprocess with `rawvideo` / `rgb24` output to
//! stdout. No libav* linkage (DIRECTIVES §10). Frames are read
//! sequentially; the caller decides what to do with them.

use anyhow::{anyhow, Context, Result};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

/// One decoded frame: timestamp relative to the extraction window
/// start, dimensions, and a tightly packed RGB buffer of length
/// `width * height * 3`.
#[derive(Debug, Clone)]
pub struct RgbFrame {
    pub t_ms: u64,
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

impl RgbFrame {
    pub fn byte_len(&self) -> usize {
        (self.width as usize) * (self.height as usize) * 3
    }
}

/// Compute the output dimensions when downscaling `src_w x src_h` so
/// the longer side is at most `max_side`, preserving aspect ratio and
/// rounding to even dimensions (H.264-friendly and matches what the
/// downstream consumers expect).
pub fn scaled_dims(src_w: u32, src_h: u32, max_side: u32) -> (u32, u32) {
    if src_w == 0 || src_h == 0 || max_side == 0 {
        return (0, 0);
    }
    let (w, h) = if src_w >= src_h {
        let scale = (max_side as f64) / (src_w as f64);
        if scale >= 1.0 {
            (src_w, src_h)
        } else {
            (max_side, ((src_h as f64) * scale).round().max(2.0) as u32)
        }
    } else {
        let scale = (max_side as f64) / (src_h as f64);
        if scale >= 1.0 {
            (src_w, src_h)
        } else {
            (((src_w as f64) * scale).round().max(2.0) as u32, max_side)
        }
    };
    // Round down to even.
    ((w & !1).max(2), (h & !1).max(2))
}

/// Extract RGB frames from `input` between `[t_start_sec, t_start_sec +
/// duration_sec)` at `fps`, scaled so the longer side is at most
/// `max_side`.
///
/// Returns frames in playback order with `t_ms` relative to
/// `t_start_sec` (first frame is ~0, last is ~duration_sec * 1000).
///
/// Blocking. Call from a background thread.
#[allow(clippy::too_many_arguments)]
pub fn extract_rgb_frames(
    ffmpeg: &Path,
    input: &Path,
    src_w: u32,
    src_h: u32,
    t_start_sec: f64,
    duration_sec: f64,
    fps: f64,
    max_side: u32,
) -> Result<Vec<RgbFrame>> {
    if duration_sec <= 0.0 || fps <= 0.0 {
        return Ok(Vec::new());
    }
    let (out_w, out_h) = scaled_dims(src_w, src_h, max_side);
    if out_w < 2 || out_h < 2 {
        return Err(anyhow!("invalid source dimensions {src_w}x{src_h}"));
    }

    let mut args: Vec<String> = vec!["-hide_banner".into(), "-loglevel".into(), "error".into()];
    // -ss BEFORE -i is fast but may drop the first keyframe's frames;
    // for analysis we care about approximate positions, not exact
    // frame-accurate seek. Keeps extraction fast on long clips.
    if t_start_sec > 0.0 {
        args.push("-ss".into());
        args.push(format!("{t_start_sec:.6}"));
    }
    args.push("-i".into());
    args.push(input.to_string_lossy().into_owned());
    args.push("-t".into());
    args.push(format!("{duration_sec:.6}"));
    args.push("-vf".into());
    args.push(format!("fps={fps:.6},scale={out_w}:{out_h}:flags=bilinear"));
    args.push("-f".into());
    args.push("rawvideo".into());
    args.push("-pix_fmt".into());
    args.push("rgb24".into());
    args.push("-".into());

    tracing::debug!(
        "frame_extract: ffmpeg args: {}",
        args.iter()
            .map(|a| if a.contains(' ') {
                format!("{a:?}")
            } else {
                a.clone()
            })
            .collect::<Vec<_>>()
            .join(" ")
    );

    let mut child = Command::new(ffmpeg)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn ffmpeg frame extraction on {}", input.display()))?;

    // Drain stderr on a helper thread so a chatty ffmpeg cannot
    // deadlock us on a full pipe.
    if let Some(mut err) = child.stderr.take() {
        std::thread::spawn(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(&mut err);
            for line in reader.lines().map_while(std::result::Result::ok) {
                if !line.trim().is_empty() {
                    tracing::warn!("frame_extract ffmpeg: {line}");
                }
            }
        });
    }

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("ffmpeg stdout missing"))?;

    let frame_size = (out_w as usize) * (out_h as usize) * 3;
    let frame_interval_ms = (1000.0 / fps).round() as u64;
    let mut frames: Vec<RgbFrame> = Vec::new();
    let mut buf = vec![0u8; frame_size];
    let mut idx: u64 = 0;

    loop {
        match stdout.read_exact(&mut buf) {
            Ok(()) => {
                frames.push(RgbFrame {
                    t_ms: idx.saturating_mul(frame_interval_ms),
                    width: out_w,
                    height: out_h,
                    rgb: buf.clone(),
                });
                idx += 1;
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                let _ = child.kill();
                return Err(anyhow!("read frame {idx} from ffmpeg: {e}"));
            }
        }
        // Upper bound: 10 minutes at 60 fps = 36000 frames. Anything
        // beyond that is a bug or a hostile input; fail loudly rather
        // than OOM the machine.
        if frames.len() > 36_000 {
            let _ = child.kill();
            return Err(anyhow!("frame extraction exceeded 36000 frames; aborting"));
        }
    }

    let status = child.wait().context("wait ffmpeg")?;
    if !status.success() {
        return Err(anyhow!(
            "ffmpeg frame extraction exited {} on {}",
            status,
            input.display()
        ));
    }

    tracing::info!(
        "frame_extract: {} frames ({}x{}) from {} [{:.2}s..{:.2}s @ {:.2}fps]",
        frames.len(),
        out_w,
        out_h,
        input.display(),
        t_start_sec,
        t_start_sec + duration_sec,
        fps,
    );

    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn scaled_dims_landscape_downscale() {
        // 1920x1080 to max 640 -> 640x360.
        assert_eq!(scaled_dims(1920, 1080, 640), (640, 360));
    }

    #[test]
    fn scaled_dims_portrait_downscale() {
        // 1080x1920 to max 640 -> 360x640.
        assert_eq!(scaled_dims(1080, 1920, 640), (360, 640));
    }

    #[test]
    fn scaled_dims_no_upscale() {
        // 320x240 to max 640 stays 320x240.
        assert_eq!(scaled_dims(320, 240, 640), (320, 240));
    }

    #[test]
    fn scaled_dims_rounds_to_even() {
        // 1000x999 to max 100 -> 100x100-ish, even on both axes.
        let (w, h) = scaled_dims(1000, 999, 100);
        assert_eq!(w % 2, 0);
        assert_eq!(h % 2, 0);
        assert_eq!(w, 100);
    }

    #[test]
    fn scaled_dims_zero_input_safe() {
        assert_eq!(scaled_dims(0, 100, 320), (0, 0));
        assert_eq!(scaled_dims(100, 0, 320), (0, 0));
        assert_eq!(scaled_dims(100, 100, 0), (0, 0));
    }

    /// End-to-end test using the lavfi `testsrc` generator as a
    /// pseudo-file. ffmpeg exposes lavfi inputs via `-f lavfi -i
    /// <graph>`; our wrapper always emits `-i <path>`, so we cannot
    /// drive lavfi through it directly. Instead, generate a small
    /// real media file in a temp dir first, then extract from it.
    /// Skipped silently when ffmpeg is not on PATH.
    #[test]
    fn extract_rgb_frames_from_generated_clip() {
        let Some(ffmpeg) = which_ffmpeg() else {
            eprintln!("ffmpeg not on PATH; skipping");
            return;
        };

        let tmp =
            std::env::temp_dir().join(format!("caprust-frame-extract-{}.mp4", std::process::id()));
        // 320x240, 10 fps, 0.5 s, h264. ~15 KB. Safe to leave behind
        // if cleanup fails; the next test run overwrites it.
        let status = std::process::Command::new(&ffmpeg)
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x240:rate=10:duration=0.5",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&tmp)
            .status();
        match status {
            Ok(s) if s.success() => {}
            _ => {
                eprintln!("failed to generate test clip; skipping");
                return;
            }
        }

        let frames =
            extract_rgb_frames(&ffmpeg, &tmp, 320, 240, 0.0, 0.5, 10.0, 320).expect("extract");
        assert!(
            frames.len() >= 4 && frames.len() <= 6,
            "expected ~5 frames, got {}",
            frames.len()
        );
        for f in &frames {
            assert_eq!(f.width, 320);
            assert_eq!(f.height, 240);
            assert_eq!(f.rgb.len(), 320 * 240 * 3);
        }
        // Timestamps are sequential at 100 ms intervals.
        for (i, f) in frames.iter().enumerate() {
            assert_eq!(f.t_ms, (i as u64) * 100, "frame {i} timestamp");
        }

        let _ = std::fs::remove_file(&tmp);
    }

    fn which_ffmpeg() -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("ffmpeg");
            if candidate.is_file() {
                return Some(candidate);
            }
            let candidate = dir.join("ffmpeg.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }
}
