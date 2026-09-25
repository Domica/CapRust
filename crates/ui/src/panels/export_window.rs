//! Export video dialog: Summary, Destination, Output, Quality, Advanced.

use crate::i18n_helper::tr;
use caprust_media_io::export::{ExportFrameRate, ExportResolution, RateMode};
use egui::Ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    H264,
    Hevc,
    Av1,
}

impl Codec {
    pub fn label(&self) -> &'static str {
        match self {
            Self::H264 => "H.264 (AVC)",
            Self::Hevc => "H.265 (HEVC)",
            Self::Av1 => "AV1",
        }
    }
    pub fn all() -> [Self; 3] {
        [Self::H264, Self::Hevc, Self::Av1]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityTier {
    Small,
    Regular,
    Large,
}

impl QualityTier {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Regular => "Regular",
            Self::Large => "Large",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorRange {
    Limited,
    Full,
}

impl ColorRange {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Limited => "Limited (16–235)",
            Self::Full => "Full (0–255)",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExportState {
    pub destination: String,
    pub resolution: ExportResolution,
    pub frame_rate: ExportFrameRate,
    pub codec: Codec,
    pub quality: QualityTier,
    pub advanced: bool,
    pub rate_mode: RateMode,
    pub bitrate_kbps: u32,
    pub color_range: ColorRange,
}

impl Default for ExportState {
    fn default() -> Self {
        Self {
            destination: default_videos_dir(),
            resolution: ExportResolution::FullHd,
            frame_rate: ExportFrameRate::Original,
            codec: Codec::H264,
            quality: QualityTier::Regular,
            advanced: false,
            rate_mode: RateMode::Vbr,
            bitrate_kbps: 8000,
            color_range: ColorRange::Limited,
        }
    }
}

fn default_videos_dir() -> String {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(|h| format!("{h}/Videos"))
        .unwrap_or_else(|_| ".".into())
}

/// Returns true if the user clicked Export.
pub fn show(ui: &mut Ui, state: &mut ExportState, duration_ms: u64, clip_count: usize) -> bool {
    let mut clicked_export = false;

    // --- Summary ---
    ui.label(egui::RichText::new(tr("exp-summary")).strong());
    ui.add_space(4.0);
    ui.label(format!(
        "{}: {}",
        tr("exp-duration"),
        format_duration(duration_ms)
    ));
    ui.label(format!("{}: {}", tr("exp-clip-count"), clip_count));
    ui.add_space(12.0);
    ui.separator();

    // --- Destination ---
    ui.label(egui::RichText::new(tr("exp-destination")).strong());
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.destination)
                .desired_width(280.0)
                .hint_text("Folder…"),
        );
        if ui.button(tr("exp-browse")).clicked() {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                state.destination = dir.to_string_lossy().to_string();
            }
        }
    });
    ui.add_space(12.0);
    ui.separator();

    // --- Output ---
    ui.label(egui::RichText::new(tr("exp-output")).strong());
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label(tr("exp-resolution"));
        egui::ComboBox::from_id_salt("exp_res")
            .selected_text(state.resolution.label())
            .width(240.0)
            .show_ui(ui, |ui| {
                for r in ExportResolution::all() {
                    ui.selectable_value(&mut state.resolution, r, r.label());
                }
            });
    });

    ui.horizontal(|ui| {
        ui.label(tr("exp-fps"));
        egui::ComboBox::from_id_salt("exp_fps")
            .selected_text(state.frame_rate.label())
            .width(240.0)
            .show_ui(ui, |ui| {
                for r in ExportFrameRate::all() {
                    ui.selectable_value(&mut state.frame_rate, r, r.label());
                }
            });
    });

    ui.horizontal(|ui| {
        ui.label(tr("exp-codec"));
        egui::ComboBox::from_id_salt("exp_codec")
            .selected_text(state.codec.label())
            .width(240.0)
            .show_ui(ui, |ui| {
                for c in Codec::all() {
                    ui.selectable_value(&mut state.codec, c, c.label());
                }
            });
    });

    ui.add_space(12.0);
    ui.separator();

    // --- Quality ---
    ui.label(egui::RichText::new(tr("exp-quality")).strong());
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for q in [QualityTier::Small, QualityTier::Regular, QualityTier::Large] {
            ui.selectable_value(&mut state.quality, q, q.label());
        }
    });

    ui.add_space(12.0);
    ui.separator();

    // --- Advanced toggle ---
    ui.checkbox(&mut state.advanced, tr("exp-advanced"));

    if state.advanced {
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.label(tr("exp-bitrate"));
            ui.selectable_value(&mut state.rate_mode, RateMode::Vbr, "VBR");
            ui.selectable_value(&mut state.rate_mode, RateMode::Cbr, "CBR");
            if state.rate_mode == RateMode::Cbr {
                ui.add(
                    egui::DragValue::new(&mut state.bitrate_kbps)
                        .range(500..=100_000)
                        .suffix(" kbps"),
                );
            }
        });

        ui.horizontal(|ui| {
            ui.label(tr("exp-color-range"));
            egui::ComboBox::from_id_salt("exp_color_range")
                .selected_text(state.color_range.label())
                .width(220.0)
                .show_ui(ui, |ui| {
                    for c in [ColorRange::Limited, ColorRange::Full] {
                        ui.selectable_value(&mut state.color_range, c, c.label());
                    }
                });
        });
    }

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(8.0);

    // --- Export button ---
    let btn = egui::Button::new(
        egui::RichText::new(tr("exp-button"))
            .size(16.0)
            .color(egui::Color32::WHITE)
            .strong(),
    )
    .fill(egui::Color32::from_rgb(34, 139, 230))
    .min_size(egui::vec2(ui.available_width(), 40.0));

    if ui.add(btn).clicked() {
        clicked_export = true;
    }

    clicked_export
}

fn format_duration(ms: u64) -> String {
    let total_s = ms / 1000;
    let h = total_s / 3600;
    let m = (total_s % 3600) / 60;
    let s = total_s % 60;
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}
