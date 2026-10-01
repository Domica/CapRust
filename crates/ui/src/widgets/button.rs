//! Button variants: primary (accent), secondary (default), ghost
//! (borderless), icon (square + tooltip).
//!
//! Panels should never call `ui.button(...)` directly. Use one of these
//! so the visual language stays consistent when the accent or the
//! elevation scale changes.

use egui::{Response, Ui, WidgetText};

/// Accent-filled primary action. Use once per screen for the "commit"
/// button (Save, Export, Apply).
pub fn primary(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    let accent = ui.visuals().selection.bg_fill;
    ui.add(egui::Button::new(text).fill(accent))
}

/// Default surface button. Use for the neutral action in a pair
/// (Cancel, Browse, Import).
pub fn secondary(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    ui.button(text)
}

/// Borderless text button. Use for tertiary actions (Reset, Clear) and
/// inline toolbar toggles that already have an icon.
pub fn ghost(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    ui.add(egui::Button::new(text).frame(false))
}

/// Square icon-only button with a hover tooltip. `glyph` is a Phosphor
/// constant (`egui_phosphor::regular::PLAY`); `tooltip` is an already-
/// translated string.
pub fn icon(ui: &mut Ui, glyph: &str, tooltip: &str) -> Response {
    ui.add(egui::Button::new(glyph).frame(false))
        .on_hover_text(tooltip)
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
    fn primary_renders_without_panic() {
        run_ui(|ui| {
            let _ = primary(ui, "OK");
        });
    }

    #[test]
    fn secondary_renders_without_panic() {
        run_ui(|ui| {
            let _ = secondary(ui, "Cancel");
        });
    }

    #[test]
    fn ghost_renders_without_panic() {
        run_ui(|ui| {
            let _ = ghost(ui, "Reset");
        });
    }

    #[test]
    fn icon_renders_without_panic() {
        run_ui(|ui| {
            let _ = icon(ui, "X", "Close");
        });
    }
}
