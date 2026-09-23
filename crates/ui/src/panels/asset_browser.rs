//! Left-side asset browser: tabbed panel above the timeline.
//!
//! Tabs: Media, Transitions, Effects, Filters, Text, Templates.
//! The Media tab renders the existing media-bin content; the others
//! show a grid of preset cards. Clicking a preset logs its id — wiring
//! to the selected timeline clip comes in a follow-up PR.

use crate::i18n_helper::tr;
use crate::panels::media_bin::{MediaBinOutput, MediaBinState};
use caprust_core::ProjectState;
use egui::{Color32, RichText, Sense, Ui, Vec2, Vec2 as V2};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AssetTab {
    #[default]
    Media,
    Transitions,
    Effects,
    Filters,
    Text,
    Templates,
}

impl AssetTab {
    pub fn all() -> [Self; 6] {
        [
            Self::Media,
            Self::Transitions,
            Self::Effects,
            Self::Filters,
            Self::Text,
            Self::Templates,
        ]
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Media => "asset-tab-media",
            Self::Transitions => "asset-tab-transitions",
            Self::Effects => "asset-tab-effects",
            Self::Filters => "asset-tab-filters",
            Self::Text => "asset-tab-text",
            Self::Templates => "asset-tab-templates",
        }
    }
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Media => "📁",
            Self::Transitions => "⇄",
            Self::Effects => "✨",
            Self::Filters => "🎨",
            Self::Text => "T",
            Self::Templates => "🧩",
        }
    }
}

#[derive(Debug, Default)]
pub struct AssetBrowserState {
    pub active: AssetTab,
    pub search: String,
    pub card_size: f32,
}

impl AssetBrowserState {
    pub fn new() -> Self {
        Self {
            active: AssetTab::Media,
            search: String::new(),
            card_size: 96.0,
        }
    }
}

/// One preset entry (transitions, effects, filters, text styles).
struct Preset {
    id: &'static str,
    label: &'static str,
    icon: &'static str,
    /// Base color for the placeholder thumbnail.
    color: [u8; 3],
}

// ---------------------------------------------------------------------------
// Presets (CapCut-style)
// ---------------------------------------------------------------------------

const TRANSITIONS: &[Preset] = &[
    Preset {
        id: "none",
        label: "None",
        icon: "▢",
        color: [70, 70, 75],
    },
    Preset {
        id: "fade",
        label: "Fade",
        icon: "◐",
        color: [120, 90, 160],
    },
    Preset {
        id: "slide_l",
        label: "Slide Left",
        icon: "←",
        color: [80, 130, 190],
    },
    Preset {
        id: "slide_r",
        label: "Slide Right",
        icon: "→",
        color: [80, 130, 190],
    },
    Preset {
        id: "slide_u",
        label: "Slide Up",
        icon: "↑",
        color: [80, 130, 190],
    },
    Preset {
        id: "slide_d",
        label: "Slide Down",
        icon: "↓",
        color: [80, 130, 190],
    },
    Preset {
        id: "zoom_in",
        label: "Zoom In",
        icon: "⤢",
        color: [200, 120, 80],
    },
    Preset {
        id: "zoom_out",
        label: "Zoom Out",
        icon: "⤡",
        color: [200, 120, 80],
    },
    Preset {
        id: "wipe_l",
        label: "Wipe Left",
        icon: "◧",
        color: [140, 170, 90],
    },
    Preset {
        id: "wipe_r",
        label: "Wipe Right",
        icon: "◨",
        color: [140, 170, 90],
    },
    Preset {
        id: "rotate",
        label: "Rotate",
        icon: "⟳",
        color: [180, 100, 140],
    },
    Preset {
        id: "blur_t",
        label: "Blur Cut",
        icon: "❋",
        color: [110, 110, 130],
    },
];

const EFFECTS: &[Preset] = &[
    Preset {
        id: "blur",
        label: "Blur",
        icon: "◌",
        color: [110, 110, 130],
    },
    Preset {
        id: "vignette",
        label: "Vignette",
        icon: "◎",
        color: [60, 50, 60],
    },
    Preset {
        id: "glitch",
        label: "Glitch",
        icon: "≠",
        color: [190, 80, 130],
    },
    Preset {
        id: "rgb_split",
        label: "RGB Split",
        icon: "◍",
        color: [220, 90, 90],
    },
    Preset {
        id: "shake",
        label: "Shake",
        icon: "≈",
        color: [200, 130, 80],
    },
    Preset {
        id: "zoom_pulse",
        label: "Zoom Pulse",
        icon: "◉",
        color: [220, 160, 60],
    },
    Preset {
        id: "flash",
        label: "Flash",
        icon: "✷",
        color: [240, 230, 200],
    },
    Preset {
        id: "mirror",
        label: "Mirror",
        icon: "◫",
        color: [90, 150, 190],
    },
    Preset {
        id: "kaleido",
        label: "Kaleidoscope",
        icon: "❋",
        color: [140, 90, 180],
    },
    Preset {
        id: "old_film",
        label: "Old Film",
        icon: "▦",
        color: [160, 140, 100],
    },
    Preset {
        id: "vhs",
        label: "VHS",
        icon: "▤",
        color: [130, 150, 100],
    },
    Preset {
        id: "light_leak",
        label: "Light Leak",
        icon: "☀",
        color: [230, 170, 90],
    },
    Preset {
        id: "particle",
        label: "Particle",
        icon: "✧",
        color: [180, 180, 210],
    },
    Preset {
        id: "sparkle",
        label: "Sparkle",
        icon: "✦",
        color: [230, 210, 140],
    },
    Preset {
        id: "ghost",
        label: "Ghost",
        icon: "◊",
        color: [160, 170, 200],
    },
    Preset {
        id: "lens_flare",
        label: "Lens Flare",
        icon: "◐",
        color: [240, 200, 130],
    },
];

const FILTERS: &[Preset] = &[
    Preset {
        id: "none",
        label: "None",
        icon: "○",
        color: [80, 80, 80],
    },
    Preset {
        id: "warm",
        label: "Warm",
        icon: "☀",
        color: [220, 150, 80],
    },
    Preset {
        id: "cool",
        label: "Cool",
        icon: "❄",
        color: [100, 160, 220],
    },
    Preset {
        id: "bw",
        label: "B&W",
        icon: "◑",
        color: [120, 120, 120],
    },
    Preset {
        id: "sepia",
        label: "Sepia",
        icon: "◒",
        color: [180, 140, 90],
    },
    Preset {
        id: "cinematic",
        label: "Cinematic",
        icon: "🎬",
        color: [70, 90, 140],
    },
    Preset {
        id: "vintage",
        label: "Vintage",
        icon: "🕰",
        color: [180, 150, 110],
    },
    Preset {
        id: "vivid",
        label: "Vivid",
        icon: "✸",
        color: [230, 90, 130],
    },
    Preset {
        id: "matte",
        label: "Matte",
        icon: "◍",
        color: [150, 150, 160],
    },
    Preset {
        id: "noir",
        label: "Noir",
        icon: "◼",
        color: [40, 40, 50],
    },
    Preset {
        id: "sunset",
        label: "Sunset",
        icon: "◓",
        color: [230, 110, 80],
    },
    Preset {
        id: "ocean",
        label: "Ocean",
        icon: "≋",
        color: [60, 130, 170],
    },
    Preset {
        id: "fade",
        label: "Fade",
        icon: "◌",
        color: [180, 180, 190],
    },
    Preset {
        id: "pastel",
        label: "Pastel",
        icon: "❀",
        color: [220, 180, 210],
    },
    Preset {
        id: "neon",
        label: "Neon",
        icon: "✷",
        color: [90, 240, 200],
    },
    Preset {
        id: "gold",
        label: "Golden",
        icon: "✦",
        color: [230, 190, 90],
    },
];

const TEXT_STYLES: &[Preset] = &[
    Preset {
        id: "default",
        label: "Default",
        icon: "T",
        color: [80, 90, 130],
    },
    Preset {
        id: "bold",
        label: "Bold Title",
        icon: "B",
        color: [160, 60, 60],
    },
    Preset {
        id: "subtitle",
        label: "Subtitle",
        icon: "s",
        color: [80, 120, 160],
    },
    Preset {
        id: "lower",
        label: "Lower Third",
        icon: "▬",
        color: [130, 130, 90],
    },
    Preset {
        id: "quote",
        label: "Quote",
        icon: "❝",
        color: [160, 120, 180],
    },
    Preset {
        id: "caption",
        label: "Caption",
        icon: "▭",
        color: [90, 140, 120],
    },
    Preset {
        id: "glow",
        label: "Neon",
        icon: "✷",
        color: [220, 90, 200],
    },
    Preset {
        id: "handwrite",
        label: "Handwritten",
        icon: "✎",
        color: [180, 160, 100],
    },
];

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

/// Result of one frame of the asset browser.
#[derive(Default)]
pub struct AssetBrowserOutput {
    pub media: MediaBinOutput,
    /// Id of the preset the user clicked (transitions/effects/filters/text).
    pub preset_clicked: Option<(&'static str, AssetTab)>,
}

pub fn show(
    ui: &mut Ui,
    project: &mut ProjectState,
    state: &mut AssetBrowserState,
    media_state: &mut MediaBinState,
) -> AssetBrowserOutput {
    let mut out = AssetBrowserOutput::default();

    // --- Tab strip ---
    ui.horizontal_wrapped(|ui| {
        for tab in AssetTab::all() {
            let selected = state.active == tab;
            let text = format!("{} {}", tab.icon(), tr(tab.label()));
            let btn = egui::Button::new(RichText::new(text).size(12.0))
                .fill(if selected {
                    ui.visuals().selection.bg_fill
                } else {
                    Color32::from_gray(50)
                })
                .min_size(V2::new(0.0, 26.0));
            if ui.add(btn).clicked() {
                state.active = tab;
            }
        }
    });
    ui.separator();

    // --- Tab content ---
    match state.active {
        AssetTab::Media => {
            out.media = crate::panels::media_bin::show(ui, project, media_state);
        }
        AssetTab::Transitions => {
            let clicked = preset_grid(ui, TRANSITIONS, state.card_size, &state.search);
            if let Some(id) = clicked {
                out.preset_clicked = Some((id, AssetTab::Transitions));
            }
        }
        AssetTab::Effects => {
            let clicked = preset_grid(ui, EFFECTS, state.card_size, &state.search);
            if let Some(id) = clicked {
                out.preset_clicked = Some((id, AssetTab::Effects));
            }
        }
        AssetTab::Filters => {
            let clicked = preset_grid(ui, FILTERS, state.card_size, &state.search);
            if let Some(id) = clicked {
                out.preset_clicked = Some((id, AssetTab::Filters));
            }
        }
        AssetTab::Text => {
            let clicked = preset_grid(ui, TEXT_STYLES, state.card_size, &state.search);
            if let Some(id) = clicked {
                out.preset_clicked = Some((id, AssetTab::Text));
            }
        }
        AssetTab::Templates => {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(tr("asset-templates-hint"))
                        .italics()
                        .color(Color32::from_gray(140)),
                );
            });
        }
    }

    out
}

/// Grid of preset cards. Returns the id of a card that was clicked, if any.
fn preset_grid(
    ui: &mut Ui,
    presets: &[Preset],
    card_size: f32,
    search: &str,
) -> Option<&'static str> {
    let mut clicked: Option<&'static str> = None;
    let h_gap = 6.0;
    let v_gap = 6.0;
    let scrollbar_reserve = 16.0;
    let avail = (ui.available_width() - scrollbar_reserve).max(card_size);
    let cols = (((avail + h_gap) / (card_size + h_gap)).floor() as usize).max(1);

    let needle = search.trim().to_lowercase();
    let filtered: Vec<&Preset> = presets
        .iter()
        .filter(|p| {
            needle.is_empty() || p.label.to_lowercase().contains(&needle) || p.id.contains(&needle)
        })
        .collect();

    if filtered.is_empty() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(tr("asset-empty"))
                    .italics()
                    .color(Color32::from_gray(140)),
            );
        });
        return None;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(h_gap, v_gap);
            for row in filtered.chunks(cols) {
                ui.horizontal(|ui| {
                    for p in row {
                        if preset_card(ui, p, card_size) {
                            clicked = Some(p.id);
                        }
                    }
                });
            }
        });

    clicked
}

fn preset_card(ui: &mut Ui, p: &Preset, card_size: f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(card_size, card_size), Sense::click());

    // Background
    let base = Color32::from_rgb(p.color[0], p.color[1], p.color[2]);
    let bg = if resp.hovered() {
        base.gamma_multiply(1.25)
    } else {
        base
    };
    ui.painter().rect_filled(rect, 6.0, bg);

    // Icon, big, centered
    ui.painter().text(
        rect.center() - Vec2::new(0.0, 6.0),
        egui::Align2::CENTER_CENTER,
        p.icon,
        egui::FontId::proportional(card_size * 0.35),
        Color32::from_white_alpha(230),
    );

    // Label, bottom
    ui.painter().text(
        egui::pos2(rect.center().x, rect.bottom() - 8.0),
        egui::Align2::CENTER_BOTTOM,
        p.label,
        egui::FontId::proportional(10.0),
        Color32::from_gray(220),
    );

    // Border
    let border = if resp.hovered() {
        Color32::from_white_alpha(160)
    } else {
        Color32::from_gray(60)
    };
    ui.painter().rect_stroke(
        rect,
        6.0,
        egui::Stroke::new(1.0_f32, border),
        egui::StrokeKind::Inside,
    );

    resp.clicked()
}
