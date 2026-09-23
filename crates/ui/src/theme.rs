//! CapCut-like dark theme for egui 0.31.

use egui::{Color32, CornerRadius, Stroke, Visuals};

pub fn apply_capcut_theme(ctx: &egui::Context) {
    let mut visuals = Visuals::dark();

    visuals.panel_fill = Color32::from_rgb(18, 18, 20);
    visuals.window_fill = Color32::from_rgb(24, 24, 27);
    visuals.extreme_bg_color = Color32::from_rgb(10, 10, 12);
    visuals.selection.bg_fill = Color32::from_rgb(59, 130, 246);
    visuals.selection.stroke = Stroke::NONE;

    let r = CornerRadius::same(6);
    visuals.window_corner_radius = r;
    visuals.menu_corner_radius = r;
    visuals.widgets.noninteractive.corner_radius = r;
    visuals.widgets.inactive.corner_radius = r;
    visuals.widgets.hovered.corner_radius = r;
    visuals.widgets.active.corner_radius = r;

    visuals.window_stroke = Stroke::new(1.0_f32, Color32::from_rgb(40, 40, 45));

    ctx.set_visuals(visuals);
}
