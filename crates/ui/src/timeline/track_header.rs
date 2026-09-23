//! Left-side header for each track: name + Lock / View / Mute icons.

use caprust_core::Track;
use egui::{Color32, RichText, Ui, Vec2};

pub const HEADER_WIDTH: f32 = 120.0;

/// Renders a compact icon button; highlights when `active` is false (muted /
/// hidden / locked is the "active warning" state).
fn chip(ui: &mut Ui, icon: &str, tooltip: &str, warning: bool) -> bool {
    let size = Vec2::new(22.0, 20.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());

    let bg = if warning {
        // muted / hidden / locked: tinted red
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
        egui::FontId::proportional(12.0),
        fg,
    );

    resp.on_hover_text(tooltip).clicked()
}

/// Returns true if any flag changed.
pub fn show(ui: &mut Ui, track: &mut Track, idx: usize) -> bool {
    let mut changed = false;

    ui.horizontal(|ui| {
        // Pinned lanes get a marker so users know they're always on top.
        if track.pinned {
            ui.label(
                RichText::new("📌")
                    .size(11.0)
                    .color(Color32::from_rgb(120, 220, 140)),
            );
        }
        // Track name + kind icon
        ui.label(
            RichText::new(format!("{} {}", track.kind.icon(), track.name))
                .strong()
                .size(12.0),
        );

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Order is right-to-left: mute, view, lock
            if chip(
                ui,
                if track.muted { "🔇" } else { "🔊" },
                "Mute",
                track.muted,
            ) {
                track.muted = !track.muted;
                changed = true;
            }
            if chip(
                ui,
                if track.visible { "👁" } else { "🚫" },
                "Show in preview",
                !track.visible,
            ) {
                track.visible = !track.visible;
                changed = true;
            }
            if chip(
                ui,
                if track.locked { "🔒" } else { "🔓" },
                "Lock",
                track.locked,
            ) {
                track.locked = !track.locked;
                changed = true;
            }
        });
    });

    let _ = idx;
    changed
}
