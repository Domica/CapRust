//! Missing-media relink dialog.

use egui::Ui;

pub fn show(ui: &mut Ui, missing_count: usize) {
    if missing_count == 0 {
        return;
    }

    ui.heading(format!("⚠ {} media file(s) not found", missing_count));
    ui.label("Files could not be located. Relink them or dismiss to continue.");

    ui.horizontal(|ui| {
        let _ = ui.button("Dismiss");
        let _ = ui.button("Relink All…");
    });
}
