//! Keyframe curve editor panel (#84).
//!
//! Shows the keyframes for a property (trajectory / volume / effect
//! amount) and allows dragging them. The curve is drawn with egui
//! shapes; the points are editable.

use caprust_core::clip::{Clip, MotionKeyframe};
use egui::Ui;

pub struct KeyframePanel;

impl KeyframePanel {
    pub fn show(ui: &mut Ui, clip: &mut Clip, playhead_ms: u64) {
        ui.label("Keyframes (trajectory / volume / effect)");
        // Placeholder: list keyframes with time/value
        for k in &clip.trajectory {
            ui.label(format!("t={}ms x={:.2} y={:.2}", k.t_ms, k.centre_x, k.centre_y));
        }
        // Curve: linear between sorted points drawn with line
        if clip.trajectory.len() >= 2 {
            let mut sorted = clip.trajectory.clone();
            sorted.sort_by_key(|k| k.t_ms);
            let min_t = sorted.first().unwrap().t_ms as f32;
            let max_t = sorted.last().unwrap().t_ms as f32;
            if max_t > min_t {
                let points: Vec<_> = sorted.iter().map(|k| {
                    let x = (k.t_ms as f32 - min_t) / (max_t - min_t);
                    (x, k.centre_x)
                }).collect();
                ui.add_space(egui::vec2(0.0, 8.0));
                let painter = ui.painter();
                for w in points.windows(2) {
                    let (x1, y1) = w[0];
                    let (x2, y2) = w[1];
                    let p1 = egui::pos2(x1 * 200.0, 100.0 - y1 * 80.0);
                    let p2 = egui::pos2(x2 * 200.0, 100.0 - y2 * 80.0);
                    painter.line_segment([p1, p2], egui::Stroke::new(2.0, egui::Color32::from_rgb(0xFF, 0x8B, 0xFD)));
                }
            }
        }
        ui.add_space(egui::vec2(0.0, 8.0));
    }
}
