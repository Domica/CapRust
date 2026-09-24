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
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, TryRecvError};
use std::time::{Duration, Instant};

const BUFFER_FRAMES: usize = 3;

pub struct PreviewRenderer {
    child: Child,
    rx: Receiver<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub started_at_ms: u64,
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
        // discards the first `start_ms / 1000` seconds of the composition.
        if start_ms > 0 {
            args.push("-ss".into());
            args.push(format!("{:.6}", start_ms as f64 / 1000.0));
        }

        args.push("-map".into());
        args.push(format!("[{v_label}]"));

        if let Some(a) = &a_label {
            // Drain audio to /dev/null for now (real playback next PR).
            args.push("-map".into());
            args.push(format!("[{a}]"));
            args.push("-f".into());
            args.push("null".into());
            args.push("-".into());
        }

        // Video: raw RGBA on stdout.
        args.push("-f".into());
        args.push("rawvideo".into());
        args.push("-pix_fmt".into());
        args.push("rgba".into());
        args.push("-s".into());
        args.push(format!("{w}x{h}"));
        args.push("-r".into());
        args.push(format!("{fps:.6}"));
        args.push("-".into());

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
    }
}
