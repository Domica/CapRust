//! Multi-camera groups panel (Faza Q).
//!
//! Lists groups created from the clip context menu, offers one
//! button per angle (click = active angle for render), a Sync button
//! (audio-envelope alignment via media-io::multicam_sync), and a
//! Remove button per group. Read-only against ProjectState; all
//! mutations flow through MultiCamOutput and app.rs.

use caprust_core::ProjectState;
use egui::{RichText, Ui};
use std::sync::mpsc::Receiver;
use uuid::Uuid;

use crate::i18n_helper::tr;
use crate::widgets::button;
use crate::widgets::chip;
use crate::widgets::empty;

/// Events the sync job posts to the UI. Mirrors
/// caprust_media_io::multicam_sync::SyncEvent but stays local so the
/// panel does not depend on media-io.
pub use caprust_media_io::multicam_sync::SyncEvent;

#[derive(Default)]
pub struct MultiCamState {
    /// Receiver for an in-flight sync job. Some while a job runs.
    pub sync_rx: Option<Receiver<SyncEvent>>,
    /// Which group the in-flight job belongs to.
    pub sync_group: Option<Uuid>,
    /// Last progress reported by the job: (current, total).
    pub sync_progress: Option<(usize, usize)>,
}

impl MultiCamState {
    pub fn is_syncing(&self) -> bool {
        self.sync_rx.is_some()
    }
}

#[derive(Default)]
pub struct MultiCamOutput {
    /// (group_id, new active angle index).
    pub set_active_angle: Option<(Uuid, usize)>,
    /// Group id to remove.
    pub remove_group: Option<Uuid>,
    /// Group id whose angles should be synced via audio correlation.
    pub sync_requested: Option<Uuid>,
}

pub fn show(ui: &mut Ui, project: &ProjectState, state: &mut MultiCamState) -> MultiCamOutput {
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

    let syncing = state.is_syncing();

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
                            if button::secondary(ui, tr("multicam-remove")).clicked() {
                                out.remove_group = Some(g.id);
                            }
                            let sync_enabled = !syncing;
                            let sync_btn = ui
                                .add_enabled(sync_enabled, egui::Button::new(tr("multicam-sync")));
                            if sync_btn.clicked() {
                                out.sync_requested = Some(g.id);
                            }
                        });
                    });
                    ui.horizontal_wrapped(|ui| {
                        for (i, _clip_id) in g.angle_clip_ids.iter().enumerate() {
                            let label = format!("{} {}", tr("multicam-angle"), i + 1);
                            let is_active = i == g.active_angle;
                            if chip::chip(ui, label, is_active).clicked() && !is_active {
                                out.set_active_angle = Some((g.id, i));
                            }
                        }
                    });
                    // Sync status block for the group currently being synced.
                    if state.sync_group == Some(g.id) {
                        if let Some((cur, total)) = state.sync_progress {
                            ui.label(
                                RichText::new(format!(
                                    "{} {cur}/{total}",
                                    tr("multicam-sync-progress")
                                ))
                                .small()
                                .weak(),
                            );
                        } else {
                            ui.label(RichText::new(tr("multicam-sync-progress")).small().weak());
                        }
                    }
                });
            }
        });

    out
}
