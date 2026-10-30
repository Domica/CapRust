//! Keyframe curve editor (#84 + #85 easing).
//!
//! Shows the keyframes for a clip's trajectory (or volume, effect
//! amount) as draggable points on a curve. Points can be added by
//! clicking on the curve area, moved by dragging, and removed with
//! right-click or the delete button. An easing selector per keyframe
//! (#85) controls the interpolation between adjacent points.

use caprust_core::clip::{EaseCurve, Keyframe, MotionKeyframe};
use egui::{Color32, Pos2, Rect, Stroke, Ui, Vec2};

pub struct KeyframeEditor;

impl KeyframeEditor {
    /// Show the curve editor for a clip's trajectory keyframes.
    pub fn show(ui: &mut Ui, keys: &mut Vec<MotionKeyframe>, playhead_ms: u64) {
        ui.label("Trajectory keyframes");
        ui.add_space(4.0);

        // Sort keyframes by time for consistent display.
        keys.sort_by_key(|k| k.t_ms);

        if keys.is_empty() {
            ui.label("No keyframes. Click on the curve to add one.");
            ui.add_space(4.0);
        }

        // Curve area - reserve a fixed height to avoid overlapping with content below
        let desired_height = 140.0;
        let (rect, _) =
            ui.allocate_at_least(Vec2::new(200.0, desired_height), egui::Sense::hover());
        let rect_min = rect.min;
        let rect_max = rect.max;
        let w = rect_max.x - rect_min.x;
        let h = rect_max.y - rect_min.y;
        let area_w = w.max(200.0);
        let area_h = h.max(120.0);

        // Background
        ui.painter().rect_filled(
            Rect::from_min_size(rect_min, Vec2::new(area_w, area_h)),
            4.0,
            Color32::from_rgb(25, 25, 35),
        );

        // Grid lines
        for i in 0..=4 {
            let y = rect_min.y + (i as f32 / 4.0) * area_h;
            ui.painter().line_segment(
                [Pos2::new(rect_min.x, y), Pos2::new(rect_min.x + area_w, y)],
                Stroke::new(0.5, Color32::from_gray(60)),
            );
        }
        for i in 0..=4 {
            let x = rect_min.x + (i as f32 / 4.0) * area_w;
            ui.painter().line_segment(
                [Pos2::new(x, rect_min.y), Pos2::new(x, rect_min.y + area_h)],
                Stroke::new(0.5, Color32::from_gray(60)),
            );
        }

        // Compute time range
        let min_t = keys.first().map(|k| k.t_ms as f32).unwrap_or(0.0);
        let max_t = keys.last().map(|k| k.t_ms as f32).unwrap_or(1000.0);
        let range = (max_t - min_t).max(1.0);

        // Draw curve between points
        if keys.len() >= 2 {
            for pair in keys.windows(2) {
                let a = pair[0];
                let b = pair[1];
                let x1 = rect_min.x + ((a.t_ms as f32 - min_t) / range) * area_w;
                let y1 = rect_min.y + (1.0 - a.centre_x) * area_h;
                let x2 = rect_min.x + ((b.t_ms as f32 - min_t) / range) * area_w;
                let y2 = rect_min.y + (1.0 - b.centre_x) * area_h;
                ui.painter().line_segment(
                    [Pos2::new(x1, y1), Pos2::new(x2, y2)],
                    Stroke::new(2.5, Color32::from_rgb(0xFF, 0x8B, 0xFD)),
                );
            }
        }

        // Draw draggable points
        let mut to_delete: Option<usize> = None;
        for (idx, k) in keys.iter_mut().enumerate() {
            let x = rect_min.x + ((k.t_ms as f32 - min_t) / range) * area_w;
            let y = rect_min.y + (1.0 - k.centre_x) * area_h;
            let cx = x;
            let cy = y;

            // Handle right-click to delete
            if ui
                .ctx()
                .pointer_hover_pos()
                .is_some_and(|p| (p.x - cx).abs() < 12.0 && (p.y - cy).abs() < 12.0)
                && ui.ctx().input(|i| i.pointer.secondary_down())
            {
                to_delete = Some(idx);
            }

            // Drag point
            if ui
                .ctx()
                .pointer_hover_pos()
                .is_some_and(|p| (p.x - cx).abs() < 12.0 && (p.y - cy).abs() < 12.0)
                && ui.ctx().input(|i| i.pointer.primary_down())
            {
                let delta = ui.ctx().input(|i| i.pointer.delta());
                let dx = delta.x / area_w;
                let dy = delta.y / area_h;
                k.t_ms = (k.t_ms as f32 + dx * range).clamp(min_t, max_t) as u64;
                k.centre_x = (k.centre_x - dy).clamp(0.0, 1.0);
            }

            // Point marker
            ui.painter()
                .circle_filled(Pos2::new(cx, cy), 7.0, Color32::from_rgb(0xFF, 0xAA, 0x00));
            // Label
            ui.painter().text(
                Pos2::new(cx + 10.0, cy - 10.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{:.0}ms", k.t_ms),
                egui::FontId::proportional(10.0),
                Color32::from_gray(200),
            );
        }

        // Delete point if right-clicked
        if let Some(idx) = to_delete {
            keys.remove(idx);
        }

        // Playhead line
        let ph_x = rect_min.x + ((playhead_ms as f32 - min_t) / range) * area_w;
        ui.painter().line_segment(
            [
                Pos2::new(ph_x, rect_min.y),
                Pos2::new(ph_x, rect_min.y + area_h),
            ],
            Stroke::new(1.5, Color32::from_rgb(0xFF, 0xFF, 0xFF)),
        );

        ui.add_space(4.0);

        // Easing selector (#85)
        ui.add_space(4.0);
        ui.label("Easing (per keyframe)");
        // Show easing for each keyframe
        for k in keys.iter_mut() {
            ui.horizontal(|ui| {
                ui.label(format!("t={}ms", k.t_ms));
                if ui.button("Linear").clicked() {
                    k.ease = EaseCurve::Linear;
                }
                if ui.button("EaseIn").clicked() {
                    k.ease = EaseCurve::EaseIn;
                }
                if ui.button("EaseOut").clicked() {
                    k.ease = EaseCurve::EaseOut;
                }
                if ui.button("EaseInOut").clicked() {
                    k.ease = EaseCurve::EaseInOut;
                }
                let _ = ui.label(format!("{:?}", k.ease));
            });
        }

        ui.add_space(4.0);
    }

    /// Show the curve editor for generic keyframes (scale, rotation, opacity, etc.)
    pub fn show_keyframes(ui: &mut Ui, keys: &mut Vec<Keyframe>, playhead_ms: u64) {
        Self::show_generic(
            ui,
            keys,
            playhead_ms,
            |k| k.t,
            |k| k.value,
            |k| &mut k.ease,
            |k, v| k.t = v,
            |k, v| k.value = v,
        );
    }

    /// Generic curve editor for any keyframe type with time, value, and easing.
    fn show_generic<K>(
        ui: &mut Ui,
        keys: &mut Vec<K>,
        playhead_ms: u64,
        get_time: impl Fn(&K) -> f64,
        get_value: impl Fn(&K) -> f32,
        get_ease: impl Fn(&mut K) -> &mut EaseCurve,
        set_time: impl Fn(&mut K, f64),
        set_value: impl Fn(&mut K, f32),
    ) where
        K: Clone + 'static,
    {
        ui.label("Keyframes");
        ui.add_space(4.0);

        // Sort keyframes by time for consistent display.
        keys.sort_by(|a, b| {
            get_time(a)
                .partial_cmp(&get_time(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if keys.is_empty() {
            ui.label("No keyframes. Click on the curve to add one.");
            ui.add_space(4.0);
        }

        // Curve area - reserve a fixed height to avoid overlapping with content below
        let desired_height = 140.0;
        let (rect, _) =
            ui.allocate_at_least(Vec2::new(200.0, desired_height), egui::Sense::hover());
        let rect_min = rect.min;
        let rect_max = rect.max;
        let w = rect_max.x - rect_min.x;
        let h = rect_max.y - rect_min.y;
        let area_w = w.max(200.0);
        let area_h = h.max(120.0);

        // Background
        ui.painter().rect_filled(
            Rect::from_min_size(rect_min, Vec2::new(area_w, area_h)),
            4.0,
            Color32::from_rgb(25, 25, 35),
        );

        // Grid lines
        for i in 0..=4 {
            let y = rect_min.y + (i as f32 / 4.0) * area_h;
            ui.painter().line_segment(
                [Pos2::new(rect_min.x, y), Pos2::new(rect_min.x + area_w, y)],
                Stroke::new(0.5, Color32::from_gray(60)),
            );
        }
        for i in 0..=4 {
            let x = rect_min.x + (i as f32 / 4.0) * area_w;
            ui.painter().line_segment(
                [Pos2::new(x, rect_min.y), Pos2::new(x, rect_min.y + area_h)],
                Stroke::new(0.5, Color32::from_gray(60)),
            );
        }

        // Compute time range
        let min_t = keys.first().map(&get_time).unwrap_or(0.0) as f32;
        let max_t = keys.last().map(&get_time).unwrap_or(1000.0) as f32;
        let range = (max_t - min_t).max(1.0);

        // Draw curve between points
        if keys.len() >= 2 {
            for pair in keys.windows(2) {
                let a = &pair[0];
                let b = &pair[1];
                let x1 = rect_min.x + ((get_time(a) as f32 - min_t) / range) * area_w;
                let y1 = rect_min.y + (1.0 - get_value(a)) * area_h;
                let x2 = rect_min.x + ((get_time(b) as f32 - min_t) / range) * area_w;
                let y2 = rect_min.y + (1.0 - get_value(b)) * area_h;
                ui.painter().line_segment(
                    [Pos2::new(x1, y1), Pos2::new(x2, y2)],
                    Stroke::new(2.5, Color32::from_rgb(0xFF, 0x8B, 0xFD)),
                );
            }
        }

        // Draw draggable points (show even with 1 keyframe)
        let mut to_delete: Option<usize> = None;
        for (idx, k) in keys.iter_mut().enumerate() {
            let x = rect_min.x + ((get_time(k) as f32 - min_t) / range) * area_w;
            let y = rect_min.y + (1.0 - get_value(k)) * area_h;
            let cx = x;
            let cy = y;

            // Handle right-click to delete
            if ui
                .ctx()
                .pointer_hover_pos()
                .is_some_and(|p| (p.x - cx).abs() < 12.0 && (p.y - cy).abs() < 12.0)
                && ui.ctx().input(|i| i.pointer.secondary_down())
            {
                to_delete = Some(idx);
            }

            // Drag point - check hover and primary down
            let is_hovered = ui
                .ctx()
                .pointer_hover_pos()
                .is_some_and(|p| (p.x - cx).abs() < 12.0 && (p.y - cy).abs() < 12.0);
            let is_dragging = is_hovered && ui.ctx().input(|i| i.pointer.primary_down());

            if is_dragging {
                let delta = ui.ctx().input(|i| i.pointer.delta());
                if delta.length() > 0.0 {
                    let dx = delta.x / area_w;
                    let dy = delta.y / area_h;
                    let new_time = (get_time(k) as f32 + dx * range).clamp(min_t, max_t) as f64;
                    let new_value = (get_value(k) - dy).clamp(0.0, 1.0);
                    set_time(k, new_time);
                    set_value(k, new_value);
                }
            }

            // Point marker
            ui.painter()
                .circle_filled(Pos2::new(cx, cy), 7.0, Color32::from_rgb(0xFF, 0xAA, 0x00));
            // Label
            ui.painter().text(
                Pos2::new(cx + 10.0, cy - 10.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{:.1}s", get_time(k) as f32),
                egui::FontId::proportional(10.0),
                Color32::from_gray(200),
            );
        }

        // Delete point if right-clicked
        if let Some(idx) = to_delete {
            keys.remove(idx);
        }

        // Playhead line
        let ph_x = rect_min.x + ((playhead_ms as f32 / 1000.0 - min_t) / range) * area_w;
        ui.painter().line_segment(
            [
                Pos2::new(ph_x, rect_min.y),
                Pos2::new(ph_x, rect_min.y + area_h),
            ],
            Stroke::new(1.5, Color32::from_rgb(0xFF, 0xFF, 0xFF)),
        );

        ui.add_space(4.0);

        // Easing selector
        ui.add_space(4.0);
        ui.label("Easing (per keyframe)");
        for k in keys.iter_mut() {
            ui.horizontal(|ui| {
                ui.label(format!("t={:.1}s", get_time(k) as f32));
                if ui.button("Linear").clicked() {
                    *get_ease(k) = EaseCurve::Linear;
                }
                if ui.button("EaseIn").clicked() {
                    *get_ease(k) = EaseCurve::EaseIn;
                }
                if ui.button("EaseOut").clicked() {
                    *get_ease(k) = EaseCurve::EaseOut;
                }
                if ui.button("EaseInOut").clicked() {
                    *get_ease(k) = EaseCurve::EaseInOut;
                }
                let _ = ui.label(format!("{:?}", get_ease(k)));
            });
        }

        ui.add_space(4.0);
    }
}
