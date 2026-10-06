//! Switch widget: compact Material-style boolean toggle.

use crate::theme::tokens::{anim, radius};
use egui::{Color32, Response, Sense, Ui, Vec2};

/// Toggle switch. A click flips `*on` in place and marks the returned
/// `Response` as changed, so callers can react with `.changed()`.
pub fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    switch_enabled(ui, on, true)
}

/// Switch with explicit enabled state.
///
/// When `enabled == false` the switch is dimmed to 40% alpha, uses
/// `Sense::hover()` so the click never reaches it, the cursor is
/// `NotAllowed`, and `*on` is not flipped even if a click is somehow
/// delivered. `switch` is a thin wrapper that passes `enabled = true`.
pub fn switch_enabled(ui: &mut Ui, on: &mut bool, enabled: bool) -> Response {
    let desired = Vec2::new(36.0, 20.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, mut response) = ui.allocate_exact_size(desired, sense);

    if enabled && response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response = response.on_hover_cursor(if enabled {
        egui::CursorIcon::PointingHand
    } else {
        egui::CursorIcon::NotAllowed
    });

    if ui.is_rect_visible(rect) {
        let t = ui
            .ctx()
            .animate_bool_with_time(response.id, *on, anim::NORMAL);
        let alpha: f32 = if enabled { 1.0 } else { 0.4 };

        let visuals = ui.visuals();
        let track_bg = if *on {
            visuals.selection.bg_fill.gamma_multiply(alpha)
        } else if enabled && response.hovered() {
            visuals.widgets.hovered.bg_fill.gamma_multiply(alpha)
        } else {
            visuals.widgets.inactive.bg_fill.gamma_multiply(alpha)
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
            Color32::WHITE.gamma_multiply(alpha)
        } else {
            visuals.strong_text_color().gamma_multiply(alpha)
        };
        ui.painter().circle_filled(knob_center, knob_r, knob_color);
    }

    response
}

/// Switch with a trailing label. Use when the switch is the only
/// interactive element on the row; for label-left layouts render the
/// label yourself and call `switch`.
pub fn switch_labeled(ui: &mut Ui, on: &mut bool, label: impl Into<egui::WidgetText>) -> Response {
    ui.horizontal(|ui| {
        let r = switch(ui, on);
        ui.label(label);
        r
    })
    .inner
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

    #[test]
    fn disabled_does_not_flip_state() {
        let ctx = egui::Context::default();
        let mut on = true;
        let mut off = false;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let r = switch_enabled(ui, &mut on, false);
                assert!(!r.clicked(), "disabled switch must not be clickable");
                let r = switch_enabled(ui, &mut off, false);
                assert!(!r.clicked(), "disabled switch must not be clickable");
            });
        });
        assert!(on, "disabled switch must not flip true -> false");
        assert!(!off, "disabled switch must not flip false -> true");
    }

    #[test]
    fn enabled_renders_without_panic() {
        let ctx = egui::Context::default();
        let mut on = true;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = switch_enabled(ui, &mut on, true);
            });
        });
    }
}
