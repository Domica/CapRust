//! Segmented control: one-of-N horizontal selector.

use crate::theme::tokens::{radius, space, text};
use egui::{Response, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetText};

/// Horizontal one-of-N selector. `selected` is the index of the active
/// segment; the returned `Response` carries the clicked index (if any)
/// so the caller owns the state transition.
///
/// Returns `(Response, Option<usize>)` where the `Option` is `Some(i)`
/// when the user clicked segment `i` (which may equal `selected`; the
/// caller should usually ignore that case).
pub fn segmented_control(
    ui: &mut Ui,
    labels: &[impl Into<WidgetText> + Clone],
    selected: usize,
) -> (Response, Option<usize>) {
    let font = egui::FontId::proportional(text::S);
    let visuals = ui.visuals().clone();

    // Measure every segment so the control is uniform height and each
    // segment is wide enough for its label.
    let galleys: Vec<_> = labels
        .iter()
        .map(|l| {
            let text = l.clone().into().text().to_string();
            ui.painter()
                .layout_no_wrap(text, font.clone(), visuals.text_color())
        })
        .collect();

    let pad_x = space::M;
    let pad_y = space::XS;
    let seg_h = galleys
        .iter()
        .map(|g| g.size().y)
        .fold(0.0, f32::max)
        .max(16.0)
        + pad_y * 2.0;
    let seg_ws: Vec<f32> = galleys.iter().map(|g| g.size().x + pad_x * 2.0).collect();
    let total_w: f32 = seg_ws.iter().sum();
    let desired = Vec2::new(total_w, seg_h);

    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);

    // Background track
    if ui.is_rect_visible(rect) {
        let cr = radius::cr(radius::MD);
        ui.painter().rect(
            rect,
            cr,
            visuals.widgets.inactive.bg_fill,
            Stroke::NONE,
            StrokeKind::Inside,
        );
    }

    // Hit test on click
    let mut clicked = None;
    if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let mut x = rect.left();
            for (i, w) in seg_ws.iter().enumerate() {
                if pos.x >= x && pos.x < x + w {
                    clicked = Some(i);
                    break;
                }
                x += w;
            }
        }
    }

    // Draw segments
    if ui.is_rect_visible(rect) {
        let cr = radius::cr(radius::MD);
        let cr_inner = radius::cr(radius::MD - 1.0);
        let mut x = rect.left();
        for (i, (w, galley)) in seg_ws.iter().zip(galleys.iter()).enumerate() {
            let seg_rect =
                egui::Rect::from_min_size(egui::pos2(x, rect.top()), Vec2::new(*w, seg_h));
            if i == selected {
                ui.painter().rect(
                    seg_rect.shrink(1.0),
                    cr_inner,
                    visuals.selection.bg_fill,
                    Stroke::NONE,
                    StrokeKind::Inside,
                );
            }
            let fg = if i == selected {
                visuals.strong_text_color()
            } else {
                visuals.text_color()
            };
            let text_pos = seg_rect.center() - galley.size() / 2.0;
            ui.painter().galley(text_pos, galley.clone(), fg);
            x += *w;
        }
        let _ = cr; // silence unused if compiler is strict
    }

    (response, clicked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_with_three_segments() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let labels = ["One", "Two", "Three"];
                let (_r, clicked) = segmented_control(ui, &labels, 1);
                assert!(clicked.is_none(), "no input should not click a segment");
            });
        });
    }

    #[test]
    fn empty_labels_does_not_panic() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let labels: [&str; 0] = [];
                let (_r, clicked) = segmented_control(ui, &labels, 0);
                assert!(clicked.is_none());
            });
        });
    }
}
