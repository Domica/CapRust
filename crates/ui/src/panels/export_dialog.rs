//! Export dialog scaffold. From Concat #175 + #143.

use caprust_media_io::export::{ExportFrameRate, ExportResolution, RateMode};
use egui::Ui;

pub fn show(ui: &mut Ui) {
    ui.heading("Export Settings");
    ui.separator();

    let mut resolution = ExportResolution::FullHd;
    egui::ComboBox::from_label("Resolution")
        .selected_text(resolution.label())
        .show_ui(ui, |ui| {
            for res in ExportResolution::all() {
                ui.selectable_value(&mut resolution, res, res.label());
            }
        });

    let mut rate = ExportFrameRate::Original;
    egui::ComboBox::from_label("Frame Rate")
        .selected_text(rate.label())
        .show_ui(ui, |ui| {
            for r in ExportFrameRate::all() {
                ui.selectable_value(&mut rate, r, r.label());
            }
        });

    let mut mode = RateMode::Vbr;
    egui::ComboBox::from_label("Bitrate Mode")
        .selected_text(mode.label())
        .show_ui(ui, |ui| {
            for m in RateMode::all() {
                ui.selectable_value(&mut mode, m, m.label());
            }
        });
}
