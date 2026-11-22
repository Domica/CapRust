//! MobileSAM segmentation (Phase K2 - Object Removal).
//!
//! MobileSAM is a lightweight SAM variant for prompt-based segmentation.
//! This implementation uses point prompts (click coordinates) to generate
//! a binary mask for object removal.
//!
//! Inference via `InferenceSession` which supports tract-onnx (CPU)
//! or ONNX Runtime (CUDA / DirectML). The encoder produces an image
//! embedding; the decoder takes the embedding plus point prompts and
//! produces a mask.
//!
//! Input (encoder):  [1, 3, 1024, 1024] f32, RGB, ImageNet-normalized
//! Input (decoder):  image_embedding + point_coords + point_labels +
//!                   has_mask_input + mask_input
//! Output:           256x256 mask logits (single channel) - upscale to target resolution

use crate::inference_backend::{BackendType, InferenceSession};
use crate::onnx_common::{bilinear_rgb, upscale_mask, MEAN, STD};
use anyhow::{anyhow, Result};
use std::path::Path;
use tract_onnx::prelude::tract_ndarray;

/// Fixed input resolution of the MobileSAM ONNX model (image encoder).
pub const MOBILE_SAM_INPUT_SIZE: u32 = 1024;
/// Mask decoder output size.
pub const MOBILE_SAM_MASK_SIZE: u32 = 256;
/// MobileSAM image embedding dimensions (ViT-T: [1, 256, 64, 64]).
pub const MOBILE_SAM_EMBED_DIM: u32 = 256;
pub const MOBILE_SAM_EMBED_HW: u32 = 64;

/// Prompt types for MobileSAM.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SamPrompt {
    /// Single point prompt (click). (x, y) in normalized coordinates [0, 1].
    Point(f32, f32),
    /// Bounding box prompt. (x1, y1, x2, y2) in normalized coordinates [0, 1].
    Box(f32, f32, f32, f32),
}

/// Loaded MobileSAM model. Construct once and reuse.
pub struct MobileSam {
    encoder: InferenceSession, // Image encoder
    decoder: InferenceSession, // Mask decoder
}

impl std::fmt::Debug for MobileSam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MobileSam")
            .field("encoder_backend", &self.encoder.backend_type())
            .field("decoder_backend", &self.decoder.backend_type())
            .finish()
    }
}

impl MobileSam {
    /// Load and optimize MobileSAM ONNX model from disk.
    /// Expects two files: mobile_sam_encoder.onnx and mobile_sam_decoder.onnx
    pub fn load(encoder_path: &Path, decoder_path: &Path, backend: BackendType) -> Result<Self> {
        if !encoder_path.is_file() {
            return Err(anyhow!(
                "MobileSAM encoder not found: {}",
                encoder_path.display()
            ));
        }
        if !decoder_path.is_file() {
            return Err(anyhow!(
                "MobileSAM decoder not found: {}",
                decoder_path.display()
            ));
        }

        let encoder = InferenceSession::load(
            encoder_path,
            [1, 3, MOBILE_SAM_INPUT_SIZE as i64, MOBILE_SAM_INPUT_SIZE as i64],
            "MobileSAM encoder",
            backend,
        )?;
        let decoder = InferenceSession::load(
            decoder_path,
            [1, MOBILE_SAM_EMBED_DIM as i64, MOBILE_SAM_EMBED_HW as i64, MOBILE_SAM_EMBED_HW as i64],
            "MobileSAM decoder",
            backend,
        )?;

        tracing::info!(
            "MobileSAM: loaded encoder {} on {:?} backend, decoder {} on {:?} backend",
            encoder_path.display(),
            encoder.backend_type(),
            decoder_path.display(),
            decoder.backend_type()
        );

        Ok(Self { encoder, decoder })
    }

    /// Run segmentation with a point prompt.
    /// Returns a single-channel mask at original resolution: 0 = background, 255 = foreground.
    pub fn segment_point(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
        point_x: f32, // normalized [0, 1]
        point_y: f32, // normalized [0, 1]
    ) -> Result<Vec<u8>> {
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

        // 1. Encode image
        let image_embedding = self.encode_image(rgb, width, height)?;

        // 2. Prepare point prompt
        let (point_coords, point_labels) =
            self.prepare_point_prompt(point_x, point_y)?;

        // 3. Decode mask
        let mask_logits = self.decode_mask(&image_embedding, &point_coords, &point_labels)?;

        // 4. Post-process: sigmoid, threshold, upscale to original resolution
        let mask = self.postprocess_mask(&mask_logits, width, height)?;

        Ok(mask)
    }

    /// Returns the inference backend actually in use for MobileSAM
    /// (after Auto resolution and any CPU fallback).
    pub fn backend_type(&self) -> BackendType {
        self.encoder.backend_type()
    }

    /// Run image encoder on RGB input. Returns the flat image embedding.
    fn encode_image(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Vec<f32>> {
        let input_arr = self.preprocess_image(rgb, width, height)?;
        let flat: Vec<f32> = input_arr.iter().copied().collect();
        let outputs = self.encoder.run(&flat)?;
        outputs
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("MobileSAM encoder produced no outputs"))
    }

    /// Prepare point prompt for decoder.
    fn prepare_point_prompt(
        &self,
        point_x: f32,
        point_y: f32,
    ) -> Result<(Vec<f32>, Vec<f32>)> {
        // MobileSAM expects point coordinates in 1024x1024 input space
        let x = point_x * MOBILE_SAM_INPUT_SIZE as f32;
        let y = point_y * MOBILE_SAM_INPUT_SIZE as f32;

        // Point coordinates: [1, 1, 2] - batch=1, num_points=1, xy=2
        let point_coords = vec![x, y];

        // Point labels: 1 = foreground, 0 = background
        let point_labels = vec![1.0]; // foreground

        Ok((point_coords, point_labels))
    }

    /// Run mask decoder with image embedding and point prompts.
    /// Uses `run_multi` for the multiple decoder inputs.
    fn decode_mask(
        &self,
        image_embedding: &[f32],
        point_coords: &[f32],
        point_labels: &[f32],
    ) -> Result<Vec<f32>> {
        // Decoder inputs: image_embedding, point_coords, point_labels,
        // has_mask_input (float [1] = 0.0 = no mask), mask_input (zeros [1,1,256,256]).
        let input_names = self.decoder.input_names();
        let embed_hw = MOBILE_SAM_EMBED_HW as i64;
        let embed_dim = MOBILE_SAM_EMBED_DIM as i64;
        let mask_hw = MOBILE_SAM_MASK_SIZE as i64;

        let has_mask_input = vec![0.0f32]; // no mask input
        let mask_input = vec![0.0f32; (mask_hw * mask_hw) as usize];

        let embed_shape: &[i64] = &[1, embed_dim, embed_hw, embed_hw];
        let coords_shape: &[i64] = &[1, 1, 2];
        let labels_shape: &[i64] = &[1, 1];
        let has_mask_shape: &[i64] = &[1];
        let mask_shape: &[i64] = &[1, 1, mask_hw, mask_hw];

        let inputs = if input_names.len() >= 5 {
            // ort backend: use actual input names from the loaded model
            vec![
                (input_names[0].as_str(), image_embedding, embed_shape),
                (input_names[1].as_str(), point_coords, coords_shape),
                (input_names[2].as_str(), point_labels, labels_shape),
                (input_names[3].as_str(), &has_mask_input, has_mask_shape),
                (input_names[4].as_str(), &mask_input, mask_shape),
            ]
        } else {
            // tract-onnx backend: names are ignored, inputs are positional
            vec![
                ("input_0", image_embedding, embed_shape),
                ("input_1", point_coords, coords_shape),
                ("input_2", point_labels, labels_shape),
                ("input_3", &has_mask_input, has_mask_shape),
                ("input_4", &mask_input, mask_shape),
            ]
        };

        let outputs = self.decoder.run_multi(&inputs[..])?;
        outputs
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("MobileSAM decoder produced no outputs"))
    }

    /// Post-process mask logits to binary mask at original resolution.
    fn postprocess_mask(
        &self,
        mask_logits: &[f32],
        target_width: u32,
        target_height: u32,
    ) -> Result<Vec<u8>> {
        // mask_logits is flat [1, 1, 256, 256] in CHW order
        let hw = MOBILE_SAM_MASK_SIZE as usize;
        let plane = hw * hw;

        let mut small = vec![0u8; plane];
        for y in 0..hw {
            for x in 0..hw {
                let logit = mask_logits[y * hw + x];
                // Apply sigmoid
                let prob = 1.0 / (1.0 + (-logit).exp());
                small[y * hw + x] = if prob > 0.5 { 255 } else { 0 };
            }
        }

        // Upscale to target resolution
        Ok(upscale_mask(
            &small,
            MOBILE_SAM_MASK_SIZE,
            MOBILE_SAM_MASK_SIZE,
            target_width,
            target_height,
        ))
    }

    /// Preprocess image for encoder: resize to 1024x1024, normalize.
    /// Returns an `Array4<f32>` in CHW layout.
    fn preprocess_image(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<tract_ndarray::Array4<f32>> {
        let hw = MOBILE_SAM_INPUT_SIZE as usize;
        let mut arr = tract_ndarray::Array4::<f32>::zeros((1, 3, hw, hw));

        let sx = width as f32 / MOBILE_SAM_INPUT_SIZE as f32;
        let sy = height as f32 / MOBILE_SAM_INPUT_SIZE as f32;

        for dy in 0..MOBILE_SAM_INPUT_SIZE {
            for dx in 0..MOBILE_SAM_INPUT_SIZE {
                let src_x = (dx as f32 + 0.5) * sx - 0.5;
                let src_y = (dy as f32 + 0.5) * sy - 0.5;
                let (r, g, b) = bilinear_rgb(rgb, width, height, src_x, src_y);
                // ImageNet normalization. Channel order is RGB.
                let dy_usize = dy as usize;
                let dx_usize = dx as usize;
                arr[[0, 0, dy_usize, dx_usize]] = (r / 255.0 - MEAN[0]) / STD[0];
                arr[[0, 1, dy_usize, dx_usize]] = (g / 255.0 - MEAN[1]) / STD[1];
                arr[[0, 2, dy_usize, dx_usize]] = (b / 255.0 - MEAN[2]) / STD[2];
            }
        }

        Ok(arr)
    }
}
