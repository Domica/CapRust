//! Modular preview: frame + transport controls + ratio/quality selectors.

use caprust_core::AspectRatio;
use egui::{Color32, RichText, Ui, Vec2};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewQuality {
    Quarter,
    Half,
    Full,
}

impl PreviewQuality {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Quarter => "1/4",
            Self::Half => "1/2",
            Self::Full => "1:1",
        }
    }
    pub fn all() -> [Self; 3] {
        [Self::Quarter, Self::Half, Self::Full]
    }
}

#[derive(Debug, Clone)]
pub struct PreviewState {
    pub playing: bool,
    pub quality: PreviewQuality,
    pub loop_playback: bool,
    /// When true, preview undocks into a floating window.
    pub floating: bool,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            playing: false,
            quality: PreviewQuality::Full,
            loop_playback: true,
            floating: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PreviewEvents {
    pub seek_back_30: bool,
    pub seek_back_5: bool,
    pub toggle_play: bool,
    pub seek_fwd_5: bool,
    pub seek_fwd_30: bool,
    pub toggle_loop: bool,
    /// User picked a different ratio → Some(new).
    pub ratio_changed: bool,
}

fn transport_button(ui: &mut Ui, icon: &str, tooltip: &str, size: Vec2) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let bg = if resp.hovered() {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        Color32::from_gray(50)
    };
    ui.painter().rect_filled(rect, 5.0, bg);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(16.0),
        Color32::from_gray(220),
    );
    resp.on_hover_text(tooltip).clicked()
}

/// Draws only the transport bar + selectors (frame area is drawn by the caller).
pub fn show_transport(
    ui: &mut Ui,
    state: &mut PreviewState,
    playhead_ms: u64,
    total_ms: u64,
    ratio: &mut AspectRatio,
) -> PreviewEvents {
    let mut ev = PreviewEvents::default();

    ui.horizontal(|ui| {
        // --- Left: ratio dropdown ---
        egui::ComboBox::from_id_salt("preview_ratio")
            .selected_text(ratio.label())
            .width(150.0)
            .show_ui(ui, |ui| {
                for preset in AspectRatio::presets() {
                    let mut p = preset.clone();
                    if ui.selectable_value(ratio, p.clone(), p.label()).clicked() {
                        ev.ratio_changed = true;
                    }
                    p = preset;
                    let _ = p;
                }
            });

        ui.separator();

        // --- Center: transport controls ---
        ui.with_layout(
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
            |ui| {
                ui.horizontal(|ui| {
                    if transport_button(ui, "⏪", "Back 30 s", Vec2::new(34.0, 28.0)) {
                        ev.seek_back_30 = true;
                    }
                    if transport_button(ui, "⏮", "Back 5 s", Vec2::new(30.0, 28.0)) {
                        ev.seek_back_5 = true;
                    }
                    let play_icon = if state.playing { "⏸" } else { "▶" };
                    if transport_button(ui, play_icon, "Play / Pause", Vec2::new(38.0, 32.0)) {
                        ev.toggle_play = true;
                    }
                    if transport_button(ui, "⏭", "Forward 5 s", Vec2::new(30.0, 28.0)) {
                        ev.seek_fwd_5 = true;
                    }
                    if transport_button(ui, "⏩", "Forward 30 s", Vec2::new(34.0, 28.0)) {
                        ev.seek_fwd_30 = true;
                    }
                    ui.separator();
                    let loop_txt = if state.loop_playback { "🔁" } else { "➡" };
                    if transport_button(ui, loop_txt, "Loop", Vec2::new(30.0, 28.0)) {
                        ev.toggle_loop = true;
                    }

                    ui.separator();
                    ui.label(
                        RichText::new(format!(
                            "{} / {}",
                            format_ms(playhead_ms),
                            format_ms(total_ms)
                        ))
                        .monospace()
                        .color(Color32::from_gray(220)),
                    );
                });
            },
        );

        // --- Right: quality dropdown ---
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            egui::ComboBox::from_id_salt("preview_quality")
                .selected_text(state.quality.label())
                .width(70.0)
                .show_ui(ui, |ui| {
                    for q in PreviewQuality::all() {
                        ui.selectable_value(&mut state.quality, q, q.label());
                    }
                });
            ui.label("Quality:");
        });
    });

    ev
}

pub fn format_ms(ms: u64) -> String {
    let total_s = ms / 1000;
    let h = total_s / 3600;
    let m = (total_s % 3600) / 60;
    let s = total_s % 60;
    let milli = ms % 1000;
    format!("{:02}:{:02}:{:02}.{:03}", h, m, s, milli)
}
