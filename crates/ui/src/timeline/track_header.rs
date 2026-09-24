//! Track header (left column): two rows — name+icon on top, chips below.

use crate::i18n_helper::tr;
use caprust_core::Track;
use egui::{Color32, RichText, Ui, Vec2};
use egui_phosphor::regular as ph;

pub const HEADER_WIDTH: f32 = 140.0;

fn chip(ui: &mut Ui, icon: &str, tooltip: &str, warning: bool) -> bool {
    let size = Vec2::new(24.0, 18.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let bg = if warning {
        Color32::from_rgb(120, 40, 40)
    } else if resp.hovered() {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        Color32::from_gray(55)
    };
    ui.painter().rect_filled(rect, 4.0, bg);

    let fg = if warning {
        Color32::from_rgb(255, 200, 200)
    } else {
        Color32::from_gray(200)
    };

    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(11.0),
        fg,
    );

    resp.on_hover_text(tooltip).clicked()
}

pub struct HeaderEvents {
    pub changed: bool,
    pub delete_requested: bool,
}

pub fn show(ui: &mut Ui, track: &mut Track, idx: usize) -> HeaderEvents {
    let mut ev = HeaderEvents {
        changed: false,
        delete_requested: false,
    };

    ui.vertical(|ui| {
        // --- Row 1: icon + name ---
        ui.horizontal(|ui| {
            if track.pinned {
                ui.label(
                    RichText::new(ph::PUSH_PIN)
                        .size(10.0)
                        .color(Color32::from_rgb(120, 220, 140)),
                );
            }
            ui.label(
                RichText::new(format!("{} {}", track.kind.icon(), track.name))
                    .strong()
                    .size(11.0),
            );
        });

        // --- Row 2: chips ---
        ui.horizontal(|ui| {
            if chip(
                ui,
                if track.locked {
                    ph::LOCK
                } else {
                    ph::LOCK_OPEN
                },
                &tr("tk-lock"),
                track.locked,
            ) {
                track.locked = !track.locked;
                ev.changed = true;
            }
            if chip(
                ui,
                if track.visible {
                    ph::EYE
                } else {
                    ph::EYE_SLASH
                },
                &tr("tk-view"),
                !track.visible,
            ) {
                track.visible = !track.visible;
                ev.changed = true;
            }
            if chip(
                ui,
                if track.muted {
                    ph::SPEAKER_SLASH
                } else {
                    ph::SPEAKER_HIGH
                },
                &tr("tk-mute"),
                track.muted,
            ) {
                track.muted = !track.muted;
                ev.changed = true;
            }
            if chip(ui, ph::X, &tr("tk-delete"), false) {
                ev.delete_requested = true;
            }
        });
    });

    let _ = idx;
    ev
}
