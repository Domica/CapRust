//! End-to-end Whisper test.
//!
//! Requires:
//!   - ffmpeg in PATH
//!   - a Whisper GGML model at <models_dir>/whisper-tiny.bin
//!
//! Ignored by default because both prerequisites are large. Run locally:
//!   cargo nextest run -p caprust-media-io --run-ignored only whisper_pipeline
//!
//! The test does not need meaningful speech output — a 1 kHz sine wave
//! produces an empty (or near-empty) transcript. What we exercise is the
//! plumbing: ffmpeg subprocess, 16 kHz mono f32 extraction, whisper-rs
//! load + transcribe, and the CaptionSegment conversion.

use std::path::PathBuf;

use caprust_media_io::whisper::{extract_16khz_mono_f32, WhisperEngine};

fn ffmpeg_path() -> Option<PathBuf> {
    // Simple PATH lookup without pulling in a which crate.
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join("ffmpeg");
        if candidate.is_file() {
            return Some(candidate);
        }
        let candidate_exe = dir.join("ffmpeg.exe");
        if candidate_exe.is_file() {
            return Some(candidate_exe);
        }
    }
    None
}

fn model_path() -> Option<PathBuf> {
    // Look next to the repo, then in the user's CapRust models dir.
    let candidates = [
        PathBuf::from("./whisper-tiny.bin"),
        PathBuf::from("./testdata/whisper-tiny.bin"),
        dirs_models().join("whisper-tiny.bin"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

fn dirs_models() -> PathBuf {
    let base = std::env::var("APPDATA")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("CapRust").join("models")
}

#[test]
#[ignore = "requires ffmpeg + whisper-tiny.bin on disk"]
fn transcribe_sine_wave_does_not_crash() -> anyhow::Result<()> {
    let ffmpeg = match ffmpeg_path() {
        Some(p) => p,
        None => {
            eprintln!("SKIP: ffmpeg not on PATH");
            return Ok(());
        }
    };
    let model = match model_path() {
        Some(p) => p,
        None => {
            eprintln!("SKIP: whisper-tiny.bin not found");
            return Ok(());
        }
    };

    // 1) Generate a 1-second 1 kHz sine WAV with ffmpeg.
    let tmp = std::env::temp_dir().join("caprust-whisper-test.wav");
    let status = std::process::Command::new(&ffmpeg)
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=1000:duration=1",
            "-ac",
            "1",
            "-ar",
            "16000",
        ])
        .arg(&tmp)
        .status()?;
    assert!(status.success(), "ffmpeg sine generation failed");

    // 2) Extract 16 kHz mono f32.
    let samples = extract_16khz_mono_f32(&ffmpeg, &tmp, 0, 1000)?;
    assert!(
        (samples.len() as i64 - 16_000).abs() < 500,
        "expected ~16000 samples, got {}",
        samples.len()
    );

    // 3) Transcribe. Sine wave is not speech, so segments may be empty.
    let engine = WhisperEngine::load(&model)?;
    let segments = engine.transcribe(&samples, Some("en"))?;
    eprintln!("whisper returned {} segments", segments.len());
    // Only structural checks here.
    for s in &segments {
        assert!(s.end_ms >= s.start_ms, "segment has inverted timestamps");
    }

    let _ = std::fs::remove_file(&tmp);
    Ok(())
}
