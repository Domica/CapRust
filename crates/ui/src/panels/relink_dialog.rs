//! Missing-media relink dialog.
//!
//! Shown on project load when the media library references files that
//! are not on disk. Actions:
//!   1. Locate folder -- the app opens an rfd picker, scans the
//!      folder, and builds a `RelinkManyCommand`.
//!   2. Skip -- close without changing anything.
//!   3. (after a partial scan) Try another folder.
//!
//! The dialog is a pure renderer. rfd, command execution, and toasts
//! live in `app.rs`.

use caprust_core::commands::relink_many::MissingRef;
use egui::Ui;

use crate::i18n_helper::tr;

#[derive(Debug, Clone, Default)]
pub enum RelinkPhase {
    #[default]
    Choose,
    Done {
        matched: usize,
        still_missing: usize,
    },
    Failed(String),
}

pub struct RelinkDialogState {
    pub missing: Vec<MissingRef>,
    pub phase: RelinkPhase,
    /// True once the user has picked a folder at least once. Flips
    /// the primary button label to "Try another folder".
    pub tried_folder: bool,
}

impl Default for RelinkDialogState {
    fn default() -> Self {
        Self {
            missing: Vec::new(),
            phase: RelinkPhase::Choose,
            tried_folder: false,
        }
    }
}

#[derive(Debug, Default)]
pub struct RelinkDialogEvents {
    pub close: bool,
    pub locate_folder: bool,
}

pub fn show(ui: &mut Ui, state: &mut RelinkDialogState) -> RelinkDialogEvents {
    let mut ev = RelinkDialogEvents::default();

    match &state.phase {
        RelinkPhase::Choose => {
            ui.label(egui::RichText::new(tr("relink-dialog-body")).size(12.5));
            ui.add_space(8.0);

            ui.label(
                egui::RichText::new(format!(
                    "{}: {}",
                    tr("relink-dialog-count-label"),
                    state.missing.len()
                ))
                .strong(),
            );

            let preview_n = 5.min(state.missing.len());
            for m in state.missing.iter().take(preview_n) {
                let extra = if m.clip_count > 0 {
                    format!("  ({} clip)", m.clip_count)
                } else {
                    String::new()
                };
                ui.label(
                    egui::RichText::new(format!("- {}{}", m.name, extra))
                        .small()
                        .color(egui::Color32::from_gray(180)),
                );
            }
            if state.missing.len() > preview_n {
                ui.label(
                    egui::RichText::new(format!(
                        "- +{} {}",
                        state.missing.len() - preview_n,
                        tr("relink-dialog-more")
                    ))
                    .small()
                    .color(egui::Color32::from_gray(180)),
                );
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let locate_label = if state.tried_folder {
                    tr("relink-dialog-retry")
                } else {
                    tr("relink-dialog-locate")
                };
                if ui
                    .button(locate_label)
                    .on_hover_text(tr("relink-dialog-locate-hint"))
                    .clicked()
                {
                    ev.locate_folder = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(tr("relink-dialog-skip"))
                        .on_hover_text(tr("relink-dialog-skip-hint"))
                        .clicked()
                    {
                        ev.close = true;
                    }
                });
            });
        }
        RelinkPhase::Done {
            matched,
            still_missing,
        } => {
            ui.label(
                egui::RichText::new(format!("{}: {}", tr("relink-dialog-done"), matched))
                    .strong()
                    .color(egui::Color32::from_rgb(120, 200, 120)),
            );
            if *still_missing > 0 {
                ui.label(
                    egui::RichText::new(format!(
                        "{}: {}",
                        tr("relink-dialog-still-missing"),
                        still_missing
                    ))
                    .color(egui::Color32::from_rgb(220, 180, 100)),
                );
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if *still_missing > 0 && ui.button(tr("relink-dialog-retry")).clicked() {
                    ev.locate_folder = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tr("relink-dialog-close")).clicked() {
                        ev.close = true;
                    }
                });
            });
        }
        RelinkPhase::Failed(msg) => {
            ui.label(
                egui::RichText::new(tr("relink-dialog-failed"))
                    .strong()
                    .color(egui::Color32::from_rgb(220, 120, 120)),
            );
            ui.label(
                egui::RichText::new(msg)
                    .small()
                    .color(egui::Color32::from_gray(180)),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(tr("relink-dialog-retry")).clicked() {
                    ev.locate_folder = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tr("relink-dialog-close")).clicked() {
                        ev.close = true;
                    }
                });
            });
        }
    }

    ev
}
