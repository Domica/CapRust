//! Time ruler above the tracks. Draws tick labels + playhead marker.

use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2};

/// Height of the ruler in pixels.
pub const RULER_HEIGHT: f32 = 26.0;

/// Picks a nice tick interval (in ms) so that labels don't overlap.
fn pick_interval(px_per_ms: f32, min_px: f32) -> u64 {
    let candidates_ms: &[u64] = &[
        100, 250, 500, 1000, 2000, 5000, 10_000, 15_000, 30_000, 60_000, 120_000, 300_000, 600_000,
    ];
    for &c in candidates_ms {
        if (c as f32) * px_per_ms >= min_px {
            return c;
        }
    }
    *candidates_ms.last().unwrap()
}

/// Draw the ruler. Returns click position (in ms) if the user clicked on it.
pub fn show(
    ui: &mut Ui,
    content_width_px: f32,
    px_per_ms: f32,
    playhead_ms: u64,
    total_ms: u64,
) -> Option<u64> {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(content_width_px, RULER_HEIGHT), Sense::click());

    let painter = ui.painter_at(rect);
    let bg = Color32::from_gray(28);
    painter.rect_filled(rect, 0.0, bg);
    painter.line_segment(
        [
            Pos2::new(rect.left(), rect.bottom() - 0.5),
            Pos2::new(rect.right(), rect.bottom() - 0.5),
        ],
        Stroke::new(1.0_f32, Color32::from_gray(50)),
    );

    let interval_ms = pick_interval(px_per_ms, 70.0);
    let label_color = Color32::from_gray(170);
    let tick_color = Color32::from_gray(90);

    let max_ms = total_ms.max(10_000) + interval_ms * 2;
    let mut t = 0u64;
    while t <= max_ms {
        let x = rect.left() + (t as f32) * px_per_ms;
        if x > rect.right() {
            break;
        }
        let tick_h = if t.is_multiple_of(interval_ms * 5) {
            10.0
        } else if t.is_multiple_of(interval_ms * 2) {
            7.0
        } else {
            5.0
        };
        painter.line_segment(
            [
                Pos2::new(x, rect.bottom() - tick_h),
                Pos2::new(x, rect.bottom()),
            ],
            Stroke::new(1.0_f32, tick_color),
        );
        if tick_h >= 10.0 {
            painter.text(
                Pos2::new(x + 3.0, rect.top() + 2.0),
                egui::Align2::LEFT_TOP,
                format_time(t),
                FontId::proportional(10.0),
                label_color,
            );
        }
        t += interval_ms;
    }

    // Playhead marker
    let ph_x = rect.left() + (playhead_ms as f32) * px_per_ms;
    if ph_x >= rect.left() && ph_x <= rect.right() {
        painter.line_segment(
            [Pos2::new(ph_x, rect.top()), Pos2::new(ph_x, rect.bottom())],
            Stroke::new(2.0_f32, Color32::from_rgb(230, 70, 70)),
        );
        painter.circle_filled(
            Pos2::new(ph_x, rect.top() + 4.0),
            4.0,
            Color32::from_rgb(230, 70, 70),
        );
    }

    // Click → seek
    if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let rel_x = pos.x - rect.left();
            let ms = (rel_x / px_per_ms).max(0.0) as u64;
            return Some(ms);
        }
    }

    None
}

pub fn format_time(ms: u64) -> String {
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

/// Helper used by the ruler to keep its output visible in-app.
pub fn _unused(_r: Rect) {}
