//! YuNet face detection for auto-reframe (Phase P2b).
//!
//! Pure-Rust inference via tract-onnx. The model file is downloaded
//! on demand from the registry (`ModelKind::FaceDetector`) and lives
//! next to the Whisper and Piper models.
//!
//! Input:  [1, 3, 320, 320] f32, **BGR**, values 0..255 (no
//!         normalization -- YuNet is trained through OpenCV's
//!         cv2.imread pipeline, which yields BGR bytes in that range).
//!         We letterbox-resize while preserving aspect ratio and pad
//!         with zeros.
//! Output: 12 tensors, 4 per stride (8, 16, 32), in the ONNX output
//!         order: cls_8, cls_16, cls_32, obj_8, obj_16, obj_32,
//!         bbox_8, bbox_16, bbox_32, kps_8, kps_16, kps_32. We only
//!         consume cls, obj and bbox; kps is skipped.
//!
//! Boxes are returned normalized to [0, 1] relative to the input
//! frame, so downstream code can crop without knowing the source
//! resolution.

use anyhow::{anyhow, Context, Result};
use std::path::Path;
use tract_onnx::prelude::*;

/// Fixed input resolution of the YuNet 2023mar ONNX model.
pub const INPUT_SIZE: u32 = 320;

/// Minimum sqrt(cls * obj) for a detection to be kept. OpenCV's demo
/// uses 0.5; the YuNet paper uses 0.6 for higher precision. Auto-
/// reframe is picky -- one wrong crop ruins the shot -- so we go with
/// the stricter value.
pub const CONFIDENCE_THRESHOLD: f32 = 0.6;

/// IoU threshold for non-maximum suppression. 0.3 is YuNet's own
/// default and works well at 320x320 where adjacent anchors overlap
/// heavily.
pub const NMS_IOU_THRESHOLD: f32 = 0.3;

/// Strides used by YuNet 2023mar, in ONNX output order.
const STRIDES: [u32; 3] = [8, 16, 32];

/// One detected face, in normalized [0, 1] image coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceBox {
    /// Top-left corner x, normalized to frame width.
    pub x: f32,
    /// Top-left corner y, normalized to frame height.
    pub y: f32,
    /// Width, normalized to frame width.
    pub w: f32,
    /// Height, normalized to frame height.
    pub h: f32,
    /// Detection confidence in [0, 1] (sqrt(cls * obj)).
    pub confidence: f32,
}

impl FaceBox {
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
}

/// Loaded YuNet model. Construct once and reuse across frames --
/// `load` parses and optimizes the ONNX graph, which is slow relative
/// to a single inference pass.
pub struct FaceDetector {
    model: RunnableModel,
}

// tract 0.23 renamed the runnable plan type; the prelude alias
// `RunnableModel` is the stable name going forward.
// tract 0.23 wraps the runnable plan in an Arc; `run()` takes
// `self: &Arc<Self>` so the alias carries the Arc.
type RunnableModel =
    std::sync::Arc<tract_onnx::prelude::RunnableModel<TypedFact, Box<dyn TypedOp>>>;

impl FaceDetector {
    /// Load and optimize a YuNet ONNX model from disk.
    pub fn load(model_path: &Path) -> Result<Self> {
        if !model_path.is_file() {
            return Err(anyhow!("face model not found: {}", model_path.display()));
        }
        let model = tract_onnx::onnx()
            .model_for_path(model_path)
            .map_err(|e| {
                // Same reasoning as background_removal.rs: tract's
                // Display hides the operator name, the Debug form
                // carries it.
                anyhow!(
                    "load ONNX model {}: {e:?}\n  (if this is an operator tract-onnx 0.23 does not support, check DIRECTIVES section 30 for known model issues)",
                    model_path.display()
                )
            })?
            .into_optimized()
            .map_err(|e| anyhow!("optimize ONNX model: {e:?}"))?
            .into_runnable()
            .map_err(|e| anyhow!("build runnable plan: {e:?}"))?;
        tracing::info!("face_detect: loaded {}", model_path.display());
        Ok(Self { model })
    }

    /// Run detection on a tightly packed RGB buffer of length
    /// `width * height * 3`. Returns every face above
    /// [`CONFIDENCE_THRESHOLD`] after NMS, in descending confidence
    /// order.
    pub fn detect(&self, rgb: &[u8], width: u32, height: u32) -> Result<Vec<FaceBox>> {
        let expected = (width as usize) * (height as usize) * 3;
        if rgb.len() != expected {
            return Err(anyhow!(
                "rgb buffer length {} != expected {} ({}x{}x3)",
                rgb.len(),
                expected,
                width,
                height
            ));
        }
        if width == 0 || height == 0 {
            return Ok(Vec::new());
        }

        let tensor = self.preprocess(rgb, width, height);

        // tract 0.22 expects a TValue, not a Tensor. Two conversions:
        // Array4<f32> -> Tensor (From impl), then Tensor -> TValue
        // (another From impl). The compiler guides this -- `tvec!`
        // wants TValue specifically.
        let input: Tensor = tensor.into();
        let outputs = self
            .model
            .run(tvec!(input.into()))
            .context("YuNet inference")?;

        if outputs.len() != 12 {
            return Err(anyhow!(
                "YuNet expected 12 output tensors, got {}",
                outputs.len()
            ));
        }

        let (scale, pad_x, pad_y) = letterbox_params(width, height);

        // ONNX output order (see module docs). Indexing is explicit on
        // purpose -- a silent shape mismatch would be far worse than
        // the readability cost.
        let cls_8 = outputs[0].to_plain_array_view::<f32>()?;
        let cls_16 = outputs[1].to_plain_array_view::<f32>()?;
        let cls_32 = outputs[2].to_plain_array_view::<f32>()?;
        let obj_8 = outputs[3].to_plain_array_view::<f32>()?;
        let obj_16 = outputs[4].to_plain_array_view::<f32>()?;
        let obj_32 = outputs[5].to_plain_array_view::<f32>()?;
        let bbox_8 = outputs[6].to_plain_array_view::<f32>()?;
        let bbox_16 = outputs[7].to_plain_array_view::<f32>()?;
        let bbox_32 = outputs[8].to_plain_array_view::<f32>()?;

        let cls = [cls_8, cls_16, cls_32];
        let obj = [obj_8, obj_16, obj_32];
        let bbox = [bbox_8, bbox_16, bbox_32];

        let mut raw: Vec<FaceBox> = Vec::new();
        for (i, stride) in STRIDES.iter().copied().enumerate() {
            decode_stride(
                stride, &cls[i], &obj[i], &bbox[i], scale, pad_x, pad_y, width, height, &mut raw,
            );
        }

        Ok(nms(raw, NMS_IOU_THRESHOLD))
    }

    /// Convenience wrapper: highest-confidence face, or `None`.
    pub fn detect_best(&self, rgb: &[u8], width: u32, height: u32) -> Result<Option<FaceBox>> {
        Ok(self.detect(rgb, width, height)?.into_iter().next())
    }

    /// Letterbox RGB into a (1, 3, 320, 320) CHW tensor, BGR channel
    /// order, values in 0..255, no normalization.
    fn preprocess(&self, rgb: &[u8], width: u32, height: u32) -> tract_ndarray::Array4<f32> {
        let (scale, pad_x, pad_y) = letterbox_params(width, height);
        let new_w = (width as f32 * scale).round() as u32;
        let new_h = (height as f32 * scale).round() as u32;

        let mut arr =
            tract_ndarray::Array4::<f32>::zeros((1, 3, INPUT_SIZE as usize, INPUT_SIZE as usize));

        // Bilinear sample from the source into the letterboxed region.
        // tract doesn't ship a resampler and YuNet was trained on
        // cv2.resize (bilinear), so nearest-neighbor would degrade
        // accuracy on small faces. 40 lines of arithmetic is cheaper
        // than a pull on the image crate's resize machinery.
        for dy in 0..new_h {
            for dx in 0..new_w {
                let sx = (dx as f32 + 0.5) / scale - 0.5;
                let sy = (dy as f32 + 0.5) / scale - 0.5;
                let (r, g, b) = bilinear_rgb(rgb, width, height, sx, sy);
                let ox = (dx + pad_x) as usize;
                let oy = (dy + pad_y) as usize;
                // BGR: channel 0 = blue, 1 = green, 2 = red.
                arr[[0, 0, oy, ox]] = b;
                arr[[0, 1, oy, ox]] = g;
                arr[[0, 2, oy, ox]] = r;
            }
        }

        arr
    }
}

/// Compute the letterbox transform for fitting `width x height` into
/// `INPUT_SIZE x INPUT_SIZE` while preserving aspect ratio.
///
/// Returns `(scale, pad_x, pad_y)` where `scale` is the linear scale
/// applied to the source, and `(pad_x, pad_y)` is the zero-padding
/// applied on the left and top of the resized image.
fn letterbox_params(width: u32, height: u32) -> (f32, u32, u32) {
    let scale = (INPUT_SIZE as f32 / width as f32).min(INPUT_SIZE as f32 / height as f32);
    let new_w = (width as f32 * scale).round() as u32;
    let new_h = (height as f32 * scale).round() as u32;
    let pad_x = (INPUT_SIZE.saturating_sub(new_w)) / 2;
    let pad_y = (INPUT_SIZE.saturating_sub(new_h)) / 2;
    (scale, pad_x, pad_y)
}

/// Bilinear sample of the RGB source at fractional coordinates
/// `(sx, sy)`. Coordinates may fall outside the source; those samples
/// clamp to the nearest edge pixel.
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

/// Decode one YuNet stride head into normalized boxes.
///
/// For stride `s` the feature map is `(INPUT_SIZE / s)^2` cells. Each
/// cell `(row, col)` has an anchor at `((col + 0.5) * s, (row + 0.5) * s)`
/// in the 320x320 padded input. The bbox head predicts `(dx, dy, dw, dh)`
/// in units of `s`; center + exp(scale) decode follows the YuNet paper.
#[allow(clippy::too_many_arguments)]
fn decode_stride(
    stride: u32,
    cls: &tract_ndarray::ArrayViewD<f32>,
    obj: &tract_ndarray::ArrayViewD<f32>,
    bbox: &tract_ndarray::ArrayViewD<f32>,
    scale: f32,
    pad_x: u32,
    pad_y: u32,
    img_w: u32,
    img_h: u32,
    out: &mut Vec<FaceBox>,
) {
    let grid = INPUT_SIZE / stride; // 40, 20, 10
    let n = (grid * grid) as usize;

    for idx in 0..n {
        let c = cls[[0, idx, 0]];
        let o = obj[[0, idx, 0]];
        let conf = (c.max(0.0) * o.max(0.0)).sqrt();
        if conf < CONFIDENCE_THRESHOLD {
            continue;
        }

        let row = (idx as u32) / grid;
        let col = (idx as u32) % grid;
        let ax = (col as f32 + 0.5) * stride as f32;
        let ay = (row as f32 + 0.5) * stride as f32;

        let dx = bbox[[0, idx, 0]];
        let dy = bbox[[0, idx, 1]];
        let dw = bbox[[0, idx, 2]];
        let dh = bbox[[0, idx, 3]];

        let cx_320 = ax + dx * stride as f32;
        let cy_320 = ay + dy * stride as f32;
        let w_320 = dw.exp() * stride as f32;
        let h_320 = dh.exp() * stride as f32;

        // Un-letterbox: subtract pad, divide by scale, normalize to
        // the original frame size.
        let cx_orig = (cx_320 - pad_x as f32) / scale;
        let cy_orig = (cy_320 - pad_y as f32) / scale;
        let w_orig = w_320 / scale;
        let h_orig = h_320 / scale;

        let cx_norm = cx_orig / img_w as f32;
        let cy_norm = cy_orig / img_h as f32;
        let w_norm = w_orig / img_w as f32;
        let h_norm = h_orig / img_h as f32;

        out.push(FaceBox {
            x: cx_norm - w_norm * 0.5,
            y: cy_norm - h_norm * 0.5,
            w: w_norm,
            h: h_norm,
            confidence: conf,
        });
    }
}

/// Greedy non-maximum suppression: sort by descending confidence,
/// keep the top box, drop any remaining box whose IoU with a kept box
/// exceeds `iou_threshold`.
fn nms(mut boxes: Vec<FaceBox>, iou_threshold: f32) -> Vec<FaceBox> {
    boxes.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut keep: Vec<FaceBox> = Vec::new();
    let mut suppressed = vec![false; boxes.len()];
    for i in 0..boxes.len() {
        if suppressed[i] {
            continue;
        }
        keep.push(boxes[i]);
        for j in (i + 1)..boxes.len() {
            if suppressed[j] {
                continue;
            }
            if iou(&boxes[i], &boxes[j]) > iou_threshold {
                suppressed[j] = true;
            }
        }
    }
    keep
}

fn iou(a: &FaceBox, b: &FaceBox) -> f32 {
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
        let (scale, px, py) = letterbox_params(320, 320);
        assert!((scale - 1.0).abs() < 1e-6, "scale = {scale}");
        assert_eq!((px, py), (0, 0));
    }

    #[test]
    fn letterbox_landscape_pads_vertically() {
        // 640x480 -> 320x240, pad_y = (320 - 240)/2 = 40.
        let (scale, px, py) = letterbox_params(640, 480);
        assert!((scale - 0.5).abs() < 1e-6, "scale = {scale}");
        assert_eq!(px, 0);
        assert_eq!(py, 40);
    }

    #[test]
    fn letterbox_portrait_pads_horizontally() {
        // 480x640 -> 240x320, pad_x = (320 - 240)/2 = 40.
        let (scale, px, py) = letterbox_params(480, 640);
        assert!((scale - 0.5).abs() < 1e-6, "scale = {scale}");
        assert_eq!(px, 40);
        assert_eq!(py, 0);
    }

    #[test]
    fn nms_drops_overlapping_keeps_distinct() {
        let a = FaceBox {
            x: 0.0,
            y: 0.0,
            w: 0.2,
            h: 0.2,
            confidence: 0.9,
        };
        let b = FaceBox {
            x: 0.01,
            y: 0.01,
            w: 0.2,
            h: 0.2,
            confidence: 0.8,
        };
        let c = FaceBox {
            x: 0.7,
            y: 0.7,
            w: 0.2,
            h: 0.2,
            confidence: 0.7,
        };
        let out = nms(vec![a, b, c], 0.3);
        assert_eq!(out.len(), 2);
        assert!((out[0].confidence - 0.9).abs() < 1e-6);
        assert!((out[1].confidence - 0.7).abs() < 1e-6);
    }

    /// Integration test that requires a real YuNet model on disk.
    /// Skipped by default because the model is downloaded on demand
    /// from the registry (see DIRECTIVES §11: never bundle weights).
    ///
    /// Run manually with:
    ///   CAPRUST_YUNET_PATH=/path/to/yunet.onnx cargo nextest run \
    ///       --run-ignored face_detect_black_image_returns_none
    #[test]
    #[ignore]
    fn face_detect_black_image_returns_none() {
        let path = match std::env::var("CAPRUST_YUNET_PATH") {
            Ok(p) => std::path::PathBuf::from(p),
            Err(_) => {
                eprintln!("CAPRUST_YUNET_PATH not set; skipping");
                return;
            }
        };
        let det = FaceDetector::load(&path).expect("load YuNet");
        let black = vec![0u8; 320 * 320 * 3];
        let boxes = det.detect(&black, 320, 320).expect("inference");
        assert!(
            boxes.is_empty(),
            "black image must produce no detections, got {boxes:?}"
        );
    }
}
