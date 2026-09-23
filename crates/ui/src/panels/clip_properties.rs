//! Clip properties panel — Video / Sound / Effects tabs.

use caprust_core::{Clip, ClipType, ProjectState};
use egui::Ui;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PropertiesTab {
    #[default]
    Video,
    Sound,
    Effects,
}

#[derive(Debug, Default)]
pub struct PropertiesState {
    pub tab: PropertiesTab,
    /// Pending edits to be committed via SetClipCommand.
    pub pending: Vec<PendingEdit>,
}

#[derive(Debug)]
pub enum PendingEdit {
    Speed(f32),
    Reverse(bool),
    FlipH(bool),
    FlipV(bool),
    VolumeDb(f32),
    TrimStart(u64),
    TrimDuration(u64),
    TrackIndex(usize),
}

pub fn show(
    ui: &mut Ui,
    project: &ProjectState,
    selected: Option<Uuid>,
    state: &mut PropertiesState,
) {
    let Some(id) = selected else {
        ui.label(
            egui::RichText::new("Select a clip to edit its properties.")
                .italics()
                .color(egui::Color32::from_gray(140)),
        );
        return;
    };
    let Some(clip) = project.clips.iter().find(|c| c.id == id) else {
        ui.label(
            egui::RichText::new("Selected clip no longer exists.")
                .italics()
                .color(egui::Color32::from_gray(140)),
        );
        return;
    };

    // --- Header ---
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(clip_kind_label(clip))
                .strong()
                .size(13.0),
        );
        ui.separator();
        ui.label(
            egui::RichText::new(format_duration(clip.start_time_ms + clip.duration_ms))
                .monospace()
                .color(egui::Color32::from_gray(180)),
        );
    });
    ui.separator();

    // --- Tabs ---
    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.tab, PropertiesTab::Video, "Video");
        ui.selectable_value(&mut state.tab, PropertiesTab::Sound, "Sound");
        ui.selectable_value(&mut state.tab, PropertiesTab::Effects, "Effects");
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match state.tab {
            PropertiesTab::Video => show_video(ui, clip, state),
            PropertiesTab::Sound => show_sound(ui, clip, state),
            PropertiesTab::Effects => show_effects(ui),
        });
}

fn show_video(ui: &mut Ui, clip: &Clip, state: &mut PropertiesState) {
    // --- Main ---
    ui.label(egui::RichText::new("Main").strong());
    ui.add_space(4.0);
    egui::Grid::new("clip_main_grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Rotation");
            let mut rot = 0.0_f32;
            ui.add(
                egui::DragValue::new(&mut rot)
                    .range(-180.0..=180.0)
                    .suffix("°"),
            );
            ui.end_row();

            ui.label("Width");
            let mut w = 100.0_f32;
            ui.add(
                egui::DragValue::new(&mut w)
                    .range(10.0..=400.0)
                    .suffix(" %"),
            );
            ui.end_row();

            ui.label("Height");
            let mut h = 100.0_f32;
            ui.add(
                egui::DragValue::new(&mut h)
                    .range(10.0..=400.0)
                    .suffix(" %"),
            );
            ui.end_row();
        });

    ui.add_space(10.0);
    ui.separator();

    // --- Speed ---
    ui.label(egui::RichText::new("Speed").strong());
    ui.add_space(4.0);
    egui::Grid::new("clip_speed_grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Speed");
            let mut speed = clip.speed;
            let combo = egui::ComboBox::from_id_salt("clip_speed")
                .selected_text(format!("{:.2}×", speed))
                .width(120.0);
            combo.show_ui(ui, |ui| {
                for s in [0.1, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0] {
                    if ui
                        .selectable_value(&mut speed, s, format!("{s:.2}×"))
                        .changed()
                    {
                        state.pending.push(PendingEdit::Speed(speed));
                    }
                }
            });
            ui.end_row();

            ui.label("Custom");
            let mut custom = speed;
            if ui
                .add(
                    egui::DragValue::new(&mut custom)
                        .range(0.1..=10.0)
                        .speed(0.01)
                        .suffix("×"),
                )
                .changed()
            {
                state.pending.push(PendingEdit::Speed(custom));
            }
            ui.end_row();

            ui.label("Reverse");
            let mut rev = clip.reversed;
            if ui.checkbox(&mut rev, "").changed() {
                state.pending.push(PendingEdit::Reverse(rev));
            }
            ui.end_row();
        });

    ui.add_space(10.0);
    ui.separator();

    // --- Mirror ---
    ui.label(egui::RichText::new("Mirror").strong());
    ui.add_space(4.0);
    egui::Grid::new("clip_mirror_grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Mirror horizontally");
            let mut h = clip.flip_h;
            if ui.checkbox(&mut h, "").changed() {
                state.pending.push(PendingEdit::FlipH(h));
            }
            ui.end_row();

            ui.label("Mirror vertically");
            let mut v = clip.flip_v;
            if ui.checkbox(&mut v, "").changed() {
                state.pending.push(PendingEdit::FlipV(v));
            }
            ui.end_row();
        });

    ui.add_space(10.0);
    ui.separator();

    // --- Trim (read-only info) ---
    ui.label(egui::RichText::new("Trim").strong());
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new("Drag the handles on the clip's edges in the timeline.")
            .small()
            .color(egui::Color32::from_gray(150)),
    );
    egui::Grid::new("clip_trim_grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Start");
            ui.label(format_duration(clip.start_time_ms));
            ui.end_row();
            ui.label("Duration");
            ui.label(format_duration(clip.duration_ms));
            ui.end_row();
            ui.label("Source");
            if clip.source_duration_ms == 0 {
                ui.label("unlimited");
            } else {
                ui.label(format_duration(clip.source_duration_ms));
            }
            ui.end_row();
        });
}

fn show_sound(ui: &mut Ui, clip: &Clip, state: &mut PropertiesState) {
    ui.label(egui::RichText::new("Volume").strong());
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let mut vol = clip.volume_db;
        if ui
            .add(
                egui::Slider::new(&mut vol, -30.0..=30.0)
                    .suffix(" dB")
                    .show_value(true),
            )
            .changed()
        {
            state.pending.push(PendingEdit::VolumeDb(vol));
        }
    });

    ui.add_space(10.0);
    ui.separator();

    ui.label(egui::RichText::new("Fade").strong());
    ui.add_space(4.0);
    let mut fade_in = 0.0_f32;
    let mut fade_out = 0.0_f32;
    egui::Grid::new("clip_fade_grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Fade in");
            ui.add(
                egui::Slider::new(&mut fade_in, 0.0..=50.0)
                    .suffix(" %")
                    .show_value(true),
            );
            ui.end_row();
            ui.label("Fade out");
            ui.add(
                egui::Slider::new(&mut fade_out, 0.0..=50.0)
                    .suffix(" %")
                    .show_value(true),
            );
            ui.end_row();
        });

    ui.add_space(10.0);
    ui.separator();
    ui.label(egui::RichText::new("Processing").strong());
    ui.add_space(4.0);
    let mut norm = false;
    let mut denoise = false;
    let mut voice_boost = false;
    ui.checkbox(&mut norm, "Normalize sound");
    ui.checkbox(&mut denoise, "Decrease noise");
    ui.checkbox(&mut voice_boost, "Voice volume increase");
    ui.label(
        egui::RichText::new("(Wired in audio-engine PR.)")
            .small()
            .italics()
            .color(egui::Color32::from_gray(140)),
    );
}

fn show_effects(ui: &mut Ui) {
    ui.label(egui::RichText::new("Effects").strong());
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new("No effect selected.")
            .italics()
            .color(egui::Color32::from_gray(140)),
    );
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(
            "Pick a filter, transition, or audio effect from the left panel \
             to attach it to this clip.",
        )
        .small()
        .color(egui::Color32::from_gray(160)),
    );
}

fn clip_kind_label(clip: &Clip) -> &'static str {
    match &clip.clip_type {
        ClipType::Video { .. } => "Video Clip",
        ClipType::Audio { .. } => "Audio Clip",
        ClipType::Image { .. } => "Image Clip",
        ClipType::TextOverlay { .. } => "Text Overlay",
        ClipType::Captions { .. } => "Captions",
        ClipType::Narration { .. } => "Narration",
    }
}

fn format_duration(ms: u64) -> String {
    let s = ms / 1000;
    let m = s / 60;
    let sec = s % 60;
    let millis = ms % 1000;
    format!("{:02}:{:02}.{:03}", m, sec, millis)
}
