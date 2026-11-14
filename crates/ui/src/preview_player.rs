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

use crate::double_buffer_preview::{
    DoubleBufferConfig, DoubleBufferPreview, PreviewFrame, SubmitOutcome,
};
use caprust_media_io::streamer::FrameStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Condvar, Mutex};

/// Stable name for the single preview texture. A constant (rather than a
/// per-frame timestamp) lets egui reuse one GPU texture across frames
/// instead of allocating a fresh one on every frame during playback —
/// the old timestamp-based name churned through thousands of orphaned
/// textures over a long preview.
const PREVIEW_TEXTURE_NAME: &str = "caprust-preview-texture";

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
    /// Mask texture for object removal preview (red overlay showing segmented object).
    pub mask_texture: Option<egui::TextureHandle>,
    pub last_key: Option<FrameKey>,
    /// Key currently being decoded (as far as the UI knows).
    pub pending: Option<FrameKey>,
    pub has_frame: bool,
    pub last_error: Option<String>,
    pub decoded: u64,
    /// Monotonic counter for frames pulled from the active stream, so each
    /// streamed frame carries a strictly increasing sequence (see 6.2).
    stream_seq: u64,
    slot: Option<Arc<Slot>>,
    /// Active streaming decoder (only while playing).
    pub stream: Option<FrameStream>,
    rx: Option<Receiver<FrameResult>>,
    /// The last decoded frame, as straight RGBA.
    ///
    /// Kept alongside the texture because egui's `TextureHandle` is an
    /// id and a manager, not pixels: in epaint 0.31 there is no way to
    /// read the image back out of it. Anything that needs the actual
    /// frame -- a remote phone asking for one over the MCP socket --
    /// has to get it from here, captured while the decoder still had
    /// it in hand.
    ///
    /// Straight RGBA, not egui's premultiplied storage, so it can go
    /// straight into an image encoder.
    frame: Option<RawFrame>,
    /// When true a remote (phone) MCP session is actively pulling preview
    /// frames, so `publish_frame` keeps a copy of the pixels for it. The
    /// copy is skipped otherwise -- during ordinary desktop editing there
    /// is no remote reader, and copying ~1 MB per frame at 30 fps is pure
    /// waste. The MCP layer raises/lowers this around its command drain.
    remote_active: bool,
    /// Double-buffer preview system for seamless playback and scrubbing.
    double_buffer: Option<DoubleBufferPreview>,
}

/// Straight-RGBA frame, kept for anything that needs pixels.
#[derive(Debug, Clone)]
pub struct RawFrame {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl PreviewPlayer {
    /// The last decoded frame, if there is one.
    pub fn raw_frame(&self) -> Option<&RawFrame> {
        self.frame.as_ref()
    }

    /// Tell the player whether a remote (phone) session is currently
    /// pulling preview frames. While false, `publish_frame` skips the
    /// per-frame pixel copy it only needs for that remote reader (6.10).
    pub fn set_remote_active(&mut self, active: bool) {
        self.remote_active = active;
    }

    /// Keep the pixels behind a frame that is already on screen.
    ///
    /// Deliberately not a public field. Every rendering path that sets
    /// `texture` has to set this too, and while the field was `pub`
    /// three of them did not -- so `raw_frame()` stayed `None` through
    /// the whole of playback and a remote client asking for a frame got
    /// `{"frame": null}` while the preview was plainly working on the
    /// desktop. A setter is the only way to make the pairing the only
    /// way in.
    pub fn capture_frame(&mut self, width: usize, height: usize, rgba: &[u8]) {
        self.frame = Some(RawFrame {
            width,
            height,
            rgba: rgba.to_vec(),
        });
    }

    /// Show a frame and keep a copy of its pixels, together.
    ///
    /// This is the only way a caller should update the texture, because
    /// it is the only way that cannot half-do the job. Setting
    /// `texture` by hand and forgetting the pixels compiles fine and
    /// produces a preview that works on the desktop and is invisible
    /// over MCP -- which is exactly what happened.
    pub fn publish_frame(&mut self, ctx: &egui::Context, width: usize, height: usize, rgba: &[u8]) {
        let img = egui::ColorImage::from_rgba_unmultiplied([width, height], rgba);
        // Reuse one stable texture across frames. `load_texture` resolves the
        // name in the texture manager and updates the existing slot in place
        // when the name is already registered, so a constant name keeps a
        // single persistent texture instead of churning a new one per frame.
        self.texture =
            Some(ctx.load_texture(PREVIEW_TEXTURE_NAME, img, egui::TextureOptions::LINEAR));
        self.has_frame = true;
        // Keep a copy of the pixels only when a remote session is watching
        // (6.10). The desktop preview never needs them; copying ~1 MB every
        // frame at 30 fps for nobody is wasted work.
        if self.remote_active {
            self.capture_frame(width, height, rgba);
        }
    }

    /// Set the mask texture for object removal preview.
    /// The mask should be a grayscale image where white = object, black = background.
    pub fn set_mask(
        &mut self,
        ctx: &egui::Context,
        name: &str,
        width: usize,
        height: usize,
        gray: &[u8],
    ) {
        // Convert grayscale to red-tinted RGBA for overlay
        let mut rgba = Vec::with_capacity(width * height * 4);
        for &g in gray {
            // Red tint with alpha based on mask intensity
            let alpha = g as f32 / 255.0;
            let r = (255.0 * alpha) as u8;
            rgba.push(r);
            rgba.push(0);
            rgba.push(0);
            rgba.push(g); // Use mask value as alpha
        }
        let img = egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba);
        self.mask_texture = Some(ctx.load_texture(name, img, egui::TextureOptions::LINEAR));
    }

    /// Clear the mask texture.
    pub fn clear_mask(&mut self) {
        self.mask_texture = None;
    }
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
            mask_texture: None,
            last_key: None,
            pending: None,
            has_frame: false,
            last_error: None,
            decoded: 0,
            stream_seq: 0,
            slot: Some(slot),
            rx: Some(rx),
            stream: None,
            frame: None,
            remote_active: false,
            double_buffer: Some(DoubleBufferPreview::new(DoubleBufferConfig::default())),
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
        // This is the one-shot paused fallback (the only caller of
        // `request`), so seek to the exact position instead of snapping to a
        // 100 ms grid. Rounding here produced stale/duplicate requests and
        // fed the backwards-seek rejection in `poll` (6.11).
        let key = FrameKey {
            clip_id,
            at_ms,
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
            at_sec: at_ms as f64 / 1000.0,
        });
        self.pending = Some(key);
        self.last_error = None;

        // Initialize double buffer if not already
        if self.double_buffer.is_none() {
            self.double_buffer = Some(DoubleBufferPreview::new(DoubleBufferConfig::default()));
        }
    }

    /// Poll for completed frames. Uploads RGBA → egui texture.
    pub fn poll(&mut self, ctx: &egui::Context) {
        // Drain the channel first, then publish. Holding `rx` across a
        // `&mut self` call borrows self twice, so the results are taken
        // into a local and uploaded afterwards.
        let mut ready = Vec::new();
        if let Some(rx) = self.rx.as_ref() {
            while let Ok(res) = rx.try_recv() {
                ready.push(res);
            }
        }
        for res in ready {
            // If we've moved on to another key since this result was
            // produced, drop it (it's stale).
            if self.pending != Some(res.key) {
                continue;
            }
            self.pending = None;

            if let Some(err) = res.error.as_ref() {
                self.last_error = Some(err.clone());
                continue;
            }
            let w = res.key.width as usize;
            let h = res.key.height as usize;
            let expected = w * h * 4;
            if res.rgba.len() < expected {
                continue;
            }

            // Submit to double-buffer system. Move the decoded pixels into the
            // frame instead of cloning them (was a full-frame copy per frame).
            if let Some(db) = &mut self.double_buffer {
                let mut rgba = res.rgba;
                rgba.truncate(expected);
                let frame = PreviewFrame {
                    rgba,
                    width: res.key.width,
                    height: res.key.height,
                    timestamp_ms: res.key.at_ms,
                    clip_id: res.key.clip_id,
                    sequence: res.key.at_ms / 100, // Use rounded ms as sequence
                };
                match db.submit_and_commit(frame) {
                    SubmitOutcome::Committed(front_frame) | SubmitOutcome::Stale(front_frame) => {
                        self.publish_frame(
                            ctx,
                            front_frame.width as usize,
                            front_frame.height as usize,
                            &front_frame.rgba,
                        );
                    }
                }
                self.last_key = Some(res.key);
                self.decoded += 1;
            } else {
                // Fallback: double buffer absent, publish directly.
                self.publish_frame(ctx, w, h, &res.rgba[..expected]);
                self.last_key = Some(res.key);
                self.decoded += 1;
            }
        }
    }

    /// Stop any active stream.
    pub fn stop_stream(&mut self) {
        if let Some(mut s) = self.stream.take() {
            s.kill();
        }
        // Clear double-buffer when stopping stream
        if let Some(db) = &mut self.double_buffer {
            db.clear();
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
        // Read everything we need from the stream and drop the borrow before
        // we touch other `self` fields (the partial borrow of `self.stream`
        // would otherwise conflict with `self.stream_seq`).
        let pulled = {
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
            (buf, w, h, s.started_at_sec, s.fps, s.clip_id)
        };

        let (buf, w, h, started_at_sec, fps, clip_id) = pulled;

        // 6.2: the old code used `started_at_sec * 1000` as both the
        // timestamp and the sequence, so every frame after the first carried
        // the same sequence and `submit_and_commit` rejected it as stale
        // (and only the defensive hand-back published it). Give each pulled
        // frame a strictly increasing sequence.
        self.stream_seq = self.stream_seq.wrapping_add(1);
        let frame_dur_ms = (1000.0 / fps.max(1.0)).round() as u64;
        let ts = (started_at_sec * 1000.0) as u64 + self.stream_seq * frame_dur_ms;

        let expected = w * h * 4;
        if let Some(db) = &mut self.double_buffer {
            let mut rgba = buf;
            rgba.truncate(expected);
            let frame = PreviewFrame {
                rgba,
                width: w as u32,
                height: h as u32,
                timestamp_ms: ts,
                clip_id,
                sequence: ts,
            };
            match db.submit_and_commit(frame) {
                SubmitOutcome::Committed(front_frame) | SubmitOutcome::Stale(front_frame) => {
                    self.publish_frame(
                        ctx,
                        front_frame.width as usize,
                        front_frame.height as usize,
                        &front_frame.rgba,
                    );
                }
            }
            self.decoded += 1;
            true
        } else {
            let mut rgba = buf;
            rgba.truncate(expected);
            self.publish_frame(ctx, w, h, &rgba);
            self.decoded += 1;
            true
        }
    }

    /// If a stream is active, returns (started_at_sec, 1/fps seconds per frame).
    pub fn stream_frame_duration_ms(&self) -> Option<u64> {
        let s = self.stream.as_ref()?;
        let fps: f64 = s.fps;
        let ms = (1000.0 / fps).round() as u64;
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
        self.frame = None;
        self.last_key = None;
        self.cancel_pending();
        self.has_frame = false;

        // Clear double-buffer state
        if let Some(db) = &mut self.double_buffer {
            db.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PreviewPlayer;

    /// The phone sees no frame because `frame` was never populated.
    ///
    /// `RemoteSession::preview_frame` answers from `raw_frame()`, and
    /// every rendering site in `app.rs` set `texture` and `has_frame`
    /// on the player without touching it. So the preview was perfect on
    /// the desktop and `{"frame": null}` over MCP, for as long as
    /// anyone cared to look. This asserts the round trip that made the
    /// bug invisible.
    #[test]
    fn publish_frame_keeps_the_pixels_for_the_remote_reader() {
        let mut player = PreviewPlayer::default();
        assert!(player.raw_frame().is_none());

        let w = 64usize;
        let h = 32usize;
        let rgba = vec![200u8; w * h * 4];
        // The capture only runs while a remote reader is watching; that is the
        // path this test exercises, so simulate it.
        player.set_remote_active(true);
        player.publish_frame(&egui::Context::default(), w, h, &rgba);

        let held = player.raw_frame().expect("frame kept for the reader");
        assert_eq!((held.width, held.height), (w, h));
        assert_eq!(held.rgba.len(), w * h * 4);
        assert_eq!(held.rgba[0], 200);
    }

    /// Nothing may assign the texture directly.
    ///
    /// `texture` stays `pub` for reading, and that is the whole hazard:
    /// a rendering site that writes it gets a working desktop preview
    /// and an empty phone preview, with no compiler complaint. The only
    /// way to see it is to look for the pattern, so this looks.
    #[test]
    fn no_rendering_path_assigns_the_texture_directly() {
        let src = include_str!("app.rs");
        for (i, line) in src.lines().enumerate() {
            let t = line.trim();
            if t.starts_with("//") || t.starts_with("assert!") {
                continue;
            }
            assert!(
                !t.contains(".texture = Some("),
                "app.rs:{} assigns the texture directly, so the pixels are \
                 not kept and preview_frame answers null. Use \
                 PreviewPlayer::publish_frame instead: {t}",
                i + 1
            );
        }
    }
}
