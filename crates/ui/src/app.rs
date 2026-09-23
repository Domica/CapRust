use crate::theme::Theme;
use caprust_core::{AspectRatio, Clip, FrameRate, ProjectState, UndoStack};
use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppMode {
    StartScreen,
    Editor,
}

#[derive(Debug, Clone)]
pub struct NewProjectDraft {
    pub name: String,
    pub location: String,
    pub aspect_ratio: AspectRatio,
    pub base_resolution: u32,
    pub frame_rate: FrameRate,
}

impl Default for NewProjectDraft {
    fn default() -> Self {
        Self {
            name: "Untitled Project".into(),
            location: default_projects_dir(),
            aspect_ratio: AspectRatio::Portrait9x16,
            base_resolution: 1080,
            frame_rate: FrameRate::FPS30,
        }
    }
}

fn default_projects_dir() -> String {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(|h| format!("{h}/CapRust"))
        .unwrap_or_else(|_| ".".into())
}

pub struct CapRustApp {
    pub mode: AppMode,
    pub project: ProjectState,
    pub undo_stack: UndoStack,
    pub draft: NewProjectDraft,
    pub theme: Theme,
    pub settings_open: bool,
}

impl CapRustApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let theme = cc
            .storage
            .and_then(|s| s.get_string("theme"))
            .and_then(|s| serde_json::from_str::<Theme>(&s).ok())
            .unwrap_or_default();
        Self {
            mode: AppMode::StartScreen,
            project: ProjectState::default(),
            undo_stack: UndoStack::new(),
            draft: NewProjectDraft::default(),
            theme,
            settings_open: false,
        }
    }

    fn create_project(&mut self) {
        let (w, h) = self
            .draft
            .aspect_ratio
            .dimensions(self.draft.base_resolution);
        tracing::info!(
            "Create project '{}' at {} ({}x{} @ {})",
            self.draft.name,
            self.draft.location,
            w,
            h,
            self.draft.frame_rate.label()
        );
        self.project = ProjectState {
            name: self.draft.name.clone(),
            aspect_ratio: self.draft.aspect_ratio.clone(),
            base_resolution: self.draft.base_resolution,
            frame_rate: self.draft.frame_rate,
            project_path: Some(self.draft.location.clone()),
            ..ProjectState::default()
        };
        self.undo_stack = UndoStack::new();
        self.mode = AppMode::Editor;
    }

    fn show_start_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.heading(egui::RichText::new("🎬 CapRust").size(34.0));
                ui.add_space(4.0);
                ui.label("Social-first video editor");
                ui.add_space(30.0);
            });

            ui.vertical_centered(|ui| {
                egui::Frame::group(ui.style())
                    .inner_margin(24.0)
                    .show(ui, |ui| {
                        ui.set_width(480.0);
                        ui.heading("New Project");
                        ui.separator();

                        egui::Grid::new("new_project_grid")
                            .num_columns(2)
                            .spacing([12.0, 10.0])
                            .show(ui, |ui| {
                                ui.label("Name");
                                ui.text_edit_singleline(&mut self.draft.name);
                                ui.end_row();

                                ui.label("Location");
                                ui.horizontal(|ui| {
                                    ui.text_edit_singleline(&mut self.draft.location);
                                    if ui.button("Browse…").clicked() {
                                        // TODO PR 12: rfd::FileDialog
                                    }
                                });
                                ui.end_row();

                                ui.label("Format");
                                egui::ComboBox::from_id_salt("draft_aspect")
                                    .selected_text(self.draft.aspect_ratio.label())
                                    .show_ui(ui, |ui| {
                                        for preset in AspectRatio::presets() {
                                            ui.selectable_value(
                                                &mut self.draft.aspect_ratio,
                                                preset.clone(),
                                                preset.label(),
                                            );
                                        }
                                    });
                                ui.end_row();

                                ui.label("Base resolution");
                                ui.add(
                                    egui::Slider::new(&mut self.draft.base_resolution, 480..=2160)
                                        .suffix(" px"),
                                );
                                ui.end_row();

                                ui.label("Frame rate");
                                egui::ComboBox::from_id_salt("draft_fps")
                                    .selected_text(self.draft.frame_rate.label())
                                    .show_ui(ui, |ui| {
                                        for fps in FrameRate::all() {
                                            ui.selectable_value(
                                                &mut self.draft.frame_rate,
                                                fps,
                                                fps.label(),
                                            );
                                        }
                                    });
                                ui.end_row();
                            });

                        ui.add_space(12.0);
                        let (w, h) = self
                            .draft
                            .aspect_ratio
                            .dimensions(self.draft.base_resolution);
                        ui.label(format!(
                            "Output: {w} × {h} @ {}",
                            self.draft.frame_rate.label()
                        ));

                        ui.add_space(16.0);
                        ui.horizontal(|ui| {
                            if ui.button("Create Project").clicked() {
                                self.create_project();
                            }
                            if ui.button("Quit").clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        });
                    });
            });
        });
    }

    fn show_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New Project…").clicked() {
                        self.mode = AppMode::StartScreen;
                        ui.close_menu();
                    }
                    if ui.button("Open Project…").clicked() {
                        // TODO PR 12
                        ui.close_menu();
                    }
                    if ui.button("Save Project").clicked() {
                        // TODO PR 12
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Import Media…").clicked() {
                        // TODO PR 12
                        ui.close_menu();
                    }
                    if ui.button("Export…").clicked() {
                        // TODO PR 12
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Clear Project Cache").clicked() {
                        if let Some(path) = self.project.project_path.clone() {
                            match caprust_core::cache::clear_cache(std::path::Path::new(&path)) {
                                Ok(n) => tracing::info!("Cleared {n} cached files"),
                                Err(e) => tracing::error!("clear_cache failed: {e}"),
                            }
                        }
                        ui.close_menu();
                    }
                    if ui.button("Settings…").clicked() {
                        self.settings_open = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Close Project").clicked() {
                        self.mode = AppMode::StartScreen;
                        ui.close_menu();
                    }
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("Edit", |ui| {
                    let can_undo = self.undo_stack.can_undo();
                    if ui
                        .add_enabled(can_undo, egui::Button::new("Undo"))
                        .clicked()
                    {
                        let _ = self.undo_stack.undo(&mut self.project);
                        ui.close_menu();
                    }
                    let can_redo = self.undo_stack.can_redo();
                    if ui
                        .add_enabled(can_redo, egui::Button::new("Redo"))
                        .clicked()
                    {
                        let _ = self.undo_stack.redo(&mut self.project);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Split at Playhead").clicked() {
                        // TODO PR 7
                        ui.close_menu();
                    }
                    if ui.button("Delete Clip").clicked() {
                        // TODO PR 7
                        ui.close_menu();
                    }
                    if ui.button("Ripple Delete").clicked() {
                        // TODO PR 7
                        ui.close_menu();
                    }
                });

                ui.menu_button("View", |ui| {
                    if ui.button("Zoom In").clicked() {
                        // TODO PR 6
                        ui.close_menu();
                    }
                    if ui.button("Zoom Out").clicked() {
                        // TODO PR 6
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.menu_button("Sort Media by", |ui| {
                        ui.label("(wired in PR 6)");
                    });
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(&self.project.name).strong());
                });
            });
        });
    }

    fn show_editor(&mut self, ctx: &egui::Context) {
        self.show_menu_bar(ctx);

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
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
                    "Clips: {} | Ratio: {} | {}",
                    self.project.clips.len(),
                    self.project.aspect_ratio.label(),
                    self.project.frame_rate.label()
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

impl eframe::App for CapRustApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.theme.apply(ctx);

        match self.mode {
            AppMode::StartScreen => self.show_start_screen(ctx),
            AppMode::Editor => self.show_editor(ctx),
        }

        if self.settings_open {
            let mut open = true;
            egui::Window::new("Settings")
                .open(&mut open)
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| {
                    crate::panels::settings_dialog::show(ui, &mut self.theme);
                });
            self.settings_open = open;
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Ok(json) = serde_json::to_string(&self.theme) {
            storage.set_string("theme", json);
        }
    }
}
