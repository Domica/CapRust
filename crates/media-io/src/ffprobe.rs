//! ffprobe wrapper — extract media metadata via subprocess.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct MediaProbe {
    pub duration_ms: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub has_video: bool,
    pub has_audio: bool,
}

#[derive(Debug, Deserialize)]
struct FfprobeOutput {
    streams: Vec<FfprobeStream>,
    format: Option<FfprobeFormat>,
}

#[derive(Debug, Deserialize)]
struct FfprobeStream {
    codec_type: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FfprobeFormat {
    duration: Option<String>,
}

/// Probe a media file. Returns Err if ffprobe missing or file unreadable.
pub fn probe(ffprobe: &Path, input: &Path) -> Result<MediaProbe> {
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(input)
        .output()
        .with_context(|| format!("spawn ffprobe on {}", input.display()))?;

    if !output.status.success() {
        anyhow::bail!("ffprobe exited {} on {}", output.status, input.display());
    }

    let parsed: FfprobeOutput =
        serde_json::from_slice(&output.stdout).context("parse ffprobe JSON")?;

    let has_video = parsed
        .streams
        .iter()
        .any(|s| s.codec_type.as_deref() == Some("video"));
    let has_audio = parsed
        .streams
        .iter()
        .any(|s| s.codec_type.as_deref() == Some("audio"));

    let vstream = parsed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"));

    let (width, height, fps) = if let Some(v) = vstream {
        let fps = v.avg_frame_rate.as_deref().and_then(parse_fraction);
        (v.width, v.height, fps)
    } else {
        (None, None, None)
    };

    // Prefer stream duration, fallback to format duration.
    let dur_s: Option<f64> = vstream
        .and_then(|v| v.duration.as_deref())
        .and_then(|s| s.parse().ok())
        .or_else(|| {
            parsed
                .format
                .as_ref()
                .and_then(|f| f.duration.as_deref())
                .and_then(|s| s.parse().ok())
        });

    let duration_ms = dur_s.map(|s| (s * 1000.0) as u64).unwrap_or(0);

    Ok(MediaProbe {
        duration_ms,
        width,
        height,
        fps,
        has_video,
        has_audio,
    })
}

/// Parse "30000/1001" or "30" into f64.
fn parse_fraction(s: &str) -> Option<f64> {
    if let Some((num, den)) = s.split_once('/') {
        let n: f64 = num.parse().ok()?;
        let d: f64 = den.parse().ok()?;
        if d == 0.0 {
            return None;
        }
        Some(n / d)
    } else {
        s.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fraction_handles_ratio_and_int() {
        assert_eq!(parse_fraction("30000/1001"), Some(29.97002997002997));
        assert_eq!(parse_fraction("30"), Some(30.0));
        assert_eq!(parse_fraction("0/0"), None);
    }
}
