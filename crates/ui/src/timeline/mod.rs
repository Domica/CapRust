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

/// Shaded region where ducking applies, as fractions (0.0..=1.0) of the
/// clip's duration. Fractions, not pixels: the cache is keyed on
/// `render_hash()` only, so nothing zoom-dependent may be stored in it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DuckZone {
    pub start_frac: f32,
    pub end_frac: f32,
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
    let dur = clip.duration_ms.max(1) as f32;
    let opacity = duck_zone_opacity(clip.duck_reduction_db);
    vec![DuckZone {
        start_frac: (overlap_start - clip_start) as f32 / dur,
        end_frac: (overlap_end - clip_start) as f32 / dur,
        opacity,
    }]
}

/// One point of the volume polyline: `(fraction of clip duration, dB)`.
pub type EnvelopeSample = (f32, f32);

/// Sample the clip's volume automation as `(fraction of clip duration,
/// dB)` points: `ENVELOPE_SAMPLES + 1` evenly spaced ones, plus one at
/// every keyframe inside the clip. The keyframe points are what make the
/// drawn line exact where the curve bends; on a long clip the even
/// spacing alone is several seconds wide and steps over short ramps.
///
/// Cache the result and invalidate on `ProjectState::render_hash()`
/// change. The samples are geometry-independent (fractions, not pixels),
/// so scroll / zoom do not invalidate the cache.
pub fn sample_envelope_db(clip: &caprust_core::clip::Clip) -> Vec<EnvelopeSample> {
    let dur = clip.duration_ms as i64;
    let n = ENVELOPE_SAMPLES;
    let mut times: Vec<i64> = (0..=n)
        .map(|i| (i as f64 / n as f64 * dur as f64).round() as i64)
        .collect();
    times.extend(
        clip.volume_keyframes
            .iter()
            .map(|k| k.t_ms as i64)
            .filter(|&t| t > 0 && t < dur),
    );
    times.sort_unstable();
    times.dedup();
    times
        .into_iter()
        .map(|t_ms| {
            let db =
                caprust_core::clip::sample_volume_at(&clip.volume_keyframes, clip.volume_db, t_ms);
            (t_ms as f32 / dur.max(1) as f32, db)
        })
        .collect()
}

/// Draw the automation polyline and duck zones onto `painter`.
/// Duck zones are drawn first, then the polyline on top.
/// `samples_db` is `(fraction, dB)` pairs with at least 2 entries (see
/// `sample_envelope_db`).
///
/// `rect` is the clip's full (unclipped) rect, so a clip scrolled partly
/// out of view keeps the overlay aligned with the time axis; `visible` is
/// the part of it that is actually on screen.
pub fn draw_audio_envelope(
    painter: &egui::Painter,
    rect: egui::Rect,
    visible: egui::Rect,
    samples_db: &[EnvelopeSample],

    duck_zones: &[DuckZone],
    line_color: egui::Color32,
    duck_zone_color: egui::Color32,
) {
    if rect.width() < 4.0 || rect.height() < 4.0 {
        return;
    }
    let clipped = painter.with_clip_rect(visible);

    // Duck zones (behind the polyline).
    for z in duck_zones {
        let left = rect.left() + z.start_frac * rect.width();
        let right = rect.left() + z.end_frac * rect.width();
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
    let mut points = Vec::with_capacity(samples_db.len());
    for &(frac, db) in samples_db {
        let x = rect.left() + frac * rect.width();
        let norm_y = db_to_normalized_y(clamp_db(db));
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

    fn kf(t_ms: u64, gain_db: f32) -> caprust_core::clip::VolumeKeyframe {
        caprust_core::clip::VolumeKeyframe { t_ms, gain_db }
    }

    fn clip_with(dur_ms: u64, kfs: Vec<caprust_core::clip::VolumeKeyframe>) -> caprust_core::Clip {
        let mut c = caprust_core::Clip::new_audio("a.wav", 0, 0, dur_ms);
        c.volume_keyframes = kfs;
        c
    }

    /// dB the drawn polyline shows at `frac` of the clip.
    fn drawn_db(samples: &[(f32, f32)], frac: f64) -> f64 {
        let i = samples
            .windows(2)
            .position(|w| frac < w[1].0 as f64)
            .unwrap_or(samples.len() - 2);
        let (a, b) = (samples[i], samples[i + 1]);
        let f = (frac - a.0 as f64) / (b.0 - a.0) as f64;
        a.1 as f64 + (b.1 - a.1) as f64 * f
    }

    #[test]
    fn polyline_follows_the_curve_on_a_long_clip() {
        // A 3 s dip (1 s down, 1 s hold, 1 s up) in a 10 min clip. The even
        // samples are ~4.7 s apart: one lands at 103.1 s (before the dip)
        // and one at 107.8 s (on the way back up), so on their own they
        // step over the hold at -40 dB.
        let clip = clip_with(
            600_000,
            vec![
                kf(0, 0.0),
                kf(105_000, 0.0),
                kf(106_000, -40.0),
                kf(107_000, -40.0),
                kf(108_000, 0.0),
            ],
        );
        let s = sample_envelope_db(&clip);
        let mut worst = 0.0f64;
        for t in (0..600_000u64).step_by(250) {
            let truth =
                caprust_core::clip::sample_volume_at(&clip.volume_keyframes, 0.0, t as i64) as f64;
            worst = worst.max((drawn_db(&s, t as f64 / 600_000.0) - truth).abs());
        }
        assert!(worst < 0.05, "polyline is off the curve by {worst:.2} dB");
    }

    #[test]
    fn every_keyframe_inside_the_clip_is_a_vertex() {
        let clip = clip_with(10_000, vec![kf(1234, -3.0), kf(5678, 6.0), kf(9001, -20.0)]);
        let s = sample_envelope_db(&clip);
        for k in &clip.volume_keyframes {
            let want = (k.t_ms as f32 / 10_000.0, k.gain_db);
            assert!(s.contains(&want), "missing vertex {want:?}");
        }
    }

    #[test]
    fn keyframes_at_or_beyond_the_clip_edges_add_no_vertices() {
        let clip = clip_with(10_000, vec![kf(0, -6.0), kf(10_000, 0.0), kf(50_000, 6.0)]);
        let s = sample_envelope_db(&clip);
        assert_eq!(s.len(), ENVELOPE_SAMPLES + 1);
        assert_eq!(s.first().unwrap().0, 0.0);
        assert_eq!(s.last().unwrap().0, 1.0);
    }

    #[test]
    fn fractions_stay_strictly_increasing_with_unsorted_and_duplicate_keyframes() {
        // 5000 coincides with the middle even sample (i = 64): it must
        // not produce a second point at the same x.
        let clip = clip_with(
            10_000,
            vec![
                kf(9000, -6.0),
                kf(5000, 0.0),
                kf(5000, -12.0),
                kf(2000, 3.0),
            ],
        );
        let s = sample_envelope_db(&clip);
        assert!(s.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(s.iter().all(|&(x, db)| x.is_finite() && db.is_finite()));
    }

    #[test]
    fn no_keyframes_is_flat_at_the_clip_volume() {
        let mut clip = clip_with(4000, vec![]);
        clip.volume_db = -9.0;
        let s = sample_envelope_db(&clip);
        assert!(s.len() >= 2 && s.iter().all(|&(_, db)| db == -9.0));
    }

    #[test]
    fn zero_length_clip_stays_finite() {
        let s = sample_envelope_db(&clip_with(0, vec![kf(0, -6.0), kf(500, 0.0)]));
        assert!(s.iter().all(|&(x, db)| x.is_finite() && db.is_finite()));
    }

    // ---- #25: fraction-based duck zones + full/visible rect ----

    fn audio(start: u64, dur: u64) -> caprust_core::clip::Clip {
        caprust_core::clip::Clip::new_audio("a.wav", 0, start, dur)
    }

    fn ducked_by(
        ctrl: &caprust_core::clip::Clip,
        start: u64,
        dur: u64,
    ) -> caprust_core::clip::Clip {
        let mut c = audio(start, dur);
        c.duck_against = Some(ctrl.id);
        c.duck_reduction_db = -12.0;
        c
    }

    #[test]
    fn duck_zone_is_a_fraction_of_the_clip() {
        // Clip spans 1000..3000, control 2000..2500 -> the zone covers
        // 0.5..0.75 of the clip, whatever the zoom.
        let ctrl = audio(2000, 500);
        let clip = ducked_by(&ctrl, 1000, 2000);
        let zones = compute_duck_zones(&clip, &[clip.clone(), ctrl]);
        assert_eq!(zones.len(), 1);
        assert!((zones[0].start_frac - 0.5).abs() < 1e-6);
        assert!((zones[0].end_frac - 0.75).abs() < 1e-6);
    }

    #[test]
    fn duck_zone_is_clipped_to_the_clip_span() {
        // Control starts before and ends after the clip.
        let ctrl = audio(0, 10_000);
        let clip = ducked_by(&ctrl, 1000, 2000);
        let zones = compute_duck_zones(&clip, &[clip.clone(), ctrl]);
        assert_eq!(zones.len(), 1);
        assert_eq!((zones[0].start_frac, zones[0].end_frac), (0.0, 1.0));
    }

    #[test]
    fn no_duck_zone_without_overlap_or_control() {
        let ctrl = audio(5000, 1000);
        let clip = ducked_by(&ctrl, 1000, 2000);
        assert!(compute_duck_zones(&clip, &[clip.clone(), ctrl.clone()]).is_empty());
        // Touching edges do not overlap.
        let touching = audio(3000, 1000);
        let clip = ducked_by(&touching, 1000, 2000);
        assert!(compute_duck_zones(&clip, &[clip.clone(), touching]).is_empty());
        // No `duck_against`, and a `duck_against` that no longer exists.
        let mut clip = audio(1000, 2000);
        assert!(compute_duck_zones(&clip, &[clip.clone()]).is_empty());
        clip.duck_against = Some(ctrl.id);
        assert!(compute_duck_zones(&clip, &[clip.clone()]).is_empty());
    }

    /// Shapes `draw_audio_envelope` emits for one frame, no GPU involved.
    fn draw(
        rect: egui::Rect,
        visible: egui::Rect,
        samples: &[EnvelopeSample],
        zones: &[DuckZone],
    ) -> Vec<egui::epaint::ClippedShape> {
        let ctx = egui::Context::default();
        ctx.run(egui::RawInput::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            draw_audio_envelope(
                &painter,
                rect,
                visible,
                samples,
                zones,
                egui::Color32::WHITE,
                egui::Color32::WHITE,
            );
        })
        .shapes
    }

    #[test]
    fn overlay_stays_anchored_to_the_full_clip_rect_when_scrolled() {
        // Clip is 1000 px wide with its left 400 px scrolled out of view.
        let full = egui::Rect::from_min_max(egui::pos2(-400.0, 10.0), egui::pos2(600.0, 70.0));
        let visible = egui::Rect::from_min_max(egui::pos2(0.0, 10.0), egui::pos2(600.0, 70.0));
        let zones = [DuckZone {
            start_frac: 0.5,
            end_frac: 0.75,
            opacity: 0.3,
        }];
        let samples: [EnvelopeSample; 3] = [(0.0, 0.0), (0.5, 0.0), (1.0, -60.0)];
        let shapes = draw(full, visible, &samples, &zones);
        assert!(shapes.iter().all(|s| s.clip_rect == visible));

        let zone = shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r) => Some(r.rect),
                _ => None,
            })
            .expect("duck zone rect");
        assert_eq!((zone.left(), zone.right()), (100.0, 350.0));

        let line = shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Path(l) => Some(l.points.clone()),
                _ => None,
            })
            .expect("polyline");
        assert_eq!(line.first().unwrap().x, -400.0);
        assert_eq!(line.last().unwrap().x, 600.0);
        // The middle sample sits at the middle of the *full* clip.
        assert_eq!(line[1].x, 100.0);

    }
}
