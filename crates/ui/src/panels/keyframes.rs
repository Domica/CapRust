//! Keyframe curve editor (#84 full) — interactive drag points.

use caprust_core::clip::MotionKeyframe;
use egui::{Color32, Pos2, Stroke, Ui};

pub struct KeyframeEditor;

impl KeyframeEditor {
    pub fn show(ui: &mut Ui, keys: &mut Vec<MotionKeyframe>, playhead_ms: u64) {
        ui.label("Trajectory keyframes — drag points to edit");
        // Sort for drawing
        keys.sort_by_key(|k| k.t_ms);
        let min_t = keys.first().map(|k| k.t_ms as f32).unwrap_or(0.0);
        let max_t = keys.last().map(|k| k.t_ms as f32).unwrap_or(1000.0);
        let range = (max_t - min_t).max(1.0);
        let (rect_min, rect_max) = ui.available_rect_before_wrap();
        let area = rect_max - rect_min;
        let w = area.x.max(200.0);
        let h = area.y.max(120.0);

        // Draw curve
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect_min, egui::vec2(w, h)),
            4.0,
            Color32::from_rgb(30, 30, 40),
        );
        if keys.len() >= 2 {
            for pair in keys.windows(2) {
                let a = pair[0];
                let b = pair[1];
                let x1 = rect_min.x + ((a.t_ms as f32 - min_t) / range) * w;
                let y1 = rect_min.y + (1.0 - a.centre_x) * h;
                let x2 = rect_min.x + ((b.t_ms as f32 - min_t) / range) * w;
                let y2 = rect_min.y + (1.0 - b.centre_x) * h;
                ui.painter().line_segment(
                    [Pos2::new(x1, y1), Pos2::new(x2, y2)],
                    Stroke::new(2.5, Color32::from_rgb(0xFF, 0x8B, 0xFD)),
                );
            }
        }
        // Draw draggable points
        for k in keys.iter_mut() {
            let x = rect_min.x + ((k.t_ms as f32 - min_t) / range) * w;
            let y = rect_min.y + (1.0 - k.centre_x) * h;
            let cx = x;
            let cy = y;
            // Simple drag: if pointer near point, update value
            if ui.pointer_hover_pos().map_or(false, |p| {
                (p.x - cx).abs() < 10.0 && (p.y - cy).abs() < 10.0
            }) && ui.pointer_down() {
                let dx = ui.pointer_delta().x / w;
                let dy = ui.pointer_delta().y / h;
                k.centre_x = (k.centre_x - dx * 2.0).clamp(0.0, 1.0);
                // Optionally adjust time with horizontal drag — kept simple
            }
            // Point marker
            ui.painter().circle_filled(
                Pos2::new(cx, cy),
                6.0,
                Color32::from_rgb(0xFF, 0xAA, 0x00),
            );
        }
        ui.add_space(egui::vec2(0.0, 8.0));
        // Playhead line
        let ph_x = rect_min.x + ((playhead_ms as f32 - min_t) / range) * w;
        ui.painter().line_segment(
            [Pos2::new(ph_x, rect_min.y), Pos2::new(ph_x, rect_max.y)],
            Stroke::new(1.0, Color32::from_rgb(0xFF, 0xFF, 0xFF)),
        );
    }
}
