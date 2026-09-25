//! Timeline toolbar with icon buttons that "press in" when active.

use crate::i18n_helper::tr;
use egui::{Color32, RichText, Ui};
use egui_phosphor::regular as ph;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TimelineToolState {
    pub pan_mode: bool,
    pub magnetic: bool,
    pub snapping: bool,
    pub captions_enabled: bool,
    pub narration_enabled: bool,
    pub follow_playhead: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TimelineToolEvents {
    pub add_track: bool,
    pub pan_toggled: bool,
    pub magnetic_toggled: bool,
    pub snapping_toggled: bool,
    pub captions_clicked: bool,
    /// Right-click dropdown on the 💬 button: caption every audio-bearing
    /// clip on the selected clip's track, sequentially.
    pub captions_all_clicked: bool,
    pub narration_clicked: bool,
    /// Click on the download-models icon. Opens the model prompt for
    /// the model family with the most missing entries.
    pub download_models_clicked: bool,
    pub follow_toggled: bool,
    pub undo: bool,
    pub redo: bool,
    pub zoom_in: bool,
    pub zoom_out: bool,
    pub zoom_fit: bool,
}

fn icon_toggle(ui: &mut Ui, icon: &str, tooltip: &str, active: bool, enabled: bool) -> bool {
    let size = egui::vec2(30.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());

    let bg = if active {
        Color32::from_gray(30)
    } else if resp.hovered() && enabled {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        Color32::from_gray(45)
    };

    ui.painter().rect_filled(rect, 5.0, bg);

    if active {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            5.0,
            egui::Stroke::new(1.0_f32, Color32::from_gray(20)),
            egui::StrokeKind::Inside,
        );
    }

    let icon_color = if !enabled {
        Color32::from_gray(80)
    } else if active {
        ui.visuals().selection.bg_fill
    } else {
        Color32::from_gray(200)
    };

    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(16.0),
        icon_color,
    );

    if enabled && resp.clicked() {
        true
    } else {
        resp.on_hover_text(tooltip);
        false
    }
}

fn icon_action(ui: &mut Ui, icon: &str, tooltip: &str, enabled: bool) -> bool {
    icon_toggle(ui, icon, tooltip, false, enabled)
}

/// Icon with an explicit tint. Used for status indicators like the
/// model-download icon that turns red when a family has no local copy.
fn icon_action_colored(
    ui: &mut Ui,
    icon: &str,
    tooltip: &str,
    enabled: bool,
    color: Color32,
) -> bool {
    let size = egui::vec2(30.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let bg = if resp.hovered() && enabled {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        Color32::from_gray(45)
    };
    ui.painter().rect_filled(rect, 5.0, bg);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(16.0),
        if enabled {
            color
        } else {
            Color32::from_gray(80)
        },
    );
    if enabled {
        let clicked = resp.clicked();
        resp.on_hover_text(tooltip);
        clicked
    } else {
        false
    }
}

/// Like `icon_action`, but returns the `Response` so callers can attach
/// a context menu. Clicking still yields `true` from `resp.clicked()`.
fn icon_action_resp(ui: &mut Ui, icon: &str, tooltip: &str, enabled: bool) -> egui::Response {
    let size = egui::vec2(30.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let bg = if resp.hovered() && enabled {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        Color32::from_gray(45)
    };
    ui.painter().rect_filled(rect, 5.0, bg);
    let icon_color = if !enabled {
        Color32::from_gray(80)
    } else {
        Color32::from_gray(200)
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(16.0),
        icon_color,
    );
    if enabled {
        resp.on_hover_text(tooltip)
    } else {
        resp
    }
}

/// Summary of local model availability, used to tint the download icon
/// and shape its tooltip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelAvailability {
    /// Every family has at least one Ready model.
    AllReady,
    /// Some families ready, some missing.
    Partial,
    /// No caption AND no narration model on disk.
    NoneReady,
    /// Count of Ready entries per family, for the tooltip.
    Status {
        captions_ready: usize,
        narration_ready: usize,
        captions_total: usize,
        narration_total: usize,
    },
}

pub fn show(
    ui: &mut Ui,
    state: &mut TimelineToolState,
    can_undo: bool,
    can_redo: bool,
    playhead_ms: u64,
    models: ModelAvailability,
) -> TimelineToolEvents {
    let mut ev = TimelineToolEvents::default();

    ui.horizontal(|ui| {
        // --- Add track ---
        if icon_action(ui, ph::PLUS, &tr("tt-add-track"), true) {
            ev.add_track = true;
        }
        ui.separator();

        // --- Pan ---
        if icon_toggle(ui, ph::HAND, &tr("tt-pan"), state.pan_mode, true) {
            state.pan_mode = !state.pan_mode;
            ev.pan_toggled = true;
        }

        // --- Magnetic ---
        if icon_toggle(ui, ph::MAGNET, &tr("tt-magnetic"), state.magnetic, true) {
            state.magnetic = !state.magnetic;
            ev.magnetic_toggled = true;
        }

        // --- Snapping ---
        if icon_toggle(ui, ph::PAPERCLIP, &tr("tt-snap"), state.snapping, true) {
            state.snapping = !state.snapping;
            ev.snapping_toggled = true;
        }

        // --- Follow playhead ---
        if icon_toggle(
            ui,
            ph::CROSSHAIR,
            &tr("tt-follow"),
            state.follow_playhead,
            true,
        ) {
            state.follow_playhead = !state.follow_playhead;
            ev.follow_toggled = true;
        }

        ui.separator();

        // --- Captions ---
        // Left click: transcribe the selected clip (existing behaviour).
        // Right click: dropdown with "caption all clips on this track".
        let cap_resp = icon_action_resp(ui, ph::CHAT_TEXT, &tr("tt-captions"), true);
        if cap_resp.clicked() {
            state.captions_enabled = true;
            ev.captions_clicked = true;
        }
        cap_resp.context_menu(|ui| {
            if ui.button(tr("tt-captions-all-in-track")).clicked() {
                ev.captions_all_clicked = true;
                ui.close_menu();
            }
        });
        // --- Narration ---
        if icon_action(ui, ph::MICROPHONE, &tr("tt-narration"), true) {
            state.narration_enabled = true;
            ev.narration_clicked = true;
        }

        // --- Download models ---
        // Colour and tooltip reflect local model availability. The icon
        // is always clickable (opens the model prompt for the family
        // that needs it most) but stands out when something is missing.
        {
            let (icon_color, tooltip) = match models {
                ModelAvailability::AllReady => (Color32::from_gray(200), tr("tt-models-ready")),
                ModelAvailability::Partial => {
                    (Color32::from_rgb(230, 200, 90), tr("tt-models-partial"))
                }
                ModelAvailability::NoneReady => {
                    (Color32::from_rgb(230, 90, 90), tr("tt-models-none"))
                }
                ModelAvailability::Status {
                    captions_ready,
                    narration_ready,
                    captions_total,
                    narration_total,
                } => {
                    let all_ready =
                        captions_ready >= captions_total && narration_ready >= narration_total;
                    let none_ready = captions_ready == 0 && narration_ready == 0;
                    let color = if all_ready {
                        Color32::from_gray(200)
                    } else if none_ready {
                        Color32::from_rgb(230, 90, 90)
                    } else {
                        Color32::from_rgb(230, 200, 90)
                    };
                    let tt = format!(
                        "{} — {} {}/{} · {} {}/{}",
                        tr("tt-models-label"),
                        tr("tt-models-captions-short"),
                        captions_ready,
                        captions_total,
                        tr("tt-models-narration-short"),
                        narration_ready,
                        narration_total,
                    );
                    (color, tt)
                }
            };
            if icon_action_colored(ui, ph::DOWNLOAD_SIMPLE, &tooltip, true, icon_color) {
                ev.download_models_clicked = true;
            }
        }

        ui.separator();

        // --- Zoom ---
        if icon_action(ui, ph::MAGNIFYING_GLASS_MINUS, &tr("tt-zoom-out"), true) {
            ev.zoom_out = true;
        }
        if icon_action(ui, ph::MAGNIFYING_GLASS_PLUS, &tr("tt-zoom-in"), true) {
            ev.zoom_in = true;
        }
        if icon_action(ui, ph::ARROWS_OUT_CARDINAL, &tr("tt-zoom-fit"), true) {
            ev.zoom_fit = true;
        }

        ui.separator();

        // --- Playhead time ---
        ui.label(
            RichText::new(format_playhead(playhead_ms))
                .monospace()
                .color(Color32::from_gray(220)),
        );

        // --- Right side: undo / redo ---
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_action(ui, ph::ARROW_U_UP_RIGHT, &tr("tt-redo"), can_redo) {
                ev.redo = true;
            }
            if icon_action(ui, ph::ARROW_U_UP_LEFT, &tr("tt-undo"), can_undo) {
                ev.undo = true;
            }
        });
    });

    ev
}

fn format_playhead(ms: u64) -> String {
    let total_s = ms / 1000;
    let h = total_s / 3600;
    let m = (total_s % 3600) / 60;
    let s = total_s % 60;
    let ms = ms % 1000;
    format!("{h:02}:{m:02}:{s:02}.{ms:03}")
}
