//! Shared ONNX helpers used by the CPU inference loaders.
//!
//! Five loaders (face_detect, scrfd, background_removal, lama, mobile_sam)
//! used to carry their own copies of the ImageNet normalization constants
//! and the bilinear sampling routines. The copies were byte-identical;
//! keeping one definition means a sampling fix reaches every model.
//!
//! Deliberately NOT unified here: the *conversion pipeline* each loader
//! applies after parsing. tract 0.23 exposes two genuinely different
//! runnable plan types -- `InferenceFact` based (`InferenceSimplePlan`,
//! used by background_removal) and `TypedFact` based (the other four) --
//! and the typed loaders further split between pinned+optimized (YuNet,
//! SCRFD, whose Resize nodes need a fixed shape) and plain typed (LaMa,
//! MobileSAM, whose exports already have concrete dims). The shared
//! helpers below remove the duplicated file-check / parse / error
//! boilerplate without changing any loader's pipeline. Forcing one
//! pipeline on all five needs the real model files to validate (they
//! are downloaded on demand, never bundled) and is deferred.

use anyhow::{anyhow, Result};
use std::path::Path;
use tract_onnx::prelude::*;

/// Runnable plan shared by the four TypedFact loaders (face_detect,
/// scrfd, lama, mobile_sam). tract 0.23 wraps the plan in an `Arc` and
/// `run()` borrows it, so the alias carries the `Arc`.
pub type TypedRunnableModel =
    std::sync::Arc<tract_onnx::prelude::RunnableModel<TypedFact, Box<dyn TypedOp>>>;

/// Runnable plan for the one InferenceFact loader (background_removal).
pub type InferenceRunnableModel = std::sync::Arc<tract_onnx::prelude::InferenceSimplePlan>;

/// Parse an ONNX file into an inference model, with the file check and
/// the descriptive load error shared by all five loaders. Each loader
/// supplies a `label` used in messages ("face", "SCRFD", "LaMa", ...).
fn parse_model(path: &Path, label: &str) -> Result<InferenceModel> {
    if !path.is_file() {
        return Err(anyhow!("{label} model not found: {}", path.display()));
    }
    tract_onnx::onnx()
        .model_for_path(path)
        .map_err(|e| {
            anyhow!(
                "load {label} ONNX model {}: {e:?}\n  (if this is an operator tract-onnx 0.23 does not support, check DIRECTIVES section 30 for known model issues)",
                path.display()
            )
        })
}

/// Load a typed model whose input must be pinned to a fixed `f32` NCHW
/// shape before optimization. Used by face_detect and scrfd: their
/// exports contain Resize nodes with a computed `size` input that tract
/// cannot resolve without the pin.
pub fn load_typed_pinned(path: &Path, label: &str, shape: [i64; 4]) -> Result<TypedRunnableModel> {
    parse_model(path, label)?
        .with_input_fact(
            0,
            InferenceFact::dt_shape(
                f32::datum_type(),
                tvec!(shape[0], shape[1], shape[2], shape[3]),
            ),
        )
        .map_err(|e| anyhow!("pin {label} input shape: {e:?}"))?
        .into_optimized()
        .map_err(|e| anyhow!("optimize {label} ONNX model: {e:?}"))?
        .into_runnable()
        .map_err(|e| anyhow!("build {label} runnable plan: {e:?}"))
}

/// Load a typed model with concrete exported dims (no shape pin).
/// Used by lama and mobile_sam. `typed_hint` is appended verbatim to a
/// type-inference failure so loaders can keep model-specific recovery
/// guidance (e.g. MobileSAM's dynamic-dimension re-export note).
pub fn load_typed(path: &Path, label: &str, typed_hint: &str) -> Result<TypedRunnableModel> {
    parse_model(path, label)?
        .into_typed()
        .map_err(|e| anyhow!("type inference {label}: {e:?}{typed_hint}"))?
        .into_runnable()
        .map_err(|e| anyhow!("build {label} runnable plan: {e:?}"))
}

/// Load an inference-fact plan (no type conversion). Used by
/// background_removal's u2netp export, which runs as an
/// `InferenceSimplePlan`.
pub fn load_inference(path: &Path, label: &str) -> Result<InferenceRunnableModel> {
    parse_model(path, label)?
        .into_runnable()
        .map_err(|e| anyhow!("build {label} runnable plan: {e:?}"))
}

/// ImageNet normalization constants used by u2netp / LaMa / MobileSAM.
pub const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
pub const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// Return the first model output, or a descriptive error if the model
/// produced none. Several loaders index `outputs[0]` unguarded; a model
/// file with an unexpected output signature otherwise panics on the UI
/// thread (1.4).
pub fn first_output<I>(outputs: I, model: &str) -> Result<TValue>
where
    I: IntoIterator<Item = TValue>,
{
    outputs
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("{model} produced no outputs; check the model file"))
}

/// Bilinear sample an RGB image at floating-point `(sx, sy)`.
///
/// `rgb` is a tightly packed `width * height * 3` buffer. Coordinates
/// outside the image clamp to the edge, so a sample just past the border
/// never reads out of bounds.
pub fn bilinear_rgb(rgb: &[u8], width: u32, height: u32, sx: f32, sy: f32) -> (f32, f32, f32) {
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

    let top_r = r00 * (1.0 - fx) + r10 * fx;
    let bot_r = r01 * (1.0 - fx) + r11 * fx;
    let top_g = g00 * (1.0 - fx) + g10 * fx;
    let bot_g = g01 * (1.0 - fx) + g11 * fx;
    let top_b = b00 * (1.0 - fx) + b10 * fx;
    let bot_b = b01 * (1.0 - fx) + b11 * fx;

    (
        top_r * (1.0 - fy) + bot_r * fy,
        top_g * (1.0 - fy) + bot_g * fy,
        top_b * (1.0 - fy) + bot_b * fy,
    )
}

/// Resize a single-channel mask from `(sw, sh)` to `(dw, dh)` with
/// bilinear sampling. Used for segmentation masks both before compositing
/// and when the model ran at a reduced resolution.
pub fn upscale_mask(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
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
        assert_eq!(out[0], 0);
        assert_eq!(out[3], 255);
        assert_eq!(out[12], 128);
        assert_eq!(out[15], 64);
    }

    #[test]
    fn bilinear_rgb_clamps_at_edges() {
        // A 1x1 image: any coordinate must return its only pixel,
        // including samples just outside the nominal range.
        let rgb = [10u8, 20, 30];
        assert_eq!(bilinear_rgb(&rgb, 1, 1, -1.0, 2.0), (10.0, 20.0, 30.0));
    }

    #[test]
    fn first_output_rejects_empty_model_output() {
        let err = first_output(Vec::new(), "LaMa").unwrap_err().to_string();
        assert!(err.contains("LaMa"), "error must name the model: {err}");
        assert!(err.contains("no outputs"), "error must explain: {err}");
    }

    #[test]
    fn load_typed_pinned_missing_file_names_model_and_label() {
        let path = Path::new("definitely-not-a-real-model.onnx");
        let err = load_typed_pinned(path, "face", [1, 3, 4, 4])
            .unwrap_err()
            .to_string();
        assert!(err.contains("face"), "error must name label: {err}");
        assert!(err.contains("not found"), "error must explain: {err}");
        assert!(
            err.contains(path.to_str().unwrap()),
            "error must show path: {err}"
        );
    }

    #[test]
    fn load_typed_missing_file_names_model_and_label() {
        let path = Path::new("definitely-not-a-real-model.onnx");
        let err = load_typed(path, "MobileSAM encoder", "")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("MobileSAM encoder"),
            "error must name label: {err}"
        );
        assert!(err.contains("not found"), "error must explain: {err}");
    }

    #[test]
    fn load_inference_missing_file_names_model_and_label() {
        let path = Path::new("definitely-not-a-real-model.onnx");
        let err = load_inference(path, "u2netp").unwrap_err().to_string();
        assert!(err.contains("u2netp"), "error must name label: {err}");
        assert!(err.contains("not found"), "error must explain: {err}");
    }
}
