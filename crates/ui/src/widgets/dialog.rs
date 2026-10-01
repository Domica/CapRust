//! Modal dialog footer: left-aligned secondary actions + right-aligned
//! primary action(s). Replaces the `ui.horizontal { ... ui.with_layout(
//! right_to_left ...) }` pattern that appears in every modal.

use egui::{Align, Layout, Ui};

/// Generic footer. `add_left` renders against the left margin, then
/// `add_right` renders in a right-to-left layout so its first widget
/// sits flush against the right edge.
pub fn footer(ui: &mut Ui, add_left: impl FnOnce(&mut Ui), add_right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        add_left(ui);
        ui.with_layout(Layout::right_to_left(Align::Center), add_right);
    });
}

/// Common case: optional left-side action(s) + a single right-aligned
/// Close/Cancel button. Returns `true` if the close button was clicked.
pub fn footer_close(ui: &mut Ui, add_left: impl FnOnce(&mut Ui), close_label: &str) -> bool {
    let mut close = false;
    footer(ui, add_left, |ui| {
        if ui.button(close_label).clicked() {
            close = true;
        }
    });
    close
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
    fn footer_renders_left_and_right() {
        run_ui(|ui| {
            footer(
                ui,
                |ui| {
                    let _ = ui.button("Retry");
                },
                |ui| {
                    let _ = ui.button("Close");
                },
            );
        });
    }

    #[test]
    fn footer_close_returns_false_without_input() {
        run_ui(|ui| {
            let clicked = footer_close(ui, |_| {}, "Close");
            assert!(!clicked);
        });
    }
}
