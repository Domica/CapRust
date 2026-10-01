//! Property row: a label followed by an editor, laid out horizontally.
//! Replaces the `ui.horizontal { ui.label(...); ui.add(editor) }` pattern
//! that appears ~30 times in `clip_properties.rs` alone.

use egui::{Ui, WidgetText};

/// Render `label` and `add_editor` on one row. Returns whatever
/// `add_editor` returns (typically a `Response` or a `bool` from
/// `DragValue`/`ComboBox` change detection).
pub fn row<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    add_editor: impl FnOnce(&mut Ui) -> R,
) -> R {
    ui.horizontal(|ui| {
        ui.label(label);
        add_editor(ui)
    })
    .inner
}

/// Same as `row`, but skip the label entirely. Useful when the editor
/// is a full-width control and the label goes into the row above.
pub fn row_bare<R>(ui: &mut Ui, add_editor: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(add_editor).inner
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
    fn row_returns_editor_result() {
        run_ui(|ui| {
            let v = row(ui, "Name", |ui| {
                let mut s = String::from("x");
                let _ = ui.text_edit_singleline(&mut s);
                7
            });
            assert_eq!(v, 7);
        });
    }

    #[test]
    fn row_bare_returns_editor_result() {
        run_ui(|ui| {
            let v = row_bare(ui, |ui| {
                let _ = ui.button("X");
                42
            });
            assert_eq!(v, 42);
        });
    }
}
