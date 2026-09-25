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
use egui_phosphor::regular as ph;

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
///
/// `label_key` is an FTL key resolved via `tr()` — never store a
/// translated string here (§8: every user-visible string goes through
/// the i18n system).
///
/// `coming_soon` marks a preset that is visible but not yet wired in
/// the filtergraph. The card renders dimmed and clicks are ignored
/// until the backing implementation lands.
struct Preset {
    id: &'static str,
    label_key: &'static str,
    icon: &'static str,
    /// Base color for the placeholder thumbnail.
    color: [u8; 3],
    coming_soon: bool,
}

// ---------------------------------------------------------------------------
// Presets (CapCut-style)
// ---------------------------------------------------------------------------

const TRANSITIONS: &[Preset] = &[
    Preset {
        id: "none",
        label_key: "asset-transition-none",
        icon: ph::PROHIBIT,
        color: [70, 70, 75],
        coming_soon: false,
    },
    Preset {
        id: "fade",
        label_key: "asset-transition-fade",
        icon: ph::CIRCLE_HALF,
        color: [120, 90, 160],
        coming_soon: false,
    },
    Preset {
        id: "slide_l",
        label_key: "asset-transition-slide_l",
        icon: ph::ARROW_LEFT,
        color: [80, 130, 190],
        coming_soon: false,
    },
    Preset {
        id: "slide_r",
        label_key: "asset-transition-slide_r",
        icon: ph::ARROW_RIGHT,
        color: [80, 130, 190],
        coming_soon: false,
    },
    Preset {
        id: "slide_u",
        label_key: "asset-transition-slide_u",
        icon: ph::ARROW_UP,
        color: [80, 130, 190],
        coming_soon: false,
    },
    Preset {
        id: "slide_d",
        label_key: "asset-transition-slide_d",
        icon: ph::ARROW_DOWN,
        color: [80, 130, 190],
        coming_soon: false,
    },
    Preset {
        id: "zoom_in",
        label_key: "asset-transition-zoom_in",
        icon: ph::ARROWS_OUT,
        color: [200, 120, 80],
        coming_soon: false,
    },
    Preset {
        id: "zoom_out",
        label_key: "asset-transition-zoom_out",
        icon: ph::ARROWS_IN,
        color: [200, 120, 80],
        coming_soon: false,
    },
    Preset {
        id: "wipe_l",
        label_key: "asset-transition-wipe_l",
        icon: ph::ARROW_LINE_LEFT,
        color: [140, 170, 90],
        coming_soon: false,
    },
    Preset {
        id: "wipe_r",
        label_key: "asset-transition-wipe_r",
        icon: ph::ARROW_LINE_RIGHT,
        color: [140, 170, 90],
        coming_soon: false,
    },
    Preset {
        id: "rotate",
        label_key: "asset-transition-rotate",
        icon: ph::ARROWS_CLOCKWISE,
        color: [180, 100, 140],
        coming_soon: false,
    },
    Preset {
        id: "blur_t",
        label_key: "asset-transition-blur_t",
        icon: ph::DROP,
        color: [110, 110, 130],
        coming_soon: false,
    },
];

const EFFECTS: &[Preset] = &[
    Preset {
        id: "blur",
        label_key: "asset-effect-blur",
        icon: ph::DROP,
        color: [110, 110, 130],
        coming_soon: false,
    },
    Preset {
        id: "vignette",
        label_key: "asset-effect-vignette",
        icon: ph::CIRCLE,
        color: [60, 50, 60],
        coming_soon: false,
    },
    Preset {
        id: "glitch",
        label_key: "asset-effect-glitch",
        icon: ph::LIGHTNING,
        color: [190, 80, 130],
        coming_soon: false,
    },
    Preset {
        id: "rgb_split",
        label_key: "asset-effect-rgb_split",
        icon: ph::GIT_BRANCH,
        color: [220, 90, 90],
        coming_soon: false,
    },
    Preset {
        id: "shake",
        label_key: "asset-effect-shake",
        icon: ph::WAVE_SINE,
        color: [200, 130, 80],
        coming_soon: false,
    },
    Preset {
        id: "zoom_pulse",
        label_key: "asset-effect-zoom_pulse",
        icon: ph::CIRCLES_THREE,
        color: [220, 160, 60],
        coming_soon: false,
    },
    Preset {
        id: "flash",
        label_key: "asset-effect-flash",
        icon: ph::LIGHTNING_SLASH,
        color: [240, 230, 200],
        coming_soon: false,
    },
    Preset {
        id: "mirror",
        label_key: "asset-effect-mirror",
        icon: ph::FLIP_HORIZONTAL,
        color: [90, 150, 190],
        coming_soon: false,
    },
    Preset {
        id: "kaleido",
        label_key: "asset-effect-kaleido",
        icon: ph::DIAMOND,
        color: [140, 90, 180],
        coming_soon: false,
    },
    Preset {
        id: "old_film",
        label_key: "asset-effect-old_film",
        icon: ph::FILM_STRIP,
        color: [160, 140, 100],
        coming_soon: false,
    },
    Preset {
        id: "vhs",
        label_key: "asset-effect-vhs",
        icon: ph::CASSETTE_TAPE,
        color: [130, 150, 100],
        coming_soon: false,
    },
    Preset {
        id: "light_leak",
        label_key: "asset-effect-light_leak",
        icon: ph::SUN,
        color: [230, 170, 90],
        coming_soon: false,
    },
    Preset {
        id: "particle",
        label_key: "asset-effect-particle",
        icon: ph::SPARKLE,
        color: [180, 180, 210],
        coming_soon: true,
    },
    Preset {
        id: "sparkle",
        label_key: "asset-effect-sparkle",
        icon: ph::STAR,
        color: [230, 210, 140],
        coming_soon: true,
    },
    Preset {
        id: "ghost",
        label_key: "asset-effect-ghost",
        icon: ph::GHOST,
        color: [160, 170, 200],
        coming_soon: true,
    },
    Preset {
        id: "lens_flare",
        label_key: "asset-effect-lens_flare",
        icon: ph::SUN_HORIZON,
        color: [240, 200, 130],
        coming_soon: true,
    },
];

const FILTERS: &[Preset] = &[
    Preset {
        id: "none",
        label_key: "asset-filter-none",
        icon: ph::CIRCLE,
        color: [80, 80, 80],
        coming_soon: false,
    },
    Preset {
        id: "warm",
        label_key: "asset-filter-warm",
        icon: ph::SUN,
        color: [220, 150, 80],
        coming_soon: false,
    },
    Preset {
        id: "cool",
        label_key: "asset-filter-cool",
        icon: ph::SNOWFLAKE,
        color: [100, 160, 220],
        coming_soon: false,
    },
    Preset {
        id: "bw",
        label_key: "asset-filter-bw",
        icon: ph::CIRCLE_HALF,
        color: [120, 120, 120],
        coming_soon: false,
    },
    Preset {
        id: "sepia",
        label_key: "asset-filter-sepia",
        icon: ph::PALETTE,
        color: [180, 140, 90],
        coming_soon: false,
    },
    Preset {
        id: "cinematic",
        label_key: "asset-filter-cinematic",
        icon: ph::FILM_SLATE,
        color: [70, 90, 140],
        coming_soon: false,
    },
    Preset {
        id: "vintage",
        label_key: "asset-filter-vintage",
        icon: ph::CLOCK_COUNTER_CLOCKWISE,
        color: [180, 150, 110],
        coming_soon: true,
    },
    Preset {
        id: "vivid",
        label_key: "asset-filter-vivid",
        icon: ph::PALETTE,
        color: [230, 90, 130],
        coming_soon: false,
    },
    Preset {
        id: "matte",
        label_key: "asset-filter-matte",
        icon: ph::SQUARE,
        color: [150, 150, 160],
        coming_soon: false,
    },
    Preset {
        id: "noir",
        label_key: "asset-filter-noir",
        icon: ph::MOON,
        color: [40, 40, 50],
        coming_soon: false,
    },
    Preset {
        id: "sunset",
        label_key: "asset-filter-sunset",
        icon: ph::SUN_HORIZON,
        color: [230, 110, 80],
        coming_soon: false,
    },
    Preset {
        id: "ocean",
        label_key: "asset-filter-ocean",
        icon: ph::WAVES,
        color: [60, 130, 170],
        coming_soon: false,
    },
    Preset {
        id: "fade",
        label_key: "asset-filter-fade",
        icon: ph::CIRCLE_HALF_TILT,
        color: [180, 180, 190],
        coming_soon: false,
    },
    Preset {
        id: "pastel",
        label_key: "asset-filter-pastel",
        icon: ph::FLOWER,
        color: [220, 180, 210],
        coming_soon: false,
    },
    Preset {
        id: "neon",
        label_key: "asset-filter-neon",
        icon: ph::LIGHTNING,
        color: [90, 240, 200],
        coming_soon: false,
    },
    Preset {
        id: "gold",
        label_key: "asset-filter-gold",
        icon: ph::CROWN,
        color: [230, 190, 90],
        coming_soon: false,
    },
];

const TEXT_STYLES: &[Preset] = &[
    Preset {
        id: "default",
        label_key: "asset-text-default",
        icon: ph::TEXT_T,
        color: [80, 90, 130],
        coming_soon: true,
    },
    Preset {
        id: "bold",
        label_key: "asset-text-bold",
        icon: ph::TEXT_B,
        color: [160, 60, 60],
        coming_soon: true,
    },
    Preset {
        id: "subtitle",
        label_key: "asset-text-subtitle",
        icon: ph::TEXT_ALIGN_LEFT,
        color: [80, 120, 160],
        coming_soon: true,
    },
    Preset {
        id: "lower",
        label_key: "asset-text-lower",
        icon: ph::TEXTBOX,
        color: [130, 130, 90],
        coming_soon: true,
    },
    Preset {
        id: "quote",
        label_key: "asset-text-quote",
        icon: ph::QUOTES,
        color: [160, 120, 180],
        coming_soon: true,
    },
    Preset {
        id: "caption",
        label_key: "asset-text-caption",
        icon: ph::TEXT_ALIGN_CENTER,
        color: [90, 140, 120],
        coming_soon: true,
    },
    Preset {
        id: "glow",
        label_key: "asset-text-glow",
        icon: ph::LIGHTNING,
        color: [220, 90, 200],
        coming_soon: true,
    },
    Preset {
        id: "handwrite",
        label_key: "asset-text-handwrite",
        icon: ph::PENCIL,
        color: [180, 160, 100],
        coming_soon: true,
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
            if needle.is_empty() {
                return true;
            }
            let label = tr(p.label_key).to_lowercase();
            label.contains(&needle) || p.id.contains(&needle)
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

    // Background. Coming-soon presets render at 55% brightness and do
    // not respond to hover, so the user reads them as unavailable.
    let base = Color32::from_rgb(p.color[0], p.color[1], p.color[2]);
    let bg = if p.coming_soon {
        base.gamma_multiply(0.55)
    } else if resp.hovered() {
        base.gamma_multiply(1.25)
    } else {
        base
    };
    ui.painter().rect_filled(rect, 6.0, bg);

    // Icon, big, centered. Dimmed icon for coming-soon.
    let icon_alpha = if p.coming_soon { 110 } else { 230 };
    ui.painter().text(
        rect.center() - Vec2::new(0.0, 6.0),
        egui::Align2::CENTER_CENTER,
        p.icon,
        egui::FontId::proportional(card_size * 0.35),
        Color32::from_white_alpha(icon_alpha),
    );

    // Label, bottom, localized.
    let label = tr(p.label_key);
    let label_color = if p.coming_soon {
        Color32::from_gray(130)
    } else {
        Color32::from_gray(220)
    };
    ui.painter().text(
        egui::pos2(rect.center().x, rect.bottom() - 8.0),
        egui::Align2::CENTER_BOTTOM,
        &label,
        egui::FontId::proportional(10.0),
        label_color,
    );

    if p.coming_soon {
        resp.on_hover_text(tr("asset-coming-soon"));
        return false;
    }

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
