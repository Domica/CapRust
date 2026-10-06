//! Switch widget: compact Material-style boolean toggle.

use crate::theme::tokens::{anim, radius};
use egui::{Color32, Response, Sense, Ui, Vec2};

/// Toggle switch. A click flips `*on` in place and marks the returned
/// `Response` as changed, so callers can react with `.changed()`.
pub fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let desired = Vec2::new(36.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(desired, Sense::click());

    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response = response.on_hover_cursor(egui::CursorIcon::PointingHand);

    if ui.is_rect_visible(rect) {
        let t = ui
            .ctx()
            .animate_bool_with_time(response.id, *on, anim::NORMAL);

        let visuals = ui.visuals();
        let track_bg = if *on {
            visuals.selection.bg_fill
        } else if response.hovered() {
            visuals.widgets.hovered.bg_fill
        } else {
            visuals.widgets.inactive.bg_fill
        };
        let track_stroke = if *on {
            egui::Stroke::NONE
        } else {
            visuals.widgets.inactive.bg_stroke
        };

        let cr = radius::cr(radius::PILL);
        ui.painter()
            .rect(rect, cr, track_bg, track_stroke, egui::StrokeKind::Inside);

        let knob_r = rect.height() * 0.38;
        let left = rect.left() + knob_r + 2.0;
        let right = rect.right() - knob_r - 2.0;
        let knob_x = left + (right - left) * t;
        let knob_center = egui::pos2(knob_x, rect.center().y);

        let knob_color = if *on {
            Color32::WHITE
        } else {
            visuals.strong_text_color()
        };
        ui.painter().circle_filled(knob_center, knob_r, knob_color);
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_on_and_off() {
        let ctx = egui::Context::default();
        let mut on = true;
        let mut off = false;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = switch(ui, &mut on);
                let _ = switch(ui, &mut off);
            });
        });
    }

    #[test]
    fn no_input_keeps_state() {
        let ctx = egui::Context::default();
        let mut on = false;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = switch(ui, &mut on);
            });
        });
        assert!(!on, "no input should not flip the switch");
    }
}
