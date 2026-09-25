//! Modal dialog for entering narration text and picking a voice.
//!
//! Shown when the user clicks 🎙 in the timeline toolbar. On Synthesize,
//! the modal closes and the caller spawns a narration job with the
//! returned text + voice_id.

use crate::i18n_helper::tr;
use caprust_core::models::{ModelKind, ModelRegistry};
use egui::{Align2, Color32, RichText};

#[derive(Default)]
pub struct NarrationInputState {
    pub open: bool,
    pub text: String,
    /// Selected voice model id, empty = use first ready voice.
    pub voice_id: String,
}

#[derive(Default)]
pub struct NarrationInputEvents {
    /// User clicked Synthesize. Some((voice_id, text)).
    pub synthesize: Option<(String, String)>,
    /// User clicked Cancel, closed the window, or pressed Esc.
    pub closed: bool,
}

pub fn show(
    ctx: &egui::Context,
    state: &mut NarrationInputState,
    models: &ModelRegistry,
) -> NarrationInputEvents {
    let mut ev = NarrationInputEvents::default();
    if !state.open {
        return ev;
    }

    let ready_voices: Vec<(String, String)> = models
        .models
        .iter()
        .filter(|m| m.kind == ModelKind::Narration && m.status == caprust_core::ModelStatus::Ready)
        .map(|m| (m.id.clone(), m.name.clone()))
        .collect();

    // Default selection = first ready voice.
    if state.voice_id.is_empty() {
        if let Some((id, _)) = ready_voices.first() {
            state.voice_id = id.clone();
        }
    }

    let mut open = true;
    egui::Window::new(tr("ni-title"))
        .open(&mut open)
        .resizable(true)
        .collapsible(false)
        .default_width(520.0)
        .default_height(320.0)
        .anchor(Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            if ready_voices.is_empty() {
                ui.label(
                    RichText::new(tr("ni-no-voice"))
                        .color(Color32::from_gray(180))
                        .italics(),
                );
                ui.add_space(8.0);
                if ui.button(tr("ni-cancel")).clicked() {
                    ev.closed = true;
                }
                return;
            }

            // Voice picker.
            ui.horizontal(|ui| {
                ui.label(tr("ni-voice-label"));
                let current_label = ready_voices
                    .iter()
                    .find(|(id, _)| id == &state.voice_id)
                    .map(|(_, name)| name.clone())
                    .unwrap_or_else(|| state.voice_id.clone());
                egui::ComboBox::from_id_salt("narration_voice")
                    .selected_text(current_label)
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        for (id, name) in &ready_voices {
                            ui.selectable_value(&mut state.voice_id, id.clone(), name);
                        }
                    });
            });

            ui.add_space(6.0);
            ui.label(tr("ni-text-label"));
            egui::ScrollArea::vertical()
                .max_height(160.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut state.text)
                            .desired_width(f32::INFINITY)
                            .desired_rows(6)
                            .hint_text(tr("ni-text-hint")),
                    );
                });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let enabled = !state.text.trim().is_empty() && !state.voice_id.is_empty();
                let synth_btn = egui::Button::new(
                    RichText::new(tr("ni-synthesize"))
                        .color(Color32::WHITE)
                        .strong(),
                )
                .fill(Color32::from_rgb(34, 139, 230))
                .min_size(egui::vec2(120.0, 30.0));
                if ui.add_enabled(enabled, synth_btn).clicked() {
                    ev.synthesize = Some((state.voice_id.clone(), state.text.trim().to_string()));
                }

                if ui
                    .add(egui::Button::new(tr("ni-cancel")).min_size(egui::vec2(90.0, 30.0)))
                    .clicked()
                {
                    ev.closed = true;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{} chars", state.text.chars().count()))
                            .small()
                            .color(Color32::from_gray(140)),
                    );
                });
            });
        });

    if !open || ev.closed {
        state.open = false;
        ev.closed = true;
    }

    ev
}
