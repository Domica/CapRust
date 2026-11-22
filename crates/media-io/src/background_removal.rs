//! Background removal via u2netp (Phase P3b).
//!
//! Inference via `InferenceSession` which supports tract-onnx (CPU)
//! or ONNX Runtime (CUDA / DirectML). The model file is downloaded on
//! demand from the registry (`ModelKind::BackgroundRemover`).
//!
//! Input:  [1, 3, 320, 320] f32, RGB, ImageNet-normalized
//!         (px/255 - mean) / std, mean=[0.485, 0.456, 0.406],
//!         std=[0.229, 0.224, 0.225]. Unlike YuNet, u2netp is a
//!         PyTorch model: RGB channel order, not BGR.
//! Output: 7 tensors d0..d6 (one per U-block level). Only d0 is at
//!         full 320x320 resolution and is the one that matters; the
//!         others are supervision outputs. We locate d0 by output size
//!         (102_400 f32 elements = [1,1,320,320]).
//!
//! infer_mask returns a single-channel alpha mask at the *original*
//! input resolution: 0 = background, 255 = foreground. Callers feed
//! this into the render graph as a mask for `alphamerge` /
//! `alphaextract`.

use crate::inference_backend::{BackendType, InferenceSession};
use crate::onnx_common::{bilinear_rgb, upscale_mask, MEAN, STD};
use anyhow::{anyhow, Result};
use std::path::Path;
use tract_onnx::prelude::tract_ndarray;

/// Fixed input resolution of the u2netp ONNX model.
pub const INPUT_SIZE: u32 = 320;

/// d0 output element count: [1, 1, 320, 320] = 102_400 f32 values.
/// All other u2netp outputs (d1..d6) are smaller supervision tensors,
/// so we identify d0 by being the largest output.
const D0_ELEMENT_COUNT: usize = INPUT_SIZE as usize * INPUT_SIZE as usize;

/// Loaded u2netp model. Construct once and reuse across frames --
/// `load` parses and optimizes the ONNX graph, which is slow relative
/// to a single inference pass.
pub struct BackgroundRemover {
    session: InferenceSession,
}

impl std::fmt::Debug for BackgroundRemover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackgroundRemover")
            .field("input_shape", &self.session.shape())
            .field("backend", &self.session.backend_type())
            .finish()
    }
}

impl BackgroundRemover {
    /// Load and optimize a u2netp ONNX model from disk.
    ///
    /// `backend` selects the inference backend: CPU (tract-onnx),
    /// CUDA (NVIDIA GPU via ONNX Runtime), or DirectML (AMD GPU via
    /// ONNX Runtime). Falls back to CPU if the GPU backend cannot load.
    pub fn load(model_path: &Path, backend: BackendType) -> Result<Self> {
        let session = InferenceSession::load(
            model_path,
            [1, 3, INPUT_SIZE as i64, INPUT_SIZE as i64],
            "u2netp",
            backend,
        )?;
        tracing::info!(
            "background_removal: loaded {} on {:?} backend",
            model_path.display(),
            session.backend_type()
        );
        Ok(Self { session })
    }

    /// Returns the inference backend actually in use for background
    /// removal (after Auto resolution and any CPU fallback).
    pub fn backend_type(&self) -> BackendType {
        self.session.backend_type()
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

        // Run inference. u2netp produces 7 outputs; d0 is the full-res
        // alpha at [1,1,320,320] = 102_400 f32 elements. We find it by
        // size since the trait returns flat Vec<f32> without shape info.
        let outputs = self.session.run_array(&tensor)?;
        let d0 = find_d0(&outputs)?;

        // The model already applies sigmoid on d0; values are in
        // 0..1. Scale to 0..255 and resample up to the caller's size.
        let mut small = vec![0u8; D0_ELEMENT_COUNT];
        for (i, &v) in d0.iter().enumerate().take(D0_ELEMENT_COUNT) {
            small[i] = v.clamp(0.0, 1.0).round() as u8 * 255;
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
/// u2netp outputs. We identify it by element count
/// (`[1, 1, 320, 320]` = 102_400 f32 values) rather than assuming
/// output index 0, so a re-export that reorders the outputs still
/// works.
fn find_d0(outputs: &[Vec<f32>]) -> Result<&Vec<f32>> {
    for output in outputs.iter() {
        if output.len() == D0_ELEMENT_COUNT {
            return Ok(output);
        }
    }
    Err(anyhow!(
        "u2netp: no output with {} elements ([1, 1, {}, {}]) found among {} tensors",
        D0_ELEMENT_COUNT,
        INPUT_SIZE,
        INPUT_SIZE,
        outputs.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let model = BackgroundRemover::load(&path, BackendType::Cpu).unwrap();
        let black = vec![0u8; 320 * 320 * 3];
        let mask = model.infer_mask(&black, 320, 320).expect("inference");
        assert_eq!(mask.len(), 320 * 320);
        // A black frame is unambiguously "no salient object"; the
        // mean alpha must be very low. Exact 0 is not guaranteed
        // because the network has biases.
        let mean: u32 = mask.iter().map(|&v| v as u32).sum::<u32>() / mask.len() as u32;
        assert!(mean < 32, "mean alpha on black frame was {mean}");
    }

    /// Missing-file test mirrors onnx_common's pattern: names model
    /// and label in the error message.
    #[test]
    fn load_missing_file_names_model_and_label() {
        let path = std::path::Path::new("definitely-not-a-real-u2netp.onnx");
        let err = BackgroundRemover::load(path, BackendType::Cpu)
            .unwrap_err()
            .to_string();
        assert!(err.contains("u2netp"), "error must name label: {err}");
        assert!(err.contains("not found"), "error must explain: {err}");
    }
}
