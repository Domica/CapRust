//! Record 5 seconds of the primary monitor to record.mp4.
//!
//! Uses `ffmpeg` from PATH. For production the app resolves
//! ffmpeg via DIRECTIVES 10.7; the example stays simple.
//!
//!   cargo run -p caprust-screen-record --example record

#[cfg(windows)]
use caprust_screen_record::record::{record_to_file, RecordHandle};
#[cfg(windows)]
use std::path::PathBuf;
#[cfg(windows)]
use std::time::Duration;

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    let ffmpeg = which_ffmpeg()?;
    let out = std::env::current_dir()?.join("record.mp4");
    println!("ffmpeg: {}", ffmpeg.display());
    println!("recording 5s of monitor 0 -> {}", out.display());

    let handle = RecordHandle::default();
    let stats = record_to_file(&ffmpeg, 0, &out, Duration::from_secs(5), 30, handle)?;
    println!(
        "done: {} frames in {:?} -> {}",
        stats.frames_written,
        stats.elapsed,
        stats.output_path.display()
    );
    Ok(())
}

/// Minimal ffmpeg lookup: PATH only. Good enough for the example.
fn which_ffmpeg() -> anyhow::Result<PathBuf> {
    let path = std::env::var_os("PATH").ok_or_else(|| anyhow::anyhow!("PATH not set"))?;
    let candidates = ["ffmpeg.exe", "ffmpeg"];
    for dir in std::env::split_paths(&path) {
        for name in &candidates {
            let p = dir.join(name);
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    anyhow::bail!("ffmpeg not found on PATH")
}

#[cfg(not(windows))]
fn main() {
    eprintln!("record: Windows only");
}
