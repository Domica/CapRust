//! Background removal via u2netp (Phase P3b).
//!
//! Pure-Rust ONNX inference via tract-onnx, same stack as YuNet
//! (face_detect.rs). The model file is downloaded on demand from the
//! registry (`ModelKind::BackgroundRemover`).
//!
//! Input:  [1, 3, 320, 320] f32, RGB, ImageNet-normalized
//!         (px/255 - mean) / std, mean=[0.485, 0.456, 0.406],
//!         std=[0.229, 0.224, 0.225]. Unlike YuNet, u2netp is a
//!         PyTorch model: RGB channel order, not BGR.
//! Output: 7 tensors d0..d6 (one per U-block level). Only d0 is at
//!         full 320x320 resolution and is the one that matters; the
//!         others are supervision outputs. We locate d0 by shape, not
//!         by index, so a model re-export that reorders outputs still
//!         works.
//!
//! infer_mask returns a single-channel alpha mask at the *original*
//! input resolution: 0 = background, 255 = foreground. Callers feed
//! this into the render graph as a mask for `alphamerge` /
//! `alphaextract`.

use anyhow::{anyhow, Result};
use std::path::Path;
use tract_onnx::prelude::*;

/// Fixed input resolution of the u2netp ONNX model.
pub const INPUT_SIZE: u32 = 320;

/// ImageNet normalization constants used by u2netp's training pipeline.
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// Loaded u2netp model. Construct once and reuse across frames --
/// `load` parses and optimizes the ONNX graph, which is slow relative
/// to a single inference pass.
pub struct BackgroundRemover {
    model: RunnableModel,
}

// tract 0.23 renamed the runnable plan type; the prelude alias
// `RunnableModel` is the stable name going forward.
// tract 0.23 wraps the runnable plan in an Arc; `run()` takes
// `self: &Arc<Self>` so the alias carries the Arc.
type RunnableModel =
    std::sync::Arc<tract_onnx::prelude::RunnableModel<TypedFact, Box<dyn TypedOp>>>;

impl BackgroundRemover {
    /// Load and optimize a u2netp ONNX model from disk.
    pub fn load(model_path: &Path) -> Result<Self> {
        if !model_path.is_file() {
            return Err(anyhow!("u2netp model not found: {}", model_path.display()));
        }
        let model = tract_onnx::onnx()
            .model_for_path(model_path)
            .map_err(|e| {
                // tract wraps its parse errors in an opaque Display
                // that hides the real reason (missing op, bad attr,
                // unsupported type). Log the Debug form so the user
                // can see what tract actually choked on.
                anyhow!(
                    "load ONNX model {}: {e:?}\n  (this usually means tract-onnx 0.22 does not support an operator in the file; try --log-level debug for the full chain)",
                    model_path.display()
                )
            })?
            .into_optimized()
            .map_err(|e| anyhow!("optimize ONNX model: {e:?}"))?
            .into_runnable()
            .map_err(|e| anyhow!("build runnable plan: {e:?}"))?;
        tracing::info!("background_removal: loaded {}", model_path.display());
        Ok(Self { model })
    }

    /// Run segmentation on a tightly packed RGB buffer of length
    /// `width * height * 3`. Returns a single-channel alpha mask at
    /// the same `width x height`: 0 = background, 255 = foreground.
    pub fn infer_mask(&self, rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
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

        let input: Tensor = tensor.into();
        let outputs = self
            .model
            .run(tvec!(input.into()))
            .map_err(|e| anyhow!("u2netp inference: {e:?}"))?;

        let d0 = find_d0(&outputs)?;

        // The model already applies sigmoid on d0; values are in
        // 0..1. Scale to 0..255 and resample up to the caller's size.
        let d0 = d0.to_plain_array_view::<f32>()?;
        let mut small = vec![0u8; (INPUT_SIZE * INPUT_SIZE) as usize];
        for y in 0..INPUT_SIZE as usize {
            for x in 0..INPUT_SIZE as usize {
                let v = d0[[0, 0, y, x]].clamp(0.0, 1.0);
                small[y * INPUT_SIZE as usize + x] = (v * 255.0).round() as u8;
            }
        }

        Ok(upscale_mask(&small, INPUT_SIZE, INPUT_SIZE, width, height))
    }

    /// Resize + normalize into a (1, 3, 320, 320) CHW tensor.
    fn preprocess(&self, rgb: &[u8], width: u32, height: u32) -> tract_ndarray::Array4<f32> {
        let mut arr =
            tract_ndarray::Array4::<f32>::zeros((1, 3, INPUT_SIZE as usize, INPUT_SIZE as usize));

        // u2net does not use letterboxing: it scales the whole frame
        // to 320x320 directly. Non-square sources therefore get a
        // stretch, which matches the training pipeline (u2net was
        // trained on non-uniformly resized DUTS/DUT-OMRON data).
        let sx = width as f32 / INPUT_SIZE as f32;
        let sy = height as f32 / INPUT_SIZE as f32;

        for dy in 0..INPUT_SIZE {
            for dx in 0..INPUT_SIZE {
                let src_x = (dx as f32 + 0.5) * sx - 0.5;
                let src_y = (dy as f32 + 0.5) * sy - 0.5;
                let (r, g, b) = bilinear_rgb(rgb, width, height, src_x, src_y);
                // ImageNet normalization. Channel order is RGB.
                arr[[0, 0, dy as usize, dx as usize]] = (r / 255.0 - MEAN[0]) / STD[0];
                arr[[0, 1, dy as usize, dx as usize]] = (g / 255.0 - MEAN[1]) / STD[1];
                arr[[0, 2, dy as usize, dx as usize]] = (b / 255.0 - MEAN[2]) / STD[2];
            }
        }

        arr
    }
}

/// Locate the d0 output (full-resolution alpha) among the seven
/// u2netp outputs. We identify it by shape `[1, 1, 320, 320]` rather
/// than assuming index 0, so a re-export that reorders the outputs
/// still works.
fn find_d0(outputs: &[TValue]) -> Result<&TValue> {
    for (i, t) in outputs.iter().enumerate() {
        let view = match t.to_plain_array_view::<f32>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let shape = view.shape();
        if shape.len() == 4
            && shape[0] == 1
            && shape[1] == 1
            && shape[2] == INPUT_SIZE as usize
            && shape[3] == INPUT_SIZE as usize
        {
            tracing::debug!("background_removal: d0 at output index {i}");
            return Ok(t);
        }
    }
    Err(anyhow!(
        "u2netp: no output with shape [1, 1, {INPUT_SIZE}, {INPUT_SIZE}] found among {} tensors",
        outputs.len()
    ))
}

/// Bilinear sample of the RGB source at fractional coordinates
/// `(sx, sy)`. Coordinates may fall outside the source; those samples
/// clamp to the nearest edge pixel. Same algorithm as face_detect.rs;
/// duplicated here to avoid a shared util module for two callers.
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

/// Bilinear upscale of a single-channel u8 mask.
fn upscale_mask(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    if sw == dw && sh == dh {
        return src.to_vec();
    }
    let mut out = vec![0u8; (dw as usize) * (dh as usize)];
    let xr = sw as f32 / dw as f32;
    let yr = sh as f32 / dh as f32;
    for dy in 0..dh {
        for dx in 0..dw {
            let sx = (dx as f32 + 0.5) * xr - 0.5;
            let sy = (dy as f32 + 0.5) * yr - 0.5;
            let x0 = sx.floor() as i32;
            let y0 = sy.floor() as i32;
            let fx = sx - x0 as f32;
            let fy = sy - y0 as f32;
            let get = |x: i32, y: i32| -> f32 {
                let xc = x.clamp(0, sw as i32 - 1) as usize;
                let yc = y.clamp(0, sh as i32 - 1) as usize;
                src[yc * sw as usize + xc] as f32
            };
            let top = get(x0, y0) * (1.0 - fx) + get(x0 + 1, y0) * fx;
            let bot = get(x0, y0 + 1) * (1.0 - fx) + get(x0 + 1, y0 + 1) * fx;
            let v = top * (1.0 - fy) + bot * fy;
            out[dy as usize * dw as usize + dx as usize] = v.clamp(0.0, 255.0) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upscale_mask_identity_is_noop() {
        let src = vec![1u8, 2, 3, 4];
        let out = upscale_mask(&src, 2, 2, 2, 2);
        assert_eq!(out, src);
    }

    #[test]
    fn upscale_mask_doubles_dimensions() {
        let src = vec![0u8, 255, 128, 64];
        let out = upscale_mask(&src, 2, 2, 4, 4);
        assert_eq!(out.len(), 16);
        // Corners should land on corner values.
        assert_eq!(out[0], 0);
        assert_eq!(out[3], 255);
        assert_eq!(out[12], 128);
        assert_eq!(out[15], 64);
    }

    /// Requires a real u2netp model on disk. Skipped by default --
    /// the model is downloaded on demand (DIRECTIVES §11: never
    /// bundle weights). Run manually with:
    ///   CAPRUST_U2NETP_PATH=/path/to/u2netp.onnx cargo nextest run \
    ///       --run-ignored background_removal_black_image_returns_dark_mask
    #[test]
    #[ignore]
    fn background_removal_black_image_returns_dark_mask() {
        let path = match std::env::var("CAPRUST_U2NETP_PATH") {
            Ok(p) => std::path::PathBuf::from(p),
            Err(_) => {
                eprintln!("CAPRUST_U2NETP_PATH not set; skipping");
                return;
            }
        };
        let model = BackgroundRemover::load(&path).expect("load u2netp");
        let black = vec![0u8; 320 * 320 * 3];
        let mask = model.infer_mask(&black, 320, 320).expect("inference");
        assert_eq!(mask.len(), 320 * 320);
        // A black frame is unambiguously "no salient object"; the
        // mean alpha must be very low. Exact 0 is not guaranteed
        // because the network has biases.
        let mean: u32 = mask.iter().map(|&v| v as u32).sum::<u32>() / mask.len() as u32;
        assert!(mean < 32, "mean alpha on black frame was {mean}");
    }
}
