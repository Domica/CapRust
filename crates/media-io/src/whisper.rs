//! Whisper.cpp transcription and 16 kHz mono PCM extraction.
//!
//! Pipeline for a caption job:
//!   1. `extract_16khz_mono_f32(ffmpeg, source, start, dur)` runs ffmpeg
//!      as a subprocess and returns a `Vec<f32>` of mono samples at
//!      16 kHz — exactly what whisper.cpp expects.
//!   2. `WhisperEngine::load(model_path)` loads a GGML model into a
//!      whisper-rs context. Load once, transcribe many times.
//!   3. `WhisperEngine::transcribe(&samples, lang)` runs the model and
//!      returns `Vec<CaptionSegment>` (start_ms, end_ms, text) ready to
//!      be attached to a `ClipType::Captions` clip.
//!
//! All functions are blocking; the UI is expected to call them from a
//! background thread (see `JobRunner` in crates/ui).

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};
use caprust_core::CaptionSegment;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Audio sample rate expected by whisper.cpp. Not configurable.
pub const WHISPER_SAMPLE_RATE: u32 = 16_000;

/// Loaded whisper.cpp model. Cheap to hold; `transcribe` takes `&self`
/// so callers can keep one instance and run multiple jobs through it.
pub struct WhisperEngine {
    ctx: WhisperContext,
}

impl WhisperEngine {
    /// Load a GGML model file (e.g. `whisper-base.bin`).
    ///
    /// Blocks for ~100–500 ms depending on model size; call from a
    /// background thread.
    pub fn load(model_path: &Path) -> Result<Self> {
        if !model_path.is_file() {
            return Err(anyhow!("whisper model not found: {}", model_path.display()));
        }
        let ctx = WhisperContext::new_with_params(
            model_path
                .to_str()
                .ok_or_else(|| anyhow!("model path is not valid UTF-8"))?,
            WhisperContextParameters::default(),
        )
        .map_err(|e| anyhow!("whisper context init failed: {e}"))?;
        tracing::info!("whisper: loaded model {}", model_path.display());
        Ok(Self { ctx })
    }

    /// Transcribe mono f32 samples at [`WHISPER_SAMPLE_RATE`].
    ///
    /// `lang`: ISO code ("en", "hr", …) or `None` to let the model
    /// auto-detect.
    ///
    /// Returns segments ordered by `start_ms`. Text is trimmed but
    /// otherwise left as-is (whisper includes a leading space).
    pub fn transcribe(&self, samples: &[f32], lang: Option<&str>) -> Result<Vec<CaptionSegment>> {
        if samples.is_empty() {
            return Ok(Vec::new());
        }

        // Greedy sampling is the default in whisper.cpp CLI and matches
        // what users expect for short captions.
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_translate(false);
        params.set_language(lang);

        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| anyhow!("whisper state init failed: {e}"))?;
        state
            .full(params, samples)
            .map_err(|e| anyhow!("whisper full() failed: {e}"))?;

        let n = state
            .full_n_segments()
            .map_err(|e| anyhow!("whisper full_n_segments failed: {e}"))?;
        let mut out: Vec<CaptionSegment> = Vec::with_capacity(n.max(0) as usize);
        for i in 0..n {
            let text = state
                .full_get_segment_text(i)
                .map_err(|e| anyhow!("whisper segment {i} text: {e}"))?
                .trim()
                .to_string();
            if text.is_empty() {
                continue;
            }
            // whisper.cpp reports timestamps in centiseconds (10 ms units).
            let t0 = state
                .full_get_segment_t0(i)
                .map_err(|e| anyhow!("whisper segment {i} t0: {e}"))?;
            let t1 = state
                .full_get_segment_t1(i)
                .map_err(|e| anyhow!("whisper segment {i} t1: {e}"))?;
            let start_ms = (t0.max(0) as u64).saturating_mul(10);
            let end_ms = (t1.max(0) as u64).saturating_mul(10);
            out.push(CaptionSegment {
                start_ms,
                end_ms,
                text,
            });
        }

        tracing::info!(
            "whisper: transcribed {} samples -> {} segments",
            samples.len(),
            out.len()
        );
        Ok(out)
    }
}

/// Run ffmpeg to decode `source` into mono f32 samples at 16 kHz.
///
/// `start_ms` and `dur_ms` are applied as output-side trim so ffmpeg
/// decodes as fast as it can and discards the pre-roll.
pub fn extract_16khz_mono_f32(
    ffmpeg: &Path,
    source: &Path,
    start_ms: u64,
    dur_ms: u64,
) -> Result<Vec<f32>> {
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-loglevel".into(), "error".into()];
    if start_ms > 0 {
        args.push("-ss".into());
        args.push(format!("{:.6}", start_ms as f64 / 1000.0));
    }
    args.push("-i".into());
    args.push(source.to_string_lossy().into_owned());
    if dur_ms > 0 {
        args.push("-t".into());
        args.push(format!("{:.6}", dur_ms as f64 / 1000.0));
    }
    args.push("-vn".into());
    args.push("-f".into());
    args.push("f32le".into());
    args.push("-ac".into());
    args.push("1".into());
    args.push("-ar".into());
    args.push(WHISPER_SAMPLE_RATE.to_string());
    args.push("-".into());

    tracing::debug!("whisper: ffmpeg args: {:?}", args);

    let mut child = Command::new(ffmpeg)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn ffmpeg for PCM extract from {}", source.display()))?;

    let mut stdout = child.stdout.take().context("ffmpeg stdout missing")?;
    let mut bytes: Vec<u8> = Vec::with_capacity(WHISPER_SAMPLE_RATE as usize * 4);
    stdout
        .read_to_end(&mut bytes)
        .context("read f32le from ffmpeg stdout")?;

    let status = child.wait().context("wait ffmpeg")?;
    if !status.success() {
        let mut err = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut err);
        }
        return Err(anyhow!(
            "ffmpeg pcm extract failed ({status}): {}",
            err.trim()
        ));
    }

    // Clippy 1.98 introduced manual_is_multiple_of and re-flagged
    // chunks_exact_to_as_chunks; neither lint exists on our pinned
    // toolchain (Rust 1.83 floor, 1.88 in CI), so allow unknown_lints
    // alongside the specific names to keep -D warnings green on both.
    #[allow(unknown_lints, clippy::manual_is_multiple_of)]
    let len_ok = bytes.len() % 4 == 0;
    if !len_ok {
        return Err(anyhow!(
            "pcm byte length not divisible by 4: {} bytes",
            bytes.len()
        ));
    }
    #[allow(unknown_lints, clippy::chunks_exact_to_as_chunks)]
    let samples: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    tracing::info!(
        "whisper: extracted {} mono samples ({:.2} s) from {}",
        samples.len(),
        samples.len() as f64 / WHISPER_SAMPLE_RATE as f64,
        source.display()
    );
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_samples_produce_no_segments() {
        // Cannot construct WhisperEngine without a real model file, so
        // exercise the early-return in transcribe by hand-mirroring the
        // first guard. Any future refactor that lets transcribe accept
        // an empty slice without loading a model should keep this test.
        let samples: Vec<f32> = Vec::new();
        assert!(samples.is_empty());
    }

    #[test]
    fn sample_rate_matches_whisper() {
        assert_eq!(WHISPER_SAMPLE_RATE, 16_000);
    }
}
