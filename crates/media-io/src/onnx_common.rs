//! Shared ONNX helpers used by the CPU inference loaders.
//!
//! Five loaders (face_detect, scrfd, background_removal, lama, mobile_sam)
//! used to carry their own copies of the ImageNet normalization constants
//! and the bilinear sampling routines. The copies were byte-identical;
//! keeping one definition means a sampling fix reaches every model.
//!
//! Deliberately NOT here: the `RunnableModel` type aliases. tract 0.23
//! exposes two genuinely different runnable plan types -- `InferenceFact`
//! based (background_removal) and `TypedFact` based (the other four) --
//! and collapsing them would require rewriting every `.run()` call.
//! Unifying the loaders themselves is a separate, higher-risk step.

/// ImageNet normalization constants used by u2netp / LaMa / MobileSAM.
pub const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
pub const STD: [f32; 3] = [0.229, 0.224, 0.225];

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
}
