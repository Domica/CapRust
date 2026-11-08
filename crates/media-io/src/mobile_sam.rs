//! MobileSAM segmentation (Phase K2 - Object Removal).
//!
//! MobileSAM is a lightweight SAM variant for prompt-based segmentation.
//! This implementation uses point prompts (click coordinates) to generate
//! a binary mask for object removal.
//!
//! Input:  [1, 3, 1024, 1024] f32, RGB, ImageNet-normalized
//! Output: 256x256 mask logits (single channel) - upscale to target resolution
//!
//! The model takes an image and point prompts (click coordinates) and
//! produces a segmentation mask. We use point prompts for click-to-mask.

use anyhow::{anyhow, Result};
use std::path::Path;
use tract_onnx::prelude::*;

/// Fixed input resolution of the MobileSAM ONNX model (image encoder).
pub const MOBILE_SAM_INPUT_SIZE: u32 = 1024;
/// Mask decoder output size.
pub const MOBILE_SAM_MASK_SIZE: u32 = 256;

/// ImageNet normalization constants used by MobileSAM's training pipeline.
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

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
    encoder: RunnableModel,   // Image encoder
    decoder: RunnableModel,   // Mask decoder
}

type RunnableModel = std::sync::Arc<tract_onnx::prelude::InferenceSimplePlan>;

impl MobileSam {
    /// Load and optimize MobileSAM ONNX model from disk.
    /// Expects two files: mobile_sam_encoder.onnx and mobile_sam_decoder.onnx
    pub fn load(encoder_path: &Path, decoder_path: &Path) -> Result<Self> {
        if !encoder_path.is_file() {
            return Err(anyhow!("MobileSAM encoder not found: {}", encoder_path.display()));
        }
        if !decoder_path.is_file() {
            return Err(anyhow!("MobileSAM decoder not found: {}", decoder_path.display()));
        }

        let encoder = tract_onnx::onnx()
            .model_for_path(encoder_path)
            .map_err(|e| anyhow!("load encoder ONNX {}: {:?}", encoder_path.display(), e))?
            .into_runnable()
            .map_err(|e| anyhow!("build encoder runnable: {:?}", e))?;

        let decoder = tract_onnx::onnx()
            .model_for_path(decoder_path)
            .map_err(|e| anyhow!("load decoder ONNX {}: {:?}", decoder_path.display(), e))?
            .into_runnable()
            .map_err(|e| anyhow!("build decoder runnable: {:?}", e))?;

        tracing::info!("MobileSAM: loaded encoder {}, decoder {}", encoder_path.display(), decoder_path.display());

        Ok(Self {
            encoder: encoder,
            decoder: decoder,
        })
    }

    /// Run segmentation with a point prompt.
    /// Returns a single-channel mask at original resolution: 0 = background, 255 = foreground.
    pub fn segment_point(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
        point_x: f32,  // normalized [0, 1]
        point_y: f32,  // normalized [0, 1]
    ) -> Result<Vec<u8>> {
        let expected = (width as usize) * (height as usize) * 3;
        if rgb.len() != expected {
            return Err(anyhow!(
                "rgb buffer length {} != expected {} ({}x{}x3)",
                rgb.len(), expected, width, height
            ));
        }
        if width == 0 || height == 0 {
            return Ok(Vec::new());
        }

        // 1. Encode image
        let image_embedding = self.encode_image(rgb, width, height)?;

        // 2. Prepare point prompt
        let (point_coords, point_labels) = self.prepare_point_prompt(point_x, point_y, width, height)?;

        // 3. Decode mask
        let mask_logits = self.decode_mask(&image_embedding, &point_coords, &point_labels)?;

        // 4. Post-process: sigmoid, threshold, upscale to original resolution
        let mask = self.postprocess_mask(&mask_logits, width, height)?;

        Ok(mask)
    }

    /// Run image encoder on RGB input.
    fn encode_image(&self, rgb: &[u8], width: u32, height: u32) -> Result<tract_onnx::prelude::TValue> {
        let input_tensor = self.preprocess_image(rgb, width, height)?;
        let input: Tensor = input_tensor.into();
        let outputs = self
            .encoder
            .run(tvec!(input.into()))
            .map_err(|e| anyhow!("encoder inference: {:?}", e))?;
        
        // Get image embedding (first output) - return TValue directly
        Ok(outputs[0].clone())
    }

    /// Prepare point prompt for decoder.
    fn prepare_point_prompt(
        &self,
        point_x: f32,
        point_y: f32,
        _orig_width: u32,
        _orig_height: u32,
    ) -> Result<(Tensor, Tensor)> {
        // MobileSAM expects point coordinates in 1024x1024 input space
        let x = point_x * MOBILE_SAM_INPUT_SIZE as f32;
        let y = point_y * MOBILE_SAM_INPUT_SIZE as f32;
        
        // Point coordinates: [1, 1, 2] - batch=1, num_points=1, xy=2
        let point_coords = tract_ndarray::Array3::<f32>::from_shape_vec(
            (1, 1, 2),
            vec![x, y],
        )?;
        
        // Point labels: 1 = foreground, 0 = background
        let point_labels = tract_ndarray::Array2::<f32>::from_shape_vec(
            (1, 1),
            vec![1.0], // foreground
        )?;

        Ok((
            point_coords.into(),
            point_labels.into(),
        ))
    }

    /// Run mask decoder with image embedding and point prompts.
    fn decode_mask(&self, image_embedding: &tract_onnx::prelude::TValue, point_coords: &Tensor, point_labels: &Tensor) -> Result<tract_onnx::prelude::TValue> {
        // Decoder inputs: image_embedding, point_coords, point_labels
        // Also needs: has_mask_input (bool), mask_input (optional)
        let _has_mask_input = Tensor::from(false); // no mask input for first iteration
        let _mask_input = Tensor::zero::<f32>(&[1, 1, 256, 256])?; // dummy
        
        let outputs = self
            .decoder
            .run(tvec!(
                image_embedding.clone().into(),
                point_coords.clone().into(),
                point_labels.clone().into(),
                Tensor::from(false).into(),
                Tensor::zero::<f32>(&[1, 1, 256, 256])?.into(),
            ))
            .map_err(|e| anyhow!("decoder inference: {:?}", e))?;
        
        // First output is mask logits [1, 1, 256, 256]
        Ok(outputs[0].clone())
    }

    /// Post-process mask logits to binary mask at original resolution.
    fn postprocess_mask(&self, mask_logits: &tract_onnx::prelude::TValue, target_width: u32, target_height: u32) -> Result<Vec<u8>> {
        let logits = mask_logits.to_plain_array_view::<f32>()?;
        let logits = logits.into_dimensionality::<tract_ndarray::Ix4>()?;
        // Shape: [1, 1, 256, 256]
        
        let mut small = vec![0u8; (MOBILE_SAM_MASK_SIZE * MOBILE_SAM_MASK_SIZE) as usize];
        for y in 0..MOBILE_SAM_MASK_SIZE as usize {
            for x in 0..MOBILE_SAM_MASK_SIZE as usize {
                let logit = logits[[0, 0, y, x]];
                // Apply sigmoid
                let prob = 1.0 / (1.0 + (-logit).exp());
                small[y * MOBILE_SAM_MASK_SIZE as usize + x] = if prob > 0.5 { 255 } else { 0 };
            }
        }
        
        // Upscale to target resolution
        Ok(upscale_mask(
            &small, 
            MOBILE_SAM_MASK_SIZE, MOBILE_SAM_MASK_SIZE,
            target_width, target_height
        ))
    }

    /// Preprocess image for encoder: resize to 1024x1024, normalize.
    fn preprocess_image(&self, rgb: &[u8], width: u32, height: u32) -> Result<tract_ndarray::Array4<f32>> {
        let mut arr = tract_ndarray::Array4::<f32>::zeros((1, 3, MOBILE_SAM_INPUT_SIZE as usize, MOBILE_SAM_INPUT_SIZE as usize));

        let sx = width as f32 / MOBILE_SAM_INPUT_SIZE as f32;
        let sy = height as f32 / MOBILE_SAM_INPUT_SIZE as f32;

        for dy in 0..MOBILE_SAM_INPUT_SIZE {
            for dx in 0..MOBILE_SAM_INPUT_SIZE {
                let src_x = (dx as f32 + 0.5) * sx - 0.5;
                let src_y = (dy as f32 + 0.5) * sy - 0.5;
                let (r, g, b) = crate::background_removal::bilinear_rgb(rgb, width, height, src_x, src_y);
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

// Note: bilinear_rgb and upscale_mask are re-exported from background_removal module
// to avoid code duplication. See background_removal.rs for implementations.

use crate::background_removal::upscale_mask;