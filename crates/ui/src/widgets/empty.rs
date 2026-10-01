//! Empty-state placeholder: vertically centered, muted, italics.
//! Replaces the `ui.add_space(space::XXL); ui.vertical_centered(...)`
//! pattern that appears in every panel that can have zero items.

use crate::theme::tokens::space;
use egui::{Color32, RichText, Ui};

/// Single-line muted placeholder. `msg` should be an already-translated
/// string; callers using FTL keys pass `tr("...")`.
pub fn placeholder(ui: &mut Ui, msg: impl Into<String>) {
    ui.add_space(space::XXL);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(msg.into())
                .italics()
                .color(Color32::from_gray(140)),
        );
    });
}

/// Same layout, but lets the caller render richer content (icon, extra
/// lines, paths). The closure runs inside `vertical_centered`.
pub fn placeholder_with(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui)) {
    ui.add_space(space::XXL);
    ui.vertical_centered(add_contents);
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
    fn placeholder_renders_without_panic() {
        run_ui(|ui| {
            placeholder(ui, "No items");
        });
    }

    #[test]
    fn placeholder_with_renders_closure() {
        run_ui(|ui| {
            placeholder_with(ui, |ui| {
                ui.label("line 1");
                ui.label("line 2");
            });
        });
    }
}
