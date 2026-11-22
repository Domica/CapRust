//! LaMa inpainting (Phase K2 - Object Removal).
//!
//! LaMa (Large Mask Inpainting) is a transformer-based inpainting model
//! that fills masked regions naturally. It's used for object removal
//! by filling the masked area with contextually appropriate content.
//!
//! Inference via `InferenceSession` which supports tract-onnx (CPU)
//! or ONNX Runtime (CUDA / DirectML). The model takes an image and a
//! binary mask, and produces an inpainted image where the masked region
//! is filled in naturally.
//!
//! Input:  [1, 3, 512, 512] f32, RGB + [1, 1, 512, 512] f32 mask
//! Output: [1, 3, 512, 512] f32 inpainted RGB

use crate::inference_backend::{BackendType, InferenceSession};
use crate::onnx_common::{bilinear_rgb, MEAN, STD};
use anyhow::{anyhow, Result};
use std::path::Path;

/// Fixed input resolution of the LaMa ONNX model.
pub const LAMA_INPUT_SIZE: u32 = 512;

/// Loaded LaMa model. Construct once and reuse.
pub struct Lama {
    session: InferenceSession,
}

impl std::fmt::Debug for Lama {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lama")
            .field("input_shape", &self.session.shape())
            .field("backend", &self.session.backend_type())
            .finish()
    }
}

impl Lama {
    /// Load and optimize LaMa ONNX model from disk.
    /// Expects file: lama.onnx
    pub fn load(model_path: &Path, backend: BackendType) -> Result<Self> {
        let session = InferenceSession::load(
            model_path,
            [1, 3, LAMA_INPUT_SIZE as i64, LAMA_INPUT_SIZE as i64],
            "LaMa",
            backend,
        )?;
        tracing::info!(
            "LaMa: loaded {} on {:?} backend",
            model_path.display(),
            session.backend_type()
        );

        if cfg!(debug_assertions) {
            tracing::warn!(
                "LaMa: running under a debug build. \
                 tract-onnx is opt-level 3 here (see Cargo.toml [profile.dev.package]), \
                 but other crates are not; expect slower inference than release. \
                 Use `cargo build --release` for full-speed inpainting."
            );
        }

        Ok(Self { session })
    }

    /// Returns the inference backend actually in use for LaMa
    /// (after Auto resolution and any CPU fallback).
    pub fn backend_type(&self) -> BackendType {
        self.session.backend_type()
    }

    /// Run inpainting on an image with a mask.
    /// Returns an inpainted RGB image at original resolution.
    pub fn inpaint(&self, rgb: &[u8], mask: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
        let expected = (width as usize) * (height as usize) * 3;
        let mask_expected = (width as usize) * (height as usize);
        if rgb.len() != expected {
            return Err(anyhow!(
                "rgb buffer length {} != expected {} ({}x{}x3)",
                rgb.len(),
                expected,
                width,
                height
            ));
        }
        if mask.len() != mask_expected {
            return Err(anyhow!(
                "mask buffer length {} != expected {} ({}x{})",
                mask.len(),
                mask_expected,
                width,
                height
            ));
        }
        if width == 0 || height == 0 {
            return Ok(Vec::new());
        }

        // 1. Preprocess image and mask
        let (image_data, mask_data) = self.preprocess(rgb, mask, width, height)?;

        // 2. Run inference
        let output = self.infer(&image_data, &mask_data)?;

        // 3. Post-process: convert to RGB at original resolution
        let result = self.postprocess(&output, width, height)?;

        Ok(result)
    }

    /// Run inpainting inference with two named inputs (image + mask).
    /// Falls back to positional inputs for tract-onnx (names ignored).
    fn infer(&self, image_data: &[f32], mask_data: &[f32]) -> Result<Vec<f32>> {
        let input_names = self.session.input_names();
        let hw = LAMA_INPUT_SIZE as i64;
        let image_shape: &[i64] = &[1, 3, hw, hw];
        let mask_shape: &[i64] = &[1, 1, hw, hw];
        let inputs = if input_names.len() >= 2 {
            // ort backend: use actual input names from the loaded model
            vec![
                (input_names[0].as_str(), image_data, image_shape),
                (input_names[1].as_str(), mask_data, mask_shape),
            ]
        } else {
            // tract-onnx backend: names are ignored, inputs are positional
            vec![
                ("input_0", image_data, image_shape),
                ("input_1", mask_data, mask_shape),
            ]
        };
        let outputs = self.session.run_multi(&inputs[..])?;
        outputs
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("LaMa produced no outputs"))
    }

    /// Post-process output flat Vec to RGB bytes at target resolution.
    ///
    /// `output` is CHW layout: [1, 3, 512, 512] = channel-major.
    fn postprocess(
        &self,
        output: &[f32],
        target_width: u32,
        target_height: u32,
    ) -> Result<Vec<u8>> {
        // Denormalize: (x * std + mean) * 255
        let mut result = vec![0u8; (target_width * target_height * 3) as usize];

        let hw = LAMA_INPUT_SIZE as usize;
        let plane = hw * hw;

        // Convert from CHW to HWC and denormalize
        for c in 0..3 {
            for y in 0..hw {
                for x in 0..hw {
                    let val = output[c * plane + y * hw + x];
                    let denorm = val * STD[c] + MEAN[c];
                    let byte = (denorm * 255.0).clamp(0.0, 255.0).round() as u8;

                    // Convert from CHW to HWC
                    let dst_idx = (y * hw + x) * 3 + c;
                    result[dst_idx] = byte;
                }
            }
        }

        // Upscale to target resolution if needed
        if LAMA_INPUT_SIZE == target_width && LAMA_INPUT_SIZE == target_height {
            Ok(result)
        } else {
            Ok(upscale_rgb(
                &result,
                LAMA_INPUT_SIZE,
                LAMA_INPUT_SIZE,
                target_width,
                target_height,
            ))
        }
    }

    /// Preprocess image and mask for LaMa inference.
    /// Returns flat CHW `Vec<f32>` slices for image and mask.
    fn preprocess(
        &self,
        rgb: &[u8],
        mask: &[u8],
        width: u32,
        height: u32,
    ) -> Result<(Vec<f32>, Vec<f32>)> {
        // Preprocess image: resize to 512x512, normalize
        let hw = LAMA_INPUT_SIZE as usize;
        let plane = hw * hw;
        let mut image_data = vec![0.0f32; 3 * plane];
        let mut mask_data = vec![0.0f32; plane];

        let sx = width as f32 / LAMA_INPUT_SIZE as f32;
        let sy = height as f32 / LAMA_INPUT_SIZE as f32;

        for dy in 0..LAMA_INPUT_SIZE {
            for dx in 0..LAMA_INPUT_SIZE {
                let src_x = (dx as f32 + 0.5) * sx - 0.5;
                let src_y = (dy as f32 + 0.5) * sy - 0.5;
                let (r, g, b) = bilinear_rgb(rgb, width, height, src_x, src_y);
                let m = bilinear_mask(mask, width, height, src_x, src_y);

                let base = dy as usize * hw + dx as usize;
                // CHW layout: channel c at offset c*H*W + y*W + x
                image_data[base] = (r / 255.0 - MEAN[0]) / STD[0];
                image_data[plane + base] = (g / 255.0 - MEAN[1]) / STD[1];
                image_data[2 * plane + base] = (b / 255.0 - MEAN[2]) / STD[2];
                mask_data[base] = m;
            }
        }

        Ok((image_data, mask_data))
    }
}

/// Bilinear sampling for single-channel mask.
fn bilinear_mask(mask: &[u8], width: u32, height: u32, sx: f32, sy: f32) -> f32 {
    let w = width as i32;
    let h = height as i32;
    let x0 = sx.floor() as i32;
    let y0 = sy.floor() as i32;
    let fx = sx - x0 as f32;
    let fy = sy - y0 as f32;

    let clamp = |v: i32, hi: i32| v.clamp(0, hi - 1);
    let at = |x: i32, y: i32| -> f32 {
        let xc = clamp(x, w);
        let yc = clamp(y, h);
        let i = (yc as usize) * (width as usize) + (xc as usize);
        mask[i] as f32 / 255.0
    };

    let (m00, m10, m01, m11) = (
        at(x0, y0),
        at(x0 + 1, y0),
        at(x0, y0 + 1),
        at(x0 + 1, y0 + 1),
    );

    let lerp = |a: f32, b: f32, c: f32, d: f32| -> f32 {
        let top = a * (1.0 - fx) + b * fx;
        let bot = c * (1.0 - fx) + d * fx;
        top * (1.0 - fy) + bot * fy
    };

    lerp(m00, m10, m01, m11)
}

/// Upscale RGB image (HWC format).
fn upscale_rgb(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    if sw == dw && sh == dh {
        return src.to_vec();
    }
    let mut out = vec![0u8; (dw as usize) * (dh as usize) * 3];
    for dy in 0..dh {
        for dx in 0..dw {
            for c in 0..3 {
                let sx = (dx as f32 + 0.5) * (sw as f32 / dw as f32) - 0.5;
                let sy = (dy as f32 + 0.5) * (sh as f32 / dh as f32) - 0.5;
                let x0 = sx.floor() as i32;
                let y0 = sy.floor() as i32;
                let fx = sx - x0 as f32;
                let fy = sy - y0 as f32;
                let get = |x: i32, y: i32, c: usize| -> f32 {
                    let xc = x.clamp(0, sw as i32 - 1) as usize;
                    let yc = y.clamp(0, sh as i32 - 1) as usize;
                    src[(yc * sw as usize + xc) * 3 + c] as f32
                };
                let top = get(x0, y0, c) * (1.0 - fx) + get(x0 + 1, y0, c) * fx;
                let bot = get(x0, y0 + 1, c) * (1.0 - fx) + get(x0 + 1, y0 + 1, c) * fx;
                let v = top * (1.0 - fy) + bot * fy;
                out[(dy as usize * dw as usize + dx as usize) * 3 + c] = v.clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}
