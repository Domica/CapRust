//! Chip widget: compact selectable pill.

use crate::theme::tokens::{radius, space, text};
use egui::{Response, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetText};

/// Compact pill-shaped toggle. Use for filter / tag / segment
/// choices where a checkbox is too heavy and a button too wide.
///
/// Returns a `Response`; the caller owns the `selected` flag and is
/// responsible for toggling it when `.clicked()` is true.
pub fn chip(ui: &mut Ui, label: impl Into<WidgetText>, selected: bool) -> Response {
    chip_enabled(ui, label, selected, true)
}

/// Chip with explicit enabled state.
///
/// When `enabled == false` the chip is dimmed to 40% alpha, uses
/// `Sense::hover()` so clicks do not reach it, and the cursor is
/// `NotAllowed`. This is the single implementation; `chip` is a
/// thin wrapper that passes `enabled = true`.
pub fn chip_enabled(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    selected: bool,
    enabled: bool,
) -> Response {
    let label = label.into();
    let visuals = ui.visuals().clone();
    let font = egui::FontId::proportional(text::S);
    let alpha: f32 = if enabled { 1.0 } else { 0.4 };

    let fg = if selected {
        visuals.strong_text_color()
    } else {
        visuals.text_color()
    }
    .gamma_multiply(alpha);

    let galley = ui
        .painter()
        .layout_no_wrap(label.text().to_string(), font.clone(), fg);

    let pad_x = space::M;
    let pad_y = space::XS;
    let desired = Vec2::new(
        galley.size().x + pad_x * 2.0,
        galley.size().y.max(16.0) + pad_y * 2.0,
    );

    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(desired, sense);
    let response = response.on_hover_cursor(if enabled {
        egui::CursorIcon::PointingHand
    } else {
        egui::CursorIcon::NotAllowed
    });

    if ui.is_rect_visible(rect) {
        let bg = if selected {
            visuals.selection.bg_fill.gamma_multiply(alpha)
        } else if enabled && response.hovered() {
            visuals.widgets.hovered.bg_fill.gamma_multiply(alpha)
        } else {
            visuals.widgets.inactive.bg_fill.gamma_multiply(alpha)
        };
        let stroke = if selected {
            Stroke::NONE
        } else {
            visuals.widgets.inactive.bg_stroke
        };
        let cr = radius::cr(radius::PILL);
        ui.painter().rect(rect, cr, bg, stroke, StrokeKind::Inside);

        let text_pos = rect.center() - galley.size() / 2.0;
        ui.painter().galley(text_pos, galley, fg);
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_selected_and_unselected() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = chip(ui, "alpha", false);
                let _ = chip(ui, "beta", true);
                let _ = chip(ui, "gamma".to_string(), false);
            });
        });
    }

    #[test]
    fn disabled_does_not_click() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let r = chip_enabled(ui, "disabled", false, false);
                assert!(!r.clicked());
                let r = chip_enabled(ui, "disabled-selected", true, false);
                assert!(!r.clicked());
            });
        });
    }

    #[test]
    fn enabled_chip_renders_on_empty_input() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let r = chip_enabled(ui, "on", false, true);
                assert!(!r.clicked(), "no pointer input -> no click");
            });
        });
    }
}
