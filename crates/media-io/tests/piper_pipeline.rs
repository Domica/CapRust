//! End-to-end Piper test.
//!
//! Requires:
//!   - internet access on first run (downloads piper binary)
//!   - a Piper voice at <models_dir>/piper-en-lessac.onnx + .onnx.json
//!
//! Ignored by default. Run locally:
//!   cargo nextest run -p caprust-media-io --run-ignored only piper_pipeline

use std::path::PathBuf;

use caprust_media_io::piper;

fn models_dir() -> PathBuf {
    let base = std::env::var("APPDATA")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("CapRust").join("models")
}

#[test]
#[ignore = "requires Piper voice + downloads piper binary on first run"]
fn synthesize_hello_world() -> anyhow::Result<()> {
    let models = models_dir();
    let voice = models.join("piper-en-lessac.onnx");
    if !voice.is_file() {
        eprintln!("SKIP: no piper voice at {}", voice.display());
        return Ok(());
    }

    let bin = piper::ensure_binary(&models)?;
    assert!(bin.is_file(), "ensure_binary did not produce a binary");

    let out = std::env::temp_dir().join("caprust-piper-test.wav");
    let _ = std::fs::remove_file(&out);
    piper::synthesize(&bin, &voice, "hello world", &out)?;

    let meta = std::fs::metadata(&out)?;
    assert!(
        meta.len() > 1_000,
        "synthesized WAV is suspiciously small: {} bytes",
        meta.len()
    );
    eprintln!("piper wrote {} bytes to {}", meta.len(), out.display());

    let _ = std::fs::remove_file(&out);
    Ok(())
}
