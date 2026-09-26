//! SCRFD-500M face detection (Phase P2c alt).
//!
//! InsightFace's deployed detector. The graph is verified tract-onnx
//! compatible (Codeberg `face-detection-api`, plakat's scrfd port).
//!
//! Input:  [1, 3, 640, 640] f32, RGB, normalized (x - 127.5) / 128.0.
//!         Top-left letterbox pad: resize preserving aspect, then
//!         zero-pad bottom-right to 640x640 (no centering).
//! Output: 9 tensors, 3 strides x 3 heads:
//!   cls_s8 (12800, 1), reg_s8 (12800, 4), kps_s8 (12800, 10)
//!   cls_s16 (3200, 1), reg_s16 (3200, 4), kps_s16 (3200, 10)
//!   cls_s32 (800, 1), reg_s32 (800, 4), kps_s32 (800, 10)
//!
//! Anchors: 2 per grid location at every stride. Anchor center for
//! grid cell (gx, gy) at stride s is ((gx + 0.5) * s, (gy + 0.5) * s).
//!
//! Decoding (distance format):
//!   x1 = cx - dx * s
//!   y1 = cy - dy * s
//!   x2 = cx + dw * s
//!   y2 = cy + dh * s
//!   where cx, cy are anchor centers and dx/dy/dw/dh the four reg
//!   values at that anchor.
//!
//! Confidence: raw cls output (the model applies sigmoid internally).

use anyhow::{anyhow, Result};
use std::path::Path;
use tract_onnx::prelude::*;

pub const INPUT_SIZE: u32 = 640;
pub const STRIDES: [u32; 3] = [8, 16, 32];
pub const ANCHORS_PER_LOCATION: usize = 2;
pub const CONFIDENCE_THRESHOLD: f32 = 0.5;
pub const NMS_IOU_THRESHOLD: f32 = 0.4;

const NORM_MEAN: f32 = 127.5;
const NORM_SCALE: f32 = 128.0;

/// One detected face, normalized [0, 1].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrfdFace {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub confidence: f32,
}

impl ScrfdFace {
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
}

type RunnableModel =
    std::sync::Arc<tract_onnx::prelude::RunnableModel<TypedFact, Box<dyn TypedOp>>>;

pub struct ScrfdDetector {
    model: RunnableModel,
}

impl ScrfdDetector {
    pub fn load(model_path: &Path) -> Result<Self> {
        if !model_path.is_file() {
            return Err(anyhow!("SCRFD model not found: {}", model_path.display()));
        }
        let model = tract_onnx::onnx()
            .model_for_path(model_path)
            .map_err(|e| anyhow!("load SCRFD ONNX: {e:?}"))?
            .with_input_fact(
                0,
                InferenceFact::dt_shape(
                    f32::datum_type(),
                    tvec!(1, 3, INPUT_SIZE as i64, INPUT_SIZE as i64),
                ),
            )
            .map_err(|e| anyhow!("pin SCRFD input shape: {e:?}"))?
            .into_optimized()
            .map_err(|e| anyhow!("optimize SCRFD: {e:?}"))?
            .into_runnable()
            .map_err(|e| anyhow!("build SCRFD plan: {e:?}"))?;
        tracing::info!("scrfd: loaded {}", model_path.display());
        Ok(Self { model })
    }

    /// `rgb` is a tightly packed RGB buffer (`w * h * 3`). Returns
    /// faces in descending confidence, normalized to [0, 1].
    pub fn detect(&self, rgb: &[u8], width: u32, height: u32) -> Result<Vec<ScrfdFace>> {
        let expected = (width as usize) * (height as usize) * 3;
        if rgb.len() != expected {
            return Err(anyhow!(
                "rgb buffer length {} != expected {}",
                rgb.len(),
                expected
            ));
        }
        if width == 0 || height == 0 {
            return Ok(Vec::new());
        }

        let tensor = self.preprocess(rgb, width, height);
        let input: Tensor = tensor.into();
        let outputs = self
            .model
            .run(tvec!(input.into()))
            .map_err(|e| anyhow!("SCRFD inference: {e:?}"))?;

        if outputs.len() != 9 {
            return Err(anyhow!("SCRFD expected 9 outputs, got {}", outputs.len()));
        }

        let (scale, pad_x, pad_y) = letterbox_params(width, height);

        // Output order (per InsightsFace export): cls_s8, cls_s16,
        // cls_s32, reg_s8, reg_s16, reg_s32, kps_s8, kps_s16, kps_s32.
        // We consume cls (0..3) and reg (3..6). kps is skipped.
        let mut raw: Vec<ScrfdFace> = Vec::new();
        for (i, stride) in STRIDES.iter().copied().enumerate() {
            let cls = outputs[i].to_plain_array_view::<f32>()?;
            let reg = outputs[i + 3].to_plain_array_view::<f32>()?;
            decode_stride(
                stride, &cls, &reg, scale, pad_x, pad_y, width, height, &mut raw,
            )?;
        }

        Ok(nms(raw, NMS_IOU_THRESHOLD))
    }

    pub fn detect_best(&self, rgb: &[u8], width: u32, height: u32) -> Result<Option<ScrfdFace>> {
        Ok(self.detect(rgb, width, height)?.into_iter().next())
    }

    fn preprocess(&self, rgb: &[u8], width: u32, height: u32) -> tract_ndarray::Array4<f32> {
        let (scale, pad_x, pad_y) = letterbox_params(width, height);
        let new_w = (width as f32 * scale).round() as u32;
        let new_h = (height as f32 * scale).round() as u32;

        let mut arr =
            tract_ndarray::Array4::<f32>::zeros((1, 3, INPUT_SIZE as usize, INPUT_SIZE as usize));

        for dy in 0..new_h {
            for dx in 0..new_w {
                let sx = (dx as f32 + 0.5) / scale - 0.5;
                let sy = (dy as f32 + 0.5) / scale - 0.5;
                let (r, g, b) = bilinear_rgb(rgb, width, height, sx, sy);
                let ox = (dx + pad_x) as usize;
                let oy = (dy + pad_y) as usize;
                // RGB channel order + SCRFD normalization.
                arr[[0, 0, oy, ox]] = (r - NORM_MEAN) / NORM_SCALE;
                arr[[0, 1, oy, ox]] = (g - NORM_MEAN) / NORM_SCALE;
                arr[[0, 2, oy, ox]] = (b - NORM_MEAN) / NORM_SCALE;
            }
        }
        arr
    }
}

fn letterbox_params(width: u32, height: u32) -> (f32, u32, u32) {
    let scale = (INPUT_SIZE as f32 / width as f32).min(INPUT_SIZE as f32 / height as f32);
    // Top-left pad (no centering): SCRFD demos use cv2.copyMakeBorder
    // with top-left origin, so pad is always 0.
    (scale, 0, 0)
}

#[allow(clippy::too_many_arguments)]
fn decode_stride(
    stride: u32,
    cls: &tract_ndarray::ArrayViewD<f32>,
    reg: &tract_ndarray::ArrayViewD<f32>,
    scale: f32,
    pad_x: u32,
    pad_y: u32,
    img_w: u32,
    img_h: u32,
    out: &mut Vec<ScrfdFace>,
) -> Result<()> {
    let grid = INPUT_SIZE / stride;
    let n_cells = (grid * grid) as usize;
    let n_anchors = n_cells * ANCHORS_PER_LOCATION;

    for i in 0..n_anchors {
        let conf = cls[[i, 0]];
        if conf < CONFIDENCE_THRESHOLD {
            continue;
        }

        let cell = i / ANCHORS_PER_LOCATION;
        let gx = (cell as u32) % grid;
        let gy = (cell as u32) / grid;
        let cx = (gx as f32 + 0.5) * stride as f32;
        let cy = (gy as f32 + 0.5) * stride as f32;

        let dx = reg[[i, 0]] * stride as f32;
        let dy = reg[[i, 1]] * stride as f32;
        let dw = reg[[i, 2]] * stride as f32;
        let dh = reg[[i, 3]] * stride as f32;

        let x1 = cx - dx;
        let y1 = cy - dy;
        let x2 = cx + dw;
        let y2 = cy + dh;

        // Un-letterbox to source resolution, then normalize.
        let x1_orig = (x1 - pad_x as f32) / scale;
        let y1_orig = (y1 - pad_y as f32) / scale;
        let x2_orig = (x2 - pad_x as f32) / scale;
        let y2_orig = (y2 - pad_y as f32) / scale;

        let xn = x1_orig / img_w as f32;
        let yn = y1_orig / img_h as f32;
        let wn = (x2_orig - x1_orig) / img_w as f32;
        let hn = (y2_orig - y1_orig) / img_h as f32;

        out.push(ScrfdFace {
            x: xn,
            y: yn,
            w: wn,
            h: hn,
            confidence: conf,
        });
    }
    let _ = n_cells; // silence unused-var when ANCHORS_PER_LOCATION == 1
    Ok(())
}

fn bilinear_rgb(rgb: &[u8], width: u32, height: u32, sx: f32, sy: f32) -> (f32, f32, f32) {
    let w = width as i32;
    let h = height as i32;
    let x0 = sx.floor() as i32;
    let y0 = sy.floor() as i32;
    let fx = sx - x0 as f32;
    let fy = sy - y0 as f32;
    let clamp = |v: i32, hi: i32| v.clamp(0, hi - 1);
    let at = |x: i32, y: i32| -> (f32, f32, f32) {
        let xc = clamp(x, w);
        let yc = clamp(y, h);
        let i = ((yc as usize) * (width as usize) + (xc as usize)) * 3;
        (rgb[i] as f32, rgb[i + 1] as f32, rgb[i + 2] as f32)
    };
    let (r00, g00, b00) = at(x0, y0);
    let (r10, g10, b10) = at(x0 + 1, y0);
    let (r01, g01, b01) = at(x0, y0 + 1);
    let (r11, g11, b11) = at(x0 + 1, y0 + 1);
    let lerp = |a: f32, b: f32, c: f32, d: f32| -> f32 {
        let top = a * (1.0 - fx) + b * fx;
        let bot = c * (1.0 - fx) + d * fx;
        top * (1.0 - fy) + bot * fy
    };
    (
        lerp(r00, r10, r01, r11),
        lerp(g00, g10, g01, g11),
        lerp(b00, b10, b01, b11),
    )
}

fn nms(mut faces: Vec<ScrfdFace>, iou_threshold: f32) -> Vec<ScrfdFace> {
    faces.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut keep: Vec<ScrfdFace> = Vec::new();
    let mut suppressed = vec![false; faces.len()];
    for i in 0..faces.len() {
        if suppressed[i] {
            continue;
        }
        keep.push(faces[i]);
        for j in (i + 1)..faces.len() {
            if !suppressed[j] && iou(&faces[i], &faces[j]) > iou_threshold {
                suppressed[j] = true;
            }
        }
    }
    keep
}

fn iou(a: &ScrfdFace, b: &ScrfdFace) -> f32 {
    let x1 = a.x.max(b.x);
    let y1 = a.y.max(b.y);
    let x2 = (a.x + a.w).min(b.x + b.w);
    let y2 = (a.y + a.h).min(b.y + b.h);
    let iw = (x2 - x1).max(0.0);
    let ih = (y2 - y1).max(0.0);
    let inter = iw * ih;
    let union = a.w * a.h + b.w * b.h - inter;
    if union <= 0.0 {
        0.0
    } else {
        inter / union
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letterbox_square_image_no_padding() {
        let (scale, px, py) = letterbox_params(640, 640);
        assert!((scale - 1.0).abs() < 1e-6);
        assert_eq!((px, py), (0, 0));
    }

    #[test]
    fn letterbox_landscape_top_left_pad() {
        let (scale, px, py) = letterbox_params(1280, 640);
        assert!((scale - 0.5).abs() < 1e-6);
        assert_eq!((px, py), (0, 0));
    }

    #[test]
    fn nms_drops_overlapping_keeps_distinct() {
        let a = ScrfdFace {
            x: 0.0,
            y: 0.0,
            w: 0.2,
            h: 0.2,
            confidence: 0.9,
        };
        let b = ScrfdFace {
            x: 0.01,
            y: 0.01,
            w: 0.2,
            h: 0.2,
            confidence: 0.8,
        };
        let c = ScrfdFace {
            x: 0.7,
            y: 0.7,
            w: 0.2,
            h: 0.2,
            confidence: 0.7,
        };
        let out = nms(vec![a, b, c], 0.4);
        assert_eq!(out.len(), 2);
    }

    #[test]
    #[ignore]
    fn scrfd_black_image_returns_no_faces() {
        let path = match std::env::var("CAPRUST_SCRFD_PATH") {
            Ok(p) => std::path::PathBuf::from(p),
            Err(_) => return,
        };
        let det = ScrfdDetector::load(&path).expect("load SCRFD");
        let black = vec![0u8; 640 * 640 * 3];
        let faces = det.detect(&black, 640, 640).expect("inference");
        assert!(faces.is_empty(), "black frame must not contain faces");
    }
}
