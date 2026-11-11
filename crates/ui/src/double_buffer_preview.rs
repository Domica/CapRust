//! Double-buffer preview system for seamless flicker-free playback and scrubbing.
//!
//! Implements a double-buffer system with front/back texture buffers
//! and a frame queue for smooth scrubbing and playback.
//!
//! Architecture:
//! - Two egui textures: front (displayed) and back (being rendered into)
//! - Frame queue for smooth scrubbing/playback
//! - Atomic swap between front/back on frame completion
//! - Frame cache for smooth scrubbing

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use uuid::Uuid;

/// A single frame in the preview pipeline.
#[derive(Debug, Clone)]
pub struct PreviewFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub timestamp_ms: u64,
    pub clip_id: Uuid,
    pub sequence: u64,
}

/// What the caller must do after handing a freshly decoded frame to the
/// double buffer.
#[derive(Debug)]
pub enum SubmitOutcome {
    /// The frame was accepted and swapped to the front. Publish `frame`.
    Committed(PreviewFrame),
    /// The frame is not newer than the one still waiting in the back
    /// buffer (a backwards seek). The buffered history was discarded;
    /// publish the raw decoded buffer directly instead.
    Stale,
}

/// A double-buffer frame pair (front/back).
#[derive(Debug)]
pub struct DoubleBufferFrame {
    pub front: Option<PreviewFrame>,
    pub back: Option<PreviewFrame>,
}

impl DoubleBufferFrame {
    pub fn new() -> Self {
        Self {
            front: None,
            back: None,
        }
    }

    /// Swap front and back buffers atomically.
    pub fn swap(&mut self) {
        std::mem::swap(&mut self.front, &mut self.back);
    }

    /// Get the front buffer for display.
    pub fn front(&self) -> Option<&PreviewFrame> {
        self.front.as_ref()
    }

    /// Get mutable reference to back buffer for rendering.
    pub fn back_mut(&mut self) -> &mut Option<PreviewFrame> {
        &mut self.back
    }
}

impl Default for DoubleBufferFrame {
    fn default() -> Self {
        Self::new()
    }
}

/// Frame queue for smooth scrubbing and playback.
#[derive(Debug)]
pub struct FrameQueue {
    queue: VecDeque<PreviewFrame>,
    max_size: usize,
    /// Most recently added frame sequence number.
    last_sequence: u64,
}

impl FrameQueue {
    pub fn new(max_size: usize) -> Self {
        Self {
            queue: VecDeque::with_capacity(max_size),
            max_size,
            last_sequence: 0,
        }
    }

    /// Push a new frame, dropping oldest if at capacity.
    pub fn push(&mut self, frame: PreviewFrame) {
        if self.queue.len() >= self.max_size {
            self.queue.pop_front();
        }
        self.last_sequence = frame.sequence;
        self.queue.push_back(frame);
    }

    /// Pop the next frame in sequence order.
    pub fn pop(&mut self) -> Option<PreviewFrame> {
        self.queue.pop_front()
    }

    /// Peek at the next frame without removing it.
    pub fn peek(&self) -> Option<&PreviewFrame> {
        self.queue.front()
    }

    /// Find a frame by sequence number.
    pub fn find_by_sequence(&self, sequence: u64) -> Option<&PreviewFrame> {
        self.queue.iter().find(|f| f.sequence == sequence)
    }

    /// Get frames around a target sequence for interpolation.
    pub fn get_around_sequence(&self, sequence: u64) -> Option<(&PreviewFrame, &PreviewFrame)> {
        let idx = self.queue.iter().position(|f| f.sequence >= sequence)?;
        if idx == 0 {
            return None;
        }
        Some((&self.queue[idx - 1], &self.queue[idx]))
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn clear(&mut self) {
        self.queue.clear();
    }
}

/// Configuration for the double-buffer preview system.
#[derive(Debug, Clone)]
pub struct DoubleBufferConfig {
    /// Maximum frames to keep in the queue.
    pub max_queue_size: usize,
    /// Target FPS for playback.
    pub target_fps: f64,
    /// Enable frame interpolation for slow-mo scrub.
    pub enable_interpolation: bool,
    /// Max frames to hold for A/V sync delay.
    pub av_delay_frames: usize,
    /// Frame interpolation factor (0.0-1.0).
    pub interpolation_factor: f32,
}

impl Default for DoubleBufferConfig {
    fn default() -> Self {
        Self {
            max_queue_size: 30, // ~1.25s at 24fps
            target_fps: 30.0,
            enable_interpolation: true,
            av_delay_frames: 3, // ~100ms at 30fps
            interpolation_factor: 0.5,
        }
    }
}

/// Core double-buffer preview engine.
pub struct DoubleBufferPreview {
    /// Double-buffered frame pair.
    frame_pair: Arc<Mutex<DoubleBufferFrame>>,
    /// Frame queue for smooth scrubbing/playback.
    frame_queue: Arc<Mutex<FrameQueue>>,
    /// Configuration.
    config: DoubleBufferConfig,
    /// Last frame time for FPS calculation.
    last_frame_time: Arc<Mutex<Option<Instant>>>,
    /// Current FPS.
    current_fps: Arc<Mutex<f64>>,
}

impl DoubleBufferPreview {
    pub fn new(config: DoubleBufferConfig) -> Self {
        Self {
            frame_pair: Arc::new(Mutex::new(DoubleBufferFrame::new())),
            frame_queue: Arc::new(Mutex::new(FrameQueue::new(config.max_queue_size))),
            config,
            last_frame_time: Arc::new(Mutex::new(None)),
            current_fps: Arc::new(Mutex::new(30.0)),
        }
    }

    /// Submit a new frame to the back buffer.
    /// Returns true if the frame was accepted.
    pub fn submit_frame(&self, frame: PreviewFrame) -> bool {
        let mut frame_pair = self.frame_pair.lock().unwrap();

        // Only accept if back buffer is free or we're overwriting stale data
        if frame_pair.back.is_some() {
            // Check if the existing back buffer is stale (older sequence)
            if let Some(existing) = &frame_pair.back {
                if existing.sequence >= frame.sequence {
                    return false; // New frame is stale, reject
                }
            }
        }

        frame_pair.back = Some(frame);
        true
    }

    /// Complete rendering into back buffer and swap.
    /// Returns the front frame that is now ready for display.
    pub fn commit_and_swap(&self) -> Option<PreviewFrame> {
        let mut frame_pair = self.frame_pair.lock().unwrap();

        let _ = frame_pair.back.as_ref()?;

        // Swap front and back
        frame_pair.swap();

        // Return the new front frame
        frame_pair.front.clone()
    }

    /// Submit a frame and commit the swap *only if it was accepted*.
    ///
    /// `submit_frame` returns false when the incoming sequence is not
    /// newer than the frame still sitting in the back buffer. Callers
    /// used to ignore that bool and call `commit_and_swap()` anyway,
    /// which swapped the stale back buffer to the front: the preview
    /// showed the previous picture while `PreviewPlayer` recorded the
    /// new frame key, and `request()` then refused to decode that frame
    /// again. On rejection we drop the buffered history instead so the
    /// caller can publish the frame it actually decoded.
    pub fn submit_and_commit(&self, frame: PreviewFrame) -> SubmitOutcome {
        if !self.submit_frame(frame) {
            self.clear();
            return SubmitOutcome::Stale;
        }
        match self.commit_and_swap() {
            Some(front) => SubmitOutcome::Committed(front),
            None => SubmitOutcome::Stale,
        }
    }

    /// Add frame to queue for smooth scrubbing/playback.
    pub fn enqueue(&self, frame: PreviewFrame) {
        self.frame_queue.lock().unwrap().push(frame);
    }

    /// Get next frame from queue in sequence order.
    pub fn dequeue(&self) -> Option<PreviewFrame> {
        self.frame_queue.lock().unwrap().pop()
    }

    /// Peek at next frame without removing.
    pub fn peek_queue(&self) -> Option<PreviewFrame> {
        self.frame_queue.lock().unwrap().peek().cloned()
    }

    /// Get frames for interpolation around a target sequence.
    pub fn get_interpolation_frames(
        &self,
        target_sequence: u64,
    ) -> Option<(PreviewFrame, PreviewFrame)> {
        self.frame_queue
            .lock()
            .unwrap()
            .get_around_sequence(target_sequence)
            .map(|(a, b)| (a.clone(), b.clone()))
    }

    /// Interpolate between two frames.
    pub fn interpolate(&self, a: &PreviewFrame, b: &PreviewFrame, factor: f32) -> PreviewFrame {
        let factor = factor.clamp(0.0, 1.0);

        // Interpolate RGBA pixels
        let mut rgba = Vec::with_capacity(a.rgba.len());
        for (a_pixel, b_pixel) in a.rgba.chunks_exact(4).zip(b.rgba.chunks_exact(4)) {
            for i in 0..4 {
                let a_val = a_pixel[i] as f32;
                let b_val = b_pixel[i] as f32;
                let interp = a_val + (b_val - a_val) * factor;
                rgba.push(interp.clamp(0.0, 255.0) as u8);
            }
        }

        PreviewFrame {
            rgba,
            width: a.width,
            height: a.height,
            timestamp_ms: ((a.timestamp_ms as f32
                + (b.timestamp_ms as f32 - a.timestamp_ms as f32) * factor)
                .round() as u64),
            clip_id: a.clip_id,
            sequence: a.sequence, // Use a's sequence as base
        }
    }

    /// Update FPS counter.
    pub fn update_fps(&self) {
        let now = Instant::now();
        let mut last_time = self.last_frame_time.lock().unwrap();
        let mut fps = self.current_fps.lock().unwrap();

        if let Some(last) = *last_time {
            let elapsed = last.elapsed().as_secs_f64();
            if elapsed > 0.0 {
                *fps = 1.0 / elapsed;
            }
        }
        *last_time = Some(now);
    }

    pub fn current_fps(&self) -> f64 {
        *self.current_fps.lock().unwrap()
    }

    pub fn frame_pair(&self) -> &Arc<Mutex<DoubleBufferFrame>> {
        &self.frame_pair
    }

    pub fn frame_queue(&self) -> &Arc<Mutex<FrameQueue>> {
        &self.frame_queue
    }

    pub fn config(&self) -> &DoubleBufferConfig {
        &self.config
    }
}

impl DoubleBufferPreview {
    /// Create a new frame with interpolation support.
    pub fn create_interpolated_frame(
        &self,
        base_frame: &PreviewFrame,
        _target_ms: u64,
        _source_fps: f64,
    ) -> Option<PreviewFrame> {
        if !self.config.enable_interpolation {
            return None;
        }

        let queue = self.frame_queue.lock().unwrap();
        let base_seq = base_frame.sequence;
        let _target_ms = base_frame.timestamp_ms;

        // Find frames around target
        if let Some((prev, next)) = queue.get_around_sequence(base_seq) {
            let factor = self.config.interpolation_factor;
            return Some(self.interpolate(prev, next, factor));
        }

        None
    }

    /// Get frame from queue by sequence.
    pub fn get_frame(&self, sequence: u64) -> Option<PreviewFrame> {
        self.frame_queue
            .lock()
            .unwrap()
            .find_by_sequence(sequence)
            .cloned()
    }

    /// Clear all buffers and queues.
    pub fn clear(&self) {
        self.frame_pair.lock().unwrap().front = None;
        self.frame_pair.lock().unwrap().back = None;
        self.frame_queue.lock().unwrap().clear();
    }

    /// Get queue depth.
    pub fn queue_depth(&self) -> usize {
        self.frame_queue.lock().unwrap().len()
    }

    /// Check if queue is ready for playback.
    pub fn is_ready(&self) -> bool {
        self.frame_queue.lock().unwrap().len() >= self.config.av_delay_frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_double_buffer_swap() {
        let config = DoubleBufferConfig::default();
        let preview = DoubleBufferPreview::new(config);

        let frame1 = PreviewFrame {
            rgba: vec![255; 64 * 4],
            width: 64,
            height: 64,
            timestamp_ms: 0,
            clip_id: Uuid::nil(),
            sequence: 1,
        };

        preview.submit_frame(frame1.clone());
        let front = preview.commit_and_swap();
        assert!(front.is_some());
        assert_eq!(front.unwrap().sequence, 1);
    }

    #[test]
    fn test_frame_queue() {
        let config = DoubleBufferConfig::default();
        let preview = DoubleBufferPreview::new(config);

        let frame1 = PreviewFrame {
            rgba: vec![255; 64],
            width: 8,
            height: 8,
            timestamp_ms: 0,
            clip_id: Uuid::nil(),
            sequence: 1,
        };

        preview.enqueue(frame1);
        assert_eq!(preview.queue_depth(), 1);

        let frame = preview.dequeue();
        assert!(frame.is_some());
        assert_eq!(preview.queue_depth(), 0);
    }

    /// Reproduces the exact forward-playback / backwards-seek sequence
    /// that used to publish the previous frame while recording the new key.
    #[test]
    fn backwards_seek_never_commits_a_stale_frame() {
        let preview = DoubleBufferPreview::new(DoubleBufferConfig::default());
        let frame = |seq: u64| PreviewFrame {
            rgba: vec![seq as u8; 4],
            width: 1,
            height: 1,
            timestamp_ms: seq * 100,
            clip_id: Uuid::nil(),
            sequence: seq,
        };

        // Forward playback. After the second swap the old front (10) is
        // left behind in the back buffer.
        match preview.submit_and_commit(frame(10)) {
            SubmitOutcome::Committed(f) => assert_eq!(f.sequence, 10),
            SubmitOutcome::Stale => panic!("first frame was rejected"),
        }
        match preview.submit_and_commit(frame(20)) {
            SubmitOutcome::Committed(f) => assert_eq!(f.sequence, 20),
            SubmitOutcome::Stale => panic!("second frame was rejected"),
        }

        // Scrub back to 5. The back buffer still holds 10, so submit_frame
        // rejects it. The old code swapped that 10 to the front anyway.
        assert!(matches!(
            preview.submit_and_commit(frame(5)),
            SubmitOutcome::Stale
        ));
        {
            let pair = preview.frame_pair().lock().unwrap();
            assert!(pair.front.is_none(), "stale front buffer survived");
            assert!(pair.back.is_none(), "stale back buffer survived");
        }

        // And playback recovers on the very next frame.
        match preview.submit_and_commit(frame(6)) {
            SubmitOutcome::Committed(f) => assert_eq!(f.sequence, 6),
            SubmitOutcome::Stale => panic!("playback did not recover after a seek"),
        }
    }

    /// `>=` in submit_frame means an equal sequence is stale too. That
    /// case must not be committed either.
    #[test]
    fn equal_sequence_is_rejected_not_swapped() {
        let preview = DoubleBufferPreview::new(DoubleBufferConfig::default());
        let frame = |seq: u64| PreviewFrame {
            rgba: vec![seq as u8; 4],
            width: 1,
            height: 1,
            timestamp_ms: seq * 100,
            clip_id: Uuid::nil(),
            sequence: seq,
        };

        preview.submit_and_commit(frame(7));
        preview.submit_and_commit(frame(8)); // front=8, back=7
        assert!(matches!(
            preview.submit_and_commit(frame(7)),
            SubmitOutcome::Stale
        ));
    }

    #[test]
    fn test_frame_interpolation() {
        let config = DoubleBufferConfig {
            enable_interpolation: true,
            interpolation_factor: 0.5,
            ..Default::default()
        };
        let preview = DoubleBufferPreview::new(config);

        let frame_a = PreviewFrame {
            rgba: (0..16).flat_map(|_| [0u8, 0, 0, 255]).collect(), // Black
            width: 4,
            height: 4,
            timestamp_ms: 0,
            clip_id: Uuid::nil(),
            sequence: 1,
        };

        let frame_b = PreviewFrame {
            rgba: (0..16).flat_map(|_| [255u8, 255, 255, 255]).collect(), // White
            width: 4,
            height: 4,
            timestamp_ms: 100,
            clip_id: Uuid::nil(),
            sequence: 2,
        };

        let interp = preview.interpolate(&frame_a, &frame_b, 0.5);

        // Middle should be gray (127, 127, 127, 255)
        assert_eq!(interp.rgba[0], 127);
        assert_eq!(interp.rgba[1], 127);
        assert_eq!(interp.rgba[2], 127);
        assert_eq!(interp.rgba[3], 255);
    }
}
