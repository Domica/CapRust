//! Multi-camera audio sync.
//!
//! Decodes each angle's audio to mono PCM, computes a coarse
//! RMS envelope, and finds the lag that maximizes the envelope
//! cross-correlation. Result is a per-angle offset in ms to
//! align against angle[0].
//!
//! Deliberately no FFT dependency: typical multicam clips are
//! under a few minutes, and a naive O(N*M) correlation on a
//! 100 Hz envelope is a few million multiply-adds. Correct is
//! enough; fast is a follow-up if a real project needs it.

use anyhow::{Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc::{channel, Receiver};

/// Envelope sample rate: 100 Hz gives 10 ms resolution, which
/// is finer than any human-perceptible sync error.
const ENVELOPE_HZ: u32 = 100;

/// Audio decoder sample rate. Higher than the envelope rate so
/// the RMS window has real data to average; kept well under the
/// typical 44.1/48 kHz source to keep the decode fast.
const DECODE_RATE: u32 = 8000;

/// Sample window for RMS computation: DECODE_RATE / ENVELOPE_HZ.
const WINDOW_SAMPLES: usize = (DECODE_RATE / ENVELOPE_HZ) as usize;

#[derive(Debug, Clone)]
pub enum SyncEvent {
    Progress {
        current: usize,
        total: usize,
    },
    Done {
        /// One offset per angle, in ms. `offsets[0] == 0`.
        offsets_ms: Vec<i64>,
    },
    Failed(String),
}

/// Compute the RMS envelope of a mono s16 buffer at DECODE_RATE.
/// Length = ceil(samples / WINDOW_SAMPLES). Values are normalized
/// to 0.0..=1.0 against i16 full scale.
pub fn rms_envelope(samples: &[i16]) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(samples.len() / WINDOW_SAMPLES + 1);
    let mut i = 0;
    while i < samples.len() {
        let end = (i + WINDOW_SAMPLES).min(samples.len());
        let mut sum_sq: f64 = 0.0;
        for &s in &samples[i..end] {
            let v = s as f64 / 32768.0;
            sum_sq += v * v;
        }
        let n = (end - i) as f64;
        out.push(((sum_sq / n).sqrt() as f32).min(1.0));
        i = end;
    }
    out
}

/// Peak cross-correlation between two envelopes, restricted to
/// lags in `[-max_lag, max_lag]` envelope samples. Returns the
/// lag (in envelope samples) that best aligns `b` onto `a`, and
/// the correlation score. Positive lag => `b` is shifted right
/// (i.e. `b` started later than `a`).
pub fn best_lag(a: &[f32], b: &[f32], max_lag: i64) -> (i64, f64) {
    if a.is_empty() || b.is_empty() {
        return (0, 0.0);
    }
    let mut best_lag: i64 = 0;
    let mut best_score: f64 = f64::NEG_INFINITY;
    for lag in -max_lag..=max_lag {
        let mut sum = 0.0f64;
        for (i, &av) in a.iter().enumerate() {
            let j = i as i64 + lag;
            if j < 0 || (j as usize) >= b.len() {
                continue;
            }
            sum += av as f64 * b[j as usize] as f64;
        }
        if sum > best_score {
            best_score = sum;
            best_lag = lag;
        }
    }
    (best_lag, best_score)
}

/// Decode `input` to mono s16 PCM at DECODE_RATE via ffmpeg.
/// Same invocation pattern as waveform::extract_peaks: protocol
/// whitelist to file, read stdout to end, s16le.
pub fn decode_to_mono_s16(ffmpeg: &Path, input: &Path) -> Result<Vec<i16>> {
    let mut child = crate::silent_cmd::silent_command(ffmpeg)
        .args(["-v", "error", "-protocol_whitelist", "file", "-i"])
        .arg(input)
        .args([
            "-vn",
            "-ac",
            "1",
            "-ar",
            &DECODE_RATE.to_string(),
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

    Ok(pcm
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect())
}

/// Maximum lag to search, expressed in ms. 5 s covers any
/// realistic multicam start-button delay. Total search space is
/// 2 * max_lag envelope samples.
const MAX_LAG_MS: i64 = 5000;

/// Spawn a background job that syncs `paths` (angle order) and
/// posts offsets through a channel. `paths[0]` is the reference;
/// returned offsets are relative to it and the first entry is 0.
pub fn spawn_multicam_sync(ffmpeg: PathBuf, paths: Vec<PathBuf>) -> Receiver<SyncEvent> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let total = paths.len();
        if total < 2 {
            let _ = tx.send(SyncEvent::Failed("need at least 2 angles to sync".into()));
            return;
        }
        let mut envelopes: Vec<Vec<f32>> = Vec::with_capacity(total);
        for (i, p) in paths.iter().enumerate() {
            let _ = tx.send(SyncEvent::Progress { current: i, total });
            match decode_to_mono_s16(&ffmpeg, p).map(|s| rms_envelope(&s)) {
                Ok(env) => envelopes.push(env),
                Err(e) => {
                    let _ = tx.send(SyncEvent::Failed(format!("decode {}: {e}", p.display())));
                    return;
                }
            }
        }
        let _ = tx.send(SyncEvent::Progress {
            current: total,
            total,
        });

        let max_lag_samples = MAX_LAG_MS * ENVELOPE_HZ as i64 / 1000;
        let ref_env = &envelopes[0];
        let mut offsets_ms: Vec<i64> = Vec::with_capacity(total);
        offsets_ms.push(0);
        for env in envelopes.iter().skip(1) {
            let (lag, _score) = best_lag(ref_env, env, max_lag_samples);
            // Positive lag => env shifted right => angle started
            // later. To align onto ref, subtract that many ms.
            let ms = -lag * 1000 / ENVELOPE_HZ as i64;
            offsets_ms.push(ms);
        }
        let _ = tx.send(SyncEvent::Done { offsets_ms });
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_of_silence_is_zeros() {
        let env = rms_envelope(&vec![0i16; 400]);
        assert_eq!(env.len(), 400 / WINDOW_SAMPLES);
        assert!(env.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn envelope_of_full_scale_is_near_one() {
        let env = rms_envelope(&vec![i16::MAX; 400]);
        assert!(env.iter().all(|&v| v > 0.99));
    }

    #[test]
    fn best_lag_finds_identical_signal_at_zero() {
        let a = vec![0.0, 0.5, 1.0, 0.5, 0.0];
        let (lag, _) = best_lag(&a, &a, 5);
        assert_eq!(lag, 0);
    }

    #[test]
    fn best_lag_finds_shift() {
        // b = a shifted right by 2 samples.
        let a = vec![0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let (lag, _) = best_lag(&a, &b, 5);
        assert_eq!(lag, 2, "b is 2 samples behind a");
    }

    #[test]
    fn best_lag_negative_shift() {
        let a = vec![0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let b = vec![0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        let (lag, _) = best_lag(&a, &b, 5);
        assert_eq!(lag, -2, "b is 2 samples ahead of a");
    }

    #[test]
    fn best_lag_empty_inputs_safe() {
        let (lag, score) = best_lag(&[], &[], 3);
        assert_eq!(lag, 0);
        assert_eq!(score, 0.0);
    }
}
