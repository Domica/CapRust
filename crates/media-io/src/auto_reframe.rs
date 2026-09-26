//! Auto-reframe keypoint computation (Phase P2c-2b).
//!
//! Given a sequence of frames sampled from a clip and a loaded
//! FaceDetector, produce a small list of ReframeKeypoint values that
//! the render graph turns into a piecewise-linear crop pan.
//!
//! Split into two layers so the algorithmic core is testable without
//! a real ONNX model:
//!   * detect_face_centers -- the only layer that touches the model
//!   * compute_keypoints_from_centers -- pure math over detected
//!     centers (gap filling, crop-travel normalization, smoothing)

use crate::face_detect::FaceDetector;
use crate::frame_extract::RgbFrame;
use anyhow::Result;
use caprust_core::clip::ReframeKeypoint;

/// Time window (in frames) over which the pan path is smoothed. A
/// wider window damps head-bob jitter but can lag on fast pans; 5 is
/// about 0.5 s at the ~10 fps sampling rate we use in practice.
pub const SMOOTHING_WINDOW: usize = 5;

/// One detection sample: frame timestamp (ms, relative to clip start)
/// plus an optional face center in normalized [0, 1] frame coordinates.
/// `None` means the frame produced no detection above the threshold.
///
/// Named so the type does not trip clippy::type_complexity in the
/// signatures that use it.
pub type FaceSample = (u64, Option<(f32, f32)>);

/// Run the detector on every frame, in order. Returns one
/// [`FaceSample`] per frame.
pub fn detect_face_centers(
    frames: &[RgbFrame],
    detector: &FaceDetector,
) -> Result<Vec<FaceSample>> {
    let mut out = Vec::with_capacity(frames.len());
    for f in frames {
        let best = detector.detect_best(&f.rgb, f.width, f.height)?;
        out.push((f.t_ms, best.map(|b| b.center())));
    }
    Ok(out)
}

/// Pure algorithm: from a sequence of `(t_ms, face_center)` samples,
/// produce the keypoint list that drives the crop pan.
///
/// `frame_aspect` = source width / source height.
/// `target_aspect` = target width / target height (e.g. 9.0/16.0).
///
/// Returns an empty Vec when fewer than two samples exist or no frame
/// had a detection -- there is nothing to follow, and the render side
/// treats empty as "no reframe" (source used as-is).
pub fn compute_keypoints_from_centers(
    samples: &[FaceSample],
    frame_aspect: f64,
    target_aspect: f64,
) -> Vec<ReframeKeypoint> {
    if samples.len() < 2 {
        return Vec::new();
    }

    // Forward-fill gaps between the first and last detection. Frames
    // before the first detection or after the last are dropped: we do
    // not want to pan towards a face that has not appeared yet.
    let Some(filled) = fill_gaps(samples) else {
        return Vec::new();
    };

    let (crop_w_frac, crop_h_frac) = crop_fractions(frame_aspect, target_aspect);
    let mut points: Vec<ReframeKeypoint> = Vec::with_capacity(filled.len());
    for (t_ms, (cx, cy)) in &filled {
        points.push(ReframeKeypoint {
            t_ms: *t_ms,
            cx_norm: travel_norm(*cx, crop_w_frac),
            cy_norm: travel_norm(*cy, crop_h_frac),
        });
    }

    smooth(&points, SMOOTHING_WINDOW)
}

/// Normalize a face center on one axis into the [0, 1] "crop travel"
/// domain. When the crop covers the whole axis (`crop_frac >= 1.0`),
/// there is no travel; return 0.5 (centered) so downstream consumers
/// always see a well-formed keypoint.
///
/// Math: the crop's top-left corner ranges over `[0, 1 - f]`, so its
/// center ranges over `[f/2, 1 - f/2]`. Normalize the face center
/// linearly into that range.
fn travel_norm(face_center: f32, crop_frac: f32) -> f32 {
    let f = crop_frac.clamp(0.0, 1.0);
    if f >= 0.999 {
        return 0.5;
    }
    let half = f * 0.5;
    let denom = 1.0 - f;
    ((face_center - half) / denom).clamp(0.0, 1.0)
}

/// Compute the crop rectangle as a fraction of the frame on each
/// axis. Exactly one axis gets the full 1.0 (nothing trimmed); the
/// other is the trimmed axis.
fn crop_fractions(frame_aspect: f64, target_aspect: f64) -> (f32, f32) {
    if frame_aspect <= 0.0 || target_aspect <= 0.0 {
        return (1.0, 1.0);
    }
    if target_aspect <= frame_aspect {
        // Target is narrower than the source: trim width.
        let w = (target_aspect / frame_aspect) as f32;
        (w.clamp(0.0, 1.0), 1.0)
    } else {
        // Target is wider than the source: trim height.
        let h = (frame_aspect / target_aspect) as f32;
        (1.0, h.clamp(0.0, 1.0))
    }
}

/// Forward-fill `None` samples between the first and last `Some`.
/// Frames outside that span are dropped (leading/trailing silence).
/// Returns `None` when no sample carried a detection.
fn fill_gaps(samples: &[FaceSample]) -> Option<Vec<(u64, (f32, f32))>> {
    let first = samples.iter().position(|(_, c)| c.is_some())?;
    let last = samples.iter().rposition(|(_, c)| c.is_some())?;
    let mut out: Vec<(u64, (f32, f32))> = Vec::with_capacity(last - first + 1);
    let mut cur = samples[first].1.unwrap();
    for (t, c) in &samples[first..=last] {
        if let Some(cc) = c {
            cur = *cc;
        }
        out.push((*t, cur));
    }
    Some(out)
}

/// Centered moving average on cx_norm / cy_norm. Timestamps are
/// preserved from the input. Window is clamped to at least 1.
fn smooth(points: &[ReframeKeypoint], window: usize) -> Vec<ReframeKeypoint> {
    if window <= 1 || points.len() < 2 {
        return points.to_vec();
    }
    let half = (window / 2) as isize;
    let mut out = Vec::with_capacity(points.len());
    for (i, p) in points.iter().enumerate() {
        let lo = (i as isize - half).max(0) as usize;
        let hi = ((i as isize + half) as usize).min(points.len() - 1);
        let n = (hi - lo + 1) as f32;
        let mut sx = 0.0f32;
        let mut sy = 0.0f32;
        for q in &points[lo..=hi] {
            sx += q.cx_norm;
            sy += q.cy_norm;
        }
        out.push(ReframeKeypoint {
            t_ms: p.t_ms,
            cx_norm: sx / n,
            cy_norm: sy / n,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kp(t_ms: u64, cx: f32, cy: f32) -> FaceSample {
        (t_ms, Some((cx, cy)))
    }
    fn gap(t_ms: u64) -> FaceSample {
        (t_ms, None)
    }

    #[test]
    fn travel_norm_centers_when_crop_covers_full_axis() {
        assert!((travel_norm(0.0, 1.0) - 0.5).abs() < 1e-6);
        assert!((travel_norm(0.7, 1.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn travel_norm_clamps_extremes() {
        // Crop covers 50% of the axis: travel range [0.25, 0.75] for
        // the crop center, so face at 0.0 clamps to 0 and face at 1.0
        // clamps to 1.
        assert!((travel_norm(0.0, 0.5) - 0.0).abs() < 1e-6);
        assert!((travel_norm(0.5, 0.5) - 0.5).abs() < 1e-6);
        assert!((travel_norm(1.0, 0.5) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn crop_fractions_narrow_target_trims_width() {
        // 16:9 source, 9:16 target -> trim width to 9/16 / (16/9) =
        // 0.3164...
        let (w, h) = crop_fractions(16.0 / 9.0, 9.0 / 16.0);
        assert!((w - (9.0 / 16.0) / (16.0 / 9.0) as f32).abs() < 1e-5);
        assert!((h - 1.0).abs() < 1e-6);
    }

    #[test]
    fn crop_fractions_wide_target_trims_height() {
        // 9:16 source, 16:9 target -> trim height.
        let (w, h) = crop_fractions(9.0 / 16.0, 16.0 / 9.0);
        assert!((w - 1.0).abs() < 1e-6);
        assert!(h < 1.0);
    }

    #[test]
    fn crop_fractions_same_aspect_no_trim() {
        let (w, h) = crop_fractions(16.0 / 9.0, 16.0 / 9.0);
        assert!((w - 1.0).abs() < 1e-6);
        assert!((h - 1.0).abs() < 1e-6);
    }

    #[test]
    fn compute_empty_when_no_faces() {
        let samples = vec![gap(0), gap(100), gap(200)];
        let out = compute_keypoints_from_centers(&samples, 16.0 / 9.0, 9.0 / 16.0);
        assert!(out.is_empty());
    }

    #[test]
    fn compute_empty_when_fewer_than_two_samples() {
        let samples = vec![kp(0, 0.5, 0.5)];
        let out = compute_keypoints_from_centers(&samples, 16.0 / 9.0, 9.0 / 16.0);
        assert!(out.is_empty());
    }

    #[test]
    fn compute_fills_gaps_between_detections() {
        let samples = vec![
            gap(0),
            kp(100, 0.3, 0.5),
            gap(200),
            gap(300),
            kp(400, 0.7, 0.5),
            gap(500),
        ];
        let out = compute_keypoints_from_centers(&samples, 16.0 / 9.0, 9.0 / 16.0);
        // Leading and trailing gaps dropped; middle kept and filled.
        // 4 samples between t=100 and t=400 survive.
        assert_eq!(out.len(), 4, "kept span = first..=last detection: {out:?}");
        // First and last timestamps preserved from the surviving span.
        assert_eq!(out[0].t_ms, 100);
        assert_eq!(out[3].t_ms, 400);
    }

    #[test]
    fn compute_smooths_adjacent_values() {
        // A step from left to right. With a 5-wide window, the interior
        // values blend; the sequence must remain monotone non-decreasing.
        let samples = vec![
            kp(0, 0.1, 0.5),
            kp(100, 0.2, 0.5),
            kp(200, 0.4, 0.5),
            kp(300, 0.6, 0.5),
            kp(400, 0.8, 0.5),
            kp(500, 0.9, 0.5),
        ];
        let out = compute_keypoints_from_centers(&samples, 16.0 / 9.0, 9.0 / 16.0);
        assert_eq!(out.len(), 6);
        for w in out.windows(2) {
            assert!(
                w[1].cx_norm >= w[0].cx_norm - 1e-5,
                "smoothed sequence must stay monotone: {:?} -> {:?}",
                w[0],
                w[1]
            );
        }
        // Interior sample gets pulled by neighbours, so it is not
        // exactly the raw value; at least it stays in range.
        for p in &out {
            assert!((0.0..=1.0).contains(&p.cx_norm));
            assert!((0.0..=1.0).contains(&p.cy_norm));
        }
    }

    #[test]
    fn compute_keeps_cy_constant_when_face_y_is_constant() {
        let samples = vec![kp(0, 0.3, 0.5), kp(100, 0.4, 0.5), kp(200, 0.5, 0.5)];
        let out = compute_keypoints_from_centers(&samples, 16.0 / 9.0, 9.0 / 16.0);
        // Target aspect is narrower, so only the horizontal axis moves;
        // vertical stays centered.
        for p in &out {
            assert!((p.cy_norm - 0.5).abs() < 1e-6, "cy drifted: {p:?}");
        }
    }
}
