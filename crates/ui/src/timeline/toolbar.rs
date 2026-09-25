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
    pub narration_clicked: bool,
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

pub fn show(
    ui: &mut Ui,
    state: &mut TimelineToolState,
    can_undo: bool,
    can_redo: bool,
    playhead_ms: u64,
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
        if icon_action(ui, ph::CHAT_TEXT, &tr("tt-captions"), true) {
            state.captions_enabled = true;
            ev.captions_clicked = true;
        }
        // --- Narration ---
        if icon_action(ui, ph::MICROPHONE, &tr("tt-narration"), true) {
            state.narration_enabled = true;
            ev.narration_clicked = true;
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
