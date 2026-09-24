//! Preview player — decodes one frame at a time on a background thread.
//!
//! Design:
//!  - A shared `slot` (Mutex<Option<FrameJob>>) holds the SINGLE pending
//!    request. `request()` overwrites it — no queue growth.
//!  - The worker takes the slot, decodes, then checks whether a newer
//!    request landed while it was busy. If so, it discards its result
//!    (the caller no longer wants it) and loops.
//!  - Results post back through a normal channel; the UI polls and uploads
//!    to egui texture.
//!
//! This gives ~1 in-flight ffmpeg process with zero backlog, so latency
//! stays bounded (~150 ms) regardless of how fast the user seeks.

use caprust_media_io::streamer::FrameStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameKey {
    pub clip_id: uuid::Uuid,
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

/// Shared slot between the UI thread and the decoder worker.
struct Slot {
    inner: Mutex<Option<FrameJob>>,
    cv: Condvar,
}

impl Slot {
    fn new() -> Self {
        Self {
            inner: Mutex::new(None),
            cv: Condvar::new(),
        }
    }

    /// Overwrite the slot. If a job was already waiting, it is dropped.
    fn put(&self, job: FrameJob) {
        *self.inner.lock().unwrap() = Some(job);
        self.cv.notify_one();
    }

    /// Wait for a job, take it.
    fn take(&self) -> FrameJob {
        let mut guard = self.inner.lock().unwrap();
        while guard.is_none() {
            guard = self.cv.wait(guard).unwrap();
        }
        guard.take().unwrap()
    }

    /// Peek whether a newer job is waiting (without taking it).
    fn has_pending(&self) -> bool {
        self.inner.lock().unwrap().is_some()
    }

    /// Clear any pending job.
    fn clear(&self) {
        *self.inner.lock().unwrap() = None;
    }
}

#[derive(Default)]
pub struct PreviewPlayer {
    pub texture: Option<egui::TextureHandle>,
    pub last_key: Option<FrameKey>,
    /// Key currently being decoded (as far as the UI knows).
    pub pending: Option<FrameKey>,
    pub has_frame: bool,
    pub decoded: u64,
    slot: Option<Arc<Slot>>,
    /// Active streaming decoder (only while playing).
    pub stream: Option<FrameStream>,
    rx: Option<Receiver<FrameResult>>,
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
        let (tx_res, rx) = channel::<FrameResult>();
        let slot = Arc::new(Slot::new());
        let worker_slot = Arc::clone(&slot);

        std::thread::spawn(move || loop {
            // Wait for the next job.
            let job = worker_slot.take();

            let result = match caprust_media_io::player::decode_frame_rgba(
                &job.ffmpeg,
                &job.input,
                job.at_sec,
                job.key.width,
                job.key.height,
            ) {
                Ok(rgba) => FrameResult {
                    key: job.key,
                    rgba,
                    error: None,
                },
                Err(e) => FrameResult {
                    key: job.key,
                    rgba: Vec::new(),
                    error: Some(e.to_string()),
                },
            };

            // If a newer job landed while we were decoding, drop this result.
            if worker_slot.has_pending() {
                continue;
            }
            if tx_res.send(result).is_err() {
                break;
            }
        });

        Self {
            texture: None,
            last_key: None,
            pending: None,
            has_frame: false,
            decoded: 0,
            slot: Some(slot),
            rx: Some(rx),
            stream: None,
        }
    }

    /// Request a frame. Non-blocking; overwrites any queued request.
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
        // Round to 100 ms grid to avoid requesting near-duplicate frames.
        let rounded = (at_ms / 100) * 100;
        let key = FrameKey {
            clip_id,
            at_ms: rounded,
            width,
            height,
        };

        // Already showing this exact frame.
        if self.last_key == Some(key) {
            return;
        }
        // Already decoding this exact frame.
        if self.pending == Some(key) {
            return;
        }

        let Some(slot) = self.slot.as_ref() else {
            return;
        };

        slot.put(FrameJob {
            key,
            ffmpeg: ffmpeg.to_path_buf(),
            input: input.to_path_buf(),
            at_sec: rounded as f64 / 1000.0,
        });
        self.pending = Some(key);
    }

    /// Poll for completed frames. Uploads RGBA → egui texture.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.rx.as_ref() else {
            return;
        };
        while let Ok(res) = rx.try_recv() {
            // If we've moved on to another key since this result was
            // produced, drop it (it's stale).
            if self.pending != Some(res.key) {
                continue;
            }
            self.pending = None;

            if res.error.is_some() {
                continue;
            }
            let w = res.key.width as usize;
            let h = res.key.height as usize;
            let expected = w * h * 4;
            if res.rgba.len() < expected {
                continue;
            }
            let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &res.rgba[..expected]);
            let name = format!("prev-{}-{}", res.key.clip_id, res.key.at_ms);
            let handle = ctx.load_texture(name, img, egui::TextureOptions::LINEAR);
            self.texture = Some(handle);
            self.last_key = Some(res.key);
            self.has_frame = true;
            self.decoded += 1;
        }
    }

    /// Stop any active stream.
    pub fn stop_stream(&mut self) {
        if let Some(mut s) = self.stream.take() {
            s.kill();
        }
    }

    /// Start (or restart) streaming from `at_sec`. No-op if the same
    /// clip+position is already streaming.
    #[allow(clippy::too_many_arguments)]
    pub fn start_stream(
        &mut self,
        ffmpeg: &Path,
        input: &Path,
        clip_id: uuid::Uuid,
        at_sec: f64,
        width: u32,
        height: u32,
        fps: f64,
    ) {
        // If an equivalent stream is already active, keep it.
        if let Some(s) = self.stream.as_ref() {
            if s.clip_id == clip_id
                && s.width == width.max(2)
                && s.height == height.max(2)
                && (s.started_at_sec - at_sec).abs() < 0.35
            {
                return;
            }
        }
        self.stop_stream();
        match FrameStream::spawn(ffmpeg, input, clip_id, at_sec, width, height, fps) {
            Ok(s) => {
                tracing::info!(
                    "preview: stream start clip={} at={:.2}s {}x{} @ {}fps",
                    clip_id,
                    at_sec,
                    width,
                    height,
                    fps
                );
                self.stream = Some(s);
            }
            Err(e) => {
                tracing::warn!("preview: stream spawn failed — {e}");
            }
        }
    }

    /// Pull one frame from the active stream (if any) and upload it.
    /// Returns true if a new frame was drawn.
    pub fn poll_stream(&mut self, ctx: &egui::Context) -> bool {
        let Some(s) = self.stream.as_ref() else {
            return false;
        };
        let Some(buf) = s.try_next() else {
            return false;
        };
        let w = s.width as usize;
        let h = s.height as usize;
        let expected = w * h * 4;
        if buf.len() < expected {
            return false;
        }
        let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &buf[..expected]);
        let handle = ctx.load_texture("preview-stream", img, egui::TextureOptions::LINEAR);
        self.texture = Some(handle);
        self.has_frame = true;
        self.decoded += 1;
        true
    }

    /// If a stream is active, returns (started_at_sec, 1/fps seconds per frame).
    pub fn stream_frame_duration_ms(&self) -> Option<u64> {
        let s = self.stream.as_ref()?;
        let ms = (1000.0 / s.fps).round() as u64;
        Some(ms.max(1))
    }

    /// Cancel any queued job. Current decode finishes; its result is
    /// dropped at the worker boundary if the slot is still empty.
    pub fn cancel_pending(&mut self) {
        self.pending = None;
        if let Some(slot) = self.slot.as_ref() {
            slot.clear();
        }
    }

    /// Forget the current frame.
    pub fn clear(&mut self) {
        self.texture = None;
        self.last_key = None;
        self.cancel_pending();
        self.has_frame = false;
    }
}
