//! Indeterminate loading indicator. Wraps `egui::Spinner` with the app's
//! design tokens so a spinner in any panel renders identically.
//!
//! Use `spinner_with` for inline "Loading X..." rows and `centered` for
//! full-panel loading states (initial project load, model download).

use crate::theme::tokens::space;
use egui::{Color32, RichText, Ui};

/// Spinner size in an inline row (to the left of a label).
/// Component-specific: matches the height of a `text::S` label row.
const SPINNER_INLINE: f32 = 16.0;
/// Spinner size in a centered, full-panel loading state.
const SPINNER_BLOCK: f32 = 32.0;

/// Bare spinner at the inline size.
pub fn spinner(ui: &mut Ui) {
    ui.add(egui::Spinner::new().size(SPINNER_INLINE));
}

/// Spinner + muted label in a horizontal row. `msg` must already be
/// translated by the caller (use `tr("...")`).
pub fn spinner_with(ui: &mut Ui, msg: impl Into<String>) {
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(SPINNER_INLINE));
        ui.add_space(space::S);
        ui.label(RichText::new(msg.into()).color(Color32::from_gray(180)));
    });
}

/// Centered full-panel loading state: spinner on top, muted label
/// underneath. Mirrors the layout of `widgets::empty::placeholder`.
pub fn centered(ui: &mut Ui, msg: impl Into<String>) {
    ui.add_space(space::XXL);
    ui.vertical_centered(|ui| {
        ui.add(egui::Spinner::new().size(SPINNER_BLOCK));
        ui.add_space(space::M);
        ui.label(RichText::new(msg.into()).color(Color32::from_gray(160)));
    });
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
    fn spinner_renders_without_panic() {
        run_ui(|ui| {
            spinner(ui);
        });
    }

    #[test]
    fn spinner_with_renders_without_panic() {
        run_ui(|ui| {
            spinner_with(ui, "Loading...");
        });
    }

    #[test]
    fn centered_renders_without_panic() {
        run_ui(|ui| {
            centered(ui, "Preparing project...");
        });
    }
}
