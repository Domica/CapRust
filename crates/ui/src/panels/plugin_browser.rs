//! CLAP plugin browser (Faza I, commit 4a).
//!
//! Read-only list of `.clap` binaries found in the two standard
//! locations. Scans lazily on first render; refresh button rescans.

use caprust_media_io::clap_host::{default_scan_dirs, scan_dirs, PluginInfo};
use egui::{Color32, RichText, Ui};
use egui_phosphor::regular as ph;
use std::path::PathBuf;

use crate::i18n_helper::tr;

#[derive(Debug, Default)]
pub struct PluginBrowserState {
    pub plugins: Vec<PluginInfo>,
    pub scanned: bool,
    pub scan_dirs: Vec<PathBuf>,
}

impl PluginBrowserState {
    pub fn rescan(&mut self) {
        if self.scan_dirs.is_empty() {
            self.scan_dirs = default_scan_dirs();
        }
        self.plugins = scan_dirs(&self.scan_dirs);
        self.scanned = true;
        tracing::info!(
            "CLAP browser: {} plugin(s) found across {} dir(s)",
            self.plugins.len(),
            self.scan_dirs.len(),
        );
    }
}

#[derive(Default)]
pub struct PluginBrowserOutput {
    /// User clicked the "+" on a plugin card; app.rs will add it to
    /// the master chain via AddMasterPluginCommand.
    pub add_requested: Option<PluginInfo>,
}

pub fn show(ui: &mut Ui, state: &mut PluginBrowserState) -> PluginBrowserOutput {
    let mut out = PluginBrowserOutput::default();
    if !state.scanned {
        state.rescan();
    }

    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("plugin-browser-heading")).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button(ph::ARROWS_CLOCKWISE)
                .on_hover_text(tr("plugin-browser-refresh"))
                .clicked()
            {
                state.rescan();
            }
        });
    });
    ui.separator();

    if state.plugins.is_empty() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(tr("plugin-browser-empty"))
                    .italics()
                    .color(Color32::from_gray(140)),
            );
            ui.add_space(8.0);
            for d in &state.scan_dirs {
                ui.label(
                    RichText::new(d.display().to_string())
                        .small()
                        .weak()
                        .monospace(),
                );
            }
        });
        return out;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for p in &state.plugins {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&p.name).strong());
                            ui.label(
                                RichText::new(format!("{} \u{2014} {}", p.vendor, p.version))
                                    .small()
                                    .weak(),
                            );
                            ui.label(RichText::new(p.id.as_str()).small().monospace().weak());
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            if ui
                                .button(ph::PLUS)
                                .on_hover_text(tr("plugin-browser-add"))
                                .clicked()
                            {
                                out.add_requested = Some(p.clone());
                            }
                        });
                    });
                });
            }
        });

    out
}
