use crate::theme::apply_capcut_theme;
use caprust_core::{Clip, ProjectState, UndoStack};
use eframe::egui;

pub struct CapRustApp {
    pub project: ProjectState,
    pub undo_stack: UndoStack,
}

impl CapRustApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            project: ProjectState::default(),
            undo_stack: UndoStack::new(),
        }
    }
}

impl eframe::App for CapRustApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        apply_capcut_theme(ctx);

        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("🎬 CapRust");
                ui.separator();
                if ui.button("➕ Add Text").clicked() {
                    let clip = Clip::new_text("Hello!", 0, 0, 3000, false);
                    let cmd = caprust_core::commands::ripple::RippleInsertCommand::new(clip);
                    let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                }
                if ui
                    .add_enabled(self.undo_stack.can_undo(), egui::Button::new("↩ Undo"))
                    .clicked()
                {
                    let _ = self.undo_stack.undo(&mut self.project);
                }
                if ui
                    .add_enabled(self.undo_stack.can_redo(), egui::Button::new("↪ Redo"))
                    .clicked()
                {
                    let _ = self.undo_stack.redo(&mut self.project);
                }
                ui.separator();
                ui.label(format!(
                    "Clips: {} | Ratio: {}",
                    self.project.clips.len(),
                    self.project.aspect_ratio.label()
                ));
            });
        });

        egui::SidePanel::left("left_panel")
            .resizable(true)
            .default_width(220.0)
            .show(ctx, |ui| {
                ui.heading("Media & Effects");
                ui.separator();
                crate::panels::media_bin::show(ui, &self.project);
            });

        egui::SidePanel::right("right_panel")
            .resizable(true)
            .default_width(260.0)
            .show(ctx, |ui| {
                ui.heading("Properties");
                ui.separator();
                crate::panels::export_dialog::show(ui);
            });

        egui::TopBottomPanel::bottom("timeline")
            .resizable(true)
            .default_height(220.0)
            .show(ctx, |ui| {
                ui.heading("Timeline");
                ui.separator();
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for clip in self.project.clips.iter() {
                            let label = match &clip.clip_type {
                                caprust_core::ClipType::TextOverlay { content, .. } => {
                                    content.clone()
                                }
                                caprust_core::ClipType::Video { path, .. } => path.clone(),
                                _ => "clip".to_string(),
                            };
                            ui.label(format!("[{}: {}ms]", label, clip.duration_ms));
                        }
                    });
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                ui.heading("Video Preview");
                ui.label("(wgpu renderer – next PR)");
            });
        });
    }
}
