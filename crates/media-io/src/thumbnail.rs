//! Thumbnail extraction via ffmpeg subprocess.

use anyhow::{Context, Result};
use std::path::Path;

/// Extract a single frame at `at_sec` from `input` and save as JPEG to `out`.
/// Returns Ok only if the JPEG is actually on disk. A frame that is black
/// is the caller's problem; a frame that does not exist is an error.
pub fn extract_jpeg(
    ffmpeg: &Path,
    input: &Path,
    out: &Path,
    at_sec: f64,
    width: u32,
) -> Result<()> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }

    // Skip the seek entirely at t=0. `-ss 0` before `-i` makes ffmpeg
    // emit no frames for still images, and it exits 0 with no output
    // file -- which reads as success to thumbnail checks. This is the
    // JPG "no thumbnail" bug: seek_for returns 0.0 for images, so the
    // command was always asked to seek to exactly the bad point.
    let seek = if at_sec > 0.0 {
        vec!["-ss".to_string(), format!("{at_sec:.3}")]
    } else {
        Vec::new()
    };

    let status = crate::silent_cmd::silent_command(ffmpeg)
        .args(["-y", "-v", "error"])
        .args(seek)
        .args(["-protocol_whitelist", "file"])
        .arg("-i")
        .arg(input)
        .args(["-frames:v", "1"])
        // HEIC/HEIF/AVIF are tile-grid images. ffmpeg builds a complex
        // filtergraph (xstack) to stitch them, and `-vf` on top of that
        // fails with "Simple and complex filtering cannot be used
        // together". Let ffmpeg write the full still, and let the caller
        // scale it if it ever needs to be smaller.
        .args(
            match input
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("heic") | Some("heif") | Some("avif") => Vec::new(),
                _ => vec!["-vf".to_string(), format!("scale={width}:-1")],
            },
        )
        .args(["-update", "1"])
        .arg(out)
        .status()
        .with_context(|| format!("spawn ffmpeg thumbnail for {}", input.display()))?;

    if !status.success() {
        anyhow::bail!("ffmpeg thumbnail exited {} for {}", status, input.display());
    }

    // Exit status alone is not enough. Asking ffmpeg for a frame that
    // does not exist -- a still image seeked past its only frame, say --
    // can leave it exiting 0 with nothing written. The caller treats Ok
    // as "the thumbnail is ready", so an empty result here marked the
    // item done with no image behind it and no error in the log.
    match std::fs::metadata(out) {
        Ok(m) if m.len() > 0 => {}
        Ok(_) => anyhow::bail!(
            "ffmpeg thumbnail wrote 0 bytes for {} - no frame at {at_sec}s",
            input.display()
        ),
        Err(e) => anyhow::bail!(
            "ffmpeg thumbnail reported success but {} is unreadable: {e}",
            out.display()
        ),
    }
    Ok(())
}

/// Extract a JPEG at the "best-looking" moment:
/// 10% into the clip, but never earlier than 0.5s and never later than 5s.
///
/// Video only. A still image is one frame at t=0, so any non-zero seek
/// lands outside the input; [`seek_for`] returns 0.0 for those.
pub fn best_frame_time(duration_ms: u64) -> f64 {
    if duration_ms == 0 {
        return 0.0;
    }
    let tenth = (duration_ms as f64) / 1000.0 * 0.10;
    tenth.clamp(0.5, 5.0)
}

/// The seek point to pass to [`extract_jpeg`] for a given media kind.
///
/// Lives here rather than at the call site so the rule travels with the
/// function it protects. A still image has exactly one frame, at t=0, and
/// seeking anywhere else makes ffmpeg write nothing: `-ss 0.5` exits -22
/// on a PNG, and exits 0 having written nothing on a JPEG — which the
/// caller's status check used to read as success.
pub fn seek_for(kind: caprust_core::MediaKind, duration_ms: u64) -> f64 {
    match kind {
        // Exactly one frame, at t=0. Seeking anywhere else means
        // ffmpeg writes nothing.
        caprust_core::MediaKind::Image => 0.0,
        // Audio never reaches here (it draws a waveform instead), but if
        // it did, the 10%-in point is the sane answer rather than a
        // silent 0.
        caprust_core::MediaKind::Audio | caprust_core::MediaKind::Video => {
            best_frame_time(duration_ms)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for ffmpeg that exits 0 and writes nothing -- which is
    /// exactly what real ffmpeg does for a still image seeked past its
    /// only frame. `cmd /c exit 0` and `/bin/true` ignore every argument
    /// handed to them, so the real code path runs unmodified.
    ///
    /// Panics rather than skipping. An earlier version returned `None`
    /// and every caller did `let Some(x) = ... else { return }`, so both
    /// tests below passed while testing nothing: the file it looked for
    /// was `cmd` and the one on disk is `cmd.exe`. A silent skip
    /// guarding a real defect is worse than no test at all.
    fn silent_success() -> std::path::PathBuf {
        if cfg!(windows) {
            // `cmd` without the extension never matches on disk, and the
            // spawn would not resolve it either.
            assert!(
                which_on_path("cmd.exe"),
                "cmd.exe is not on PATH; this platform cannot run the test"
            );
            std::path::PathBuf::from("cmd.exe")
        } else {
            ["/bin/true", "/usr/bin/true"]
                .iter()
                .map(std::path::PathBuf::from)
                .find(|p| p.exists())
                .expect("no /bin/true or /usr/bin/true on this platform")
        }
    }

    fn which_on_path(name: &str) -> bool {
        let Some(path) = std::env::var_os("PATH") else {
            return false;
        };
        std::env::split_paths(&path).any(|dir| dir.join(name).exists())
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("caprust-thumb-test");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn success_with_no_output_is_an_error() {
        let fake = silent_success();
        let input = scratch("in.png");
        let out = scratch("never-written.jpg");
        let _ = std::fs::remove_file(&out);

        // Exit 0, no file at all. Before the guard this returned Ok,
        // which set `thumb_done` with an empty cache behind it and no log
        // line -- the JPEG branch of this bug, where ffmpeg exits clean
        // and writes nothing.
        let r = extract_jpeg(&fake, &input, &out, 0.5, 320);
        let msg = r
            .expect_err("a run that wrote no file is not a thumbnail")
            .to_string();
        // The guard separates "created but empty" from "never created".
        // Either way it is a failure, and both messages name the output so
        // the log says which path was involved.
        assert!(
            msg.contains("never-written.jpg"),
            "error should name the output file, got: {msg}"
        );
    }

    #[test]
    fn an_empty_file_is_also_an_error() {
        let fake = silent_success();
        let input = scratch("in.png");
        let out = scratch("touched.jpg");
        // ffmpeg can create the output and then write nothing into it, so
        // an existing-but-zero-length file has to fail as well.
        std::fs::write(&out, b"").unwrap();

        let r = extract_jpeg(&fake, &input, &out, 0.5, 320);
        let msg = r
            .expect_err("an empty output file is not a thumbnail")
            .to_string();
        assert!(
            msg.contains("0 bytes"),
            "error should say the file is empty, got: {msg}"
        );
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn best_frame_time_is_for_video_only() {
        // Documents the rule seek_for enforces: images must bypass this
        // entirely, because a still has exactly one frame, at 0.
        assert_eq!(best_frame_time(0), 0.5);
        assert_eq!(best_frame_time(10_000), 1.0);
        assert_eq!(best_frame_time(100_000), 5.0);
    }

    #[test]
    fn images_never_seek_past_their_only_frame() {
        use caprust_core::MediaKind;
        // Any non-zero seek here is the bug: `-ss 0.5` on a PNG exits
        // -22, and on a JPEG exits 0 having written nothing.
        for duration in [0u64, 1, 40, 1_000, 10_000, 1_000_000] {
            assert_eq!(
                seek_for(MediaKind::Image, duration),
                0.0,
                "an image must be read at t=0 whatever ffprobe claims \
                 its duration is (probed {duration} ms)"
            );
        }

        // Video is unaffected: still the 10%-in point, which is what
        // makes a thumbnail readable instead of a black frame.
        assert_eq!(seek_for(MediaKind::Video, 10_000), 1.0);
        assert_eq!(seek_for(MediaKind::Video, 0), 0.5);
        assert_eq!(seek_for(MediaKind::Audio, 10_000), 1.0);
    }
}
