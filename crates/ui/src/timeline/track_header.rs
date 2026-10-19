//! Track header (left column): two rows — name+icon on top, chips below.

use crate::i18n_helper::tr;
use crate::theme::Theme;
use caprust_core::Track;
use egui::{Color32, RichText, Ui, Vec2};
use egui_phosphor::regular as ph;

pub const HEADER_WIDTH: f32 = 140.0;

fn chip(ui: &mut Ui, icon: &str, tooltip: &str, warning: bool) -> egui::Response {
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

    resp.on_hover_text(tooltip)
}

pub struct HeaderEvents {
    /// How far the height divider was dragged this frame, in screen
    /// pixels. Positive is down.
    pub height_drag_px: f32,
    pub changed: bool,
    pub delete_requested: bool,
    pub duplicate_requested: bool,
    pub rename_requested: bool,
}

/// The height divider's hit rectangle: on the row's own bottom edge.
///
/// Not on the bottom of the header's content. The content is two rows of
/// icons, about 36px, whatever the row height is -- so a divider placed
/// after the content ended up in the middle of an L-sized row, 32px
/// below its top, nowhere near the boundary it is on. It looked like a
/// line in the header and there was no handle on the edge of the track
/// to grab, which is what "not resizable at all" was.
pub fn height_divider_rect(row: egui::Rect) -> egui::Rect {
    const DIVIDER_HIT: f32 = 8.0;
    egui::Rect::from_min_size(
        egui::pos2(row.left(), row.bottom() - DIVIDER_HIT),
        egui::vec2(row.width(), DIVIDER_HIT),
    )
}

pub fn show(ui: &mut Ui, track: &mut Track, idx: usize, theme: &Theme, row_h: f32) -> HeaderEvents {
    let mut ev = HeaderEvents {
        height_drag_px: 0.0,
        changed: false,
        delete_requested: false,
        duplicate_requested: false,
        rename_requested: false,
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
        let row_resp = ui
            .horizontal(|ui| {
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
            })
            .response;

        // Context menu on the name row. Rename and Duplicate are
        // offered for editable kinds; Rename is disabled for the two
        // singleton kinds to discourage cosmetic renaming of lanes
        // that must stay identifiable ("Overlay", "Captions").
        row_resp.context_menu(|ui| {
            let editable = !matches!(
                track.kind,
                caprust_core::TrackKind::Overlay | caprust_core::TrackKind::Captions
            );
            if ui
                .add_enabled(editable, egui::Button::new(tr("tk-rename")))
                .clicked()
            {
                ev.rename_requested = true;
                ui.close_menu();
            }
            if ui
                .add_enabled(editable, egui::Button::new(tr("tk-duplicate")))
                .clicked()
            {
                ev.duplicate_requested = true;
                ui.close_menu();
            }
            ui.separator();
            if ui.button(tr("tk-delete")).clicked() {
                ev.delete_requested = true;
                ui.close_menu();
            }
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
            )
            .clicked()
            {
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
            )
            .clicked()
            {
                track.visible = !track.visible;
                ev.changed = true;
            }
            let mute_resp = chip(
                ui,
                if track.muted {
                    ph::SPEAKER_SLASH
                } else {
                    ph::SPEAKER_HIGH
                },
                &tr("tk-mute"),
                track.muted,
            );
            if mute_resp.clicked() {
                track.muted = !track.muted;
                ev.changed = true;
            }
            // Right-click the mute chip to open the track fader.
            // Range -60..=+6 dB (unity 0.0), linear in the slider so
            // the user can see where unity sits. Direct write, not
            // an undoable command, matching the other header chips.
            mute_resp.context_menu(|ui| {
                ui.set_min_width(200.0);
                ui.label(tr("tk-volume"));
                let mut v = track.volume_db;
                if ui
                    .add(
                        egui::Slider::new(&mut v, -60.0..=6.0)
                            .suffix(" dB")
                            .fixed_decimals(1),
                    )
                    .changed()
                {
                    track.volume_db = v;
                    ev.changed = true;
                }
                ui.horizontal(|ui| {
                    if ui.button(tr("tk-volume-reset")).clicked() {
                        track.volume_db = 0.0;
                        ev.changed = true;
                        ui.close_menu();
                    }
                    if ui.button(tr("tk-volume-mute")).clicked() {
                        track.volume_db = -60.0;
                        ev.changed = true;
                        ui.close_menu();
                    }
                });
            });
            if chip(ui, ph::X, &tr("tk-delete"), false).clicked() {
                ev.delete_requested = true;
            }
        });
    });

    // --- The height divider ---
    //
    // On the row's own bottom edge, not on the bottom of the content.
    // The content is two rows of icons, about 36px, whatever the row
    // height is -- so a divider placed after the content ended up in the
    // middle of an L-sized row, 32px below its top, nowhere near the
    // boundary it is supposed to be on. It looked like a line in the
    // header and dragged nothing that looked like a resize.
    //
    // Here the row's own rectangle is known, so the divider is put on
    // its edge, and a taller hit zone than the 2px line it draws.
    let div_rect = height_divider_rect(bg_rect);
    let div = ui.interact(
        div_rect,
        egui::Id::new(("track_height_div", idx)),
        egui::Sense::drag(),
    );
    if div.hovered() || div.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    // The line itself, thin, on the very bottom.
    ui.painter().line_segment(
        [
            egui::pos2(bg_rect.left(), bg_rect.bottom() - 0.5),
            egui::pos2(bg_rect.right(), bg_rect.bottom() - 0.5),
        ],
        egui::Stroke::new(
            1.0,
            if div.hovered() || div.dragged() {
                egui::Color32::from_gray(140)
            } else {
                egui::Color32::from_gray(70)
            },
        ),
    );
    if div.dragged() {
        ev.height_drag_px = div.drag_delta().y;
    }

    ev
}

#[cfg(test)]
mod tests {
    /// The divider is on the row's bottom edge, whatever the row is
    /// tall. This is the whole fix: it used to be placed after the
    /// header's content, which is two rows of icons, so in L it sat in
    /// the middle of the row instead of on the boundary, and there was
    /// nothing on the edge of the track to grab.
    #[test]
    fn the_divider_is_on_the_rows_bottom_edge() {
        for row_h in [40.0f32, 46.0, 78.0, 160.0, 272.0] {
            let row = egui::Rect::from_min_size(egui::pos2(10.0, 100.0), egui::vec2(120.0, row_h));
            let d = super::height_divider_rect(row);
            assert!(
                (d.bottom() - row.bottom()).abs() < 0.01,
                "a row {row_h} tall: the divider ends at {}, the row at {}",
                d.bottom(),
                row.bottom()
            );
            // And it is a real target, not a hairline.
            assert!(d.height() >= 6.0, "the hit zone is {} tall", d.height());
            assert!(d.width() >= 100.0, "the hit zone is {} wide", d.width());
        }
    }

    /// And it does not depend on where the content ended, which is the
    /// thing that put it in the wrong place.
    #[test]
    fn the_divider_ignores_where_the_content_ended() {
        // The content is the same whatever the row height is: two rows of
        // icons, about 36px. The divider must not follow it.
        let content_bottom = 100.0 + 36.0;
        for row_h in [46.0f32, 78.0, 160.0] {
            let row = egui::Rect::from_min_size(egui::pos2(0.0, 100.0), egui::vec2(120.0, row_h));
            let d = super::height_divider_rect(row);
            assert!(
                (d.bottom() - content_bottom).abs() > 1.0,
                "a row {row_h} tall: the divider is at the content's bottom"
            );
        }
    }
}
