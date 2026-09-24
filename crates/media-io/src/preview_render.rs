//! Timeline-level preview renderer.
//!
//! Spawns ONE ffmpeg process that renders the whole timeline through the
//! SAME filtergraph as export. Output is raw RGBA video on stdout, one
//! frame at a time. UI reads frames and uploads to egui textures.
//!
//! Audio: emitted to a second pipe in `-f f32le` format. For now we
//! drain it to avoid blocking; a future PR wires it to cpal.

use crate::export_graph::RenderPlan;
use anyhow::{Context, Result};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, TryRecvError};

const BUFFER_FRAMES: usize = 4;

pub struct PreviewRenderer {
    child: Child,
    rx: Receiver<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub started_at_ms: u64,
}

impl PreviewRenderer {
    /// Start rendering the timeline from `start_ms` (0 = beginning).
    /// Frames before `start_ms` are decoded but discarded, so xfades
    /// and per-track chains are still correct at the start position.
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

        // Image inputs get -loop 1 + -framerate
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
            } else {
                // `-re` = read input at native rate. Without this, ffmpeg
                // decodes as fast as the CPU allows, the buffer saturates,
                // and preview runs 5-10x too fast.
                args.push("-re".into());
            }
            args.push("-ss".into());
            args.push(format!("{:.6}", inp.source_start_sec));
            args.push("-t".into());
            args.push(format!("{:.6}", inp.duration_sec));
            args.push("-i".into());
            args.push(inp.path.to_string_lossy().to_string());
        }

        args.push("-filter_complex".into());
        args.push(fg);
        args.push("-map".into());
        args.push(format!("[{v_label}]"));

        // Audio: if available, drain to null (real playback comes next PR)
        if let Some(a) = &a_label {
            args.push("-map".into());
            args.push(format!("[{a}]"));
            args.push("-f".into());
            args.push("null".into());
            args.push("-".into());
        }

        // Video output: raw RGBA on stdout
        args.push("-f".into());
        args.push("rawvideo".into());
        args.push("-pix_fmt".into());
        args.push("rgba".into());
        args.push("-s".into());
        args.push(format!("{w}x{h}"));
        args.push("-r".into());
        args.push(format!("{fps:.6}"));
        args.push("-".into());

        let mut child = Command::new(ffmpeg)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("spawn preview ffmpeg for {} inputs", plan.inputs.len()))?;

        // Drain stderr in background so we can log issues
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
        let frames_to_skip = ((start_ms as f64 / 1000.0) * fps).round() as u64;

        std::thread::spawn(move || {
            let mut buf = vec![0u8; frame_size];
            for _ in 0..frames_to_skip {
                if stdout.read_exact(&mut buf).is_err() {
                    return;
                }
            }
            loop {
                if stdout.read_exact(&mut buf).is_err() {
                    break;
                }
                if tx.send(buf.clone()).is_err() {
                    break;
                }
            }
        });

        tracing::info!(
            "preview: renderer spawn from {start_ms}ms ({} inputs, {} frames to skip)",
            plan.inputs.len(),
            frames_to_skip
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
