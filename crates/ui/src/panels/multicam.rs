//! Multi-camera groups panel (Faza Q).
//!
//! Lists groups created from the clip context menu, offers one
//! button per angle (click = active angle for render), and a
//! Remove button per group. Read-only against ProjectState; all
//! mutations flow through MultiCamOutput and app.rs.

use caprust_core::ProjectState;
use egui::{RichText, Ui};
use uuid::Uuid;

use crate::i18n_helper::tr;
use crate::widgets::empty;

#[derive(Default)]
pub struct MultiCamState {}

#[derive(Default)]
pub struct MultiCamOutput {
    /// (group_id, new active angle index).
    pub set_active_angle: Option<(Uuid, usize)>,
    /// Group id to remove.
    pub remove_group: Option<Uuid>,
}

pub fn show(ui: &mut Ui, project: &ProjectState, _state: &mut MultiCamState) -> MultiCamOutput {
    let mut out = MultiCamOutput::default();

    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("multicam-heading")).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(format!("{}", project.multicam_groups.len()))
                    .small()
                    .weak(),
            );
        });
    });
    ui.separator();

    if project.multicam_groups.is_empty() {
        empty::placeholder(ui, tr("multicam-empty"));
        return out;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for g in &project.multicam_groups {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&g.name).strong());
                            ui.label(
                                RichText::new(format!(
                                    "{} {}",
                                    g.angle_clip_ids.len(),
                                    tr("multicam-angles-suffix")
                                ))
                                .small()
                                .weak(),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(tr("multicam-remove")).clicked() {
                                out.remove_group = Some(g.id);
                            }
                        });
                    });
                    ui.horizontal_wrapped(|ui| {
                        for (i, _clip_id) in g.angle_clip_ids.iter().enumerate() {
                            let label = format!("{} {}", tr("multicam-angle"), i + 1);
                            let is_active = i == g.active_angle;
                            if ui.selectable_label(is_active, label).clicked() && !is_active {
                                out.set_active_angle = Some((g.id, i));
                            }
                        }
                    });
                });
            }
        });

    out
}
