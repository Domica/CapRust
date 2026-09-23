//! Preview player — decodes one frame at a time on a background thread.
//!
//! MVP: seek-based. When the app wants a frame for `(clip_id, at_ms)`,
//! it calls `request()`. The worker spawns ffmpeg for that frame and
//! sends the RGBA bytes back. `poll()` converts them to an egui texture.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameKey {
    pub clip_id: uuid::Uuid,
    /// Rounded to 100 ms — prevents spamming requests per animation frame.
    pub at_ms: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone)]
struct FrameJob {
    key: FrameKey,
    ffmpeg: PathBuf,
    input: PathBuf,
    at_sec: f64,
}

struct FrameResult {
    key: FrameKey,
    rgba: Vec<u8>,
    error: Option<String>,
}

#[derive(Default)]
pub struct PreviewPlayer {
    pub texture: Option<egui::TextureHandle>,
    pub last_key: Option<FrameKey>,
    pub pending: Option<FrameKey>,
    /// True once at least one frame is decoded (for "playing" indicator).
    pub has_frame: bool,
    /// Total frames decoded this session (debug).
    pub decoded: u64,
    tx: Option<Sender<FrameJob>>,
    rx: Option<Receiver<FrameResult>>,
    /// Latest queued-but-not-yet-sent request. Replaces `pending` when
    /// the worker finishes the current frame. Guarantees at most 1 job
    /// in flight + 1 job waiting.
    want: Option<FrameJob>,
}

impl std::fmt::Debug for PreviewPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewPlayer")
            .field("has_texture", &self.texture.is_some())
            .field("last_key", &self.last_key)
            .field("pending", &self.pending)
            .field("decoded", &self.decoded)
            .finish()
    }
}

impl PreviewPlayer {
    pub fn new() -> Self {
        let (tx, rx_job) = channel::<FrameJob>();
        let (tx_res, rx) = channel::<FrameResult>();

        std::thread::spawn(move || {
            while let Ok(job) = rx_job.recv() {
                match caprust_media_io::player::decode_frame_rgba(
                    &job.ffmpeg,
                    &job.input,
                    job.at_sec,
                    job.key.width,
                    job.key.height,
                ) {
                    Ok(rgba) => {
                        let _ = tx_res.send(FrameResult {
                            key: job.key,
                            rgba,
                            error: None,
                        });
                    }
                    Err(e) => {
                        let _ = tx_res.send(FrameResult {
                            key: job.key,
                            rgba: Vec::new(),
                            error: Some(e.to_string()),
                        });
                    }
                }
            }
        });

        Self {
            texture: None,
            last_key: None,
            pending: None,
            has_frame: false,
            decoded: 0,
            tx: Some(tx),
            rx: Some(rx),
            want: None,
        }
    }

    /// Request a frame. Non-blocking. Skips if the same key is already
    /// loaded or in flight.
    #[allow(clippy::too_many_arguments)]
    pub fn request(
        &mut self,
        ffmpeg: &Path,
        input: &Path,
        clip_id: uuid::Uuid,
        at_ms: u64,
        width: u32,
        height: u32,
    ) {
        let rounded = (at_ms / 100) * 100; // 100 ms grid
        let key = FrameKey {
            clip_id,
            at_ms: rounded,
            width,
            height,
        };

        if self.last_key == Some(key) || self.pending == Some(key) {
            return;
        }
        let Some(tx) = self.tx.as_ref() else {
            tracing::warn!("preview: no worker channel");
            return;
        };
        tracing::info!(
            "preview: request clip={} at={}ms {}x{}",
            clip_id,
            rounded,
            width,
            height
        );
        self.pending = Some(key);
        let _ = tx.send(FrameJob {
            key,
            ffmpeg: ffmpeg.to_path_buf(),
            input: input.to_path_buf(),
            at_sec: rounded as f64 / 1000.0,
        });
    }

    /// Poll for completed frames. Uploads RGBA → egui texture.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.rx.as_ref() else {
            return;
        };
        while let Ok(res) = rx.try_recv() {
            if self.pending == Some(res.key) {
                self.pending = None;
            }
            if let Some(e) = res.error.as_ref() {
                tracing::warn!("preview: decode error — {e}");
                continue;
            }
            tracing::info!(
                "preview: got {} bytes for clip={} at={}ms",
                res.rgba.len(),
                res.key.clip_id,
                res.key.at_ms
            );
            let w = res.key.width as usize;
            let h = res.key.height as usize;
            let expected = w * h * 4;
            if res.rgba.len() < expected {
                continue;
            }
            let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &res.rgba[..expected]);
            let name = format!("prev-{}-{}-{}x{}", res.key.clip_id, res.key.at_ms, w, h);
            let handle = ctx.load_texture(name, img, egui::TextureOptions::LINEAR);
            self.texture = Some(handle);
            self.last_key = Some(res.key);
            self.has_frame = true;
            self.decoded += 1;
        }

        // Send the latest queued request if any (keeps at most 1 in flight).
        if self.pending.is_none() {
            if let Some(job) = self.want.take() {
                if self.last_key != Some(job.key) {
                    if let Some(tx) = self.tx.as_ref() {
                        tracing::info!(
                            "preview: send queued clip={} at={}ms",
                            job.key.clip_id,
                            job.key.at_ms
                        );
                        self.pending = Some(job.key);
                        let _ = tx.send(job);
                    }
                }
            }
        }
    }

    /// Forget the current frame (e.g. after project change).
    pub fn clear(&mut self) {
        self.texture = None;
        self.last_key = None;
        self.pending = None;
        self.has_frame = false;
    }
}
