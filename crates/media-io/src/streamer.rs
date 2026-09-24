//! Streaming frame decoder. One ffmpeg process emits RGBA frames on
//! stdout; a reader thread buffers them; the UI consumes one per
//! animation frame.
//!
//! This is the fast path used when the transport is in "Playing" state.
//! For scrubbing (paused), the seek-based `player::decode_frame_rgba`
//! is still used — one frame at a time, no process kept alive.

use anyhow::{Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, TryRecvError};

/// Number of frames we can buffer ahead of the consumer.
const BUFFER: usize = 4;

pub struct FrameStream {
    child: Child,
    rx: Receiver<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    /// Source time (seconds) at which the stream was started.
    pub started_at_sec: f64,
    /// Assumed frame rate for playhead advance.
    pub fps: f64,
    pub input: PathBuf,
    pub clip_id: uuid::Uuid,
}

impl FrameStream {
    /// Spawn ffmpeg streaming frames from `at_sec` at native rate.
    /// `fps_hint` is used only to force the output rate via `-r`.
    pub fn spawn(
        ffmpeg: &Path,
        input: &Path,
        clip_id: uuid::Uuid,
        at_sec: f64,
        width: u32,
        height: u32,
        fps_hint: f64,
    ) -> Result<Self> {
        let w = width.max(2);
        let h = height.max(2);
        let fps = if fps_hint.is_finite() && fps_hint > 1.0 {
            fps_hint
        } else {
            30.0
        };

        // Capture stderr so we can log ffmpeg errors if the stream dies.
        // `-re` forces ffmpeg to emit frames in real time (30fps of video per
        // 1s of wall clock). Without it, ffmpeg decodes as fast as the CPU
        // can, the 4-frame buffer fills instantly, and the UI sees 10 frames
        // arrive in one render cycle → apparent fast-forward.
        let mut child = Command::new(ffmpeg)
            .args(["-v", "error"])
            .args(["-re"])
            .args(["-ss", &format!("{at_sec:.3}")])
            .arg("-i")
            .arg(input)
            .args(["-an", "-sn", "-dn"])
            .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
            .args(["-sws_flags", "fast_bilinear"])
            .args(["-s", &format!("{w}x{h}")])
            .args(["-r", &format!("{fps:.6}")])
            .arg("-")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("spawn ffmpeg stream for {}", input.display()))?;

        // Drain stderr in a background thread, forwarding lines to tracing.
        if let Some(mut err) = child.stderr.take() {
            std::thread::spawn(move || {
                use std::io::BufRead;
                let reader = std::io::BufReader::new(&mut err);
                for line in reader.lines().map_while(Result::ok) {
                    tracing::warn!("ffmpeg stream stderr: {line}");
                }
            });
        }

        let mut stdout = child.stdout.take().context("ffmpeg stdout missing")?;
        let (tx, rx) = sync_channel::<Vec<u8>>(BUFFER);
        let frame_size = (w * h * 4) as usize;

        std::thread::spawn(move || {
            let mut buf = vec![0u8; frame_size];
            while stdout.read_exact(&mut buf).is_ok() {
                // Blocks if the UI is behind; provides back-pressure.
                if tx.send(buf.clone()).is_err() {
                    break;
                }
            }
        });

        Ok(Self {
            child,
            rx,
            width: w,
            height: h,
            started_at_sec: at_sec,
            fps,
            input: input.to_path_buf(),
            clip_id,
        })
    }

    /// Non-blocking: try to grab the next frame.
    pub fn try_next(&self) -> Option<Vec<u8>> {
        match self.rx.try_recv() {
            Ok(buf) => Some(buf),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => None,
        }
    }

    /// Terminate the ffmpeg process.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// True if the process is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for FrameStream {
    fn drop(&mut self) {
        self.kill();
    }
}
