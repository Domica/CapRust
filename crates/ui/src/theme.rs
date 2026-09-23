//! Theme system: light / dark / custom + accent color.

use egui::{Color32, CornerRadius, Stroke, Visuals};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    Dark,
    Light,
    Custom,
}

impl ThemeMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::Custom => "Custom",
        }
    }
    pub fn all() -> [Self; 3] {
        [Self::Dark, Self::Light, Self::Custom]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub mode: ThemeMode,
    /// Accent color as RGB.
    pub accent: [u8; 3],
    /// Custom mode: panel background, window background, main text color.
    pub custom_panel: [u8; 3],
    pub custom_window: [u8; 3],
    pub custom_text: [u8; 3],
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            mode: ThemeMode::Dark,
            accent: [59, 130, 246], // blue-500
            custom_panel: [18, 18, 20],
            custom_window: [24, 24, 27],
            custom_text: [220, 220, 224],
        }
    }
}

impl Theme {
    pub fn accent_color(&self) -> Color32 {
        Color32::from_rgb(self.accent[0], self.accent[1], self.accent[2])
    }

    pub fn apply(&self, ctx: &egui::Context) {
        let mut v = match self.mode {
            ThemeMode::Dark | ThemeMode::Custom => Visuals::dark(),
            ThemeMode::Light => Visuals::light(),
        };

        if self.mode == ThemeMode::Custom {
            v.panel_fill = Color32::from_rgb(
                self.custom_panel[0],
                self.custom_panel[1],
                self.custom_panel[2],
            );
            v.window_fill = Color32::from_rgb(
                self.custom_window[0],
                self.custom_window[1],
                self.custom_window[2],
            );
            v.extreme_bg_color = darken(v.panel_fill, 0.6);
            v.override_text_color = Some(Color32::from_rgb(
                self.custom_text[0],
                self.custom_text[1],
                self.custom_text[2],
            ));
        } else if self.mode == ThemeMode::Dark {
            v.panel_fill = Color32::from_rgb(18, 18, 20);
            v.window_fill = Color32::from_rgb(24, 24, 27);
            v.extreme_bg_color = Color32::from_rgb(10, 10, 12);
        }

        let accent = self.accent_color();
        v.selection.bg_fill = accent;
        v.selection.stroke = Stroke::NONE;
        v.hyperlink_color = accent;

        let r = CornerRadius::same(6);
        v.window_corner_radius = r;
        v.menu_corner_radius = r;
        v.widgets.noninteractive.corner_radius = r;
        v.widgets.inactive.corner_radius = r;
        v.widgets.hovered.corner_radius = r;
        v.widgets.active.corner_radius = r;

        v.widgets.hovered.bg_fill = accent.gamma_multiply(0.4);
        v.widgets.active.bg_fill = accent.gamma_multiply(0.6);

        let border = darken(v.window_fill, 0.85);
        v.window_stroke = Stroke::new(1.0_f32, border);

        ctx.set_visuals(v);
    }
}

fn darken(c: Color32, factor: f32) -> Color32 {
    Color32::from_rgb(
        (c.r() as f32 * factor) as u8,
        (c.g() as f32 * factor) as u8,
        (c.b() as f32 * factor) as u8,
    )
}

/// Preset accent colors for the picker.
pub const ACCENT_PRESETS: &[(&str, [u8; 3])] = &[
    ("Blue", [59, 130, 246]),
    ("Violet", [139, 92, 246]),
    ("Pink", [236, 72, 153]),
    ("Red", [239, 68, 68]),
    ("Orange", [249, 115, 22]),
    ("Amber", [245, 158, 11]),
    ("Green", [34, 197, 94]),
    ("Teal", [20, 184, 166]),
    ("Cyan", [6, 182, 212]),
];
