//! Pre-rendered audio PCM cache.
//!
//! Renders the whole project audio mix to a single stable PCM file
//! on disk, using the same filtergraph the export and preview use
//! (`RenderPlan::build_audio_only_filtergraph`). The reader side
//! (`AudioPlayer::play_pcm_file`) then only has to seek into this
//! file, so:
//!
//!   * audio no longer restarts when the preview renderer respawns
//!     (K1 hash change, seek, scrub),
//!   * seek latency drops from "decode every input from t=0" to
//!     "open a file and byte-seek",
//!   * the accumulated-playback-baseline drift goes away.
//!
//! The cache is invalidated whenever `ProjectState::audio_render_hash`
//! changes. Callers re-spawn on hash change (see `app.rs`).

use crate::export_graph::RenderPlan;
use anyhow::{Context, Result};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Sample rate requested from ffmpeg. Must match `AudioPlayer::play_pcm_file`.
pub const AUDIO_SAMPLE_RATE: u32 = 48_000;
/// Channels requested from ffmpeg. Must match `AudioPlayer::play_pcm_file`.
pub const AUDIO_CHANNELS: u16 = 2;

/// Path for the cached PCM file for a given `audio_render_hash`.
/// Lives in `%TEMP%` and embeds the process id + hash so multiple
/// hashes never collide and a stray file does not fool a later run.
///
/// TODO(alpha): old cache files are not deleted when a new hash
/// replaces them. OS-level temp cleanup handles it eventually; a
/// follow-up commit should delete them on project close or when a
/// new render completes.
pub fn cache_path(hash: u64) -> PathBuf {
    std::env::temp_dir().join(format!(
        "caprust-audio-cache-{}-{:016x}.pcm",
        std::process::id(),
        hash
    ))
}

/// Progress and completion events emitted by `spawn_audio_render`.
#[derive(Debug)]
pub enum AudioRenderEvent {
    /// ffmpeg started; `target` is the final (non-`.part`) path.
    Started { target: PathBuf },
    /// ffmpeg exited cleanly and the file was renamed into place.
    Done { target: PathBuf, size_bytes: u64 },
    /// ffmpeg exited with a non-zero status, or the file is missing.
    Failed(String),
}

/// Owns the background ffmpeg child. Killed on drop.
///
/// The caller polls [`AudioRenderJob::poll`] every frame (or on a
/// timer). When it returns `Some(Done)`, the PCM file is at
/// `target_path`. When it returns `Some(Failed)`, the `.part` file
/// has been cleaned up.
///
/// Dropping the job kills a still-running render and removes the
/// `.part` file, so a seek that fires a new render cancels the old
/// one cleanly.
pub struct AudioRenderJob {
    child: Child,
    /// `<target>.part` while running; renamed to `<target>` on success.
    part_path: PathBuf,
    target_path: PathBuf,
    /// The stderr forwarder. Joined on drop so we do not leak threads.
    stderr_thread: Option<std::thread::JoinHandle<()>>,
}

impl AudioRenderJob {
    /// Non-blocking poll. Returns `Some(Done)` once the render has
    /// finished successfully (and the `.part` file has been renamed
    /// to `target_path`), or `Some(Failed)` on any error. Returns
    /// `None` while the child is still running.
    pub fn poll(&mut self) -> Option<AudioRenderEvent> {
        match self.child.try_wait() {
            Ok(None) => None,
            Ok(Some(status)) => {
                let ev = if status.success() {
                    match std::fs::rename(&self.part_path, &self.target_path) {
                        Ok(()) => {
                            let size = std::fs::metadata(&self.target_path)
                                .map(|m| m.len())
                                .unwrap_or(0);
                            tracing::info!(
                                "audio_render: done {} ({size} bytes)",
                                self.target_path.display()
                            );
                            AudioRenderEvent::Done {
                                target: self.target_path.clone(),
                                size_bytes: size,
                            }
                        }
                        Err(e) => {
                            let _ = std::fs::remove_file(&self.part_path);
                            AudioRenderEvent::Failed(format!(
                                "rename {} -> {}: {e}",
                                self.part_path.display(),
                                self.target_path.display(),
                            ))
                        }
                    }
                } else {
                    let _ = std::fs::remove_file(&self.part_path);
                    AudioRenderEvent::Failed(format!("ffmpeg exit status {status}"))
                };
                Some(ev)
            }
            Err(e) => Some(AudioRenderEvent::Failed(format!("wait: {e}"))),
        }
    }
}

impl Drop for AudioRenderJob {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(h) = self.stderr_thread.take() {
            let _ = h.join();
        }
        // Only remove the `.part` file. If poll() already renamed it,
        // this is a no-op; if not, we are abandoning an incomplete
        // render and want the temp file gone.
        if self.part_path.exists() {
            let _ = std::fs::remove_file(&self.part_path);
        }
    }
}

/// Render the audio pipeline of `plan` to `target` as
/// `s16le / 48 kHz / 2 ch`. Runs on a background thread; emits
/// progress through the returned receiver.
///
/// `ffmpeg` is the resolved ffmpeg.exe path. `target` should live in
/// `%TEMP%` or the project's cache dir -- the caller decides.
///
/// The function returns as soon as the child is spawned. The thread
/// that forwards stderr and the terminal event owns `child` inside
/// the job struct.
pub fn spawn_audio_render(
    ffmpeg: &Path,
    plan: &RenderPlan,
    target: PathBuf,
) -> Result<AudioRenderJob> {
    let fg = plan
        .build_audio_only_filtergraph()
        .context("build audio-only filtergraph")?
        .ok_or_else(|| anyhow::anyhow!("project has no audio to render"))?;

    let part_path = target.with_extension("pcm.part");
    // Ensure the directory exists.
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::File::create(&part_path).with_context(|| format!("create {}", part_path.display()))?;

    let mut args: Vec<String> = vec![
        "-y".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
    ];
    // Inputs: reuse the same per-input ordering as the preview/export
    // path so the ffmpeg_index values in the filtergraph resolve.
    // Images do not contribute audio, so no -loop is needed here.
    for inp in &plan.inputs {
        args.push("-protocol_whitelist".into());
        args.push("file".into());
        args.push("-i".into());
        args.push(inp.path.to_string_lossy().to_string());
    }
    args.push("-filter_complex".into());
    args.push(fg);
    args.push("-map".into());
    args.push("[a_final]".into());
    args.push("-f".into());
    args.push("s16le".into());
    args.push("-ar".into());
    args.push(AUDIO_SAMPLE_RATE.to_string());
    args.push("-ac".into());
    args.push(AUDIO_CHANNELS.to_string());
    args.push(part_path.to_string_lossy().to_string());

    tracing::info!(
        "audio_render: spawn ffmpeg ({} inputs) -> {}",
        plan.inputs.len(),
        target.display()
    );

    let mut child = Command::new(ffmpeg)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn audio_render ffmpeg ({} inputs)", plan.inputs.len()))?;

    // Forward stderr to tracing so a failed render is diagnosable.
    let stderr_thread = child.stderr.take().map(|err| {
        std::thread::spawn(move || {
            let reader = BufReader::new(err);
            for line in reader.lines().map_while(Result::ok) {
                if !line.is_empty() {
                    tracing::debug!("audio_render: {line}");
                }
            }
        })
    });

    Ok(AudioRenderJob {
        child,
        part_path,
        target_path: target,
        stderr_thread,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_rate_matches_player() {
        // AudioPlayer::play_pcm_file hard-codes 48 kHz stereo s16le.
        assert_eq!(AUDIO_SAMPLE_RATE, 48_000);
        assert_eq!(AUDIO_CHANNELS, 2);
    }
}
