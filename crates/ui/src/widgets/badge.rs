//! Badge widget: small colored status pill.

use crate::theme::tokens::{radius, space, text};
use egui::{Color32, Response, Sense, Ui, Vec2};

/// Semantic color for a badge. Neutral is the "no status" fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeKind {
    Info,
    Success,
    Warn,
    Error,
    Neutral,
}

impl BadgeKind {
    fn bg(self) -> Color32 {
        match self {
            Self::Info => Color32::from_rgb(56, 132, 220),
            Self::Success => Color32::from_rgb(60, 170, 90),
            Self::Warn => Color32::from_rgb(220, 160, 40),
            Self::Error => Color32::from_rgb(210, 70, 70),
            Self::Neutral => Color32::from_rgb(110, 110, 120),
        }
    }
}

/// Small rounded pill with a colored background and white text.
/// Non-interactive; returns the layout `Response` for hover checks.
pub fn badge(ui: &mut Ui, label: impl Into<egui::WidgetText>, kind: BadgeKind) -> Response {
    let label = label.into();
    let font = egui::FontId::proportional(text::XS);
    let fg = Color32::WHITE;

    let galley = ui
        .painter()
        .layout_no_wrap(label.text().to_string(), font, fg);

    let pad_x = space::S;
    let pad_y = space::XXS;
    let desired = Vec2::new(
        galley.size().x + pad_x * 2.0,
        galley.size().y.max(12.0) + pad_y * 2.0,
    );

    let (rect, response) = ui.allocate_exact_size(desired, Sense::hover());

    if ui.is_rect_visible(rect) {
        let cr = radius::cr(radius::SM);
        ui.painter().rect_filled(rect, cr, kind.bg());

        let text_pos = rect.center() - galley.size() / 2.0;
        ui.painter().galley(text_pos, galley, fg);
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_every_kind() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for kind in [
                    BadgeKind::Info,
                    BadgeKind::Success,
                    BadgeKind::Warn,
                    BadgeKind::Error,
                    BadgeKind::Neutral,
                ] {
                    let _ = badge(ui, "label", kind);
                }
            });
        });
    }
}
