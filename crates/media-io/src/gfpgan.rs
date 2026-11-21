//! GFPGAN v1.4 face restoration via tract-onnx (CPU) or ONNX Runtime (GPU).
//!
//! Pure-Rust inference for the GFPGAN v1.4 ONNX model. Takes a full
//! frame (or face crop), restores facial detail while preserving
//! identity, expression and mouth motion — suitable for talking-head
//! footage and AI-generated clips with garbled facial features.
//!
//! Verified ONNX contract (probed against the real GFPGANv1.4.onnx):
//! Input:  [1, 3, 512, 512] f32, **BGR**, values **-1..1** with
//!         `(v - 127.5) / 127.5`. Raw 0..255 input produces garbage
//!         outputs of magnitude 1e17 -- normalization is mandatory.
//! Output: [1, 3, 512, 512] f32, **BGR**, values -1..1. Convert with
//!         `(v + 1) * 127.5` clamped to 0..255, then swap channels
//!         0 <-> 2 to get RGB. (Verified visually: without the swap the
//!         restored face comes out blue.)
//!
//! The model file (`GFPGANv1.4.onnx`) is downloaded on demand from
//! the model registry (`gfpgan-v1.4`, ModelKind::Enhancer) and lives
//! next to the Whisper and Real-ESRGAN models.
//!
//! Usage note: `restore()` returns a 512x512 RGB image. For full-frame
//! video, detect faces with YuNet/SCRFD, crop each face box with
//! padding, run `restore()` on each crop, and paste the result back
//! into the original frame with alpha blending. The paste-back step
//! is intentionally not part of this module so the restorer stays
//! frame-agnostic.

use crate::inference_backend::{BackendType, InferenceSession};
use anyhow::{anyhow, Result};
use std::path::Path;
use tract_onnx::prelude::tract_ndarray;

/// Fixed input resolution of the GFPGAN v1.4 ONNX model.
pub const GFPGAN_INPUT_SIZE: u32 = 512;

/// Loaded GFPGAN model. Construct once and reuse across frames --
/// `load` parses and optimizes the ONNX graph, which is slow relative
/// to a single inference pass.
pub struct GfpganRestorer {
    session: InferenceSession,
}

impl std::fmt::Debug for GfpganRestorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GfpganRestorer")
            .field("input_shape", &self.session.shape())
            .finish()
    }
}

impl GfpganRestorer {
    /// Load and optimize a GFPGAN ONNX model from disk.
    ///
    /// `backend` selects the inference backend: CPU (tract-onnx),
    /// CUDA (NVIDIA GPU via ONNX Runtime), or DirectML (AMD GPU via
    /// ONNX Runtime). Falls back to CPU if the GPU backend cannot load.
    pub fn load(model_path: &Path, backend: BackendType) -> Result<Self> {
        let session = InferenceSession::load(
            model_path,
            [1, 3, GFPGAN_INPUT_SIZE as i64, GFPGAN_INPUT_SIZE as i64],
            "GFPGAN v1.4",
            backend,
        )?;
        tracing::info!(
            "gfpgan: loaded {} on {:?} backend",
            model_path.display(),
            backend
        );
        Ok(Self { session })
    }

    /// Restore faces in an RGB frame at `width x height`.
    ///
    /// Resizes to 512x512 via bilinear sampling, runs GFPGAN inference,
    /// returns restored RGB bytes at 512x512 resolution. Caller is
    /// responsible for pasting restored regions back into the original
    /// frame (e.g. over detected face boxes).
    pub fn restore(&self, rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
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

        // 1. Preprocess: resize to 512x512, BGR channel order, 0..255.
        let tensor = self.preprocess(rgb, width, height)?;

        // 2. Run inference.
        let output_flat = self.session.run_array_single(&tensor)?;

        // 3. Postprocess: output is [1, 3, 512, 512] BGR in NCHW order.
        self.postprocess(&output_flat)
    }

    /// Preprocess RGB into a (1, 3, 512, 512) CHW tensor, BGR channel
    /// order, values -1..1. The model was verified to require
    /// `(v - 127.5) / 127.5`; feeding raw 0..255 yields nonsense.
    fn preprocess(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<tract_ndarray::Array4<f32>> {
        let mut arr = tract_ndarray::Array4::<f32>::zeros((
            1,
            3,
            GFPGAN_INPUT_SIZE as usize,
            GFPGAN_INPUT_SIZE as usize,
        ));

        let sx = width as f32 / GFPGAN_INPUT_SIZE as f32;
        let sy = height as f32 / GFPGAN_INPUT_SIZE as f32;

        for dy in 0..GFPGAN_INPUT_SIZE {
            for dx in 0..GFPGAN_INPUT_SIZE {
                let src_x = (dx as f32 + 0.5) * sx - 0.5;
                let src_y = (dy as f32 + 0.5) * sy - 0.5;
                let (r, g, b) = crate::onnx_common::bilinear_rgb(rgb, width, height, src_x, src_y);
                // BGR channels, normalized to -1..1.
                arr[[0, 0, dy as usize, dx as usize]] = (b - 127.5) / 127.5;
                arr[[0, 1, dy as usize, dx as usize]] = (g - 127.5) / 127.5;
                arr[[0, 2, dy as usize, dx as usize]] = (r - 127.5) / 127.5;
            }
        }

        Ok(arr)
    }

    /// Postprocess flat output to RGB bytes at 512x512.
    ///
    /// Input is flat `Vec<f32>` in NCHW order `[1, 3, 512, 512]`, BGR
    /// channels. Converts to RGB HWC and maps -1..1 -> 0..255.
    fn postprocess(&self, output: &[f32]) -> Result<Vec<u8>> {
        let size = GFPGAN_INPUT_SIZE as usize;
        let expected = 3 * size * size;
        if output.len() != expected {
            return Err(anyhow!(
                "GFPGAN output length {} != expected {}",
                output.len(),
                expected
            ));
        }

        let mut result = vec![0u8; size * size * 3];

        // Convert BGR CHW -> RGB HWC, map -1..1 -> 0..255.
        // output layout: [c][y][x] where c=0 is B, c=1 is G, c=2 is R.
        for c in 0..3usize {
            let rgb_channel = 2 - c; // BGR 0,1,2 -> RGB 2,1,0
            for y in 0..size {
                for x in 0..size {
                    let idx = (c * size + y) * size + x;
                    let val = output[idx];
                    let byte = ((val + 1.0) * 127.5).clamp(0.0, 255.0).round() as u8;
                    let dst_idx = (y * size + x) * 3 + rgb_channel;
                    result[dst_idx] = byte;
                }
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gfpgan_input_size_is_512() {
        assert_eq!(GFPGAN_INPUT_SIZE, 512);
    }

    /// Missing-file test mirrors onnx_common's pattern: names model
    /// and label in the error message.
    #[test]
    fn load_missing_file_names_model_and_label() {
        let path = std::path::Path::new("definitely-not-a-real-gfpgan.onnx");
        let err = GfpganRestorer::load(path, BackendType::Cpu)
            .unwrap_err()
            .to_string();
        assert!(err.contains("GFPGAN v1.4"), "error must name label: {err}");
        assert!(err.contains("not found"), "error must explain: {err}");
    }

    /// With the real model installed, verify a single inference returns
    /// a finite 512x512 RGB image. Skipped on CI (models never bundled).
    #[test]
    fn restore_real_model_returns_finite_rgb() {
        let path = std::path::Path::new(r"F:\Minimax H3\Git NEW APP\Models\gfpgan-v1.4.onnx");
        if !path.is_file() {
            return;
        }
        let restorer = GfpganRestorer::load(path, BackendType::Cpu).unwrap();
        let rgb = vec![128u8; 64 * 64 * 3];
        let out = restorer.restore(&rgb, 64, 64).unwrap();
        assert_eq!(out.len(), 512 * 512 * 3);
        assert!(out.iter().any(|&v| v != 0), "output must not be all black");
    }
}
