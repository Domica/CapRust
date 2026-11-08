//! LaMa inpainting (Phase K2 - Object Removal).
//!
//! LaMa (Large Mask Inpainting) is a transformer-based inpainting model
//! that fills masked regions naturally. It's used for object removal
//! by filling the masked area with contextually appropriate content.
//!
//! Input:  [1, 3, 512, 512] f32, RGB + [1, 1, 512, 512] f32 mask
//! Output: [1, 3, 512, 512] f32 inpainted RGB
//!
//! The model takes an image and a binary mask, and produces an inpainted
//! image where the masked region is filled in naturally.

use anyhow::{anyhow, Result};
use std::path::Path;
use tract_onnx::prelude::*;

/// Fixed input resolution of the LaMa ONNX model.
pub const LAMA_INPUT_SIZE: u32 = 512;

/// ImageNet normalization constants used by LaMa's training pipeline.
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// Loaded LaMa model. Construct once and reuse.
pub struct Lama {
    model: RunnableModel,
}

type RunnableModel = std::sync::Arc<tract_onnx::prelude::RunnableModel<TypedFact, Box<dyn TypedOp>>>;

impl Lama {
    /// Load and optimize LaMa ONNX model from disk.
    /// Expects file: lama.onnx
    pub fn load(model_path: &Path) -> Result<Self> {
        if !model_path.is_file() {
            return Err(anyhow!("LaMa model not found: {}", model_path.display()));
        }

        let model = tract_onnx::onnx()
            .model_for_path(model_path)
            .map_err(|e| anyhow!("load LaMa ONNX {}: {:?}", model_path.display(), e))?
            .with_input_fact(
                0,
                InferenceFact::dt_shape(
                    f32::datum_type(),
                    tvec!(1, 3, LAMA_INPUT_SIZE as i64, LAMA_INPUT_SIZE as i64),
                ),
            )
            .map_err(|e| anyhow!("pin LaMa input 0 shape: {e:?}"))?
            .with_input_fact(
                1,
                InferenceFact::dt_shape(
                    f32::datum_type(),
                    tvec!(1, 1, LAMA_INPUT_SIZE as i64, LAMA_INPUT_SIZE as i64),
                ),
            )
            .map_err(|e| anyhow!("pin LaMa input 1 shape: {e:?}"))?
            .into_typed()
            .map_err(|e| anyhow!("type inference LaMa: {:?}", e))?
            .into_runnable()
            .map_err(|e| anyhow!("build LaMa runnable: {:?}", e))?;

        tracing::info!("LaMa: loaded {}", model_path.display());

        if cfg!(debug_assertions) {
            tracing::warn!(
                "LaMa: running under a debug build. \
                 tract-onnx is opt-level 3 here (see Cargo.toml [profile.dev.package]), \
                 but other crates are not; expect slower inference than release. \
                 Use `cargo build --release` for full-speed inpainting."
            );
        }

        Ok(Self {
            model: model,
        })
    }

    /// Run inpainting on an image with a mask.
    /// Returns an inpainted RGB image at original resolution.
    pub fn inpaint(
        &self,
        rgb: &[u8],
        mask: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>> {
        let expected = (width as usize) * (height as usize) * 3;
        let mask_expected = (width as usize) * (height as usize);
        if rgb.len() != expected {
            return Err(anyhow!(
                "rgb buffer length {} != expected {} ({}x{}x3)",
                rgb.len(), expected, width, height
            ));
        }
        if mask.len() != mask_expected {
            return Err(anyhow!(
                "mask buffer length {} != expected {} ({}x{})",
                mask.len(), mask_expected, width, height
            ));
        }
        if width == 0 || height == 0 {
            return Ok(Vec::new());
        }

        // 1. Preprocess image and mask
        let (image_tensor, mask_tensor) = self.preprocess(rgb, mask, width, height)?;

        // 2. Run inference
        let output = self.infer(&image_tensor, &mask_tensor)?;

        // 3. Post-process: convert to RGB at original resolution
        let result = self.postprocess(&output, width, height)?;

        Ok(result)
    }

    /// Run inpainting inference.
    fn infer(&self, image_tensor: &Tensor, mask_tensor: &Tensor) -> Result<Tensor> {
        let outputs = self
            .model
            .run(tvec!(image_tensor.clone().into(), mask_tensor.clone().into()))
            .map_err(|e| anyhow!("LaMa inference: {:?}", e))?;

        // First output is inpainted image [1, 3, 512, 512]
        let output = outputs[0].clone().into_tensor();
        Ok(output)
    }

    /// Post-process output tensor to RGB bytes at target resolution.
    fn postprocess(&self, output: &Tensor, target_width: u32, target_height: u32) -> Result<Vec<u8>> {
        let output = output.to_plain_array_view::<f32>()?;
        let output = output.into_dimensionality::<tract_ndarray::Ix4>()?;
        // Shape: [1, 3, 512, 512]

        // Denormalize: (x * std + mean) * 255
        let mut result = vec![0u8; (target_width * target_height * 3) as usize];
        
        // Convert from CHW to HWC and denormalize
        for c in 0..3 {
            for y in 0..LAMA_INPUT_SIZE as usize {
                for x in 0..LAMA_INPUT_SIZE as usize {
                    let val = output[[0, c, y, x]];
                    let denorm = val * STD[c] + MEAN[c];
                    let byte = (denorm * 255.0).clamp(0.0, 255.0).round() as u8;
                    
                    // Convert from CHW to HWC
                    let src_idx = c * (LAMA_INPUT_SIZE as usize) * (LAMA_INPUT_SIZE as usize) + y * (LAMA_INPUT_SIZE as usize) + x;
                    let dst_idx = (y * LAMA_INPUT_SIZE as usize + x) * 3 + c;
                    result[dst_idx] = byte;
                }
            }
        }

        // Upscale to target resolution if needed
        if LAMA_INPUT_SIZE == target_width && LAMA_INPUT_SIZE == target_height {
            Ok(result)
        } else {
            Ok(upscale_rgb(&result, LAMA_INPUT_SIZE, LAMA_INPUT_SIZE, target_width, target_height))
        }
    }

    /// Preprocess image and mask for LaMa inference.
    fn preprocess(&self, rgb: &[u8], mask: &[u8], width: u32, height: u32) -> Result<(Tensor, Tensor)> {
        // Preprocess image: resize to 512x512, normalize
        let mut image_arr = tract_ndarray::Array4::<f32>::zeros((1, 3, LAMA_INPUT_SIZE as usize, LAMA_INPUT_SIZE as usize));
        let mut mask_arr = tract_ndarray::Array4::<f32>::zeros((1, 1, LAMA_INPUT_SIZE as usize, LAMA_INPUT_SIZE as usize));

        let sx = width as f32 / LAMA_INPUT_SIZE as f32;
        let sy = height as f32 / LAMA_INPUT_SIZE as f32;

        for dy in 0..LAMA_INPUT_SIZE {
            for dx in 0..LAMA_INPUT_SIZE {
                let src_x = (dx as f32 + 0.5) * sx - 0.5;
                let src_y = (dy as f32 + 0.5) * sy - 0.5;
                let (r, g, b) = crate::background_removal::bilinear_rgb(rgb, width, height, src_x, src_y);
                let m = bilinear_mask(mask, width, height, src_x, src_y);

                let dy_usize = dy as usize;
                let dx_usize = dx as usize;
                image_arr[[0, 0, dy_usize, dx_usize]] = (r / 255.0 - MEAN[0]) / STD[0];
                image_arr[[0, 1, dy_usize, dx_usize]] = (g / 255.0 - MEAN[1]) / STD[1];
                image_arr[[0, 2, dy_usize, dx_usize]] = (b / 255.0 - MEAN[2]) / STD[2];
                mask_arr[[0, 0, dy_usize, dx_usize]] = m;
            }
        }

        let image_tensor: Tensor = image_arr.into();
        let mask_tensor: Tensor = mask_arr.into();

        Ok((image_tensor, mask_tensor))
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
        let i = ((yc as usize) * (width as usize) + (xc as usize));
        mask[i] as f32 / 255.0
    };

    let w = width as i32;
    let h = height as i32;

    let (m00, m10, m01, m11) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));

    let lerp = |a: f32, b: f32, c: f32, d: f32| -> f32 {
        let top = a * (1.0 - fx) + b * fx;
        let bot = c * (1.0 - fx) + d * fx;
        top * (1.0 - fy) + bot * fy
    };

    lerp(m00, m10, m01, m11)
}

// Reuse upscale_mask from background_removal for single-channel masks
fn upscale_mask(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    if sw == dw && sh == dh {
        return src.to_vec();
    }
    let mut out = vec![0u8; (dw as usize) * (dh as usize)];
    let xr = sw as f32 / dw as f32;
    let yr = sh as f32 / dh as f32;
    for dy in 0..dh {
        for dx in 0..dw {
            let sx = (dx as f32 + 0.5) * (sw as f32 / dw as f32) - 0.5;
            let sy = (dy as f32 + 0.5) * (sh as f32 / dh as f32) - 0.5;
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

/// Upscale RGB image (HWC format).
fn upscale_rgb(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    if sw == dw && sh == dh {
        return src.to_vec();
    }
    let mut out = vec![0u8; (dw as usize) * (dh as usize) * 3];
    let xr = sw as f32 / dw as f32;
    let yr = sh as f32 / dh as f32;
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

// Note: bilinear_rgb is re-exported from background_removal module
// to avoid code duplication. See background_removal.rs for implementations.

use crate::background_removal::bilinear_rgb;