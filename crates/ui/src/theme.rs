//! Theme system: light / dark / custom + accent color.

use caprust_core::TrackKind;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PlayheadSize {
    /// Line only inside the ruler strip (default).
    #[default]
    Compact,
    /// Line runs from the ruler top down through every lane, ending at
    /// the bottom of the last visible track.
    Full,
}

impl PlayheadSize {
    pub fn label_key(&self) -> &'static str {
        match self {
            Self::Compact => "set-appearance-playhead-size-compact",
            Self::Full => "set-appearance-playhead-size-full",
        }
    }
    pub fn all() -> [Self; 2] {
        [Self::Compact, Self::Full]
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
    /// Track header/lane colors, one per track family. Headers use the
    /// raw RGB; lanes use a dimmed variant so clips stay readable. All
    /// four are user-editable in Settings → Theme → Track colors.
    #[serde(default = "default_track_video")]
    pub track_video: [u8; 3],
    #[serde(default = "default_track_audio")]
    pub track_audio: [u8; 3],
    #[serde(default = "default_track_captions")]
    pub track_captions: [u8; 3],
    #[serde(default = "default_track_text")]
    pub track_text: [u8; 3],
    /// Playhead line color. Default is a warm red.
    #[serde(default = "default_playhead_color")]
    pub playhead: [u8; 3],
    /// Playhead line extent.
    #[serde(default)]
    pub playhead_size: PlayheadSize,
}

fn default_playhead_color() -> [u8; 3] {
    [230, 70, 70]
}

fn default_track_video() -> [u8; 3] {
    [128, 170, 232]
}
fn default_track_audio() -> [u8; 3] {
    [128, 200, 148]
}
fn default_track_captions() -> [u8; 3] {
    [232, 208, 120]
}
fn default_track_text() -> [u8; 3] {
    [184, 148, 224]
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            mode: ThemeMode::Dark,
            accent: [59, 130, 246], // blue-500
            custom_panel: [18, 18, 20],
            custom_window: [24, 24, 27],
            custom_text: [220, 220, 224],
            track_video: default_track_video(),
            track_audio: default_track_audio(),
            track_captions: default_track_captions(),
            track_text: default_track_text(),
            playhead: default_playhead_color(),
            playhead_size: PlayheadSize::default(),
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

    /// Raw RGB chosen for a track kind.
    pub fn track_color(&self, kind: TrackKind) -> [u8; 3] {
        match kind {
            TrackKind::Video | TrackKind::Overlay => self.track_video,
            TrackKind::Audio => self.track_audio,
            TrackKind::Captions => self.track_captions,
            TrackKind::Text => self.track_text,
        }
    }

    /// Header background. Uses the raw track color in every mode — the
    /// palette is already pastel enough to stay legible on both dark and
    /// light chrome.
    pub fn track_header_bg(&self, kind: TrackKind) -> Color32 {
        let [r, g, b] = self.track_color(kind);
        Color32::from_rgb(r, g, b)
    }

    /// Ink color for the header's text and icons. Dark on every mode
    /// because the pastel backgrounds are always light.
    pub fn track_header_fg(&self, _kind: TrackKind) -> Color32 {
        Color32::from_rgb(24, 24, 30)
    }

    /// Playhead line color as Color32.
    pub fn playhead_color(&self) -> Color32 {
        Color32::from_rgb(self.playhead[0], self.playhead[1], self.playhead[2])
    }

    /// Border color drawn around every clip on the timeline. Uses the
    /// pastel track color so the border is always a different hue from
    /// the clip's own saturated fill, which is what makes two adjacent
    /// clips distinguishable at a glance.
    ///
    /// Dark theme: pastel kept bright — reads as a light outline on the
    /// dark clip fill.
    /// Light theme: pastel darkened — reads as a deeper outline so it
    /// still contrasts on a lighter lane.
    pub fn clip_border_color(&self, kind: TrackKind) -> Color32 {
        let [r, g, b] = self.track_color(kind);
        let base = Color32::from_rgb(r, g, b);
        match self.mode {
            ThemeMode::Light => base.gamma_multiply(0.55),
            ThemeMode::Dark | ThemeMode::Custom => base.gamma_multiply(1.15),
        }
    }

    /// Lane background. Pastel tint that respects theme mode: brighter
    /// wash on light chrome, darker wash on dark. Hidden tracks are
    /// dimmed further so the difference between visible/hidden is
    /// obvious without a separate pattern.
    pub fn track_lane_bg(&self, kind: TrackKind, visible: bool) -> Color32 {
        let [r, g, b] = self.track_color(kind);
        let base = match self.mode {
            ThemeMode::Light => 0.78,
            ThemeMode::Dark => 0.16,
            ThemeMode::Custom => 0.16,
        };
        let f = if visible { base } else { base * 0.55 };
        Color32::from_rgb(
            (r as f32 * f) as u8,
            (g as f32 * f) as u8,
            (b as f32 * f) as u8,
        )
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
