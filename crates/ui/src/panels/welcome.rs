//! The first-run welcome dialog.
//!
//! Shown once on the start screen of a fresh install. It says what
//! CapRust is in three lines and offers to walk through it. It is not
//! a marketing splash and it does not ask for anything: both buttons
//! close it, and the difference is only whether the guided tips
//! follow.

use egui::Ui;
use egui_phosphor::regular as ph;

use crate::i18n_helper::tr;
use crate::theme::tokens::space;
use crate::widgets::button;

/// What the user asked for by clicking.
#[derive(Debug, Default)]
pub struct WelcomeEvents {
    /// Show the guided tips in the editor.
    pub start: bool,
    /// Do not show them.
    pub skip: bool,
}

/// The three things CapRust does, in the order you do them.
const POINTS: [&str; 3] = [
    "welcome-point-import",
    "welcome-point-cut",
    "welcome-point-export",
];

/// Draw the body of the dialog. The caller owns the window.
pub fn show(ui: &mut Ui) -> WelcomeEvents {
    let mut ev = WelcomeEvents::default();

    ui.label(egui::RichText::new(tr("welcome-title")).size(19.0).strong());
    ui.add_space(space::XS);
    ui.label(
        egui::RichText::new(tr("welcome-subtitle"))
            .size(12.5)
            .color(egui::Color32::from_gray(160)),
    );
    ui.add_space(space::M_PLUS);

    for key in POINTS {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(ph::CHECK).color(egui::Color32::from_rgb(120, 200, 120)));
            ui.label(egui::RichText::new(tr(key)).size(12.5));
        });
        ui.add_space(space::XS);
    }

    ui.add_space(space::M);
    ui.horizontal(|ui| {
        if button::primary(ui, tr("welcome-start")).clicked() {
            ev.start = true;
        }
        if button::ghost(ui, tr("welcome-skip")).clicked() {
            ev.skip = true;
        }
    });

    ev
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render the dialog body and collect every piece of text it drew.
    fn texts() -> Vec<String> {
        let ctx = egui::Context::default();
        let out = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = show(ui);
            });
        });
        out.shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn it_says_the_three_things_caprust_does() {
        let t = texts();
        for what in [
            "Import your clips, images and audio.",
            "Cut them on the timeline",
            "Export when it looks right.",
        ] {
            assert!(t.iter().any(|x| x.contains(what)), "missing: {what}\n{t:?}");
        }
    }

    #[test]
    fn it_offers_both_ways_out() {
        let t = texts();
        assert!(t.iter().any(|x| x.contains("Show me around")), "{t:?}");
        assert!(t.iter().any(|x| x.contains("Skip")), "{t:?}");
    }
}
