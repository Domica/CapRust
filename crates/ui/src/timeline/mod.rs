pub mod ruler;
pub mod toolbar;
pub mod track_header;

pub use toolbar::{TimelineToolEvents, TimelineToolState};

// --- Timeline audio envelope helpers (Issue #14) ---

/// Number of samples used when building the automation polyline cache.
/// Fixed count = cache is independent of zoom / scroll; only the
/// projection to screen coordinates happens every frame.
pub const ENVELOPE_SAMPLES: usize = 128;

/// Map a dB value to a normalized vertical position inside a clip rect.
///
/// 0.0 = bottom of rect, 1.0 = top. CapCut-style scale:
///   +12 dB -> 1.00 (top)
///     0 dB -> 0.60
///   -60 dB -> 0.00 (bottom)
pub fn db_to_normalized_y(db: f32) -> f32 {
    let d = db.clamp(-60.0, 12.0);
    if d >= 0.0 {
        0.60 + (d / 12.0) * 0.40
    } else {
        0.60 + (d / 60.0) * 0.60
    }
}

/// Clamp a dB value to the displayable range.
pub fn clamp_db(db: f32) -> f32 {
    db.clamp(-60.0, 12.0)
}

/// Shaded region where ducking applies. Pixel offsets from the clip
/// rect's left edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DuckZone {
    pub x1_px: f32,
    pub x2_px: f32,
    /// Fill opacity for this zone, 0.0..=1.0. Derived from the clip's
    /// `duck_reduction_db` via `duck_zone_opacity`.
    pub opacity: f32,
}

/// Compute the duck zones for `clip` given the whole clip list.
/// A zone exists wherever the control clip (`clip.duck_against`)
/// overlaps this clip on the timeline. Returns an empty vec when
/// there is no control, or no overlap.
/// Map a duck reduction (dB, negative) to a fill opacity.
/// Bigger reduction -> more opaque. Clamped to [0.10, 0.45] so the
/// shading never dominates the waveform underneath.
fn duck_zone_opacity(reduction_db: f32) -> f32 {
    (reduction_db.abs() / 30.0).clamp(0.10, 0.45)
}

pub fn compute_duck_zones(
    clip: &caprust_core::clip::Clip,
    all_clips: &[caprust_core::clip::Clip],
    px_per_ms: f32,
) -> Vec<DuckZone> {
    let Some(control_id) = clip.duck_against else {
        return Vec::new();
    };
    let Some(control) = all_clips.iter().find(|c| c.id == control_id) else {
        return Vec::new();
    };
    let clip_start = clip.start_time_ms as i64;
    let clip_end = clip_start + clip.duration_ms as i64;
    let ctrl_start = control.start_time_ms as i64;
    let ctrl_end = ctrl_start + control.duration_ms as i64;
    let overlap_start = clip_start.max(ctrl_start);
    let overlap_end = clip_end.min(ctrl_end);
    if overlap_start >= overlap_end {
        return Vec::new();
    }
    let x1 = (overlap_start - clip_start) as f32 * px_per_ms;
    let x2 = (overlap_end - clip_start) as f32 * px_per_ms;
    let opacity = duck_zone_opacity(clip.duck_reduction_db);
    vec![DuckZone {
        x1_px: x1,
        x2_px: x2,
        opacity,
    }]
}

/// Sample the clip's volume automation at `ENVELOPE_SAMPLES + 1` evenly
/// spaced fractions of its duration. Returns dB values.
///
/// Cache the result and invalidate on `ProjectState::render_hash()`
/// change. The samples are geometry-independent (fractions, not pixels),
/// so scroll / zoom do not invalidate the cache.
pub fn sample_envelope_db(clip: &caprust_core::clip::Clip) -> Vec<f32> {
    let dur = clip.duration_ms as i64;
    let n = ENVELOPE_SAMPLES;
    let mut out = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let frac = i as f64 / n as f64;
        let t_ms = (frac * dur as f64).round() as i64;
        let db = caprust_core::clip::sample_volume_at(&clip.volume_keyframes, clip.volume_db, t_ms);
        out.push(db);
    }
    out
}

/// Draw the automation polyline and duck zones onto `painter`.
/// Duck zones are drawn first, then the polyline on top.
/// `samples_db` must have at least 2 entries (see `sample_envelope_db`).
pub fn draw_audio_envelope(
    painter: &egui::Painter,
    rect: egui::Rect,
    samples_db: &[f32],
    duck_zones: &[DuckZone],
    line_color: egui::Color32,
    duck_zone_color: egui::Color32,
) {
    if rect.width() < 4.0 || rect.height() < 4.0 {
        return;
    }
    let clipped = painter.with_clip_rect(rect);

    // Duck zones (behind the polyline).
    for z in duck_zones {
        let left = rect.left() + z.x1_px;
        let right = rect.left() + z.x2_px;
        let r = egui::Rect::from_min_max(
            egui::pos2(left, rect.top()),
            egui::pos2(right, rect.bottom()),
        );
        let fill = duck_zone_color.gamma_multiply(z.opacity);
        clipped.rect_filled(r, 0.0, fill);
    }

    if samples_db.len() < 2 {
        return;
    }
    let n = samples_db.len();
    let mut points = Vec::with_capacity(n);
    for (i, db) in samples_db.iter().enumerate() {
        let x = rect.left() + (i as f32 / (n - 1) as f32) * rect.width();
        let norm_y = db_to_normalized_y(clamp_db(*db));
        let y = rect.bottom() - norm_y * rect.height();
        points.push(egui::pos2(x, y));
    }
    clipped.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.5_f32, line_color),
    ));
}

#[cfg(test)]
mod overlay_tests {
    use super::*;

    #[test]
    fn zero_db_is_sixty_percent() {
        assert!((db_to_normalized_y(0.0) - 0.60).abs() < 1e-4);
    }

    #[test]
    fn plus_twelve_is_top() {
        assert!((db_to_normalized_y(12.0) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn minus_sixty_is_bottom() {
        assert!((db_to_normalized_y(-60.0) - 0.0).abs() < 1e-4);
    }

    #[test]
    fn plus_twelve_is_top_and_clamps() {
        // anything above +12 dB must clamp to the top
        assert!((db_to_normalized_y(999.0) - 1.0).abs() < 1e-4);
        // anything below -60 dB must clamp to the bottom
        assert!((db_to_normalized_y(-999.0) - 0.0).abs() < 1e-4);
    }

    #[test]
    fn duck_opacity_low_reduction_hits_floor() {
        assert!((duck_zone_opacity(-3.0) - 0.10).abs() < 1e-4);
    }

    #[test]
    fn duck_opacity_high_reduction_hits_ceiling() {
        assert!((duck_zone_opacity(-30.0) - 0.45).abs() < 1e-4);
        assert!((duck_zone_opacity(-60.0) - 0.45).abs() < 1e-4);
    }

    #[test]
    fn duck_opacity_midpoint_linear() {
        // -9 dB -> 0.30 (linear region, before clamping)
        assert!((duck_zone_opacity(-9.0) - 0.30).abs() < 1e-4);
    }

    #[test]
    fn clamp_db_bounds() {
        assert_eq!(clamp_db(99.0), 12.0);
        assert_eq!(clamp_db(-99.0), -60.0);
    }
}
