//! Status banner: full-width row with an icon, a message, and an
//! optional dismiss button. Three kinds -- Info, Warning, Error -- each
//! with its own background, border, and icon tint.
//!
//! Callers use the convenience wrappers `info` / `warn` / `error` for
//! non-dismissible banners, or `show` when they want the dismiss
//! return value.

use crate::theme::tokens::{radius, space, text};
use egui::{Color32, RichText, Ui};
use egui_phosphor::regular as ph;

/// Severity of the banner. Colors are banner-local (not tokens) because
/// each kind carries a specific semantic meaning that should not be
/// remapped by theme changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerKind {
    Info,
    Warning,
    Error,
}

impl BannerKind {
    fn icon(self) -> &'static str {
        match self {
            Self::Info => ph::INFO,
            Self::Warning => ph::WARNING,
            Self::Error => ph::X_CIRCLE,
        }
    }

    fn bg(self) -> Color32 {
        match self {
            Self::Info => Color32::from_rgb(40, 60, 80),
            Self::Warning => Color32::from_rgb(80, 65, 35),
            Self::Error => Color32::from_rgb(80, 40, 40),
        }
    }

    fn border(self) -> Color32 {
        match self {
            Self::Info => Color32::from_rgb(90, 130, 180),
            Self::Warning => Color32::from_rgb(200, 160, 60),
            Self::Error => Color32::from_rgb(200, 80, 80),
        }
    }

    fn icon_color(self) -> Color32 {
        match self {
            Self::Info => Color32::from_rgb(140, 180, 240),
            Self::Warning => Color32::from_rgb(240, 200, 100),
            Self::Error => Color32::from_rgb(240, 130, 130),
        }
    }
}

/// Full-width banner. Returns `true` if the dismiss button was clicked
/// this frame. `msg` must already be translated.
pub fn show(ui: &mut Ui, kind: BannerKind, msg: &str, dismissible: bool) -> bool {
    let mut dismissed = false;
    egui::Frame::NONE
        .fill(kind.bg())
        .stroke(egui::Stroke::new(1.0_f32, kind.border()))
        .corner_radius(radius::cr(radius::SM))
        .inner_margin(egui::Margin::symmetric(space::M as i8, space::S as i8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(kind.icon())
                        .color(kind.icon_color())
                        .size(text::L),
                );
                ui.add_space(space::XS);
                ui.label(RichText::new(msg).color(Color32::from_gray(230)));
                if dismissible {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(ph::X).clicked() {
                            dismissed = true;
                        }
                    });
                }
            });
        });
    dismissed
}

/// Non-dismissible info banner.
pub fn info(ui: &mut Ui, msg: &str) {
    let _ = show(ui, BannerKind::Info, msg, false);
}

/// Non-dismissible warning banner.
pub fn warn(ui: &mut Ui, msg: &str) {
    let _ = show(ui, BannerKind::Warning, msg, false);
}

/// Non-dismissible error banner.
pub fn error(ui: &mut Ui, msg: &str) {
    let _ = show(ui, BannerKind::Error, msg, false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Context;

    fn run_ui<F: FnOnce(&mut Ui)>(f: F) {
        let ctx = Context::default();
        let mut f = Some(f);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            if let Some(f) = f.take() {
                egui::CentralPanel::default().show(ctx, f);
            }
        });
    }

    #[test]
    fn info_renders_without_panic() {
        run_ui(|ui| {
            info(ui, "Downloaded 3 models.");
        });
    }

    #[test]
    fn warn_renders_without_panic() {
        run_ui(|ui| {
            warn(ui, "FFmpeg not found on PATH.");
        });
    }

    #[test]
    fn error_renders_without_panic() {
        run_ui(|ui| {
            error(ui, "Export failed: disk full.");
        });
    }

    #[test]
    fn dismissible_show_returns_false_without_input() {
        run_ui(|ui| {
            let dismissed = show(ui, BannerKind::Info, "Hello", true);
            assert!(!dismissed);
        });
    }

    #[test]
    fn banner_kinds_have_distinct_colors() {
        assert_ne!(BannerKind::Info.bg(), BannerKind::Warning.bg());
        assert_ne!(BannerKind::Warning.bg(), BannerKind::Error.bg());
        assert_ne!(BannerKind::Info.icon(), BannerKind::Warning.icon());
    }
}
