//! Left-side asset browser: tabbed panel above the timeline.
//!
//! Tabs: Media, Transitions, Effects, Filters, Text, Templates.
//! The Media tab renders the existing media-bin content; the others
//! show a grid of preset cards. Clicking a preset logs its id — wiring
//! to the selected timeline clip comes in a follow-up PR.

use crate::i18n_helper::tr;
use crate::panels::media_bin::{MediaBinOutput, MediaBinState, TabLayout};
use crate::theme::tokens::elev;
use crate::widgets::empty;
use caprust_core::ProjectState;
use egui::{Color32, CursorIcon, RichText, Sense, Ui, Vec2, Vec2 as V2};
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
    Plugins,
}

impl AssetTab {
    pub fn all() -> [Self; 7] {
        [
            Self::Media,
            Self::Transitions,
            Self::Effects,
            Self::Filters,
            Self::Text,
            Self::Templates,
            Self::Plugins,
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
            Self::Plugins => "asset-tab-plugins",
        }
    }
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Media => "📁",
            Self::Transitions => ph::ARROWS_LEFT_RIGHT,
            Self::Effects => ph::SPARKLE,
            Self::Filters => "🎨",
            Self::Text => "T",
            Self::Plugins => ph::PLUG,
            Self::Templates => "🧩",
        }
    }
    pub fn id(&self) -> &'static str {
        match self {
            Self::Media => "media",
            Self::Transitions => "transitions",
            Self::Effects => "effects",
            Self::Filters => "filters",
            Self::Text => "text",
            Self::Templates => "templates",
            Self::Plugins => "plugins",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "media" => Self::Media,
            "transitions" => Self::Transitions,
            "effects" => Self::Effects,
            "filters" => Self::Filters,
            "text" => Self::Text,
            "templates" => Self::Templates,
            "plugins" => Self::Plugins,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PresetCardSize {
    #[default]
    Small,
    Medium,
    Large,
}

impl PresetCardSize {
    pub fn px(self) -> f32 {
        match self {
            Self::Small => 72.0,
            Self::Medium => 96.0,
            Self::Large => 128.0,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Small => "S",
            Self::Medium => "M",
            Self::Large => "L",
        }
    }
    pub fn all() -> [Self; 3] {
        [Self::Small, Self::Medium, Self::Large]
    }
}

#[derive(Debug, Default)]
pub struct AssetBrowserState {
    pub active: AssetTab,
    pub search: String,
    /// Card size S/M/L (affects all preset tabs).
    pub card_size: PresetCardSize,
    /// Selected Transitions left-column category ("all" = no filter).
    pub transition_cat: String,
    /// Selected Effects left-column category ("all" = no filter).
    pub effect_cat: String,
    /// Selected Filters left-column category ("all" = no filter).
    pub filter_cat: String,
    /// Left sidebar width in pixels (persisted per session).
    pub sidebar_width: f32,
    /// Selected tab layout mode (NoFilter/Filtered).
    pub tab_layout: TabLayout,
    pub plugin_browser: crate::panels::plugin_browser::PluginBrowserState,
}

impl AssetBrowserState {
    pub fn new() -> Self {
        Self {
            active: AssetTab::Media,
            search: String::new(),
            card_size: PresetCardSize::Medium,
            transition_cat: "all".to_string(),
            effect_cat: "all".to_string(),
            filter_cat: "all".to_string(),
            sidebar_width: 104.0,
            tab_layout: TabLayout::default(),
            plugin_browser: Default::default(),
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
        id: "fadewhite",
        label_key: "asset-transition-fadewhite",
        icon: ph::SUN,
        color: [220, 220, 230],
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
        id: "wipe_u",
        label_key: "asset-transition-wipe_u",
        icon: ph::ARROW_LINE_UP,
        color: [140, 170, 90],
        coming_soon: false,
    },
    Preset {
        id: "wipe_d",
        label_key: "asset-transition-wipe_d",
        icon: ph::ARROW_LINE_DOWN,
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
    Preset {
        id: "dissolve",
        label_key: "asset-transition-dissolve",
        icon: ph::CIRCLE,
        color: [150, 130, 200],
        coming_soon: false,
    },
    Preset {
        id: "smooth_l",
        label_key: "asset-transition-smooth_l",
        icon: ph::ARROW_LEFT,
        color: [90, 150, 200],
        coming_soon: false,
    },
    Preset {
        id: "smooth_r",
        label_key: "asset-transition-smooth_r",
        icon: ph::ARROW_RIGHT,
        color: [90, 150, 200],
        coming_soon: false,
    },
    Preset {
        id: "smooth_u",
        label_key: "asset-transition-smooth_u",
        icon: ph::ARROW_UP,
        color: [90, 150, 200],
        coming_soon: false,
    },
    Preset {
        id: "smooth_d",
        label_key: "asset-transition-smooth_d",
        icon: ph::ARROW_DOWN,
        color: [90, 150, 200],
        coming_soon: false,
    },
    Preset {
        id: "circle_close",
        label_key: "asset-transition-circle_close",
        icon: ph::CIRCLES_THREE,
        color: [200, 120, 80],
        coming_soon: false,
    },
    Preset {
        id: "pixelize",
        label_key: "asset-transition-pixelize",
        icon: ph::SQUARE,
        color: [120, 140, 90],
        coming_soon: false,
    },
    Preset {
        id: "cover_l",
        label_key: "asset-transition-cover_l",
        icon: ph::ARROW_RIGHT,
        color: [100, 130, 180],
        coming_soon: false,
    },
    Preset {
        id: "cover_r",
        label_key: "asset-transition-cover_r",
        icon: ph::ARROW_LEFT,
        color: [100, 130, 180],
        coming_soon: false,
    },
    Preset {
        id: "cover_u",
        label_key: "asset-transition-cover_u",
        icon: ph::ARROW_DOWN,
        color: [100, 130, 180],
        coming_soon: false,
    },
    Preset {
        id: "cover_d",
        label_key: "asset-transition-cover_d",
        icon: ph::ARROW_UP,
        color: [100, 130, 180],
        coming_soon: false,
    },
    Preset {
        id: "reveal_l",
        label_key: "asset-transition-reveal_l",
        icon: ph::ARROW_RIGHT,
        color: [180, 130, 100],
        coming_soon: false,
    },
    Preset {
        id: "reveal_r",
        label_key: "asset-transition-reveal_r",
        icon: ph::ARROW_LEFT,
        color: [180, 130, 100],
        coming_soon: false,
    },
    Preset {
        id: "reveal_u",
        label_key: "asset-transition-reveal_u",
        icon: ph::ARROW_DOWN,
        color: [180, 130, 100],
        coming_soon: false,
    },
    Preset {
        id: "reveal_d",
        label_key: "asset-transition-reveal_d",
        icon: ph::ARROW_UP,
        color: [180, 130, 100],
        coming_soon: false,
    },
    Preset {
        id: "diag_tl",
        label_key: "asset-transition-diag_tl",
        icon: ph::CARET_UP,
        color: [150, 120, 160],
        coming_soon: false,
    },
    Preset {
        id: "diag_tr",
        label_key: "asset-transition-diag_tr",
        icon: ph::CARET_UP,
        color: [150, 120, 160],
        coming_soon: false,
    },
    Preset {
        id: "diag_bl",
        label_key: "asset-transition-diag_bl",
        icon: ph::CARET_DOWN,
        color: [150, 120, 160],
        coming_soon: false,
    },
    Preset {
        id: "diag_br",
        label_key: "asset-transition-diag_br",
        icon: ph::CARET_DOWN,
        color: [150, 120, 160],
        coming_soon: false,
    },
    Preset {
        id: "wipe_tl",
        label_key: "asset-transition-wipe_tl",
        icon: ph::CARET_LEFT,
        color: [120, 160, 120],
        coming_soon: false,
    },
    Preset {
        id: "wipe_tr",
        label_key: "asset-transition-wipe_tr",
        icon: ph::CARET_RIGHT,
        color: [120, 160, 120],
        coming_soon: false,
    },
    Preset {
        id: "wipe_bl",
        label_key: "asset-transition-wipe_bl",
        icon: ph::CARET_LEFT,
        color: [120, 160, 120],
        coming_soon: false,
    },
    Preset {
        id: "wipe_br",
        label_key: "asset-transition-wipe_br",
        icon: ph::CARET_RIGHT,
        color: [120, 160, 120],
        coming_soon: false,
    },
    Preset {
        id: "vert_open",
        label_key: "asset-transition-vert_open",
        icon: ph::CARET_UP,
        color: [140, 140, 160],
        coming_soon: false,
    },
    Preset {
        id: "vert_close",
        label_key: "asset-transition-vert_close",
        icon: ph::CARET_DOWN,
        color: [140, 140, 160],
        coming_soon: false,
    },
    Preset {
        id: "horz_open",
        label_key: "asset-transition-horz_open",
        icon: ph::CARET_RIGHT,
        color: [140, 140, 160],
        coming_soon: false,
    },
    Preset {
        id: "horz_close",
        label_key: "asset-transition-horz_close",
        icon: ph::CARET_LEFT,
        color: [140, 140, 160],
        coming_soon: false,
    },
    Preset {
        id: "fadegrays",
        label_key: "asset-transition-fadegrays",
        icon: ph::MOON,
        color: [100, 100, 120],
        coming_soon: false,
    },
    Preset {
        id: "distance",
        label_key: "asset-transition-distance",
        icon: ph::RULER,
        color: [120, 110, 140],
        coming_soon: false,
    },
    Preset {
        id: "hblur",
        label_key: "asset-transition-hblur",
        icon: ph::DROP,
        color: [110, 120, 150],
        coming_soon: false,
    },
    Preset {
        id: "circle_crop",
        label_key: "asset-transition-circle_crop",
        icon: ph::CIRCLE,
        color: [130, 150, 120],
        coming_soon: false,
    },
    Preset {
        id: "rect_crop",
        label_key: "asset-transition-rect_crop",
        icon: ph::SQUARE,
        color: [130, 150, 120],
        coming_soon: false,
    },
    Preset {
        id: "zoom_blur",
        label_key: "asset-transition-zoom_blur",
        icon: ph::MAGNIFYING_GLASS,
        color: [160, 130, 100],
        coming_soon: false,
    },
    Preset {
        id: "squeeze_h",
        label_key: "asset-transition-squeeze_h",
        icon: ph::ARROWS_IN,
        color: [140, 120, 140],
        coming_soon: false,
    },
    Preset {
        id: "squeeze_v",
        label_key: "asset-transition-squeeze_v",
        icon: ph::ARROWS_OUT,
        color: [140, 120, 140],
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
        coming_soon: false,
    },
    Preset {
        id: "sparkle",
        label_key: "asset-effect-sparkle",
        icon: ph::STAR,
        color: [230, 210, 140],
        coming_soon: false,
    },
    Preset {
        id: "ghost",
        label_key: "asset-effect-ghost",
        icon: ph::GHOST,
        color: [160, 170, 200],
        coming_soon: false,
    },
    Preset {
        id: "lens_flare",
        label_key: "asset-effect-lens_flare",
        icon: ph::SUN_HORIZON,
        color: [240, 200, 130],
        coming_soon: false,
    },
    Preset {
        id: "negative",
        label_key: "asset-effect-negative",
        icon: ph::CIRCLE_HALF_TILT,
        color: [40, 40, 50],
        coming_soon: false,
    },
    Preset {
        id: "pixelate",
        label_key: "asset-effect-pixelate",
        icon: ph::SQUARE,
        color: [120, 140, 90],
        coming_soon: false,
    },
    Preset {
        id: "sketch",
        label_key: "asset-effect-sketch",
        icon: ph::PENCIL,
        color: [170, 150, 130],
        coming_soon: false,
    },
    Preset {
        id: "sharpen",
        label_key: "asset-effect-sharpen",
        icon: ph::SNOWFLAKE,
        color: [150, 190, 220],
        coming_soon: false,
    },
    Preset {
        id: "grain",
        label_key: "asset-effect-grain",
        icon: ph::DOTS_THREE,
        color: [150, 150, 150],
        coming_soon: false,
    },
    Preset {
        id: "glow",
        label_key: "asset-effect-glow",
        icon: ph::LIGHTBULB,
        color: [240, 220, 160],
        coming_soon: false,
    },
    Preset {
        id: "cinematic_bars",
        label_key: "asset-effect-cinematic_bars",
        icon: ph::FILM_STRIP,
        color: [60, 60, 70],
        coming_soon: false,
    },
    Preset {
        id: "teal_orange",
        label_key: "asset-effect-teal_orange",
        icon: ph::SUN,
        color: [80, 160, 160],
        coming_soon: false,
    },
    Preset {
        id: "cross_process",
        label_key: "asset-effect-cross_process",
        icon: ph::PALETTE,
        color: [160, 120, 140],
        coming_soon: false,
    },
    Preset {
        id: "bleach_bypass",
        label_key: "asset-effect-bleach_bypass",
        icon: ph::ARROWS_IN,
        color: [140, 140, 150],
        coming_soon: false,
    },
    Preset {
        id: "moonlight",
        label_key: "asset-effect-moonlight",
        icon: ph::MOON,
        color: [80, 100, 160],
        coming_soon: false,
    },
    Preset {
        id: "dreamy_haze",
        label_key: "asset-effect-dreamy_haze",
        icon: ph::CLOUD,
        color: [200, 180, 220],
        coming_soon: false,
    },
    Preset {
        id: "flip3d",
        label_key: "asset-effect-flip3d",
        icon: ph::ARROWS_CLOCKWISE,
        color: [120, 140, 180],
        coming_soon: false,
    },
    Preset {
        id: "freezeframe",
        label_key: "asset-effect-freezeframe",
        icon: ph::PAUSE,
        color: [100, 120, 140],
        coming_soon: false,
    },
    Preset {
        id: "echo",
        label_key: "asset-effect-echo",
        icon: ph::WAVES,
        color: [140, 160, 180],
        coming_soon: false,
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
        coming_soon: false,
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
        coming_soon: false,
    },
    Preset {
        id: "bold",
        label_key: "asset-text-bold",
        icon: ph::TEXT_B,
        color: [160, 60, 60],
        coming_soon: false,
    },
    Preset {
        id: "subtitle",
        label_key: "asset-text-subtitle",
        icon: ph::TEXT_ALIGN_LEFT,
        color: [80, 120, 160],
        coming_soon: false,
    },
    Preset {
        id: "lower",
        label_key: "asset-text-lower",
        icon: ph::TEXTBOX,
        color: [130, 130, 90],
        coming_soon: false,
    },
    Preset {
        id: "quote",
        label_key: "asset-text-quote",
        icon: ph::QUOTES,
        color: [160, 120, 180],
        coming_soon: false,
    },
    Preset {
        id: "caption",
        label_key: "asset-text-caption",
        icon: ph::TEXT_ALIGN_CENTER,
        color: [90, 140, 120],
        coming_soon: false,
    },
    Preset {
        id: "glow",
        label_key: "asset-text-glow",
        icon: ph::LIGHTNING,
        color: [220, 90, 200],
        coming_soon: false,
    },
    Preset {
        id: "handwrite",
        label_key: "asset-text-handwrite",
        icon: ph::PENCIL,
        color: [180, 160, 100],
        coming_soon: false,
    },
    Preset {
        id: "outline",
        label_key: "asset-text-outline",
        icon: ph::SQUARES_FOUR,
        color: [100, 180, 160],
        coming_soon: false,
    },
    Preset {
        id: "shadow",
        label_key: "asset-text-shadow",
        icon: ph::EYE_CLOSED,
        color: [120, 100, 180],
        coming_soon: false,
    },
    Preset {
        id: "typewriter",
        label_key: "asset-text-typewriter",
        icon: ph::KEY,
        color: [160, 120, 80],
        coming_soon: false,
    },
    Preset {
        id: "neon",
        label_key: "asset-text-neon",
        icon: ph::LIGHTNING,
        color: [90, 240, 200],
        coming_soon: false,
    },
    Preset {
        id: "3d",
        label_key: "asset-text-3d",
        icon: ph::CUBE,
        color: [220, 160, 60],
        coming_soon: false,
    },
    Preset {
        id: "gradient",
        label_key: "asset-text-gradient",
        icon: ph::FADERS,
        color: [230, 130, 160],
        coming_soon: false,
    },
    Preset {
        id: "stamp",
        label_key: "asset-text-stamp",
        icon: ph::STAMP,
        color: [180, 60, 60],
        coming_soon: false,
    },
];

// ---------------------------------------------------------------------------
// Categories (left-column filter on Transitions + Effects)
// ---------------------------------------------------------------------------

/// (category id, FTL label key) for the Transitions left column.
const TRANSITION_CATS: &[(&str, &str)] = &[
    ("all", "asset-cat-all"),
    ("slide", "asset-cat-slide"),
    ("wipe", "asset-cat-wipe"),
    ("zoom", "asset-cat-zoom"),
    ("smooth", "asset-cat-smooth"),
    ("style", "asset-cat-style"),
];

/// (category id, FTL label key) for the Effects left column.
const EFFECT_CATS: &[(&str, &str)] = &[
    ("all", "asset-cat-all"),
    ("color", "asset-cat-color"),
    ("distort", "asset-cat-distort"),
    ("texture", "asset-cat-texture"),
    ("motion", "asset-cat-motion"),
    ("adjust", "asset-cat-adjust"),
];

/// Map a preset id to its left-column category. Transition, effect,
/// and filter id namespaces do not overlap, so one function covers
/// all three tabs.
fn preset_category(id: &str) -> &'static str {
    match id {
        // Transitions
        "slide_l" | "slide_r" | "slide_u" | "slide_d" => "slide",
        "wipe_l" | "wipe_r" | "wipe_u" | "wipe_d" => "wipe",
        "wipe_tl" | "wipe_tr" | "wipe_bl" | "wipe_br" => "wipe",
        "zoom_in" | "zoom_out" => "zoom",
        "zoom_blur" => "zoom",
        "smooth_l" | "smooth_r" | "smooth_u" | "smooth_d" => "smooth",
        "cover_l" | "cover_r" | "cover_u" | "cover_d" => "slide",
        "reveal_l" | "reveal_r" | "reveal_u" | "reveal_d" => "slide",
        "diag_tl" | "diag_tr" | "diag_bl" | "diag_br" => "wipe",
        "vert_open" | "vert_close" | "horz_open" | "horz_close" => "wipe",
        "fadegrays" | "distance" | "hblur" => "fade",
        "circle_crop" | "rect_crop" => "zoom",
        "squeeze_h" | "squeeze_v" => "distort",
        // Effects
        "negative" | "rgb_split" | "flash" | "light_leak" | "lens_flare" | "old_film" | "vhs" => {
            "color"
        }
        "glitch" | "mirror" | "kaleido" | "pixelate" => "distort",
        "grain" | "sketch" | "sparkle" | "particle" => "texture",
        "shake" | "zoom_pulse" | "ghost" | "echo" => "motion",
        "blur" | "sharpen" | "vignette" | "glow" | "dreamy_haze" => "adjust",
        "cinematic_bars" | "flip3d" | "freezeframe" => "motion",
        "teal_orange" | "cross_process" | "bleach_bypass" | "moonlight" => "color",
        // Filters
        "warm" | "cool" | "sunset" | "gold" | "ocean" | "cinematic" | "vivid" => "color",
        "bw" | "noir" | "sepia" | "vintage" | "matte" | "pastel" | "neon" => "style",
        "fade" => "style",
        _ => "style",
    }
}

/// (category id, FTL label key) for the Filters left column.
const FILTER_CATS: &[(&str, &str)] = &[
    ("all", "asset-cat-all"),
    ("color", "asset-cat-color"),
    ("style", "asset-cat-style"),
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
    /// User asked to add a CLAP plugin to the master chain.
    pub plugin_add_requested: Option<caprust_media_io::clap_host::PluginInfo>,
}

pub fn show(
    ui: &mut Ui,
    project: &mut ProjectState,
    state: &mut AssetBrowserState,
    media_state: &mut MediaBinState,
) -> AssetBrowserOutput {
    // Sync tab_layout from media_state to local state
    state.tab_layout = media_state.tab_layout;
    let mut out = AssetBrowserOutput::default();

    // --- Vertical tab strip on the left, content on the right ---
    ui.horizontal_top(|ui| {
        // Left: vertical tab strip
        ui.vertical(|ui| {
            ui.set_min_width(120.0);
            for tab in AssetTab::all() {
                let selected = state.active == tab;
                let text = format!("{} {}", tab.icon(), tr(tab.label()));
                let btn = egui::Button::new(RichText::new(text).size(12.0))
                    .fill(if selected {
                        ui.visuals().selection.bg_fill
                    } else {
                        Color32::from_gray(50)
                    })
                    .min_size(V2::new(0.0, 30.0));
                if ui.add(btn).clicked() {
                    state.active = tab;
                }
            }
        });

        ui.separator();

        // Right: tab content
        ui.vertical(|ui| {
            let content = render_tab_content(
                ui,
                state.active,
                project,
                state,
                media_state,
                media_state.tab_layout,
            );
            out.media = content.media;
            out.preset_clicked = content.preset_clicked;
        });
    });

    out
}

/// Render the content of one tab, without the tab strip. Shared
/// between the main asset browser and any docked (floating) copy of a
/// tab.
pub fn render_tab_content(
    ui: &mut Ui,
    tab: AssetTab,
    project: &mut ProjectState,
    state: &mut AssetBrowserState,
    media_state: &mut MediaBinState,
    tab_layout: TabLayout,
) -> AssetBrowserOutput {
    let mut out = AssetBrowserOutput::default();
    match tab {
        AssetTab::Media => {
            out.media = crate::panels::media_bin::show(ui, project, media_state);
        }
        AssetTab::Transitions => {
            let clicked = categorized_grid(
                ui,
                TRANSITIONS,
                TRANSITION_CATS,
                "trans_cats",
                &mut state.transition_cat,
                state.card_size.px(),
                &state.search,
                &mut state.sidebar_width,
                &mut state.card_size,
                tab_layout,
            );
            if let Some(id) = clicked {
                out.preset_clicked = Some((id, AssetTab::Transitions));
            }
        }
        AssetTab::Effects => {
            let clicked = categorized_grid(
                ui,
                EFFECTS,
                EFFECT_CATS,
                "effect_cats",
                &mut state.effect_cat,
                state.card_size.px(),
                &state.search,
                &mut state.sidebar_width,
                &mut state.card_size,
                tab_layout,
            );
            if let Some(id) = clicked {
                out.preset_clicked = Some((id, AssetTab::Effects));
            }
        }
        AssetTab::Filters => {
            let clicked = categorized_grid(
                ui,
                FILTERS,
                FILTER_CATS,
                "filter_cats",
                &mut state.filter_cat,
                state.card_size.px(),
                &state.search,
                &mut state.sidebar_width,
                &mut state.card_size,
                tab_layout,
            );
            if let Some(id) = clicked {
                out.preset_clicked = Some((id, AssetTab::Filters));
            }
        }
        AssetTab::Text => {
            // Text styles use a single "all" category but still get the
            // S/M/L size picker and resizable sidebar.
            const TEXT_CATS: &[(&str, &str)] = &[("all", "asset-cat-all")];
            let clicked = categorized_grid(
                ui,
                TEXT_STYLES,
                TEXT_CATS,
                "text_cats",
                &mut "all".to_string(), // dummy, not used for text
                state.card_size.px(),
                &state.search,
                &mut state.sidebar_width,
                &mut state.card_size,
                tab_layout,
            );
            if let Some(id) = clicked {
                out.preset_clicked = Some((id, AssetTab::Text));
            }
        }
        AssetTab::Plugins => {
            let pb_out = crate::panels::plugin_browser::show(ui, &mut state.plugin_browser);
            if let Some(info) = pb_out.add_requested {
                out.plugin_add_requested = Some(info);
            }
        }
        AssetTab::Templates => {
            empty::placeholder(ui, tr("asset-templates-hint"));
        }
    }
    out
}

/// Left-column category filter with the preset grid on the right.
/// Used by the Transitions, Effects, Filters, and Text tabs. Returns the id
/// of a card that was clicked, if any.
#[allow(clippy::too_many_arguments)]
fn categorized_grid(
    ui: &mut Ui,
    presets: &[Preset],
    cats: &[(&str, &str)],
    salt: &str,
    selected: &mut String,
    _card_size: f32,
    search: &str,
    sidebar_width: &mut f32,
    card_size_state: &mut PresetCardSize,
    tab_layout: TabLayout,
) -> Option<&'static str> {
    // In NoFilter mode, skip the sidebar and show the grid directly
    if tab_layout == TabLayout::NoFilter {
        return preset_grid(
            ui,
            presets,
            card_size_state.px(),
            search,
            Some(selected.as_str()),
            ui.available_height(),
        );
    }

    // Capture the height before entering the horizontal layout:
    // available_height() reads 0 inside one (§5), and both columns
    // need an explicit bound for their scroll areas.
    let h = ui.available_height();
    let mut clicked: Option<&'static str> = None;
    ui.horizontal_top(|ui| {
        // Left sidebar: category list + size picker
        let _sidebar_resp = egui::ScrollArea::vertical()
            .id_salt(salt)
            .max_height(h)
            .auto_shrink([true, false])
            .show(ui, |ui| {
                ui.set_min_width(80.0);
                ui.set_max_width(200.0);
                ui.set_width(*sidebar_width);
                ui.vertical(|ui| {
                    // Size picker (S/M/L) — above categories
                    ui.horizontal(|ui| {
                        for sz in PresetCardSize::all() {
                            let _resp = ui.selectable_value(card_size_state, sz, sz.label());
                        }
                    });
                    ui.separator();
                    for (id, key) in cats {
                        ui.selectable_value(selected, (*id).to_string(), tr(key));
                    }
                });
            });
        // Drag handle to resize sidebar
        let sep_resp = ui.allocate_response(Vec2::new(4.0, h), Sense::drag());
        if sep_resp.dragged() {
            *sidebar_width = (*sidebar_width + sep_resp.drag_delta().x).clamp(80.0, 200.0);
        }
        sep_resp.on_hover_cursor(CursorIcon::ResizeHorizontal);
        ui.separator();
        // Right: preset grid
        ui.vertical(|ui| {
            clicked = preset_grid(
                ui,
                presets,
                card_size_state.px(),
                search,
                Some(selected.as_str()),
                h,
            );
        });
    });
    clicked
}

/// Grid of preset cards. Returns the id of a card that was clicked, if any.
fn preset_grid(
    ui: &mut Ui,
    presets: &[Preset],
    card_size: f32,
    search: &str,
    category: Option<&str>,
    max_h: f32,
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
            // Category first ("none" always stays: it clears the
            // transition and is not a style). Empty/"all" = no filter.
            let cat_ok = match category {
                None => true,
                Some(c) if c == "all" || c.is_empty() => true,
                Some(c) => p.id == "none" || preset_category(p.id) == c,
            };
            if !cat_ok {
                return false;
            }
            if needle.is_empty() {
                return true;
            }
            let label = tr(p.label_key).to_lowercase();
            label.contains(&needle) || p.id.contains(&needle)
        })
        .collect();

    if filtered.is_empty() {
        empty::placeholder(ui, tr("asset-empty"));
        return None;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(max_h)
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
        egui::Stroke::new(elev::STROKE_HAIRLINE, border),
        egui::StrokeKind::Inside,
    );

    resp.clicked()
}
