//! Settings dialog — Appearance, AI Models, Shortcuts, Language, Paths.

use crate::i18n_helper::tr;
use crate::theme::{Theme, ThemeMode, ACCENT_PRESETS};
use caprust_core::models::{ModelKind, ModelStatus};
use caprust_core::{detect_ffmpeg, AppSettings, FfmpegStatus, ModelRegistry};
use egui::Ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Appearance,
    Models,
    Shortcuts,
    Language,
    Paths,
    Audio,
}

pub struct SettingsEvents {
    pub save: bool,
    pub close: bool,
}

pub fn show(
    ui: &mut Ui,
    theme: &mut Theme,
    models: &mut ModelRegistry,
    settings: &mut AppSettings,
    tab: &mut SettingsTab,
    ffmpeg_status: &mut FfmpegStatus,
) -> SettingsEvents {
    let mut ev = SettingsEvents {
        save: false,
        close: false,
    };

    ui.horizontal(|ui| {
        ui.selectable_value(tab, SettingsTab::Appearance, tr("set-tab-appearance"));
        ui.selectable_value(tab, SettingsTab::Models, tr("set-tab-models"));
        ui.selectable_value(tab, SettingsTab::Shortcuts, tr("set-tab-shortcuts"));
        ui.selectable_value(tab, SettingsTab::Language, tr("set-tab-language"));
        ui.selectable_value(tab, SettingsTab::Paths, tr("set-tab-paths"));
        ui.selectable_value(tab, SettingsTab::Audio, tr("set-tab-audio"));
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .max_height(400.0)
        .show(ui, |ui| match tab {
            SettingsTab::Appearance => show_appearance(ui, theme),
            SettingsTab::Models => show_models(ui, models, settings),
            SettingsTab::Shortcuts => show_shortcuts(ui, &mut settings.enable_shortcuts),
            SettingsTab::Language => show_language(ui, &mut settings.language),
            SettingsTab::Paths => show_paths(ui, settings, ffmpeg_status),
            SettingsTab::Audio => show_audio(ui, settings),
        });

    ui.separator();
    ui.horizontal(|ui| {
        let save_btn = egui::Button::new(
            egui::RichText::new(tr("set-save"))
                .color(egui::Color32::WHITE)
                .strong(),
        )
        .fill(egui::Color32::from_rgb(34, 139, 230))
        .min_size(egui::vec2(120.0, 32.0));
        if ui.add(save_btn).clicked() {
            ev.save = true;
        }

        if ui
            .add(egui::Button::new(tr("set-cancel")).min_size(egui::vec2(100.0, 32.0)))
            .clicked()
        {
            ev.close = true;
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(tr("set-save-hint"))
                    .small()
                    .color(egui::Color32::from_gray(150)),
            );
        });
    });

    ev
}

// ---------------------------------------------------------------------------
// Appearance
// ---------------------------------------------------------------------------

fn show_appearance(ui: &mut Ui, theme: &mut Theme) {
    ui.label(egui::RichText::new(tr("set-tab-appearance")).strong());
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label(tr("set-appearance-mode"));
        for m in ThemeMode::all() {
            ui.selectable_value(&mut theme.mode, m, m.label());
        }
    });

    ui.add_space(8.0);

    ui.horizontal(|ui| {
        ui.label(tr("set-appearance-accent"));
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
        ui.label(egui::RichText::new(tr("set-appearance-custom")).strong());
        egui::Grid::new("custom_theme_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(tr("set-appearance-panel"));
                ui.color_edit_button_srgb(&mut theme.custom_panel);
                ui.end_row();
                ui.label(tr("set-appearance-window"));
                ui.color_edit_button_srgb(&mut theme.custom_window);
                ui.end_row();
                ui.label(tr("set-appearance-text"));
                ui.color_edit_button_srgb(&mut theme.custom_text);
                ui.end_row();
            });
    }

    ui.add_space(12.0);
    ui.separator();
    if ui.button(tr("set-appearance-reset")).clicked() {
        *theme = Theme::default();
    }
}

// ---------------------------------------------------------------------------
// AI Models
// ---------------------------------------------------------------------------

fn show_models(ui: &mut Ui, models: &mut ModelRegistry, settings: &AppSettings) {
    ui.label(egui::RichText::new(tr("set-models-heading")).strong());
    ui.label(
        egui::RichText::new(format!(
            "Models are downloaded on first use and stored in {}",
            settings.effective_models_dir().display()
        ))
        .small()
        .color(egui::Color32::from_gray(140)),
    );
    ui.add_space(8.0);

    ui.label(egui::RichText::new(tr("set-models-captions")).strong());
    show_model_group(ui, models, ModelKind::Caption);

    ui.add_space(12.0);
    ui.label(egui::RichText::new(tr("set-models-narration")).strong());
    show_model_group(ui, models, ModelKind::Narration);
}

fn show_model_group(ui: &mut Ui, models: &mut ModelRegistry, kind: ModelKind) {
    // NOTE: download location is read at Settings load; the fake downloader
    // below will use `models_dir()` from core. Real HTTP comes in Faza D.
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
                            if ui.button(tr("set-models-download")).clicked() {
                                // TODO: capture actual dir from settings — currently
                                // falls back to core default. Pass settings into
                                // show_model_group in a follow-up PR.
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
                            ui.checkbox(&mut m.enabled, tr("set-models-enabled"));
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

// ---------------------------------------------------------------------------
// Shortcuts
// ---------------------------------------------------------------------------

fn show_shortcuts(ui: &mut Ui, enable_shortcuts: &mut bool) {
    ui.label(egui::RichText::new(tr("set-shortcuts-heading")).strong());
    ui.add_space(6.0);
    ui.checkbox(enable_shortcuts, tr("set-shortcuts-enable"));
    ui.add_space(10.0);

    egui::Grid::new("kbd_shortcuts")
        .num_columns(2)
        .spacing([20.0, 6.0])
        .show(ui, |ui| {
            ui.label("R");
            ui.label("Reverse selected clip");
            ui.end_row();
            ui.label("H");
            ui.label("Mirror horizontally");
            ui.end_row();
            ui.label("V");
            ui.label("Mirror vertically");
            ui.end_row();
            ui.label("Ctrl+A");
            ui.label("Select all clips");
            ui.end_row();
            ui.label("Delete");
            ui.label("Delete selected clip");
            ui.end_row();
            ui.label("S");
            ui.label("Split at playhead");
            ui.end_row();
        });
}

// ---------------------------------------------------------------------------
// Language
// ---------------------------------------------------------------------------

fn show_language(ui: &mut Ui, language: &mut String) {
    ui.label(egui::RichText::new(tr("set-language-heading")).strong());
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(tr("set-language-applied"))
            .small()
            .color(egui::Color32::from_gray(150)),
    );
    ui.add_space(8.0);

    let current_name = caprust_i18n::LANGUAGES
        .iter()
        .find(|(c, _)| c == language)
        .map(|(_, n)| *n)
        .unwrap_or("English");

    egui::ComboBox::from_id_salt("language_picker")
        .selected_text(current_name)
        .width(200.0)
        .show_ui(ui, |ui| {
            for (code, name) in caprust_i18n::LANGUAGES {
                ui.selectable_value(language, code.to_string(), *name);
            }
        });
}

// ---------------------------------------------------------------------------
// Paths (models dir + ffmpeg)
// ---------------------------------------------------------------------------

fn show_paths(ui: &mut Ui, settings: &mut AppSettings, status: &mut FfmpegStatus) {
    ui.label(egui::RichText::new(tr("set-paths-models")).strong());
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut settings.models_dir)
                .desired_width(360.0)
                .hint_text("Where AI models are stored…"),
        );
        if ui.button(tr("new-button-browse")).clicked() {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                settings.models_dir = dir.to_string_lossy().to_string();
            }
        }
    });

    ui.add_space(16.0);
    ui.separator();
    ui.label(egui::RichText::new(tr("set-paths-ffmpeg")).strong());
    ui.label(
        egui::RichText::new(tr("set-paths-ffmpeg-hint"))
            .small()
            .color(egui::Color32::from_gray(150)),
    );
    ui.add_space(6.0);

    egui::Grid::new("ffmpeg_paths")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("ffmpeg");
            ui.horizontal(|ui| {
                let mut p = settings.ffmpeg_path.clone().unwrap_or_default();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut p)
                            .desired_width(280.0)
                            .hint_text("auto"),
                    )
                    .changed()
                {
                    settings.ffmpeg_path = if p.trim().is_empty() { None } else { Some(p) };
                }
                if ui.button(tr("new-button-browse")).clicked() {
                    if let Some(f) = rfd::FileDialog::new().pick_file() {
                        settings.ffmpeg_path = Some(f.to_string_lossy().to_string());
                    }
                }
            });
            ui.end_row();

            ui.label("ffprobe");
            ui.horizontal(|ui| {
                let mut p = settings.ffprobe_path.clone().unwrap_or_default();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut p)
                            .desired_width(280.0)
                            .hint_text("auto"),
                    )
                    .changed()
                {
                    settings.ffprobe_path = if p.trim().is_empty() { None } else { Some(p) };
                }
                if ui.button(tr("new-button-browse")).clicked() {
                    if let Some(f) = rfd::FileDialog::new().pick_file() {
                        settings.ffprobe_path = Some(f.to_string_lossy().to_string());
                    }
                }
            });
            ui.end_row();
        });

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button(tr("set-paths-detect")).clicked() {
            *status = detect_ffmpeg(settings);
        }
        ui.separator();
        match (&status.ffmpeg, &status.ffprobe) {
            (Some(_), Some(_)) => {
                ui.label(
                    egui::RichText::new(tr("set-paths-detected"))
                        .color(egui::Color32::from_rgb(80, 200, 120)),
                );
            }
            (Some(_), None) => {
                ui.label(
                    egui::RichText::new(tr("set-paths-partial"))
                        .color(egui::Color32::from_rgb(230, 180, 90)),
                );
            }
            (None, Some(_)) => {
                ui.label(
                    egui::RichText::new(tr("set-paths-partial"))
                        .color(egui::Color32::from_rgb(230, 180, 90)),
                );
            }
            (None, None) => {
                ui.label(
                    egui::RichText::new(tr("set-paths-not-detected"))
                        .color(egui::Color32::from_rgb(230, 90, 90)),
                );
            }
        }
    });

    if let (Some(p), _) = (&status.ffmpeg, &status.ffprobe) {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(p)
                .small()
                .monospace()
                .color(egui::Color32::from_gray(140)),
        );
    }
}

// ---------------------------------------------------------------------------
// Audio tab
// ---------------------------------------------------------------------------

fn show_audio(ui: &mut Ui, settings: &mut AppSettings) {
    ui.label(egui::RichText::new(tr("set-tab-audio")).strong());
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(tr("set-audio-hint"))
            .small()
            .color(egui::Color32::from_gray(150)),
    );
    ui.add_space(12.0);

    ui.horizontal(|ui| {
        // Mute toggle on the left, matching the transport bar order.
        let mute_label = if settings.muted {
            tr("set-audio-unmute")
        } else {
            tr("set-audio-mute")
        };
        if ui.button(mute_label).clicked() {
            settings.muted = !settings.muted;
        }

        ui.separator();

        // Master volume slider. Disabled while muted (visual cue only;
        // the value is preserved so unmuting restores the previous level).
        ui.add_enabled_ui(!settings.muted, |ui| {
            ui.label(tr("set-audio-volume"));
            let slider =
                egui::Slider::new(&mut settings.master_volume, 0.0..=1.0).show_value(false);
            ui.add_sized([200.0, 20.0], slider);

            // Show as integer percent so the user sees a stable value.
            let pct = (settings.master_volume * 100.0).round() as i32;
            ui.label(format!("{pct}%"));
        });
    });

    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(tr("set-audio-preview-only"))
            .small()
            .italics()
            .color(egui::Color32::from_gray(130)),
    );
}
