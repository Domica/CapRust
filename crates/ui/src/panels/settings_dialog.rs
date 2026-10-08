//! Settings dialog — Appearance, AI Models, Shortcuts, Language, Paths.

use crate::i18n_helper::tr;
use crate::theme::tokens::space;
use crate::theme::{Theme, ThemeMode, ACCENT_PRESETS};
use crate::widgets::button;
use crate::widgets::chip;
use crate::widgets::segmented_control;
use crate::widgets::switch;
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
    Translation,
    About,
}

pub struct SettingsEvents {
    pub save: bool,
    pub close: bool,
    /// Model id whose Download button was clicked this frame, if any.
    /// The app layer consumes this by calling start_model_download; the
    /// settings panel itself has no way to spawn threads.
    pub download_requested: Option<String>,
    /// User clicked "Export settings" in Paths -> Backup. app.rs opens
    /// a save dialog and writes AppSettings via export_to_file.
    pub export_requested: bool,
    /// User clicked "Import settings" in Paths -> Backup. app.rs opens
    /// a pick dialog and loads AppSettings via import_from_file.
    pub import_requested: bool,
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
        download_requested: None,
        export_requested: false,
        import_requested: false,
    };

    ui.horizontal(|ui| {
        ui.selectable_value(tab, SettingsTab::Appearance, tr("set-tab-appearance"));
        ui.selectable_value(tab, SettingsTab::Models, tr("set-tab-models"));
        ui.selectable_value(tab, SettingsTab::Shortcuts, tr("set-tab-shortcuts"));
        ui.selectable_value(tab, SettingsTab::Language, tr("set-tab-language"));
        ui.selectable_value(tab, SettingsTab::Paths, tr("set-tab-paths"));
        ui.selectable_value(tab, SettingsTab::Audio, tr("set-tab-audio"));
        ui.selectable_value(tab, SettingsTab::Translation, tr("set-tab-translation"));
        ui.selectable_value(tab, SettingsTab::About, tr("set-tab-about"));
    });
    ui.separator();

    // Fill the available height of the universal settings window
    // so tabs of different content lengths do not resize the panel.
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match tab {
            SettingsTab::Appearance => show_appearance(ui, theme, settings),
            SettingsTab::Models => {
                if let Some(id) = show_models(ui, models, settings) {
                    ev.download_requested = Some(id);
                }
            }
            SettingsTab::Shortcuts => show_shortcuts(ui, &mut settings.enable_shortcuts),
            SettingsTab::Language => show_language(ui, &mut settings.language),
            SettingsTab::Paths => show_paths(ui, settings, ffmpeg_status, &mut ev),
            SettingsTab::Audio => show_audio(ui, settings),
            SettingsTab::Translation => show_translation(ui, settings),
            SettingsTab::About => show_about(ui),
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

fn show_appearance(ui: &mut Ui, theme: &mut Theme, settings: &mut AppSettings) {
    ui.label(egui::RichText::new(tr("set-tab-appearance")).strong());
    ui.add_space(space::XS);

    ui.horizontal(|ui| {
        ui.label(tr("set-appearance-mode"));
        let all = ThemeMode::all();
        let labels: Vec<_> = all.iter().map(|m| m.label()).collect();
        let selected = all.iter().position(|m| *m == theme.mode).unwrap_or(0);
        let (_, clicked) = segmented_control::segmented_control(ui, &labels, selected);
        if let Some(i) = clicked {
            if i != selected {
                theme.mode = all[i];
            }
        }
    });

    ui.add_space(space::M);

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

    ui.add_space(space::L);

    // --- Font ---
    ui.label(egui::RichText::new(tr("set-appearance-font")).strong());
    ui.add_space(space::XS);
    ui.horizontal(|ui| {
        ui.label(tr("set-appearance-font-family"));
        egui::ComboBox::from_id_salt("font_family_combo")
            .selected_text(theme.font_family.label())
            .width(180.0)
            .show_ui(ui, |ui| {
                for f in crate::theme::UiFontFamily::all() {
                    ui.selectable_value(&mut theme.font_family, f, f.label());
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label(tr("set-appearance-font-scale"));
        ui.add(
            egui::Slider::new(&mut theme.font_scale, 0.8..=1.5)
                .fixed_decimals(2)
                .suffix("\u{00d7}"),
        );
        if button::ghost(ui, "1.0").clicked() {
            theme.font_scale = 1.0;
        }
    });
    ui.add_space(space::L);
    ui.separator();
    ui.label(egui::RichText::new(tr("set-appearance-tracks")).strong());
    ui.label(
        egui::RichText::new(tr("set-appearance-tracks-hint"))
            .small()
            .color(egui::Color32::from_gray(150)),
    );
    ui.add_space(space::S);
    egui::Grid::new("track_colors_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(tr("set-appearance-track-video"));
            ui.color_edit_button_srgb(&mut theme.track_video);
            ui.end_row();
            ui.label(tr("set-appearance-track-audio"));
            ui.color_edit_button_srgb(&mut theme.track_audio);
            ui.end_row();
            ui.label(tr("set-appearance-track-captions"));
            ui.color_edit_button_srgb(&mut theme.track_captions);
            ui.end_row();
            ui.label(tr("set-appearance-track-text"));
            ui.color_edit_button_srgb(&mut theme.track_text);
            ui.end_row();
        });

    ui.add_space(space::L);
    ui.separator();
    ui.label(egui::RichText::new(tr("set-appearance-timeline")).strong());
    ui.add_space(space::S);
    egui::Grid::new("timeline_colors_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(tr("set-appearance-overlap"));
            let mut c = egui::Color32::from_rgba_unmultiplied(
                theme.overlap_shading[0],
                theme.overlap_shading[1],
                theme.overlap_shading[2],
                theme.overlap_shading[3],
            );
            if ui.color_edit_button_srgba(&mut c).changed() {
                theme.overlap_shading = [c.r(), c.g(), c.b(), c.a()];
            }
            ui.end_row();
            ui.label(tr("set-appearance-waveform"));
            ui.color_edit_button_srgb(&mut theme.waveform);
            ui.end_row();
            ui.label(tr("set-appearance-waveform-positive"));
            ui.color_edit_button_srgb(&mut theme.waveform_positive);
            ui.end_row();
            ui.label(tr("set-appearance-waveform-negative"));
            ui.color_edit_button_srgb(&mut theme.waveform_negative);
            ui.end_row();
            ui.label(tr("set-appearance-waveform-center"));
            ui.color_edit_button_srgb(&mut theme.waveform_center);
            ui.end_row();
        });

    ui.add_space(space::L);
    ui.separator();
    ui.label(egui::RichText::new(tr("set-appearance-playhead")).strong());
    ui.add_space(space::S);
    egui::Grid::new("playhead_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(tr("set-appearance-playhead-color"));
            ui.color_edit_button_srgb(&mut theme.playhead);
            ui.end_row();
            ui.label(tr("set-appearance-playhead-size"));
            ui.horizontal(|ui| {
                let all = crate::theme::PlayheadSize::all();
                let labels: Vec<_> = all.iter().map(|s| tr(s.label_key())).collect();
                let selected = all
                    .iter()
                    .position(|s| *s == theme.playhead_size)
                    .unwrap_or(0);
                let (_, clicked) = segmented_control::segmented_control(ui, &labels, selected);
                if let Some(i) = clicked {
                    if i != selected {
                        theme.playhead_size = all[i];
                    }
                }
            });
            ui.end_row();
        });

    if theme.mode == ThemeMode::Custom {
        ui.add_space(space::L);
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

    ui.add_space(space::L);
    ui.separator();
    if button::ghost(ui, tr("set-appearance-reset")).clicked() {
        *theme = Theme::default();
    }

    ui.add_space(space::L);
    ui.separator();
    ui.label(egui::RichText::new(tr("set-appearance-logs")).strong());
    ui.label(
        egui::RichText::new(tr("set-appearance-logs-hint"))
            .small()
            .color(egui::Color32::from_gray(150)),
    );
    if button::secondary(ui, tr("set-appearance-open-logs")).clicked() {
        let dir = logs_dir();
        let _ = std::fs::create_dir_all(&dir);
        if let Err(e) = open::that(&dir) {
            tracing::warn!("open logs dir: {e}");
        }
    }

    ui.add_space(space::L);
    ui.separator();
    ui.label(egui::RichText::new(tr("set-appearance-updates")).strong());
    switch::switch_labeled(
        ui,
        &mut settings.check_for_updates,
        tr("set-appearance-check-updates"),
    );
    ui.label(
        egui::RichText::new(tr("set-appearance-updates-hint"))
            .small()
            .color(egui::Color32::from_gray(150)),
    );
}

// ---------------------------------------------------------------------------
// AI Models
// ---------------------------------------------------------------------------

/// Returns the id of a model whose Download button was clicked this
/// frame, if any. The caller (app.rs) hands it to start_model_download;
/// this panel cannot spawn threads itself.
fn show_models(ui: &mut Ui, models: &mut ModelRegistry, settings: &AppSettings) -> Option<String> {
    // Backfill registry entries added since this project was saved,
    // so a model introduced in a later build appears in Settings
    // without requiring a project reopen.
    models.merge_missing_defaults();
    ui.label(egui::RichText::new(tr("set-models-heading")).strong());
    ui.label(
        egui::RichText::new(format!(
            "Models are downloaded on first use and stored in {}",
            settings.effective_models_dir().display()
        ))
        .small()
        .color(egui::Color32::from_gray(140)),
    );
    ui.add_space(space::M);

    let mut clicked: Option<String> = None;

    ui.label(egui::RichText::new(tr("set-models-captions")).strong());
    if let Some(id) = show_model_group(ui, models, ModelKind::Caption) {
        clicked = Some(id);
    }

    ui.add_space(space::L);
    ui.label(egui::RichText::new(tr("set-models-narration")).strong());
    if let Some(id) = show_model_group(ui, models, ModelKind::Narration) {
        clicked = Some(id);
    }

    ui.add_space(space::L);
    ui.label(egui::RichText::new(tr("set-models-face")).strong());
    if let Some(id) = show_model_group(ui, models, ModelKind::FaceDetector) {
        clicked = Some(id);
    }

    ui.add_space(space::L);
    ui.label(egui::RichText::new(tr("set-models-bg")).strong());
    if let Some(id) = show_model_group(ui, models, ModelKind::BackgroundRemover) {
        clicked = Some(id);
    }

    ui.add_space(space::L);
    ui.label(egui::RichText::new(tr("set-models-scrfd")).strong());
    if let Some(id) = show_model_group(ui, models, ModelKind::ScrfdDetector) {
        clicked = Some(id);
    }

    clicked
}

/// Returns the id of a model whose Download button was clicked this
/// frame, if any. Does not mutate `status` itself -- the real
/// downloader in app.rs owns the state transition (see the note in the
/// NotDownloaded arm).
fn show_model_group(ui: &mut Ui, models: &mut ModelRegistry, kind: ModelKind) -> Option<String> {
    let ids: Vec<String> = models
        .models
        .iter()
        .filter(|m| m.kind == kind)
        .map(|m| m.id.clone())
        .collect();

    let mut download_request: Option<String> = None;
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
                            if button::primary(ui, tr("set-models-download")).clicked() {
                                // Only report the click. Flipping status
                                // here used to leave the button gone and
                                // a fake 0% progress bar behind, because
                                // nothing actually started a download.
                                // start_model_download in app.rs owns
                                // the state transition now.
                                download_request = Some(id.clone());
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
                            switch::switch_labeled(ui, &mut m.enabled, tr("set-models-enabled"));
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
    download_request
}

// ---------------------------------------------------------------------------
// Shortcuts
// ---------------------------------------------------------------------------

fn show_shortcuts(ui: &mut Ui, enable_shortcuts: &mut bool) {
    ui.label(egui::RichText::new(tr("set-shortcuts-heading")).strong());
    ui.add_space(space::S);
    switch::switch_labeled(ui, enable_shortcuts, tr("set-shortcuts-enable"));
    ui.add_space(space::M_PLUS);

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
    ui.add_space(space::S);
    ui.label(
        egui::RichText::new(tr("set-language-applied"))
            .small()
            .color(egui::Color32::from_gray(150)),
    );
    ui.add_space(space::M);

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

fn show_paths(
    ui: &mut Ui,
    settings: &mut AppSettings,
    status: &mut FfmpegStatus,
    ev: &mut SettingsEvents,
) {
    ui.label(egui::RichText::new(tr("set-paths-models")).strong());
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut settings.models_dir)
                .desired_width(360.0)
                .hint_text("Where AI models are stored…"),
        );
        if button::secondary(ui, tr("new-button-browse")).clicked() {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                settings.models_dir = dir.to_string_lossy().to_string();
            }
        }
    });

    ui.add_space(space::XL);
    ui.separator();
    ui.label(egui::RichText::new(tr("set-paths-captures")).strong());
    ui.label(
        egui::RichText::new(tr("set-paths-captures-hint"))
            .small()
            .color(egui::Color32::from_gray(150)),
    );
    ui.add_space(space::S);

    egui::Grid::new("capture_folders")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(tr("set-paths-screenshots"));
            ui.horizontal(|ui| {
                let mut d = settings.screenshots_dir.clone().unwrap_or_default();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut d)
                            .desired_width(280.0)
                            .hint_text(tr("set-paths-folder-auto")),
                    )
                    .changed()
                {
                    settings.screenshots_dir = if d.trim().is_empty() { None } else { Some(d) };
                }
                if button::secondary(ui, tr("new-button-browse")).clicked() {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        settings.screenshots_dir = Some(dir.to_string_lossy().to_string());
                    }
                }
                if settings.screenshots_dir.is_some()
                    && button::icon(ui, egui_phosphor::regular::X, &tr("set-backup-sync-clear"))
                        .clicked()
                {
                    settings.screenshots_dir = None;
                }
            });
            ui.end_row();

            ui.label(tr("set-paths-recordings"));
            ui.horizontal(|ui| {
                let mut d = settings.recordings_dir.clone().unwrap_or_default();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut d)
                            .desired_width(280.0)
                            .hint_text(tr("set-paths-folder-auto")),
                    )
                    .changed()
                {
                    settings.recordings_dir = if d.trim().is_empty() { None } else { Some(d) };
                }
                if button::secondary(ui, tr("new-button-browse")).clicked() {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        settings.recordings_dir = Some(dir.to_string_lossy().to_string());
                    }
                }
                if settings.recordings_dir.is_some()
                    && button::icon(ui, egui_phosphor::regular::X, &tr("set-backup-sync-clear"))
                        .clicked()
                {
                    settings.recordings_dir = None;
                }
            });
            ui.end_row();
        });

    ui.add_space(space::XL);
    ui.separator();
    ui.label(egui::RichText::new(tr("set-paths-ffmpeg")).strong());
    ui.label(
        egui::RichText::new(tr("set-paths-ffmpeg-hint"))
            .small()
            .color(egui::Color32::from_gray(150)),
    );
    ui.add_space(space::S);

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
                if button::secondary(ui, tr("new-button-browse")).clicked() {
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
                if button::secondary(ui, tr("new-button-browse")).clicked() {
                    if let Some(f) = rfd::FileDialog::new().pick_file() {
                        settings.ffprobe_path = Some(f.to_string_lossy().to_string());
                    }
                }
            });
            ui.end_row();
        });

    ui.add_space(space::M);
    ui.horizontal(|ui| {
        if button::secondary(ui, tr("set-paths-detect")).clicked() {
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
        ui.add_space(space::S);
        ui.label(
            egui::RichText::new(p)
                .small()
                .monospace()
                .color(egui::Color32::from_gray(140)),
        );
    }

    ui.add_space(space::XL);
    ui.separator();
    ui.label(egui::RichText::new(tr("set-backup-heading")).strong());
    ui.label(
        egui::RichText::new(tr("set-backup-sync-hint"))
            .small()
            .color(egui::Color32::from_gray(150)),
    );
    ui.add_space(space::XS);
    ui.horizontal(|ui| {
        let mut f = settings.sync_folder.clone().unwrap_or_default();
        if ui
            .add(
                egui::TextEdit::singleline(&mut f)
                    .desired_width(280.0)
                    .hint_text(tr("set-backup-sync-none")),
            )
            .changed()
        {
            settings.sync_folder = if f.trim().is_empty() { None } else { Some(f) };
        }
        if button::secondary(ui, tr("new-button-browse")).clicked() {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                settings.sync_folder = Some(dir.to_string_lossy().to_string());
            }
        }
        if settings.sync_folder.is_some()
            && button::icon(ui, egui_phosphor::regular::X, &tr("set-backup-sync-clear")).clicked()
        {
            settings.sync_folder = None;
        }
    });
    if let Some(ts) = settings.last_synced_at {
        ui.add_space(space::XXS);
        ui.label(
            egui::RichText::new(format!("{} {}", tr("set-backup-sync-last"), ts))
                .small()
                .color(egui::Color32::from_gray(150)),
        );
    } else if settings.sync_path().is_some() {
        ui.add_space(space::XXS);
        ui.label(
            egui::RichText::new(tr("set-backup-sync-never"))
                .small()
                .color(egui::Color32::from_gray(150)),
        );
    }
    ui.add_space(space::M);
    ui.label(
        egui::RichText::new(tr("set-backup-hint"))
            .small()
            .color(egui::Color32::from_gray(150)),
    );
    ui.add_space(space::S);
    ui.horizontal(|ui| {
        if button::primary(ui, tr("set-backup-export")).clicked() {
            ev.export_requested = true;
        }
        if button::secondary(ui, tr("set-backup-import")).clicked() {
            ev.import_requested = true;
        }
    });
}

// ---------------------------------------------------------------------------
// Audio tab
// ---------------------------------------------------------------------------

fn show_audio(ui: &mut Ui, settings: &mut AppSettings) {
    ui.label(egui::RichText::new(tr("set-tab-audio")).strong());
    ui.add_space(space::XS);
    ui.label(
        egui::RichText::new(tr("set-audio-hint"))
            .small()
            .color(egui::Color32::from_gray(150)),
    );
    ui.add_space(space::L);

    ui.horizontal(|ui| {
        // Mute toggle on the left, matching the transport bar order.
        let mute_label = if settings.muted {
            tr("set-audio-unmute")
        } else {
            tr("set-audio-mute")
        };
        if chip::chip(ui, mute_label, settings.muted).clicked() {
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

    ui.add_space(space::M);
    ui.label(
        egui::RichText::new(tr("set-audio-preview-only"))
            .small()
            .italics()
            .color(egui::Color32::from_gray(130)),
    );
}

/// Supported languages for the translation feature. Source allows
/// "auto" so MyMemory can pick; target must be concrete.
const TRANSLATE_SOURCE_LANGS: &[&str] = &["auto", "en", "hr", "de", "fr", "es", "it"];
const TRANSLATE_TARGET_LANGS: &[&str] = &["en", "hr", "de", "fr", "es", "it"];

fn show_translation(ui: &mut Ui, settings: &mut AppSettings) {
    ui.label(egui::RichText::new(tr("set-tab-translation")).strong());
    ui.add_space(space::XS);
    ui.label(
        egui::RichText::new(tr("set-translation-hint"))
            .small()
            .color(egui::Color32::from_gray(150)),
    );
    ui.add_space(space::M_PLUS);

    ui.horizontal(|ui| {
        ui.label(tr("set-translation-source"));
        egui::ComboBox::from_id_salt("translate_source")
            .selected_text(&settings.translate_source_lang)
            .width(100.0)
            .show_ui(ui, |ui| {
                for code in TRANSLATE_SOURCE_LANGS {
                    ui.selectable_value(
                        &mut settings.translate_source_lang,
                        (*code).to_string(),
                        *code,
                    );
                }
            });
    });

    ui.add_space(space::XS);

    ui.horizontal(|ui| {
        ui.label(tr("set-translation-target"));
        egui::ComboBox::from_id_salt("translate_target")
            .selected_text(&settings.translate_target_lang)
            .width(100.0)
            .show_ui(ui, |ui| {
                for code in TRANSLATE_TARGET_LANGS {
                    ui.selectable_value(
                        &mut settings.translate_target_lang,
                        (*code).to_string(),
                        *code,
                    );
                }
            });
    });

    ui.add_space(space::L);
    ui.label(
        egui::RichText::new(tr("set-translation-privacy"))
            .small()
            .color(egui::Color32::from_gray(150)),
    );
    ui.add_space(space::XS);
    ui.label(egui::RichText::new(tr("set-translation-email")).strong());
    ui.label(
        egui::RichText::new(tr("set-translation-email-hint"))
            .small()
            .color(egui::Color32::from_gray(150)),
    );
    ui.add_space(space::XS);
    let mut email = settings.translate_email.clone().unwrap_or_default();
    if ui
        .add(
            egui::TextEdit::singleline(&mut email)
                .desired_width(280.0)
                .hint_text("(optional)"),
        )
        .changed()
    {
        settings.translate_email = if email.trim().is_empty() {
            None
        } else {
            Some(email.clone())
        };
    }

    ui.add_space(space::L);
    ui.separator();
    ui.add_space(space::S);
    ui.label(
        egui::RichText::new(tr("set-translation-provider-note"))
            .small()
            .italics()
            .color(egui::Color32::from_gray(130)),
    );
}

// ---------------------------------------------------------------------------
// About tab
// ---------------------------------------------------------------------------

/// Build identity: version + commit so a screenshot of this tab always
/// tells which binary is under test.
fn show_about(ui: &mut Ui) {
    ui.label(egui::RichText::new(tr("set-tab-about")).strong());
    ui.add_space(space::XS);
    ui.label(
        egui::RichText::new(tr("about-hint"))
            .small()
            .color(egui::Color32::from_gray(150)),
    );
    ui.add_space(space::M);

    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let build = format!(
        "{profile} / {}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    egui::Grid::new("about_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(tr("about-version"));
            ui.label(egui::RichText::new(env!("CARGO_PKG_VERSION")).monospace());
            ui.end_row();
            ui.label(tr("about-commit"));
            ui.label(
                egui::RichText::new(option_env!("CAPRUST_COMMIT").unwrap_or("unknown")).monospace(),
            );
            ui.end_row();
            ui.label(tr("about-build"));
            ui.label(egui::RichText::new(build).monospace());
            ui.end_row();
            ui.label(tr("about-author"));
            ui.label("Domica / CapRust contributors");
            ui.end_row();
            ui.label(tr("about-license"));
            ui.label("MIT");
            ui.end_row();
        });
}

/// Where the app writes its log files. Duplicated from the app
/// crate so the Settings dialog can open the folder without
/// depending on the binary.
fn logs_dir() -> std::path::PathBuf {
    let base = std::env::var("APPDATA")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    std::path::Path::new(&base).join("CapRust").join("logs")
}
