//! Keyframe curve editor (#84 + #85 easing).
//!
//! Two editors share one drawing path: trajectory keyframes
//! (`MotionKeyframe`, milliseconds from clip start + x/y position) and
//! generic tracks (`Keyframe`, seconds from clip start + value).
//!
//! The curve is a *view* over the keyframe vector: we draw in time order
//! through an index list but never reorder the vector itself. That is what
//! makes a drag identity stable when the dragged point crosses a neighbour
//! (3.0.3) -- sorting the vector every frame used to move the point under
//! the stored index, so the drag jumped to a different keyframe.
//!
//! The time axis is the clip duration (not the first/last keyframe), and
//! the playhead is relative to `clip.start_time_ms`. Drag state lives in
//! egui memory keyed by an editor `Id`, so several open editors no longer
//! share one thread-local drag index (3.1).

use caprust_core::clip::{Clip, EaseCurve, Keyframe, KeyframeTrack, MotionKeyframe};
use egui::{Color32, Id, Painter, Pos2, Sense, Stroke, Ui, Vec2};

const CURVE_HEIGHT: f32 = 120.0;
const POINT_RADIUS: f32 = 6.0;
const HIT_RADIUS: f32 = 12.0;
const EASE_STEPS: usize = 12;
const MIN_RANGE_S: f64 = 0.1;

const TRAJECTORY_X: Color32 = Color32::from_rgb(0x55, 0xD6, 0xBE);
const TRAJECTORY_Y: Color32 = Color32::from_rgb(0xFF, 0x8B, 0xFD);
const POINT_COLOR: Color32 = Color32::from_rgb(0xFF, 0xAA, 0x00);

pub struct KeyframeEditor;

/// Which trajectory coordinate an editor instance edits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    X,
    Y,
}

/// Drag state stored in egui memory. `last_time` is the identity of the
/// dragged keyframe: if something reorders/removes points while the drag
/// is active we can find it again instead of dragging a neighbour.
#[derive(Clone, Copy, Debug)]
struct DragState {
    idx: usize,
    last_time: f64,
}

/// Accessors are boxed so both concrete keyframe types can share one
/// implementation without a pile of positional arguments. Named aliases
/// keep the struct readable (and clippy's `type_complexity` happy).
type TimeGetter<K> = Box<dyn Fn(&K) -> f64>;
type TimeSetter<K> = Box<dyn Fn(&mut K, f64)>;
type ValueGetter<K> = Box<dyn Fn(&K) -> f32>;
type ValueSetter<K> = Box<dyn Fn(&mut K, f32)>;
type EaseGetter<K> = Box<dyn Fn(&K) -> EaseCurve>;
type EaseSetter<K> = Box<dyn Fn(&mut K, EaseCurve)>;
type KeyMaker<K> = Box<dyn Fn(f64, f32) -> K>;

/// Everything the generic editor needs to know about a keyframe type.
struct CurveSpec<K> {
    id: Id,
    title: String,
    range_s: f64,
    playhead_s: f64,
    color: Color32,
    show_easing: bool,
    get_time: TimeGetter<K>,
    set_time: TimeSetter<K>,
    get_value: ValueGetter<K>,
    set_value: ValueSetter<K>,
    get_ease: EaseGetter<K>,
    set_ease: EaseSetter<K>,
    make_key: KeyMaker<K>,
}

/// Indices in time order. The source vector is left untouched, so an
/// index remains a valid identity across frames (3.0.3).
fn sorted_order<K>(keys: &[K], get_time: &dyn Fn(&K) -> f64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..keys.len()).collect();
    order.sort_by(|&a, &b| {
        get_time(&keys[a])
            .partial_cmp(&get_time(&keys[b]))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    order
}

/// Clip duration is the time axis. A 100 ms floor keeps a zero-length
/// (or brand new) clip from producing an infinite scale.
fn range_s(duration_ms: u64) -> f64 {
    (duration_ms as f64 / 1000.0).max(MIN_RANGE_S)
}

/// Playhead shown in the properties panel is absolute timeline time;
/// keyframe times are relative to the clip start (3.9).
fn relative_seconds(playhead_ms: u64, clip_start_ms: u64) -> f64 {
    playhead_ms.saturating_sub(clip_start_ms) as f64 / 1000.0
}

/// Endpoints are movable across the whole clip, not pinned to their own
/// position like the old `clamp(min_t, max_t)` (3.4).
fn clamp_time(t: f64, range: f64) -> f64 {
    t.clamp(0.0, range.max(MIN_RANGE_S))
}

fn time_to_x(t: f64, range: f64, min_x: f32, width: f32) -> f32 {
    ((t / range.max(MIN_RANGE_S)) as f32).clamp(0.0, 1.0) * width + min_x
}

fn value_to_y(v: f32, min_y: f32, height: f32) -> f32 {
    min_y + (1.0 - v.clamp(0.0, 1.0)) * height
}

/// Point on an eased segment. Time stays linear; only the value is eased,
/// which mirrors `EaseCurve::apply` used by the renderer.
fn eased_pos(a: Pos2, b: Pos2, ease: EaseCurve, f: f32) -> Pos2 {
    Pos2::new(a.x + (b.x - a.x) * f, a.y + (b.y - a.y) * ease.apply(f))
}

fn draw_eased_segment(painter: &Painter, a: Pos2, b: Pos2, ease: EaseCurve, color: Color32) {
    let mut prev = a;
    for step in 1..=EASE_STEPS {
        let next = eased_pos(a, b, ease, step as f32 / EASE_STEPS as f32);
        painter.line_segment([prev, next], Stroke::new(2.0, color));
        prev = next;
    }
}

fn hit_at(pos: Pos2, points: &[(usize, Pos2)]) -> Option<usize> {
    points
        .iter()
        .rev()
        .find(|(_, p)| p.distance(pos) < HIT_RADIUS)
        .map(|&(idx, _)| idx)
}

/// Resolve the dragged keyframe. Normally the stored index is still valid
/// (we never reorder the vector); the time match is the fallback for an
/// external reorder or a concurrent delete.
fn resolve_drag<K>(keys: &[K], state: DragState, get_time: &dyn Fn(&K) -> f64) -> Option<usize> {
    if state.idx < keys.len() && (get_time(&keys[state.idx]) - state.last_time).abs() < 1e-6 {
        return Some(state.idx);
    }
    keys.iter()
        .position(|k| (get_time(k) - state.last_time).abs() < 1e-6)
}

impl KeyframeEditor {
    /// Trajectory editor. Renders one curve for X and one for Y so a
    /// horizontal sticker move on the preview is visible on a graph
    /// (3.0.2). Both views edit the same `clip.trajectory` vector.
    ///
    /// Returns `true` when keyframes changed this frame; the caller routes
    /// that through `PendingEdit::Trajectory` / `SetClipCommand` (undo).
    pub fn show(ui: &mut Ui, clip: &mut Clip, playhead_ms: u64) -> bool {
        ui.label("Trajectory keyframes (X and Y)");
        ui.label("Drag a point · click empty space to add · right-click to delete");
        ui.add_space(4.0);

        let clip_rel_ms = playhead_ms.saturating_sub(clip.start_time_ms);
        let playhead_s = relative_seconds(playhead_ms, clip.start_time_ms);
        let range = range_s(clip.duration_ms);
        let base_id = Id::new(("trajectory", clip.id.to_string()));

        // A new point needs a value for the axis that is not being edited.
        // Use the trajectory's position at the playhead (or the first
        // keyframe, or the frame centre) instead of a hardcoded 0.5.
        let (base_x, base_y) = caprust_core::clip::trajectory_at(&clip.trajectory, clip_rel_ms)
            .unwrap_or_else(|| {
                clip.trajectory
                    .first()
                    .map(|k| (k.centre_x, k.centre_y))
                    .unwrap_or((0.5, 0.5))
            });

        let mut changed = false;
        for (axis, label, color, show_easing) in [
            (Axis::X, "X position", TRAJECTORY_X, false),
            (Axis::Y, "Y position", TRAJECTORY_Y, true),
        ] {
            let spec = CurveSpec::<MotionKeyframe> {
                id: base_id.with(axis as u8),
                title: format!(
                    "{label} — drag a point, click empty space to add, right-click to delete"
                ),
                range_s: range,
                playhead_s,
                color,
                show_easing,
                get_time: Box::new(|k: &MotionKeyframe| k.t_ms as f64 / 1000.0),
                set_time: Box::new(|k: &mut MotionKeyframe, t: f64| {
                    k.t_ms = (t * 1000.0).round().max(0.0) as u64
                }),
                get_value: Box::new(move |k: &MotionKeyframe| {
                    if axis == Axis::X {
                        k.centre_x
                    } else {
                        k.centre_y
                    }
                }),
                set_value: Box::new(move |k: &mut MotionKeyframe, v: f32| {
                    if axis == Axis::X {
                        k.centre_x = v;
                    } else {
                        k.centre_y = v;
                    }
                }),
                get_ease: Box::new(|k: &MotionKeyframe| k.ease),
                set_ease: Box::new(|k: &mut MotionKeyframe, e: EaseCurve| k.ease = e),
                make_key: Box::new(move |t: f64, v: f32| {
                    let (x, y) = if axis == Axis::X {
                        (v, base_y)
                    } else {
                        (base_x, v)
                    };
                    MotionKeyframe::new((t * 1000.0).round().max(0.0) as u64, x, y)
                }),
            };
            changed |= Self::show_generic(ui, &mut clip.trajectory, &spec);
        }
        changed
    }

    /// Editor for a generic `KeyframeTrack` (scale, opacity, rotation…).
    /// `clip_rel_ms` is the playhead minus `clip.start_time_ms`;
    /// `clip_duration_ms` defines the time axis. `editor_id` must be
    /// unique per open track so drags do not leak between editors (3.1).
    pub fn show_keyframe_track(
        ui: &mut Ui,
        track: &mut KeyframeTrack,
        clip_rel_ms: u64,
        clip_duration_ms: u64,
        editor_id: Id,
    ) -> bool {
        let spec = CurveSpec::<Keyframe> {
            id: editor_id,
            title: format!(
                "{} — drag a point, click empty space to add, right-click to delete",
                track.property
            ),
            range_s: range_s(clip_duration_ms),
            playhead_s: clip_rel_ms as f64 / 1000.0,
            color: TRAJECTORY_Y,
            show_easing: true,
            get_time: Box::new(|k: &Keyframe| k.t),
            set_time: Box::new(|k: &mut Keyframe, t: f64| k.t = t.max(0.0)),
            get_value: Box::new(|k: &Keyframe| k.value),
            set_value: Box::new(|k: &mut Keyframe, v: f32| k.value = v),
            get_ease: Box::new(|k: &Keyframe| k.ease),
            set_ease: Box::new(|k: &mut Keyframe, e: EaseCurve| k.ease = e),
            make_key: Box::new(|t: f64, v: f32| Keyframe {
                t,
                value: v,
                ease: EaseCurve::default(),
            }),
        };
        Self::show_generic(ui, &mut track.keyframes, &spec)
    }

    fn show_generic<K: 'static>(ui: &mut Ui, keys: &mut Vec<K>, spec: &CurveSpec<K>) -> bool {
        let mut changed = false;
        ui.label(&spec.title);
        ui.add_space(2.0);

        let (rect, response) =
            ui.allocate_at_least(Vec2::new(200.0, CURVE_HEIGHT), Sense::click_and_drag());
        let area_w = rect.width();
        let area_h = rect.height();
        // Clip paint (including point labels) to the curve rectangle (3.9).
        let painter = ui.painter().with_clip_rect(rect);

        painter.rect_filled(rect, 4.0, Color32::from_rgb(25, 25, 35));
        for i in 0..=4 {
            let y = rect.min.y + (i as f32 / 4.0) * area_h;
            painter.line_segment(
                [Pos2::new(rect.min.x, y), Pos2::new(rect.max.x, y)],
                Stroke::new(0.5, Color32::from_gray(60)),
            );
            let x = rect.min.x + (i as f32 / 4.0) * area_w;
            painter.line_segment(
                [Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)],
                Stroke::new(0.5, Color32::from_gray(60)),
            );
        }

        let point_pos = |k: &K| {
            Pos2::new(
                time_to_x((spec.get_time)(k), spec.range_s, rect.min.x, area_w),
                value_to_y((spec.get_value)(k), rect.min.y, area_h),
            )
        };

        // Positions before interaction: hit-testing must use the frame the
        // user actually pressed on, not the post-drag layout.
        let before_order = sorted_order(keys, spec.get_time.as_ref());
        let before_points: Vec<(usize, Pos2)> = before_order
            .iter()
            .map(|&i| (i, point_pos(&keys[i])))
            .collect();

        let drag_id = spec.id.with("drag");
        // A drag can only be alive while the primary button is held. This
        // also clears a stale drag if the editor was hidden on the frame
        // the pointer was released (tab switch / panel close).
        let primary_down = ui.ctx().input(|i| i.pointer.primary_down());
        let mut drag: Option<DragState> = if primary_down {
            ui.ctx().memory(|m| m.data.get_temp(drag_id))
        } else {
            None
        };

        if response.drag_started() {
            if let Some(pos) = response
                .interact_pointer_pos()
                .filter(|p| rect.contains(*p))
            {
                if let Some(idx) = hit_at(pos, &before_points) {
                    drag = Some(DragState {
                        idx,
                        last_time: (spec.get_time)(&keys[idx]),
                    });
                }
            }
        }
        if response.drag_stopped() {
            drag = None;
        }

        // Right-click delete uses `secondary_clicked()` (one shot), not
        // `secondary_down()` (deletes every frame while held) (3.5).
        let mut to_delete: Option<usize> = None;
        if ui.ctx().input(|i| i.pointer.secondary_clicked()) {
            if let Some(pos) = ui.ctx().pointer_hover_pos().filter(|p| rect.contains(*p)) {
                to_delete = hit_at(pos, &before_points);
            }
        }

        if response.dragged() {
            if let Some(state) = drag {
                if let Some(idx) = resolve_drag(keys, state, spec.get_time.as_ref()) {
                    let delta = response.drag_delta();
                    if delta.length() > 0.0 {
                        let dt = (delta.x as f64 / area_w as f64) * spec.range_s;
                        let dv = -(delta.y / area_h);
                        let t = clamp_time((spec.get_time)(&keys[idx]) + dt, spec.range_s);
                        let v = ((spec.get_value)(&keys[idx]) + dv).clamp(0.0, 1.0);
                        (spec.set_time)(&mut keys[idx], t);
                        (spec.set_value)(&mut keys[idx], v);
                        // Store the value the keyframe actually kept:
                        // trajectory times are rounded to whole ms, so
                        // remembering the unrounded `t` would break the
                        // identity check on the very next frame.
                        let stored_time = (spec.get_time)(&keys[idx]);
                        drag = Some(DragState {
                            idx,
                            last_time: stored_time,
                        });
                        changed = true;
                        ui.ctx().request_repaint();
                    }
                }
            }
        }

        // Click on empty curve space adds a keyframe at that time/value.
        if response.clicked() {
            if let Some(pos) = response
                .interact_pointer_pos()
                .filter(|p| rect.contains(*p))
            {
                if hit_at(pos, &before_points).is_none() {
                    let t = ((pos.x - rect.min.x) as f64 / area_w as f64 * spec.range_s)
                        .clamp(0.0, spec.range_s);
                    let v = (1.0 - (pos.y - rect.min.y) / area_h).clamp(0.0, 1.0);
                    keys.push((spec.make_key)(t, v));
                    changed = true;
                }
            }
        }

        if let Some(idx) = to_delete {
            if idx < keys.len() {
                keys.remove(idx);
                changed = true;
                drag = None;
            }
        }

        ui.ctx().memory_mut(|m| {
            if let Some(state) = drag {
                m.data.insert_temp(drag_id, state);
            } else {
                m.data.remove::<DragState>(drag_id);
            }
        });

        // Curve and markers are drawn AFTER this frame's mutations, so the
        // picture is never one frame behind the point being dragged.
        let order = sorted_order(keys, spec.get_time.as_ref());
        let points: Vec<(usize, Pos2)> = order.iter().map(|&i| (i, point_pos(&keys[i]))).collect();
        for pair in points.windows(2) {
            let (ia, a) = pair[0];
            let (_, b) = pair[1];
            draw_eased_segment(&painter, a, b, (spec.get_ease)(&keys[ia]), spec.color);
        }

        for (idx, p) in &points {
            let is_dragged = drag.is_some_and(|d| d.idx == *idx);
            painter.circle_filled(
                *p,
                POINT_RADIUS,
                if is_dragged {
                    Color32::WHITE
                } else {
                    POINT_COLOR
                },
            );
            painter.text(
                Pos2::new(p.x + 8.0, p.y - 8.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{:.2}s", (spec.get_time)(&keys[*idx])),
                egui::FontId::proportional(10.0),
                Color32::from_gray(200),
            );
        }

        // Playhead is clamped to the curve rectangle (3.9).
        let ph_x = time_to_x(spec.playhead_s, spec.range_s, rect.min.x, area_w);
        painter.line_segment(
            [Pos2::new(ph_x, rect.min.y), Pos2::new(ph_x, rect.max.y)],
            Stroke::new(1.5, Color32::WHITE),
        );

        ui.add_space(4.0);
        if spec.show_easing {
            ui.label("Easing (per keyframe, shared by X and Y)");
            for &idx in &order {
                let before_ease = (spec.get_ease)(&keys[idx]);
                let mut ease = before_ease;
                // Key by the vector index, not the display row: sorting
                // must not swap the combo's persistent state.
                egui::ComboBox::from_id_salt(spec.id.with(("ease", idx)))
                    .selected_text(format!("{:.2}s · {ease:?}", (spec.get_time)(&keys[idx])))
                    .show_ui(ui, |ui| {
                        for (variant, label) in [
                            (EaseCurve::Linear, "Linear"),
                            (EaseCurve::EaseIn, "EaseIn"),
                            (EaseCurve::EaseOut, "EaseOut"),
                            (EaseCurve::EaseInOut, "EaseInOut"),
                        ] {
                            ui.selectable_value(&mut ease, variant, label);
                        }
                    });
                if ease != before_ease {
                    (spec.set_ease)(&mut keys[idx], ease);
                    changed = true;
                }
            }
        }
        ui.add_space(4.0);
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use caprust_core::clip::{Keyframe, MotionKeyframe};

    fn times(keys: &[MotionKeyframe]) -> Vec<u64> {
        keys.iter().map(|k| k.t_ms).collect()
    }

    fn sample() -> Vec<MotionKeyframe> {
        vec![
            MotionKeyframe::new(2000, 0.2, 0.2),
            MotionKeyframe::new(0, 0.1, 0.1),
            MotionKeyframe::new(1000, 0.5, 0.5),
        ]
    }

    #[test]
    fn sorted_order_is_a_view_and_never_mutates_the_source() {
        let keys = sample();
        let get = |k: &MotionKeyframe| k.t_ms as f64 / 1000.0;
        let order = sorted_order(&keys, &get);
        assert_eq!(order, vec![1, 2, 0]);
        assert_eq!(times(&keys), vec![2000, 0, 1000]);
    }

    #[test]
    fn time_axis_comes_from_clip_duration_not_the_keyframes() {
        let keys = sample();
        let get = |k: &MotionKeyframe| k.t_ms as f64 / 1000.0;
        // A 10 s clip with points only at 0..2 s must still spread the
        // whole 10 s across the curve, so endpoints are not pinned.
        let range = range_s(10_000);
        assert_eq!(range, 10.0);
        assert_eq!(clamp_time(-5.0, range), 0.0);
        assert_eq!(clamp_time(50.0, range), 10.0);
        assert_eq!(clamp_time(7.25, range), 7.25);
        let first_x = time_to_x(get(&keys[0]), range, 100.0, 400.0);
        assert!((first_x - 180.0).abs() < 1e-4, "x was {first_x}");
    }

    #[test]
    fn playhead_is_relative_to_clip_start_and_clamped_to_the_curve() {
        assert_eq!(relative_seconds(15_000, 5_000), 10.0);
        assert_eq!(relative_seconds(2_000, 5_000), 0.0);
        let right = time_to_x(99.0, 10.0, 100.0, 400.0);
        assert_eq!(right, 500.0);
        let left = time_to_x(-99.0, 10.0, 100.0, 400.0);
        assert_eq!(left, 100.0);
    }

    #[test]
    fn zero_length_clip_still_has_a_finite_range() {
        assert_eq!(range_s(0), MIN_RANGE_S);
        assert!(time_to_x(1.0, range_s(0), 0.0, 200.0).is_finite());
    }

    #[test]
    fn drag_identity_survives_an_external_reorder() {
        let mut keys = sample();
        let get = |k: &MotionKeyframe| k.t_ms as f64 / 1000.0;
        let state = DragState {
            idx: 2,
            last_time: 1.0,
        };
        assert_eq!(resolve_drag(&keys, state, &get), Some(2));
        keys.swap(1, 2);
        // The point that was at index 2 (t=1s) is now at index 1.
        assert_eq!(resolve_drag(&keys, state, &get), Some(1));
    }

    #[test]
    fn drag_identity_uses_the_stored_rounded_time() {
        let mut keys = vec![MotionKeyframe::new(1000, 0.5, 0.5)];
        let get = |k: &MotionKeyframe| k.t_ms as f64 / 1000.0;
        let set = |k: &mut MotionKeyframe, t: f64| {
            k.t_ms = (t * 1000.0).round().max(0.0) as u64;
        };
        let unrounded = 1.23456;
        set(&mut keys[0], unrounded);
        let stored = get(&keys[0]);
        let good = DragState {
            idx: 0,
            last_time: stored,
        };
        assert_eq!(resolve_drag(&keys, good, &get), Some(0));
        // Remembering the unrounded value instead would lose the drag on
        // the next frame (trajectory times are whole milliseconds).
        let bad = DragState {
            idx: 0,
            last_time: unrounded,
        };
        assert_eq!(resolve_drag(&keys, bad, &get), None);
    }

    #[test]
    fn drag_identity_is_not_stolen_by_a_neighbour() {
        let keys = vec![
            MotionKeyframe::new(1000, 0.1, 0.1),
            MotionKeyframe::new(2000, 0.9, 0.9),
        ];
        let get = |k: &MotionKeyframe| k.t_ms as f64 / 1000.0;
        let state = DragState {
            idx: 0,
            last_time: 1.0,
        };
        let idx = resolve_drag(&keys, state, &get).expect("drag target");
        assert_eq!(get(&keys[idx]), 1.0);
        assert!((keys[idx].centre_y - 0.1).abs() < 1e-6);
    }

    #[test]
    fn eased_segments_start_and_end_on_the_keyframes() {
        let a = Pos2::new(10.0, 90.0);
        let b = Pos2::new(110.0, 10.0);
        for ease in [
            EaseCurve::Linear,
            EaseCurve::EaseIn,
            EaseCurve::EaseOut,
            EaseCurve::EaseInOut,
        ] {
            let start = eased_pos(a, b, ease, 0.0);
            let end = eased_pos(a, b, ease, 1.0);
            assert_eq!(start, a, "{ease:?} start");
            assert!((end.x - b.x).abs() < 1e-5 && (end.y - b.y).abs() < 1e-5);
        }
        let linear = eased_pos(a, b, EaseCurve::Linear, 0.5).y;
        let ease_in = eased_pos(a, b, EaseCurve::EaseIn, 0.5).y;
        assert!(ease_in > linear, "EaseIn should stay closer to the start");
    }

    #[test]
    fn hit_test_picks_the_topmost_point_within_the_radius() {
        let points = vec![(0, Pos2::new(100.0, 100.0)), (1, Pos2::new(150.0, 100.0))];
        assert_eq!(hit_at(Pos2::new(101.0, 101.0), &points), Some(0));
        assert_eq!(hit_at(Pos2::new(300.0, 100.0), &points), None);
    }

    #[test]
    fn generic_keyframes_use_seconds_and_clamp_values() {
        let keys = vec![
            Keyframe {
                t: 2.0,
                value: 0.25,
                ease: EaseCurve::EaseOut,
            },
            Keyframe {
                t: 0.5,
                value: 0.75,
                ease: EaseCurve::Linear,
            },
        ];
        let get = |k: &Keyframe| k.t;
        assert_eq!(sorted_order(&keys, &get), vec![1, 0]);
        assert_eq!(value_to_y(-1.0, 10.0, 100.0), 110.0);
        assert_eq!(value_to_y(2.0, 10.0, 100.0), 10.0);
    }
}
