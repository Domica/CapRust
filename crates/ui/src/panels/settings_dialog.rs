//! Settings dialog — Appearance + AI Models.

use crate::theme::{Theme, ThemeMode, ACCENT_PRESETS};
use caprust_core::models::{ModelKind, ModelStatus};
use caprust_core::ModelRegistry;
use egui::Ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Appearance,
    Models,
}

pub fn show(ui: &mut Ui, theme: &mut Theme, models: &mut ModelRegistry, tab: &mut SettingsTab) {
    ui.horizontal(|ui| {
        ui.selectable_value(tab, SettingsTab::Appearance, "Appearance");
        ui.selectable_value(tab, SettingsTab::Models, "AI Models");
    });
    ui.separator();

    match tab {
        SettingsTab::Appearance => show_appearance(ui, theme),
        SettingsTab::Models => show_models(ui, models),
    }
}

fn show_appearance(ui: &mut Ui, theme: &mut Theme) {
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

fn show_models(ui: &mut Ui, models: &mut ModelRegistry) {
    ui.label(egui::RichText::new("AI Models").strong());
    ui.label(
        egui::RichText::new(format!(
            "Models are downloaded on first use and stored in {}",
            caprust_core::models::models_dir().display()
        ))
        .small()
        .color(egui::Color32::from_gray(140)),
    );
    ui.add_space(8.0);

    ui.label(egui::RichText::new("Captions (speech-to-text)").strong());
    show_model_group(ui, models, ModelKind::Caption);

    ui.add_space(12.0);
    ui.label(egui::RichText::new("Narration (text-to-speech)").strong());
    show_model_group(ui, models, ModelKind::Narration);
}

fn show_model_group(ui: &mut Ui, models: &mut ModelRegistry, kind: ModelKind) {
    let ids: Vec<String> = models
        .models
        .iter()
        .filter(|m| m.kind == kind)
        .map(|m| m.id.clone())
        .collect();

    for id in ids {
        let m = models.models.iter_mut().find(|m| m.id == id).unwrap();
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&m.name).strong());
                        ui.label(
                            egui::RichText::new(format!("· {} MB · {}", m.size_mb, m.language))
                                .small()
                                .color(egui::Color32::from_gray(140)),
                        );
                    });
                    ui.label(
                        egui::RichText::new(&m.description)
                            .small()
                            .color(egui::Color32::from_gray(170)),
                    );
                });

                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| match m.status {
                        ModelStatus::NotDownloaded => {
                            if ui.button("⬇ Download").clicked() {
                                m.status = ModelStatus::Downloading;
                                m.progress = 0.0;
                            }
                        }
                        ModelStatus::Downloading => {
                            ui.add(
                                egui::ProgressBar::new(m.progress)
                                    .desired_width(120.0)
                                    .show_percentage(),
                            );
                        }
                        ModelStatus::Ready => {
                            ui.checkbox(&mut m.enabled, "Enabled");
                        }
                        ModelStatus::Error => {
                            ui.label(
                                egui::RichText::new("Error")
                                    .color(egui::Color32::from_rgb(230, 90, 90)),
                            );
                        }
                    },
                );
            });
        });
    }
}
