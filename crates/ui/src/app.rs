use crate::panels::export_window::ExportState;
use crate::panels::media_bin::{MediaBinState, PreviewSize};
use crate::theme::Theme;
use crate::timeline::{TimelineToolEvents, TimelineToolState};
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
    pub export_open: bool,
    pub export_state: ExportState,
    pub media_bin: MediaBinState,
    /// Media item currently being dragged from the bin.
    pub dragging_media: Option<uuid::Uuid>,
    pub preview_size: PreviewSize,
    pub timeline_tools: TimelineToolState,
    pub playhead_ms: u64,
    pub timeline_zoom: f32,
    pub settings_tab: crate::panels::settings_dialog::SettingsTab,
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
            export_open: false,
            export_state: ExportState::default(),
            media_bin: MediaBinState::default(),
            dragging_media: None,
            preview_size: PreviewSize::Medium,
            timeline_tools: TimelineToolState::default(),
            playhead_ms: 0,
            timeline_zoom: 1.0,
            settings_tab: Default::default(),
        }
    }

    fn create_project(&mut self) {
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
                                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                            self.draft.location = dir.to_string_lossy().to_string();
                                        }
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
                    ui.separator();
                    if ui.button("Export…").clicked() {
                        self.export_open = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Clear Project Cache").clicked() {
                        if let Some(path) = self.project.project_path.clone() {
                            let _ = caprust_core::cache::clear_cache(std::path::Path::new(&path));
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
                        ui.close_menu();
                    }
                    if ui.button("Delete Clip").clicked() {
                        ui.close_menu();
                    }
                    if ui.button("Ripple Delete").clicked() {
                        ui.close_menu();
                    }
                });

                ui.menu_button("View", |ui| {
                    ui.menu_button("Sort Media by", |ui| {
                        for s in crate::panels::media_bin::MediaSort::all() {
                            let sel = self.media_bin.sort == s;
                            if ui.selectable_label(sel, s.label()).clicked() {
                                self.media_bin.sort = s;
                                ui.close_menu();
                            }
                        }
                    });
                    ui.menu_button("Preview Size", |ui| {
                        for sz in [PreviewSize::Small, PreviewSize::Medium, PreviewSize::Large] {
                            let sel = self.preview_size == sz;
                            if ui.selectable_label(sel, sz.label()).clicked() {
                                self.preview_size = sz;
                                self.media_bin.preview = sz;
                                ui.close_menu();
                            }
                        }
                    });
                });

                // Right side: Export button
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let export_btn = egui::Button::new(
                        egui::RichText::new("Export ⬆")
                            .color(egui::Color32::WHITE)
                            .strong(),
                    )
                    .fill(egui::Color32::from_rgb(34, 139, 230));
                    if ui.add(export_btn).clicked() {
                        self.export_open = true;
                    }
                    ui.separator();
                    ui.label(egui::RichText::new(&self.project.name).strong());
                });
            });
        });
    }

    fn show_toolbar(&mut self, ctx: &egui::Context) {
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
                    "Clips: {} | Media: {} | {} | {}",
                    self.project.clips.len(),
                    self.project.media.items.len(),
                    self.project.aspect_ratio.label(),
                    self.project.frame_rate.label()
                ));
            });
        });
    }

    fn handle_timeline_events(&mut self, ev: TimelineToolEvents) {
        if ev.undo {
            let _ = self.undo_stack.undo(&mut self.project);
        }
        if ev.redo {
            let _ = self.undo_stack.redo(&mut self.project);
        }
        if ev.zoom_in {
            self.timeline_zoom = (self.timeline_zoom * 1.25).min(8.0);
        }
        if ev.zoom_out {
            self.timeline_zoom = (self.timeline_zoom / 1.25).max(0.25);
        }
        if ev.zoom_fit {
            self.timeline_zoom = 1.0;
        }
        if ev.add_track {
            tracing::info!("Add track requested (track model in next PR)");
        }
        if ev.captions_clicked {
            let ready = self.project.models.ready_captions();
            if let Some(model) = ready.first() {
                let model_id = model.id.clone();
                let lang = model.language.clone();
                let clip =
                    caprust_core::Clip::new_captions(0, self.playhead_ms, 4000, &model_id, &lang);
                let cmd = caprust_core::commands::ripple::RippleInsertCommand::new(clip);
                let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
            } else {
                tracing::warn!("No caption model ready — download one in Settings → AI Models");
            }
        }
        if ev.narration_clicked {
            let ready = self.project.models.ready_narration();
            if let Some(model) = ready.first() {
                let model_id = model.id.clone();
                let voice_id = model.id.clone();
                let clip = caprust_core::Clip::new_narration(
                    0,
                    self.playhead_ms,
                    3000,
                    &model_id,
                    &voice_id,
                    "Narration text goes here",
                );
                let cmd = caprust_core::commands::ripple::RippleInsertCommand::new(clip);
                let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
            } else {
                tracing::warn!("No narration model ready — download one in Settings → AI Models");
            }
        }
    }

    fn show_timeline(&mut self, ctx: &egui::Context) {
        let screen_h = ctx.screen_rect().height();
        egui::TopBottomPanel::bottom("timeline")
            .resizable(true)
            .default_height(240.0)
            .min_height(160.0)
            .max_height(screen_h * 0.7)
            .show(ctx, |ui| {
                // Ensure content always fills minimum height, prevents
                // resize-snap-back when timeline is empty.
                ui.set_min_height(140.0);

                // tick downloads (fake progress for now)
                self.project.models.tick_downloads(1.0 / 60.0);

                // toolbar
                let can_undo = self.undo_stack.can_undo();
                let can_redo = self.undo_stack.can_redo();
                let mut tools = self.timeline_tools;
                let ev = crate::timeline::toolbar::show(
                    ui,
                    &mut tools,
                    can_undo,
                    can_redo,
                    self.playhead_ms,
                );
                self.timeline_tools = tools;
                ui.separator();
                self.handle_timeline_events(ev);
                ui.separator();

                // Drop zone
                let frame = egui::Frame::NONE
                    .inner_margin(4.0)
                    .fill(ui.visuals().extreme_bg_color);
                let (_id, dropped_payload) = ui.dnd_drop_zone::<uuid::Uuid, _>(frame, |ui| {
                    ui.set_min_height(ui.available_height().max(100.0));

                    if self.project.clips.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(30.0);
                            ui.label(
                                egui::RichText::new(
                                    "Drop media from the left bin, or use Import → Clips",
                                )
                                .color(egui::Color32::from_gray(120)),
                            );
                            ui.label(
                                egui::RichText::new("Nothing on the timeline yet.")
                                    .italics()
                                    .color(egui::Color32::from_gray(90)),
                            );
                        });
                    } else {
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
                    }
                });

                // Handle drop
                if let Some(payload_id) = dropped_payload {
                    let media = self
                        .project
                        .media
                        .items
                        .iter()
                        .find(|m| m.id == *payload_id)
                        .cloned();
                    if let Some(item) = media {
                        let clip = match item.kind {
                            caprust_core::MediaKind::Video => {
                                Clip::new_video(&item.path, 0, 0, item.duration_ms.max(2000))
                            }
                            caprust_core::MediaKind::Audio => {
                                Clip::new_video(&item.path, 0, 0, item.duration_ms.max(2000))
                            }
                            caprust_core::MediaKind::Image => {
                                Clip::new_video(&item.path, 0, 0, 3000)
                            }
                        };
                        let cmd = caprust_core::commands::ripple::RippleInsertCommand::new(clip);
                        let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                    }
                }
            });
    }

    fn show_editor(&mut self, ctx: &egui::Context) {
        self.show_menu_bar(ctx);
        self.show_toolbar(ctx);

        egui::SidePanel::left("left_panel")
            .resizable(true)
            .default_width(260.0)
            .min_width(200.0)
            .show(ctx, |ui| {
                ui.heading("Media Library");
                ui.separator();
                let dragging =
                    crate::panels::media_bin::show(ui, &mut self.project, &mut self.media_bin);
                if dragging.is_some() {
                    self.dragging_media = dragging;
                }
            });

        egui::SidePanel::right("right_panel")
            .resizable(true)
            .default_width(260.0)
            .show(ctx, |ui| {
                ui.heading("Properties");
                ui.separator();
                ui.label("Select a clip to edit its properties.");
            });

        self.show_timeline(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.4);
                ui.heading("🎬 Preview");
                ui.label("(wgpu renderer – upcoming PR)");
            });
        });
    }

    fn show_export_window(&mut self, ctx: &egui::Context) {
        let total_ms: u64 = self
            .project
            .clips
            .iter()
            .map(|c| c.start_time_ms + c.duration_ms)
            .max()
            .unwrap_or(0);

        let mut open = self.export_open;
        egui::Window::new("⬆  Export video")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                if crate::panels::export_window::show(
                    ui,
                    &mut self.export_state,
                    total_ms,
                ) {
                    tracing::info!(
                        "Export → dest={}, res={:?}, fps={:?}, codec={:?}, q={:?}, adv={}, mode={:?}, bitrate={}kbps, range={:?}",
                        self.export_state.destination,
                        self.export_state.resolution,
                        self.export_state.frame_rate,
                        self.export_state.codec,
                        self.export_state.quality,
                        self.export_state.advanced,
                        self.export_state.rate_mode,
                        self.export_state.bitrate_kbps,
                        self.export_state.color_range,
                    );
                    self.export_open = false;
                }
            });
        self.export_open = open;
    }

    fn show_settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new("Settings")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_width(520.0)
            .show(ctx, |ui| {
                crate::panels::settings_dialog::show(
                    ui,
                    &mut self.theme,
                    &mut self.project.models,
                    &mut self.settings_tab,
                );
            });
        self.settings_open = open;
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
            self.show_settings_window(ctx);
        }
        if self.export_open {
            self.show_export_window(ctx);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Ok(json) = serde_json::to_string(&self.theme) {
            storage.set_string("theme", json);
        }
    }
}
