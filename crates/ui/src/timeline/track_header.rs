//! Track header (left column): two rows — name+icon on top, chips below.

use crate::i18n_helper::tr;
use crate::theme::Theme;
use caprust_core::Track;
use egui::{Color32, RichText, Ui, Vec2};
use egui_phosphor::regular as ph;

pub const HEADER_WIDTH: f32 = 140.0;

fn chip(ui: &mut Ui, icon: &str, tooltip: &str, warning: bool) -> bool {
    let size = Vec2::new(24.0, 18.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    // Chips are the "dark markings" on the pastel header. Always dark
    // enough for the light chip icon to read, regardless of theme mode.
    let bg = if warning {
        Color32::from_rgb(150, 40, 40)
    } else if resp.hovered() {
        Color32::from_rgb(38, 38, 44)
    } else {
        Color32::from_rgb(28, 28, 34)
    };
    ui.painter().rect_filled(rect, 4.0, bg);

    let fg = if warning {
        Color32::from_rgb(255, 220, 220)
    } else {
        Color32::from_gray(220)
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

pub fn show(ui: &mut Ui, track: &mut Track, idx: usize, theme: &Theme, row_h: f32) -> HeaderEvents {
    let mut ev = HeaderEvents {
        changed: false,
        delete_requested: false,
    };

    // Paint the header row background in the track's own color before
    // drawing any content. The row occupies the full allocated width
    // and the exact row height so it lines up with the lane on the
    // right (which uses allocate_exact_size).
    let top_left = ui.cursor().min;
    let bg_rect = egui::Rect::from_min_size(top_left, egui::vec2(HEADER_WIDTH, row_h));
    let header_bg = theme.track_header_bg(track.kind);
    ui.painter().rect_filled(bg_rect, 0.0, header_bg);

    // Dark ink for every pastel header.
    let ink = theme.track_header_fg(track.kind);
    ui.visuals_mut().override_text_color = Some(ink);

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
