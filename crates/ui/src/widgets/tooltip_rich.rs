//! Rich tooltip: title + body, styled with tokens.

use crate::theme::tokens::{space, text};

/// Attach a two-line tooltip (title in bold, body in regular) to an
/// existing `Response`. Prefer this over `Response::on_hover_text`
/// when the message has structure: a heading plus a sentence or two.
pub fn tooltip_rich(response: egui::Response, title: &str, body: &str) -> egui::Response {
    response.on_hover_ui(|ui| {
        // Constrain width so long bodies wrap instead of stretching
        // the tooltip across the screen.
        ui.set_max_width(360.0);
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(title).strong().size(text::M));
            if !body.is_empty() {
                ui.add_space(space::XXS);
                ui.label(
                    egui::RichText::new(body)
                        .size(text::S)
                        .color(ui.visuals().weak_text_color()),
                );
            }
        });
    })
}

/// Variant that skips the title if empty, useful when the caller only
/// has body text and does not want a blank heading line.
pub fn tooltip_body(response: egui::Response, body: &str) -> egui::Response {
    response.on_hover_ui(|ui| {
        ui.set_max_width(360.0);
        ui.label(
            egui::RichText::new(body)
                .size(text::S)
                .color(ui.visuals().weak_text_color()),
        );
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_response(ui: &mut egui::Ui) -> egui::Response {
        ui.allocate_response(egui::Vec2::new(10.0, 10.0), egui::Sense::hover())
    }

    #[test]
    fn tooltip_rich_attaches_without_panic() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let r = make_response(ui);
                let _ = tooltip_rich(r, "Title", "Body text goes here.");
            });
        });
    }

    #[test]
    fn tooltip_rich_empty_body() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let r = make_response(ui);
                let _ = tooltip_rich(r, "Title only", "");
            });
        });
    }

    #[test]
    fn tooltip_body_attaches_without_panic() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let r = make_response(ui);
                let _ = tooltip_body(r, "just a body");
            });
        });
    }
}
