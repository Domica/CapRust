//! Timeline-level preview renderer.
//!
//! Strategy:
//!  - No `-re` on input (that would force real-time decode from t=0,
//!    making a 30s seek wait 30 real seconds).
//!  - Use `-ss <start>` as an OUTPUT option, so ffmpeg decodes as fast
//!    as it can and just discards frames before `start`.
//!  - Reader thread THROTTLES its own emission to the target fps using
//!    `std::thread::sleep`, so the consumer sees exactly fps frames/sec.

use crate::export_graph::RenderPlan;
use anyhow::{Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// Number of decoded frames to buffer between the ffmpeg stdout reader
/// and the UI. Previously 3 (~125 ms at 24 fps), which was not enough to
/// survive a UI thread stall: when the UI was busy, the reader blocked on
/// `tx.send`, ffmpeg blocked on the stdout pipe, audio also stopped being
/// written to the PCM file, and the ringbuf drained to silence. The result
/// was an audible dropout and a frozen playhead for several hundred ms.
///
/// 24 frames gives ~1 second of slack at 24 fps (a bit under 2 s at 12 fps),
/// which comfortably covers typical scheduler hiccups.
const BUFFER_FRAMES: usize = 24;

pub struct PreviewRenderer {
    child: Child,
    rx: Receiver<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub started_at_ms: u64,
    /// Path to the s16le PCM file written by ffmpeg (None if no audio track).
    pub pcm_path: Option<PathBuf>,
}

impl PreviewRenderer {
    pub fn spawn(
        ffmpeg: &Path,
        plan: &RenderPlan,
        start_ms: u64,
        width: u32,
        height: u32,
        fps: f64,
    ) -> Result<Self> {
        let w = width.max(2) & !1;
        let h = height.max(2) & !1;
        let fps = if fps > 1.0 { fps } else { 30.0 };

        let (fg, v_label, a_label) = plan.build_filtergraph()?;

        let mut args: Vec<String> = vec![
            "-y".into(),
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
        ];

        // Inputs: images need -loop 1; everything else is plain -i.
        let image_indices: std::collections::HashSet<usize> = plan
            .video_clips
            .iter()
            .filter(|c| c.is_image)
            .map(|c| c.input_index)
            .collect();

        for inp in &plan.inputs {
            if image_indices.contains(&inp.ffmpeg_index) {
                args.push("-loop".into());
                args.push("1".into());
                args.push("-framerate".into());
                args.push(format!("{fps:.6}"));
            }
            args.push("-i".into());
            args.push(inp.path.to_string_lossy().to_string());
        }

        args.push("-filter_complex".into());
        args.push(fg.clone());

        // OUTPUT-side seek. Ffmpeg decodes everything as fast as it can,
        // discards the first `start_ms / 1000` seconds of the video output.
        // Audio is NOT seeked here — the full PCM file is written from t=0,
        // and AudioPlayer::play_pcm_file(path, start_from) seeks the reader
        // by byte offset instead. This keeps the ffmpeg arg list simple
        // (one -ss, one output) and avoids duplicating -ss per output.
        if start_ms > 0 {
            args.push("-ss".into());
            args.push(format!("{:.6}", start_ms as f64 / 1000.0));
        }

        // ---- OUTPUT 1: video to stdout ----
        // Every -map / -f / target combination must be adjacent, otherwise
        // ffmpeg lumps all preceding -map options into whichever output
        // target appears next — which previously sent [v] into the PCM file.
        args.push("-map".into());
        args.push(format!("[{v_label}]"));
        args.push("-f".into());
        args.push("rawvideo".into());
        args.push("-pix_fmt".into());
        args.push("rgba".into());
        args.push("-s".into());
        args.push(format!("{w}x{h}"));
        args.push("-r".into());
        args.push(format!("{fps:.6}"));
        args.push("-".into());

        // ---- OUTPUT 2: audio PCM file (optional) ----
        let mut pcm_path: Option<PathBuf> = None;
        if let Some(a) = &a_label {
            let path = std::env::temp_dir().join(format!(
                "caprust-audio-{}-{}.pcm",
                std::process::id(),
                start_ms
            ));
            std::fs::File::create(&path).ok();
            args.push("-map".into());
            args.push(format!("[{a}]"));
            args.push("-f".into());
            args.push("s16le".into());
            args.push("-ar".into());
            args.push("48000".into());
            args.push("-ac".into());
            args.push("2".into());
            args.push(path.to_string_lossy().to_string());
            pcm_path = Some(path);
        }

        tracing::debug!(
            "preview: ffmpeg args: {}",
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
            .with_context(|| format!("spawn preview ffmpeg ({} inputs)", plan.inputs.len()))?;

        // Forward ffmpeg stderr to tracing.
        if let Some(mut err) = child.stderr.take() {
            std::thread::spawn(move || {
                use std::io::BufRead;
                let reader = std::io::BufReader::new(&mut err);
                for line in reader.lines().map_while(Result::ok) {
                    if !line.trim().is_empty() {
                        tracing::warn!("preview ffmpeg: {line}");
                    }
                }
            });
        }

        let mut stdout = child.stdout.take().context("ffmpeg stdout missing")?;
        let (tx, rx) = sync_channel::<Vec<u8>>(BUFFER_FRAMES);
        let frame_size = (w * h * 4) as usize;
        let frame_interval = Duration::from_secs_f64(1.0 / fps);

        std::thread::spawn(move || {
            let mut buf = vec![0u8; frame_size];
            let mut next_emit = Instant::now();
            loop {
                if stdout.read_exact(&mut buf).is_err() {
                    break;
                }
                // Throttle: sleep so we emit at exactly `fps` frames/sec.
                let now = Instant::now();
                if next_emit > now {
                    std::thread::sleep(next_emit - now);
                }
                next_emit = Instant::now() + frame_interval;
                if tx.send(buf.clone()).is_err() {
                    break;
                }
            }
        });

        tracing::info!(
            "preview: renderer spawn from {start_ms}ms ({} inputs @ {fps:.2}fps)",
            plan.inputs.len()
        );

        Ok(Self {
            child,
            rx,
            pcm_path,
            width: w,
            height: h,
            fps,
            started_at_ms: start_ms,
        })
    }

    pub fn try_next(&self) -> Option<Vec<u8>> {
        match self.rx.try_recv() {
            Ok(b) => Some(b),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => None,
        }
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for PreviewRenderer {
    fn drop(&mut self) {
        self.kill();
        if let Some(p) = &self.pcm_path {
            let _ = std::fs::remove_file(p);
        }
    }
}
