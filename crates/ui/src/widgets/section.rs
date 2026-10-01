//! Section header pattern: heading + separator, with token-driven
//! spacing. Replaces the `ui.heading(tr("...")); ui.separator();` pair
//! that appears in ~20 panels.

use crate::theme::tokens::space;
use egui::{RichText, Ui};

/// Heading followed by a thin separator line. Adds `space::XS` above the
/// heading and `space::XXS` between the heading and the line so sections
/// breathe the same way everywhere.
pub fn header(ui: &mut Ui, title: impl Into<RichText>) {
    ui.add_space(space::XS);
    ui.heading(title);
    ui.add_space(space::XXS);
    ui.separator();
}

/// Secondary line: small, muted. Use for "Model: x | Language: y" style
/// summary rows. Caller keeps control of color via `RichText::color`.
pub fn subtext(ui: &mut Ui, text: impl Into<RichText>) {
    ui.label(text.into().small());
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Context;

    fn run_ui<F: FnOnce(&mut Ui)>(f: F) {
        let ctx = Context::default();
        let mut f = Some(f);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            if let Some(f) = f.take() {
                egui::CentralPanel::default().show(ctx, f);
            }
        });
    }

    #[test]
    fn header_renders_without_panic() {
        run_ui(|ui| {
            header(ui, "Section");
        });
    }

    #[test]
    fn subtext_renders_without_panic() {
        run_ui(|ui| {
            subtext(ui, "Model: whisper-tiny");
        });
    }
}
