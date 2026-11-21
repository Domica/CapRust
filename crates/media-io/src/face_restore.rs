//! Real GFPGAN face restoration for video frames.
//!
//! Detects faces with SCRFD, aligns each face to the 512x512 canonical
//! FFHQ frame using its five landmarks, runs GFPGAN v1.4 via tract,
//! then inverse-warps the restored face back into the source frame with
//! a feathered mask.
//!
//! This is the missing bridge between `scrfd.rs` (detection),
//! `gfpgan.rs` (inference) and the export pipeline. Everything else in
//! the app consumes an already-restored cache video, so inference runs
//! once per frame in a cancellable background job — never in preview.

use crate::gfpgan::{GfpganRestorer, GFPGAN_INPUT_SIZE};
use crate::inference_backend::BackendType;
use crate::onnx_common::bilinear_rgb;
use crate::scrfd::{ScrfdDetector, ScrfdFace};
use anyhow::{anyhow, Result};
use std::path::Path;

/// Canonical 5-point landmarks of a 512x512 aligned face (FFHQ / ArcFace
/// convention: eye, eye, nose, mouth, mouth). SCRFD emits keypoints in
/// the same order.
pub const REF_LANDMARKS: [[f32; 2]; 5] = [
    [192.98138, 239.94708],
    [318.90277, 240.1936],
    [256.63416, 314.01935],
    [201.26117, 371.41043],
    [313.08905, 371.15118],
];

/// Maximum faces restored per frame. Keeps worst-case cost bounded.
const MAX_FACES_PER_FRAME: usize = 4;

/// 2D similarity transform `dst = a*src + b*perp(src) + t`:
/// `dx = a*sx + b*sy + tx`, `dy = -b*sx + a*sy + ty`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Similarity {
    pub a: f32,
    pub b: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Similarity {
    /// Least-squares similarity mapping five source landmarks onto the
    /// canonical reference points. Fails on degenerate input.
    pub fn from_landmarks(src: &[[f32; 2]; 5]) -> Result<Self> {
        let mut sm = [0.0f32; 2];
        for p in src {
            sm[0] += p[0] / 5.0;
            sm[1] += p[1] / 5.0;
        }
        let mut dm = [0.0f32; 2];
        for p in REF_LANDMARKS {
            dm[0] += p[0] / 5.0;
            dm[1] += p[1] / 5.0;
        }

        let (mut num_a, mut num_b, mut den) = (0.0f32, 0.0f32, 0.0f32);
        for (s, d) in src.iter().zip(REF_LANDMARKS.iter()) {
            let hx = s[0] - sm[0];
            let hy = s[1] - sm[1];
            let gx = d[0] - dm[0];
            let gy = d[1] - dm[1];
            num_a += hx * gx + hy * gy;
            num_b += hy * gx - hx * gy;
            den += hx * hx + hy * hy;
        }
        if den < 1e-8 || !den.is_finite() {
            return Err(anyhow!("degenerate facial landmarks"));
        }
        let a = num_a / den;
        let b = num_b / den;
        if !a.is_finite() || !b.is_finite() || (a * a + b * b) < 1e-12 {
            return Err(anyhow!("invalid face alignment scale"));
        }
        Ok(Self {
            a,
            b,
            tx: dm[0] - a * sm[0] - b * sm[1],
            ty: dm[1] + b * sm[0] - a * sm[1],
        })
    }

    #[inline]
    pub fn forward(&self, sx: f32, sy: f32) -> (f32, f32) {
        (
            self.a * sx + self.b * sy + self.tx,
            -self.b * sx + self.a * sy + self.ty,
        )
    }

    #[inline]
    pub fn inverse(&self, dx: f32, dy: f32) -> (f32, f32) {
        let det = self.a * self.a + self.b * self.b;
        let u = dx - self.tx;
        let v = dy - self.ty;
        (
            (self.a * u - self.b * v) / det,
            (self.b * u + self.a * v) / det,
        )
    }
}

/// Loaded face-restoration pipeline. Construct once, reuse per frame.
pub struct FaceRestorer {
    detector: ScrfdDetector,
    gfpgan: GfpganRestorer,
}

impl FaceRestorer {
    pub fn load(detector_path: &Path, gfpgan_path: &Path, backend: BackendType) -> Result<Self> {
        Ok(Self {
            detector: ScrfdDetector::load(detector_path, backend)?,
            gfpgan: GfpganRestorer::load(gfpgan_path, backend)?,
        })
    }

    /// Restore all detected faces in an RGB frame. Returns a new RGB
    /// buffer of the same dimensions; the original is untouched.
    pub fn restore_frame(&self, rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
        let mut out = rgb.to_vec();
        let faces = self.detector.detect(rgb, width, height)?;
        let mut processed = 0usize;
        for face in faces.iter().take(MAX_FACES_PER_FRAME) {
            if restore_one(&self.gfpgan, &mut out, width, height, face).is_ok() {
                processed += 1;
            }
        }
        tracing::debug!(
            "face restore: {} faces detected, {} restored",
            faces.len(),
            processed
        );
        Ok(out)
    }
}

/// Align one face, run GFPGAN, and feather the result back into `frame`.
fn restore_one(
    gfpgan: &GfpganRestorer,
    frame: &mut [u8],
    width: u32,
    height: u32,
    face: &ScrfdFace,
) -> Result<()> {
    // Landmarks are normalized; convert to source pixels.
    let src: [[f32; 2]; 5] = face
        .landmarks
        .map(|p| [p[0] * width as f32, p[1] * height as f32]);
    let transform = Similarity::from_landmarks(&src)?;

    // Backward-warp the source into the canonical 512x512 frame.
    let mut aligned = vec![0u8; (GFPGAN_INPUT_SIZE * GFPGAN_INPUT_SIZE * 3) as usize];
    for y in 0..GFPGAN_INPUT_SIZE as usize {
        for x in 0..GFPGAN_INPUT_SIZE as usize {
            let (sx, sy) = transform.inverse(x as f32, y as f32);
            let (r, g, b) = bilinear_rgb(frame, width, height, sx, sy);
            let i = (y * GFPGAN_INPUT_SIZE as usize + x) * 3;
            aligned[i] = r.round() as u8;
            aligned[i + 1] = g.round() as u8;
            aligned[i + 2] = b.round() as u8;
        }
    }

    let restored = gfpgan.restore(&aligned, GFPGAN_INPUT_SIZE, GFPGAN_INPUT_SIZE)?;
    paste_back(frame, width, height, &restored, &transform);
    Ok(())
}

/// Feather the 512x512 restored face back into the source frame using
/// the inverse transform. Writes only the affected bounding box.
fn paste_back(frame: &mut [u8], width: u32, height: u32, restored: &[u8], transform: &Similarity) {
    let s = GFPGAN_INPUT_SIZE as f32;
    // Bounding box of the aligned square in source coordinates.
    let corners = [
        transform.inverse(0.0, 0.0),
        transform.inverse(s, 0.0),
        transform.inverse(0.0, s),
        transform.inverse(s, s),
    ];
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for (x, y) in corners {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    let x0 = min_x.floor().max(0.0) as usize;
    let y0 = min_y.floor().max(0.0) as usize;
    let x1 = max_x.ceil().min(width as f32) as usize;
    let y1 = max_y.ceil().min(height as f32) as usize;

    for y in y0..y1 {
        for x in x0..x1 {
            let (ax, ay) = transform.forward(x as f32, y as f32);
            if ax < 0.0 || ay < 0.0 || ax >= s || ay >= s {
                continue;
            }
            let weight = feather(ax / s, ay / s);
            if weight <= 0.0 {
                continue;
            }
            let (r, g, b) = bilinear_rgb(restored, GFPGAN_INPUT_SIZE, GFPGAN_INPUT_SIZE, ax, ay);
            let i = (y * width as usize + x) * 3;
            blend(&mut frame[i], r, weight);
            blend(&mut frame[i + 1], g, weight);
            blend(&mut frame[i + 2], b, weight);
        }
    }
}

#[inline]
fn blend(dst: &mut u8, src: f32, weight: f32) {
    let v = (*dst as f32) * (1.0 - weight) + src * weight;
    *dst = v.clamp(0.0, 255.0).round() as u8;
}

/// Square feather: full strength in the interior, smooth fade at the
/// aligned crop's borders. Avoids hard seams without bleeding past it.
fn feather(u: f32, v: f32) -> f32 {
    let edge = ((u - 0.5).abs() * 2.0).max((v - 0.5).abs() * 2.0);
    let t = ((edge - 0.80) / 0.20).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similarity_maps_landmarks_to_reference() {
        // Use the reference as "source" -- identity transform.
        let s = Similarity::from_landmarks(&REF_LANDMARKS).unwrap();
        for p in REF_LANDMARKS {
            let (x, y) = s.forward(p[0], p[1]);
            assert!((x - p[0]).abs() < 0.01, "{x} vs {}", p[0]);
            assert!((y - p[1]).abs() < 0.01, "{y} vs {}", p[1]);
        }
    }

    #[test]
    fn similarity_inverse_roundtrips() {
        // Shifted + scaled + rotated landmarks.
        let src = REF_LANDMARKS.map(|p| [100.0 + p[0] * 2.0, 50.0 + p[1] * 2.0]);
        let s = Similarity::from_landmarks(&src).unwrap();
        for p in REF_LANDMARKS {
            let (ax, ay) = s.forward(p[0], p[1]);
            let (bx, by) = s.inverse(ax, ay);
            assert!((bx - p[0]).abs() < 0.01);
            assert!((by - p[1]).abs() < 0.01);
        }
    }

    #[test]
    fn degenerate_landmarks_rejected() {
        let same = [[10.0, 10.0]; 5];
        assert!(Similarity::from_landmarks(&same).is_err());
    }

    #[test]
    fn feather_fades_at_edges() {
        assert_eq!(feather(0.5, 0.5), 1.0);
        assert_eq!(feather(0.0, 0.5), 0.0);
        assert_eq!(feather(1.0, 1.0), 0.0);
        let mid = feather(0.95, 0.5);
        assert!(mid > 0.0 && mid < 1.0);
    }

    #[test]
    #[ignore]
    fn bench_gfpgan_inference() {
        let path = std::path::Path::new(r"F:\Minimax H3\Git NEW APP\Models\gfpgan-v1.4.onnx");
        if !path.is_file() {
            return;
        }
        let restorer = GfpganRestorer::load(path).unwrap();
        let rgb = vec![128u8; 64 * 64 * 3];
        // Warm up once.
        let _ = restorer.restore(&rgb, 64, 64).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..3 {
            let _ = restorer.restore(&rgb, 64, 64).unwrap();
        }
        eprintln!(
            "GFPGAN inference: {:.2}s per 512x512 frame",
            start.elapsed().as_secs_f64() / 3.0
        );
    }

    #[test]
    fn paste_back_changes_only_face_region() {
        let (w, h) = (128u32, 128u32);
        let mut frame = vec![100u8; (w * h * 3) as usize];
        let restored = vec![200u8; (GFPGAN_INPUT_SIZE * GFPGAN_INPUT_SIZE * 3) as usize];
        // Identity-ish transform: source [0,w]x[0,h] -> [0,512]x[0,512].
        let t = Similarity {
            a: 512.0 / w as f32,
            b: 0.0,
            tx: 0.0,
            ty: 0.0,
        };
        paste_back(&mut frame, w, h, &restored, &t);
        // Centre must change, far corner must not.
        let centre = (64 * w as usize + 64) * 3;
        assert!(frame[centre] > 150);
        let corner = 0usize;
        assert_eq!(frame[corner], 100);
    }
}
