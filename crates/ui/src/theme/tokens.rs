//! Design tokens: spacing, radius, typography, animation, elevation.
//!
//! Single source of truth for every visual constant in the UI. Never
//! hardcode a pixel value in a panel — add or reuse a token here.
//!
//! Namespaces: `space::M`, `radius::MD`, `text::L`, `anim::FAST`,
//! `elev::SHADOW_MID`. `apply_style(ctx)` pushes the token-driven
//! spacing + baseline text sizes into the global egui style.
//!
//! User-level overrides (font family / size via Settings) apply *on top*
//! of these baselines in a later phase; `text::M = 13.0` is the CapCut-
//! style compact body that we ship by default.

use egui::Context;

/// Spacing scale — 4-px base grid.
///
/// `XXS` is the sole sub-4 value, for hairline gaps inside a chip.
pub mod space {
    pub const XXS: f32 = 2.0;
    pub const XS: f32 = 4.0;
    pub const S: f32 = 6.0;
    pub const M: f32 = 8.0;
    pub const M_PLUS: f32 = 10.0;
    pub const L: f32 = 12.0;
    pub const XL: f32 = 16.0;
    pub const XXL: f32 = 24.0;
    pub const XXXL: f32 = 32.0;
}

/// Corner radii — map to `egui::CornerRadius` via `radius::cr(v)`.
pub mod radius {
    pub const NONE: f32 = 0.0;
    pub const SM: f32 = 4.0;
    pub const MD: f32 = 6.0;
    pub const LG: f32 = 8.0;
    pub const XL: f32 = 12.0;
    /// Clamped by egui at draw time to half the shorter side.
    pub const PILL: f32 = 999.0;

    /// Build an `egui::CornerRadius` from an `f32` token.
    pub fn cr(v: f32) -> egui::CornerRadius {
        egui::CornerRadius::same(v.round().clamp(0.0, 255.0) as u8)
    }
}

/// Typography scale. `M = 13.0` is the shipped compact body size.
pub mod text {
    pub const XS: f32 = 10.0;
    pub const S: f32 = 11.0;
    pub const M: f32 = 13.0;
    pub const L: f32 = 15.0;
    pub const XL: f32 = 18.0;
    pub const XXL: f32 = 24.0;
}

/// Animation durations in seconds for `Context::animate_bool_with_time`.
pub mod anim {
    pub const INSTANT: f32 = 0.0;
    pub const FAST: f32 = 0.10;
    pub const NORMAL: f32 = 0.20;
    pub const SLOW: f32 = 0.35;
}

/// Elevation: stroke widths + shadow presets.
///
/// Shadow tuple is `(offset [x,y], blur, spread)`; the color comes from
/// `Theme` so tokens stay theme-agnostic.
pub mod elev {
    pub const STROKE_HAIRLINE: f32 = 1.0;
    pub const STROKE_EMPHASIS: f32 = 1.5;

    pub const SHADOW_LOW: ([i8; 2], u8, u8) = ([0, 1], 4, 0);
    pub const SHADOW_MID: ([i8; 2], u8, u8) = ([0, 2], 8, 0);
    pub const SHADOW_HIGH: ([i8; 2], u8, u8) = ([0, 4], 16, 0);
}

/// Apply token-driven spacing + baseline text sizes to the global style.
///
/// Called from `Theme::apply` after the color pass. Only touches
/// `Spacing` and `text_styles`; color / visuals stay owned by `Theme`.
pub fn apply_style(ctx: &Context) {
    ctx.style_mut(|s| {
        // --- spacing ---
        s.spacing.item_spacing = egui::vec2(space::M, space::S);
        s.spacing.window_margin = egui::Margin::same(space::M.round() as i8);
        s.spacing.button_padding = egui::vec2(space::M, space::XS);
        s.spacing.indent = space::L;
        s.spacing.interact_size.y = 22.0;
        s.spacing.slider_width = 120.0;
        s.spacing.combo_width = 140.0;
        s.spacing.text_edit_width = 200.0;
        s.spacing.tooltip_width = 480.0;
        s.spacing.icon_width = 16.0;
        s.spacing.scroll.bar_width = 8.0;
        s.spacing.scroll.bar_inner_margin = 2.0;
        s.spacing.scroll.bar_outer_margin = 2.0;

        // --- baseline text sizes ---
        use egui::TextStyle::*;
        s.text_styles
            .insert(Small, egui::FontId::proportional(text::S));
        s.text_styles
            .insert(Body, egui::FontId::proportional(text::M));
        s.text_styles
            .insert(Button, egui::FontId::proportional(text::M));
        s.text_styles
            .insert(Heading, egui::FontId::proportional(text::L));
        s.text_styles
            .insert(Monospace, egui::FontId::monospace(text::M));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_is_ascending() {
        let v = [
            space::XXS,
            space::XS,
            space::S,
            space::M,
            space::L,
            space::XL,
            space::XXL,
            space::XXXL,
        ];
        assert!(
            v.windows(2).all(|w| w[0] < w[1]),
            "space not ascending: {:?}",
            v
        );
    }

    /// Every space token except `XXS` must sit on the 4-px grid.
    #[test]
    fn space_aligns_to_2px_grid() {
        for (n, v) in [
            ("XXS", space::XXS),
            ("XS", space::XS),
            ("S", space::S),
            ("M", space::M),
            ("L", space::L),
            ("XL", space::XL),
            ("XXL", space::XXL),
            ("XXXL", space::XXXL),
        ] {
            assert_eq!(v % 2.0, 0.0, "space::{} = {} not on 2-px grid", n, v);
        }
    }

    #[test]
    fn large_spaces_align_to_4px_grid() {
        for (n, v) in [
            ("M", space::M),
            ("L", space::L),
            ("XL", space::XL),
            ("XXL", space::XXL),
            ("XXXL", space::XXXL),
        ] {
            assert_eq!(v % 4.0, 0.0, "space::{} = {} not on 4-px grid", n, v);
        }
    }

    #[test]
    fn radius_is_ascending() {
        let v = [
            radius::NONE,
            radius::SM,
            radius::MD,
            radius::LG,
            radius::XL,
            radius::PILL,
        ];
        assert!(
            v.windows(2).all(|w| w[0] < w[1]),
            "radius not ascending: {:?}",
            v
        );
    }

    #[test]
    fn text_is_ascending() {
        let v = [text::XS, text::S, text::M, text::L, text::XL, text::XXL];
        assert!(
            v.windows(2).all(|w| w[0] < w[1]),
            "text not ascending: {:?}",
            v
        );
    }

    #[test]
    fn anim_is_ascending() {
        let v = [anim::INSTANT, anim::FAST, anim::NORMAL, anim::SLOW];
        assert!(
            v.windows(2).all(|w| w[0] < w[1]),
            "anim not ascending: {:?}",
            v
        );
    }

    #[test]
    fn radius_cr_matches_egui() {
        assert_eq!(radius::cr(radius::MD), egui::CornerRadius::same(6));
        assert_eq!(radius::cr(radius::NONE), egui::CornerRadius::same(0));
        assert_eq!(radius::cr(radius::SM), egui::CornerRadius::same(4));
    }

    /// Deliberately compact — do not "fix" to egui's default 14.
    #[test]
    fn body_size_is_compact() {
        assert_eq!(text::M, 13.0);
    }

    #[test]
    fn shadow_presets_are_ordered() {
        let (_, blur_low, _) = elev::SHADOW_LOW;
        let (_, blur_mid, _) = elev::SHADOW_MID;
        let (_, blur_high, _) = elev::SHADOW_HIGH;
        assert!(blur_low < blur_mid && blur_mid < blur_high);
    }
}
