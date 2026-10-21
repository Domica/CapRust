//! The guided tips: three steps through the editor, shown once.
//!
//! The tips are not a tutorial video and not a modal that blocks the
//! app. They are a small card that sits over the editor and says what
//! to do next, while the editor stays usable. The point is that a
//! first-time user learns the shape of the thing by doing it, not by
//! reading four paragraphs about it.
//!
//! Which step is showing is the app's business (it knows what the user
//! has already done); this module only draws the card and reports what
//! the user clicked.

use egui::Ui;

use crate::i18n_helper::{tr, tr_with};
use crate::theme::tokens::space;
use crate::widgets::button;

/// The steps, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TourStep {
    /// Bring a file into the media bin.
    Import,
    /// Put something on the timeline.
    Timeline,
    /// Render the result.
    Export,
}

impl TourStep {
    /// How many steps there are in total.
    pub const TOTAL: usize = 3;

    /// The step's number, counting from one.
    pub fn number(self) -> usize {
        match self {
            Self::Import => 1,
            Self::Timeline => 2,
            Self::Export => 3,
        }
    }

    /// The next step, or `None` at the end.
    pub fn next(self) -> Option<Self> {
        match self {
            Self::Import => Some(Self::Timeline),
            Self::Timeline => Some(Self::Export),
            Self::Export => None,
        }
    }

    /// The previous step, or `None` at the start.
    pub fn prev(self) -> Option<Self> {
        match self {
            Self::Import => None,
            Self::Timeline => Some(Self::Import),
            Self::Export => Some(Self::Timeline),
        }
    }

    /// The heading.
    pub fn title(self) -> String {
        match self {
            Self::Import => tr("tour-import-title"),
            Self::Timeline => tr("tour-timeline-title"),
            Self::Export => tr("tour-export-title"),
        }
    }

    /// The body text: what to do, in one or two sentences.
    pub fn body(self) -> String {
        match self {
            Self::Import => tr("tour-import-body"),
            Self::Timeline => tr("tour-timeline-body"),
            Self::Export => tr("tour-export-body"),
        }
    }
}

/// What the user asked for by clicking.
#[derive(Debug, Default)]
pub struct TourEvents {
    /// Move to the next step, or finish if this was the last one.
    pub next: bool,
    /// Move back a step.
    pub back: bool,
    /// Close the tips without finishing them.
    pub skip: bool,
}

/// Draw the card.
///
/// `done` says whether the user has already done what this step asks
/// for. It is not used to move the tips along by itself: a step that
/// jumps forward on its own is a step the user cannot go back from.
pub fn show(ui: &mut Ui, step: TourStep, done: bool) -> TourEvents {
    let mut ev = TourEvents::default();

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(tr_with(
                "tour-counter",
                &[
                    ("current", &step.number().to_string()),
                    ("total", &TourStep::TOTAL.to_string()),
                ],
            ))
            .small()
            .color(egui::Color32::from_gray(140)),
        );
        if done {
            ui.label(
                egui::RichText::new(tr("tour-done-hint"))
                    .small()
                    .color(egui::Color32::from_rgb(120, 200, 120)),
            );
        }
    });

    ui.add_space(space::XS);
    ui.label(egui::RichText::new(step.title()).strong());
    ui.label(
        egui::RichText::new(step.body())
            .size(12.5)
            .color(egui::Color32::from_gray(175)),
    );
    ui.add_space(space::S);

    // Right to left, so the primary button sits on the right where
    // the eye expects it.
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let label = if step.next().is_some() {
            tr("tour-next")
        } else {
            tr("tour-finish")
        };
        if button::primary(ui, label).clicked() {
            ev.next = true;
        }
        if button::ghost(ui, tr("tour-skip")).clicked() {
            ev.skip = true;
        }
        if step.prev().is_some() && button::ghost(ui, tr("tour-back")).clicked() {
            ev.back = true;
        }
    });

    ev
}

/// Draw the card inside its own window.
///
/// The window is anchored to the top centre of the screen: the top of
/// the editor, clear of the timeline at the bottom and of the
/// properties panel on the right. It is a plain window and not a
/// modal, so the editor stays usable while the tips are up -- the
/// point is to do the thing while it is being described.
pub fn show_window(ctx: &egui::Context, step: TourStep, done: bool) -> TourEvents {
    let mut ev = TourEvents::default();
    egui::Window::new(tr("tour-title"))
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 92.0))
        .collapsible(false)
        .resizable(false)
        .default_width(340.0)
        .show(ctx, |ui| {
            ev = show(ui, step, done);
        });
    ev
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render the card and collect every piece of text it drew.
    fn texts(step: TourStep, done: bool) -> Vec<String> {
        let ctx = egui::Context::default();
        let out = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = show(ui, step, done);
            });
        });
        out.shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
                _ => None,
            })
            .map(|t| {
                // Fluent wraps every interpolated value in directional
                // isolate marks, so "1 / 3" arrives as "<2068>1<2069> /
                // <2068>3<2069>". Strip them before matching.
                t.chars()
                    .filter(|c| !matches!(c, '\u{2066}'..='\u{2069}'))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn the_card_says_what_the_step_is() {
        let t = texts(TourStep::Import, false);
        assert!(
            t.iter().any(|x| x.contains("Import your media")),
            "the card does not name the step: {t:?}"
        );
        assert!(t.iter().any(|x| x.contains("1 / 3")), "no counter: {t:?}");
    }

    #[test]
    fn the_counter_follows_the_step() {
        assert!(texts(TourStep::Timeline, false)
            .iter()
            .any(|x| x.contains("2 / 3")));
        assert!(texts(TourStep::Export, false)
            .iter()
            .any(|x| x.contains("3 / 3")));
    }

    #[test]
    fn a_step_that_is_already_done_says_so() {
        assert!(!texts(TourStep::Import, false)
            .iter()
            .any(|x| x.contains("done")));
        assert!(texts(TourStep::Import, true)
            .iter()
            .any(|x| x.contains("done")));
    }

    #[test]
    fn the_steps_walk_forward_and_back() {
        assert_eq!(TourStep::Import.next(), Some(TourStep::Timeline));
        assert_eq!(TourStep::Timeline.next(), Some(TourStep::Export));
        assert_eq!(TourStep::Export.next(), None);

        assert_eq!(TourStep::Import.prev(), None);
        assert_eq!(TourStep::Timeline.prev(), Some(TourStep::Import));
        assert_eq!(TourStep::Export.prev(), Some(TourStep::Timeline));
    }

    #[test]
    fn every_step_has_a_title_and_a_body() {
        for step in [TourStep::Import, TourStep::Timeline, TourStep::Export] {
            assert!(!step.title().is_empty());
            assert!(!step.body().is_empty());
        }
    }

    #[test]
    fn the_card_sits_at_the_top_of_the_editor() {
        // The card is anchored to the top centre, so it never lands on
        // the timeline and never covers the properties panel. The
        // anchor is checked here rather than by eye: a card that
        // drifts over the timeline hides the thing it is talking
        // about.
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1920.0, 1080.0),
            )),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        // Two frames: the first one builds the font atlas and paints no
        // text at all, so there is nothing to measure in it.
        let _ = ctx.run(raw.clone(), |ctx| {
            show_window(ctx, TourStep::Import, false);
        });
        let out = ctx.run(raw, |ctx| {
            show_window(ctx, TourStep::Import, false);
        });

        let rects: Vec<egui::Rect> = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                // The galley's own rectangle is relative to nothing;
                // the shape's position is where it lands on screen.
                egui::Shape::Text(t) => {
                    Some(egui::Rect::from_min_size(t.pos, t.galley.rect.size()))
                }
                _ => None,
            })
            .collect();
        assert!(!rects.is_empty(), "the card drew nothing");

        let min_x = rects.iter().map(|r| r.min.x).fold(f32::MAX, f32::min);
        let max_x = rects.iter().map(|r| r.max.x).fold(f32::MIN, f32::max);
        let centre = (min_x + max_x) / 2.0;
        assert!(
            (centre - 960.0).abs() < 120.0,
            "the card is not centred: it spans {min_x}..{max_x}"
        );
        let lowest = rects.iter().map(|r| r.max.y).fold(f32::MIN, f32::max);
        assert!(
            lowest < 400.0,
            "the card reaches down to y={lowest}, into the timeline"
        );
    }
}
