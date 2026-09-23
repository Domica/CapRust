//! Settings dialog (currently: appearance / theme).

use crate::theme::{Theme, ThemeMode, ACCENT_PRESETS};
use egui::Ui;

pub fn show(ui: &mut Ui, theme: &mut Theme) {
    ui.heading("Settings");
    ui.separator();

    ui.label(egui::RichText::new("Appearance").strong());
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label("Mode:");
        for m in ThemeMode::all() {
            ui.selectable_value(&mut theme.mode, m, m.label());
        }
    });

    ui.add_space(8.0);

    ui.horizontal(|ui| {
        ui.label("Accent:");
        for (name, rgb) in ACCENT_PRESETS {
            let color = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
            let selected = theme.accent == *rgb;
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
            ui.painter().rect_filled(rect, 4.0, color);
            if selected {
                ui.painter().rect_stroke(
                    rect,
                    4.0,
                    egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
                    egui::StrokeKind::Outside,
                );
            }
            if resp.clicked() {
                theme.accent = *rgb;
            }
            resp.on_hover_text(*name);
        }
        ui.separator();
        let mut rgb = theme.accent;
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            theme.accent = rgb;
        }
    });

    if theme.mode == ThemeMode::Custom {
        ui.add_space(12.0);
        ui.label(egui::RichText::new("Custom colors").strong());
        egui::Grid::new("custom_theme_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Panel background");
                ui.color_edit_button_srgb(&mut theme.custom_panel);
                ui.end_row();

                ui.label("Window background");
                ui.color_edit_button_srgb(&mut theme.custom_window);
                ui.end_row();

                ui.label("Text color");
                ui.color_edit_button_srgb(&mut theme.custom_text);
                ui.end_row();
            });
    }

    ui.add_space(12.0);
    ui.separator();
    if ui.button("Reset to defaults").clicked() {
        *theme = Theme::default();
    }
}
