//! GFPGAN v1.4 face restoration via tract-onnx.
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

use crate::onnx_common::{first_output, load_typed_pinned, TypedRunnableModel as RunnableModel};
use anyhow::{anyhow, Result};
use std::path::Path;
use tract_onnx::prelude::*;

/// Fixed input resolution of the GFPGAN v1.4 ONNX model.
pub const GFPGAN_INPUT_SIZE: u32 = 512;

/// Loaded GFPGAN model. Construct once and reuse across frames --
/// `load` parses and optimizes the ONNX graph, which is slow relative
/// to a single inference pass.
#[derive(Debug)]
pub struct GfpganRestorer {
    model: RunnableModel,
}

impl GfpganRestorer {
    /// Load and optimize a GFPGAN ONNX model from disk.
    ///
    /// Pins the input shape to `[1, 3, 512, 512]` so tract can resolve
    /// any Resize nodes that were exported with a computed `size` input.
    pub fn load(model_path: &Path) -> Result<Self> {
        let model = load_typed_pinned(
            model_path,
            "GFPGAN v1.4",
            [1, 3, GFPGAN_INPUT_SIZE as i64, GFPGAN_INPUT_SIZE as i64],
        )?;
        tracing::info!("gfpgan: loaded {}", model_path.display());
        Ok(Self { model })
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
        let input: Tensor = tensor.into();
        let outputs = self
            .model
            .run(tvec!(input.into()))
            .map_err(|e| anyhow!("GFPGAN inference: {e:?}"))?;

        // 3. Postprocess: first output is restored RGB [1, 3, 512, 512].
        let output = first_output(outputs, "GFPGAN")?.into_tensor();
        self.postprocess(&output)
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

    /// Postprocess output tensor to RGB bytes at 512x512.
    ///
    /// Converts from BGR CHW to RGB HWC and maps -1..1 -> 0..255.
    fn postprocess(&self, output: &Tensor) -> Result<Vec<u8>> {
        let output = output.to_plain_array_view::<f32>()?;
        let output = output.into_dimensionality::<tract_ndarray::Ix4>()?;
        // Shape: [1, 3, 512, 512] BGR.

        let mut result = vec![0u8; (GFPGAN_INPUT_SIZE * GFPGAN_INPUT_SIZE * 3) as usize];

        // Convert BGR CHW -> RGB HWC, map -1..1 -> 0..255.
        for c in 0..3 {
            let rgb_channel = 2 - c; // BGR 0,1,2 -> RGB 2,1,0
            for y in 0..GFPGAN_INPUT_SIZE as usize {
                for x in 0..GFPGAN_INPUT_SIZE as usize {
                    let val = output[[0, c, y, x]];
                    let byte = ((val + 1.0) * 127.5).clamp(0.0, 255.0).round() as u8;
                    let dst_idx = (y * GFPGAN_INPUT_SIZE as usize + x) * 3 + rgb_channel;
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
        let err = GfpganRestorer::load(path).unwrap_err().to_string();
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
        let restorer = GfpganRestorer::load(path).unwrap();
        let rgb = vec![128u8; 64 * 64 * 3];
        let out = restorer.restore(&rgb, 64, 64).unwrap();
        assert_eq!(out.len(), 512 * 512 * 3);
        assert!(out.iter().any(|&v| v != 0), "output must not be all black");
    }
}
