//! Master-bus CLAP chain panel (Faza I, commit 4b-2).
//!
//! Lists the project's master plugin instances with per-instance
//! Bypass toggle and Remove button. Read-only against `ProjectState`;
//! all mutations go through `MasterChainOutput` and app.rs, which
//! executes the corresponding commands on the undo stack.

use caprust_core::ProjectState;
use egui::{Color32, RichText, Ui};
use egui_phosphor::regular as ph;
use uuid::Uuid;

use crate::i18n_helper::tr;
use crate::theme::tokens::space;

#[derive(Default)]
pub struct MasterChainState {}

#[derive(Default)]
pub struct MasterChainOutput {
    /// Instance id to remove.
    pub remove_instance: Option<Uuid>,
    /// (instance_id, new_bypassed_value).
    pub set_bypass: Option<(Uuid, bool)>,
}

pub fn show(
    ui: &mut Ui,
    project: &ProjectState,
    _state: &mut MasterChainState,
) -> MasterChainOutput {
    let mut out = MasterChainOutput::default();

    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("master-chain-heading")).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(format!("{}", project.master_plugins.len()))
                    .small()
                    .weak(),
            );
        });
    });
    ui.separator();

    if project.master_plugins.is_empty() {
        ui.add_space(space::XXL);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(tr("master-chain-empty"))
                    .italics()
                    .color(Color32::from_gray(140)),
            );
        });
        return out;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for inst in &project.master_plugins {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&inst.name).strong());
                            ui.label(RichText::new(&inst.plugin_id).small().monospace().weak());
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            if ui
                                .button(ph::X)
                                .on_hover_text(tr("master-chain-remove"))
                                .clicked()
                            {
                                out.remove_instance = Some(inst.id);
                            }
                            let mut bypassed = inst.bypassed;
                            if ui
                                .add(egui::Checkbox::new(
                                    &mut bypassed,
                                    tr("master-chain-bypass"),
                                ))
                                .changed()
                            {
                                out.set_bypass = Some((inst.id, bypassed));
                            }
                        });
                    });
                });
            }
        });

    out
}
