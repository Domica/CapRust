//! Waveform peak extraction for the timeline.
//!
//! Renders a compact `Vec<f32>` of absolute peak values per bucket,
//! suitable for drawing a CapCut-style waveform strip behind audio
//! clips. FFmpeg decodes the source to mono s16le PCM on stdout; the
//! rust side buckets samples and keeps the peak (max |sample|).
//!
//! We deliberately do not parse file formats ourselves: ffmpeg
//! already handles every codec we care about and PCM-on-stdout keeps
//! the pipeline simple and cache-friendly.

use anyhow::{Context, Result};
use std::io::Read;
use std::path::Path;
use std::process::Stdio;

/// Target output sample rate for the decoder. 8 kHz mono is plenty
/// for a visual waveform: peaks look identical at 8 kHz and 48 kHz,
/// and the decode is ~6x faster.
const TARGET_RATE: u32 = 8000;

/// Decode to mono s16le PCM samples at `rate` Hz via ffmpeg.
/// Shared by the waveform peak extractor and beat analysis so both
/// see identical samples.
pub fn decode_mono_s16(ffmpeg: &Path, input: &Path, rate: u32) -> Result<Vec<i16>> {
    let mut child = crate::silent_cmd::silent_command(ffmpeg)
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            "file",
            "-i",
            // The path argument is passed as a &str via .arg below so
            // we do not need shell quoting.
        ])
        .arg(input)
        .args([
            "-vn",
            "-ac",
            "1",
            "-ar",
            &rate.to_string(),
            "-f",
            "s16le",
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn ffmpeg for {}", input.display()))?;

    let mut pcm = Vec::new();
    child
        .stdout
        .as_mut()
        .context("no stdout from ffmpeg")?
        .read_to_end(&mut pcm)
        .context("read ffmpeg pcm")?;

    let status = child.wait().context("wait ffmpeg")?;
    if !status.success() {
        let mut err = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut err);
        }
        anyhow::bail!(
            "ffmpeg exited {} for {}: {}",
            status,
            input.display(),
            err.trim()
        );
    }

    // Interpret pcm as little-endian i16.
    Ok(pcm
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect())
}

/// Extract peaks by running ffmpeg on `input` and bucketing the
/// decoded samples into `buckets` slots. Returns `buckets` values in
/// `0.0..=1.0`. Empty files return a zero-filled vector.
pub fn extract_peaks(ffmpeg: &Path, input: &Path, buckets: usize) -> Result<Vec<f32>> {
    let buckets = buckets.max(1);
    let samples = decode_mono_s16(ffmpeg, input, TARGET_RATE)?;
    Ok(peaks_from_samples(&samples, buckets))
}

/// Pure bucketing: split `samples` into `buckets` groups and return
/// each group's peak (max absolute value) normalized to 0.0..=1.0.
/// Kept separate so tests can drive it without spawning ffmpeg.
pub fn peaks_from_samples(samples: &[i16], buckets: usize) -> Vec<f32> {
    let buckets = buckets.max(1);
    if samples.is_empty() {
        return vec![0.0; buckets];
    }
    let len = samples.len();
    let mut out = Vec::with_capacity(buckets);
    for b in 0..buckets {
        let start = b * len / buckets;
        let end = (b + 1) * len / buckets;
        let mut peak = 0i32;
        for &s in &samples[start..end] {
            let v = (s as i32).abs();
            if v > peak {
                peak = v;
            }
        }
        // i16::MIN.abs() is 32768 which doesn't fit i16, so we compare
        // against i16::MAX as the ceiling: normalize by 32768.0 so
        // full-scale stays at 1.0.
        out.push((peak as f32 / 32768.0).min(1.0));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_gives_all_zeros() {
        let p = peaks_from_samples(&vec![0i16; 8000], 100);
        assert_eq!(p.len(), 100);
        assert!(p.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn full_scale_gives_one() {
        let p = peaks_from_samples(&vec![i16::MAX; 8000], 10);
        assert_eq!(p.len(), 10);
        assert!(p.iter().all(|&v| (v - 1.0).abs() < 0.001));
    }

    #[test]
    fn bucket_picks_max_within_group() {
        // 1000 samples, 10 buckets -> 100 samples per bucket. Bucket 5
        // contains a lone i16::MAX sample, the rest are zero.
        let mut s = vec![0i16; 1000];
        s[550] = i16::MAX;
        let p = peaks_from_samples(&s, 10);
        assert_eq!(p.len(), 10);
        assert!((p[5] - 1.0).abs() < 0.001, "bucket 5 should be 1.0");
        for (i, &v) in p.iter().enumerate() {
            if i != 5 {
                assert_eq!(v, 0.0, "bucket {i} should be 0.0");
            }
        }
    }

    #[test]
    fn short_input_still_returns_requested_buckets() {
        // 5 samples but 20 buckets: later buckets repeat the sample
        // grid (start==end) and must not panic.
        let p = peaks_from_samples(&[100i16, 200, 300, 400, 500], 20);
        assert_eq!(p.len(), 20);
    }

    #[test]
    fn empty_input_returns_zero_buckets() {
        let p = peaks_from_samples(&[], 50);
        assert_eq!(p.len(), 50);
        assert!(p.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn negative_peaks_use_absolute_value() {
        let p = peaks_from_samples(&[i16::MIN, 0, 0, 0], 1);
        assert!((p[0] - 1.0).abs() < 0.001, "i16::MIN abs should be ~1.0");
    }
}
