//! Chip widget: compact selectable pill.

use crate::theme::tokens::{radius, space, text};
use egui::{Response, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetText};

/// Compact pill-shaped toggle. Use for filter / tag / segment
/// choices where a checkbox is too heavy and a button too wide.
///
/// Returns a `Response`; the caller owns the `selected` flag and is
/// responsible for toggling it when `.clicked()` is true.
pub fn chip(ui: &mut Ui, label: impl Into<WidgetText>, selected: bool) -> Response {
    let label = label.into();
    let visuals = ui.visuals().clone();
    let font = egui::FontId::proportional(text::S);

    let fg = if selected {
        visuals.strong_text_color()
    } else {
        visuals.text_color()
    };

    let galley = ui
        .painter()
        .layout_no_wrap(label.text().to_string(), font.clone(), fg);

    let pad_x = space::M;
    let pad_y = space::XS;
    let desired = Vec2::new(
        galley.size().x + pad_x * 2.0,
        galley.size().y.max(16.0) + pad_y * 2.0,
    );

    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);

    if ui.is_rect_visible(rect) {
        let bg = if selected {
            visuals.selection.bg_fill
        } else if response.hovered() {
            visuals.widgets.hovered.bg_fill
        } else {
            visuals.widgets.inactive.bg_fill
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
}
