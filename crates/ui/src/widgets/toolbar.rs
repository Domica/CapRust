//! Toolbar wrapper: `horizontal_wrapped` with a stable cursor height.
//!
//! A plain `ui.horizontal(...)` clips trailing buttons when the dock
//! zone narrows; `horizontal_wrapped` reflows instead. Use this wherever
//! a row of buttons sits at the top of a panel.

use egui::Ui;

/// Run `add_contents` inside a wrapped horizontal row, returning its
/// value. Replaces `ui.horizontal_wrapped(|ui| { ... });`.
pub fn bar<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal_wrapped(add_contents).inner
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
    fn bar_returns_inner_value() {
        run_ui(|ui| {
            let n = bar(ui, |ui| {
                let _ = ui.button("A");
                let _ = ui.button("B");
                42
            });
            assert_eq!(n, 42);
        });
    }
}
