//! Ids of the presets the asset browser offers.
//!
//! These live in core rather than in the UI panel because the id is not
//! a display concern -- it is the string the export graph matches on.
//! A second copy in the UI would drift silently, and drift here means a
//! remote client applies a preset that renders as nothing.
//!
//! Only ids, no icons and no label keys: those stay with the UI, which
//! is the only place that can draw them. `asset_browser` asserts its
//! own tables cover exactly these sets, so adding an id in one place
//! and forgetting the other is a test failure rather than a preset that
//! quietly does nothing.
//!
//! `none` appears in both the transition and filter tables because
//! "no transition" and "no filter" are different clears on different
//! fields. Anything selecting from these must therefore carry which
//! table it came from.

/// One selectable preset. Just the id today; a struct so adding a
/// label or category later is not a signature change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    pub id: &'static str,
}

/// Which table a preset belongs to. Needed because the id namespaces
/// overlap (`none`, `fade`, `vintage` all appear in more than one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetKind {
    Transition,
    Effect,
    Filter,
}

impl PresetKind {
    /// Wire name used by the MCP `list_assets` / `apply_preset` tools.
    pub fn as_str(self) -> &'static str {
        match self {
            PresetKind::Transition => "transition",
            PresetKind::Effect => "effect",
            PresetKind::Filter => "filter",
        }
    }

    /// Parse a wire name. Case-insensitive because a phone keyboard
    /// capitalises things and the error is otherwise baffling.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "transition" => Some(PresetKind::Transition),
            "effect" | "effects" => Some(PresetKind::Effect),
            "filter" | "filters" => Some(PresetKind::Filter),
            _ => None,
        }
    }
}

macro_rules! presets {
    ($name:ident, $($id:literal),* $(,)?) => {
        pub const $name: &[Preset] = &[$(Preset { id: $id }),*];
    };
}

presets!(
    TRANSITIONS,
    "none",
    "fade",
    "fadewhite",
    "slide_l",
    "slide_r",
    "slide_u",
    "slide_d",
    "zoom_in",
    "zoom_out",
    "wipe_l",
    "wipe_r",
    "wipe_u",
    "wipe_d",
    "rotate",
    "blur_t",
    "dissolve",
    "smooth_l",
    "smooth_r",
    "smooth_u",
    "smooth_d",
    "circle_close",
    "pixelize",
    "cover_l",
    "cover_r",
    "cover_u",
    "cover_d",
    "reveal_l",
    "reveal_r",
    "reveal_u",
    "reveal_d",
    "diag_tl",
    "diag_tr",
    "diag_bl",
    "diag_br",
    "wipe_tl",
    "wipe_tr",
    "wipe_bl",
    "wipe_br",
    "vert_open",
    "vert_close",
    "horz_open",
    "horz_close",
    "fadegrays",
    "distance",
    "hblur",
    "circle_crop",
    "rect_crop",
    "zoom_blur",
    "squeeze_h",
    "squeeze_v",
    "fade_fast",
    "fade_slow",
    "slice_hl",
    "slice_hr",
    "slice_vu",
    "slice_vd",
    "wind_right",
    "wind_up",
    "wind_down",
);

presets!(
    EFFECTS,
    "blur",
    "vignette",
    "glitch",
    "rgb_split",
    "shake",
    "zoom_pulse",
    "flash",
    "mirror",
    "kaleido",
    "old_film",
    "vhs",
    "light_leak",
    "particle",
    "sparkle",
    "ghost",
    "lens_flare",
    "negative",
    "pixelate",
    "sketch",
    "sharpen",
    "enhance",
    "grain",
    "glow",
    "cinematic_bars",
    "teal_orange",
    "cross_process",
    "bleach_bypass",
    "moonlight",
    "dreamy_haze",
    "flip3d",
    "freezeframe",
    "echo",
    "beauty",
    "spin",
    "scanlines",
    "edge_glow",
    "trails",
    "stutter",
    "ripple",
    "pixel_shuffle",
);

presets!(
    FILTERS,
    "none",
    "warm",
    "cool",
    "bw",
    "sepia",
    "cinematic",
    "vintage",
    "vivid",
    "matte",
    "noir",
    "sunset",
    "ocean",
    "fade",
    "pastel",
    "neon",
    "gold",
    "infrared",
    "blueprint",
    "poster",
    "bleach",
    "day_night",
    "ink",
    "faded_film",
);

/// The table a kind refers to.
pub const fn table(kind: PresetKind) -> &'static [Preset] {
    match kind {
        PresetKind::Transition => TRANSITIONS,
        PresetKind::Effect => EFFECTS,
        PresetKind::Filter => FILTERS,
    }
}

/// Is `id` in `kind`'s table?
pub fn contains(kind: PresetKind, id: &str) -> bool {
    table(kind).iter().any(|p| p.id == id)
}

/// The id that clears whatever this kind sets, rather than adding one.
pub const CLEAR: &str = "none";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_have_no_duplicate_ids() {
        for kind in [
            PresetKind::Transition,
            PresetKind::Effect,
            PresetKind::Filter,
        ] {
            let t = table(kind);
            for (i, a) in t.iter().enumerate() {
                for b in &t[i + 1..] {
                    assert_ne!(a.id, b.id, "duplicate preset {} in {:?}", a.id, kind);
                }
            }
        }
    }

    #[test]
    fn clear_exists_in_the_tables_that_have_one() {
        // Effects have no "none": the clear is removing the instance,
        // not adding a null one. Transitions and filters clear by
        // writing an absent value.
        assert!(contains(PresetKind::Transition, CLEAR));
        assert!(contains(PresetKind::Filter, CLEAR));
        assert!(!contains(PresetKind::Effect, CLEAR));
    }

    #[test]
    fn kind_round_trips_through_its_wire_name() {
        for kind in [
            PresetKind::Transition,
            PresetKind::Effect,
            PresetKind::Filter,
        ] {
            assert_eq!(PresetKind::parse(kind.as_str()), Some(kind));
        }
        // Plural, and wrong capitalisation, are the two things a phone
        // actually sends.
        assert_eq!(PresetKind::parse("Effects"), Some(PresetKind::Effect));
        assert_eq!(PresetKind::parse("FILTERS"), Some(PresetKind::Filter));
        assert_eq!(PresetKind::parse("nope"), None);
    }

    #[test]
    fn overlapping_ids_are_reachable_from_each_table() {
        // "fade" is both a transition and a filter, and applying the
        // wrong one is a no-op the user cannot see. Assert the lookup
        // actually distinguishes them.
        assert!(contains(PresetKind::Transition, "fade"));
        assert!(contains(PresetKind::Filter, "fade"));
        assert!(!contains(PresetKind::Effect, "fade"));
    }
}
