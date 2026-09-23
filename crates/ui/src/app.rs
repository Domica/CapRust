use crate::panels::export_window::ExportState;
use crate::panels::media_bin::{MediaBinState, PreviewSize};
use crate::panels::preview_window::{PreviewEvents, PreviewState};
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
    pub preview_size: PreviewSize,
    pub timeline_tools: TimelineToolState,
    pub playhead_ms: u64,
    pub timeline_zoom: f32,
    pub settings_tab: crate::panels::settings_dialog::SettingsTab,
    pub preview: PreviewState,
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
            preview_size: PreviewSize::Medium,
            timeline_tools: TimelineToolState::default(),
            playhead_ms: 0,
            timeline_zoom: 1.0,
            settings_tab: Default::default(),
            preview: PreviewState::default(),
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

    fn total_duration_ms(&self) -> u64 {
        self.project
            .clips
            .iter()
            .map(|c| c.start_time_ms + c.duration_ms)
            .max()
            .unwrap_or(0)
    }

    // ---------------------------------------------------------------
    // Start screen
    // ---------------------------------------------------------------
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

    // ---------------------------------------------------------------
    // Menu bar
    // ---------------------------------------------------------------
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

    // ---------------------------------------------------------------
    // Toolbar
    // ---------------------------------------------------------------
    fn show_toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("➕ Add Text").clicked() {
                    let clip = Clip::new_text("Hello!", 0, self.playhead_ms, 3000, false);
                    let cmd = caprust_core::commands::ripple::RippleInsertCommand::new(clip);
                    let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                }
                ui.separator();
                ui.label(format!(
                    "Clips: {} | Media: {} | Tracks: {} | {} | {}",
                    self.project.clips.len(),
                    self.project.media.items.len(),
                    self.project.tracks.len(),
                    self.project.aspect_ratio.label(),
                    self.project.frame_rate.label()
                ));
            });
        });
    }

    // ---------------------------------------------------------------
    // Timeline events
    // ---------------------------------------------------------------
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
            let idx = self.project.tracks.len() + 1;
            let kind = caprust_core::TrackKind::Video;
            self.project
                .tracks
                .push(caprust_core::Track::new(&format!("V{}", idx), kind));
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

    fn handle_preview_events(&mut self, ev: PreviewEvents, total_ms: u64) {
        if ev.seek_back_30 {
            self.playhead_ms = self.playhead_ms.saturating_sub(30_000);
        }
        if ev.seek_back_5 {
            self.playhead_ms = self.playhead_ms.saturating_sub(5_000);
        }
        if ev.seek_fwd_5 {
            self.playhead_ms = (self.playhead_ms + 5_000).min(total_ms);
        }
        if ev.seek_fwd_30 {
            self.playhead_ms = (self.playhead_ms + 30_000).min(total_ms);
        }
        if ev.toggle_play {
            self.preview.playing = !self.preview.playing;
        }
        if ev.toggle_loop {
            self.preview.loop_playback = !self.preview.loop_playback;
        }
    }

    // ---------------------------------------------------------------
    // Timeline panel
    // ---------------------------------------------------------------
    fn show_timeline(&mut self, ctx: &egui::Context) {
        let screen_h = ctx.screen_rect().height();
        egui::TopBottomPanel::bottom("timeline")
            .resizable(true)
            .default_height(240.0)
            .min_height(150.0)
            .max_height(screen_h * 0.75)
            .show(ctx, |ui| {
                ui.set_min_height(150.0);

                // Tick fake model downloads
                self.project.models.tick_downloads(1.0 / 60.0);

                // --- Toolbar ---
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

                // --- Ruler + lanes ---
                let total_ms = self.total_duration_ms();
                let content_ms = (total_ms + 20_000).max(30_000);
                let px_per_ms =
                    (ui.available_width() * 0.85 * self.timeline_zoom) / content_ms as f32;
                let px_per_ms = px_per_ms.max(0.002);
                let content_width = (content_ms as f32 * px_per_ms).max(ui.available_width());

                let header_w = crate::timeline::track_header::HEADER_WIDTH;
                let ruler_h = crate::timeline::ruler::RULER_HEIGHT;

                // Scroll wrapper
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // ---------------- Ruler row ----------------
                        ui.horizontal(|ui| {
                            // Spacer matching header width
                            ui.allocate_space(egui::vec2(header_w, ruler_h));

                            if let Some(ms) = crate::timeline::ruler::show(
                                ui,
                                content_width,
                                px_per_ms,
                                self.playhead_ms,
                                total_ms,
                            ) {
                                self.playhead_ms = ms.min(total_ms.max(1));
                            }
                        });

                        // ---------------- Track rows ----------------
                        // Build a helper index: which track_idx does each lane belong to?
                        let tracks_len = self.project.tracks.len();
                        if tracks_len == 0 {
                            ui.label("No tracks.");
                            return;
                        }

                        // Snapshot tracks for iteration (avoid borrow conflicts)
                        let mut updated_tracks: Vec<caprust_core::Track> =
                            self.project.tracks.clone();
                        let mut header_changed = false;

                        // Pinned lanes first (e.g. Overlay), then normal order.
                        let order = caprust_core::track::display_order(&updated_tracks);

                        // Dragged media id (if any) — used below after we render lanes
                        let mut pending_drop: Option<(uuid::Uuid, usize, u64)> = None;

                        for idx in order {
                            let mut track = updated_tracks[idx].clone();
                            let row_h = track.height;

                            // Reserve full row (header + lane). We draw manually so the lane
                            // is a drop target that knows its own bounds.
                            let row_total_w = header_w + content_width;
                            let (row_rect, _row_resp) = ui.allocate_exact_size(
                                egui::vec2(row_total_w, row_h),
                                egui::Sense::hover(),
                            );
                            let header_rect = egui::Rect::from_min_size(
                                row_rect.min,
                                egui::vec2(header_w, row_h),
                            );
                            let lane_rect = egui::Rect::from_min_size(
                                egui::pos2(row_rect.min.x + header_w, row_rect.min.y),
                                egui::vec2(content_width, row_h),
                            );

                            // --- Header ---
                            let mut header_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(header_rect)
                                    .layout(egui::Layout::top_down(egui::Align::Min)),
                            );
                            if crate::timeline::track_header::show(&mut header_ui, &mut track, idx)
                            {
                                header_changed = true;
                            }

                            // --- Lane bg ---
                            let lane_bg = if track.visible {
                                egui::Color32::from_gray(22)
                            } else {
                                egui::Color32::from_gray(16)
                            };
                            ui.painter().rect_filled(lane_rect, 0.0, lane_bg);

                            // Pinned marker: subtle green line on the left edge
                            if track.pinned {
                                ui.painter().line_segment(
                                    [
                                        egui::Pos2::new(lane_rect.left(), lane_rect.top() + 2.0),
                                        egui::Pos2::new(lane_rect.left(), lane_rect.bottom() - 2.0),
                                    ],
                                    egui::Stroke::new(
                                        3.0_f32,
                                        egui::Color32::from_rgb(80, 200, 120),
                                    ),
                                );
                            }
                            ui.painter().line_segment(
                                [
                                    egui::Pos2::new(lane_rect.left(), lane_rect.bottom() - 0.5),
                                    egui::Pos2::new(lane_rect.right(), lane_rect.bottom() - 0.5),
                                ],
                                egui::Stroke::new(1.0_f32, egui::Color32::from_gray(35)),
                            );

                            // --- Draw clips on this lane ---
                            for clip in self.project.clips.iter() {
                                if clip.track_index != idx {
                                    continue;
                                }
                                let x0 = lane_rect.left() + (clip.start_time_ms as f32) * px_per_ms;
                                let x1 = lane_rect.left()
                                    + ((clip.start_time_ms + clip.duration_ms) as f32) * px_per_ms;
                                let clip_rect = egui::Rect::from_min_max(
                                    egui::pos2(x0, lane_rect.top() + 3.0),
                                    egui::pos2(x1.max(x0 + 6.0), lane_rect.bottom() - 3.0),
                                );
                                let color = match &clip.clip_type {
                                    caprust_core::ClipType::Video { .. } => {
                                        egui::Color32::from_rgb(60, 110, 180)
                                    }
                                    caprust_core::ClipType::Audio { .. } => {
                                        egui::Color32::from_rgb(90, 60, 140)
                                    }
                                    caprust_core::ClipType::Image { .. } => {
                                        egui::Color32::from_rgb(60, 140, 110)
                                    }
                                    caprust_core::ClipType::TextOverlay { .. } => {
                                        egui::Color32::from_rgb(180, 130, 60)
                                    }
                                    caprust_core::ClipType::Captions { .. } => {
                                        egui::Color32::from_rgb(180, 80, 120)
                                    }
                                    caprust_core::ClipType::Narration { .. } => {
                                        egui::Color32::from_rgb(120, 100, 200)
                                    }
                                };
                                ui.painter().rect_filled(clip_rect, 4.0, color);
                                let label = match &clip.clip_type {
                                    caprust_core::ClipType::TextOverlay { content, .. } => {
                                        content.clone()
                                    }
                                    caprust_core::ClipType::Captions { .. } => "💬 Captions".into(),
                                    caprust_core::ClipType::Narration { .. } => {
                                        "🎙 Narration".into()
                                    }
                                    caprust_core::ClipType::Video { path, .. }
                                    | caprust_core::ClipType::Audio { path, .. }
                                    | caprust_core::ClipType::Image { path, .. } => {
                                        std::path::Path::new(path)
                                            .file_name()
                                            .map(|s| s.to_string_lossy().to_string())
                                            .unwrap_or_else(|| "clip".into())
                                    }
                                };
                                ui.painter().text(
                                    clip_rect.left_top() + egui::vec2(6.0, 4.0),
                                    egui::Align2::LEFT_TOP,
                                    label,
                                    egui::FontId::proportional(11.0),
                                    egui::Color32::WHITE,
                                );
                            }

                            // --- Lane as drop target ---
                            let lane_resp = ui.interact(
                                lane_rect,
                                egui::Id::new(("lane_drop", idx)),
                                egui::Sense::hover(),
                            );

                            if let Some(payload) = lane_resp.dnd_hover_payload::<uuid::Uuid>() {
                                // Highlight
                                ui.painter().rect_stroke(
                                    lane_rect.shrink(2.0),
                                    4.0,
                                    egui::Stroke::new(
                                        2.0_f32,
                                        egui::Color32::from_rgb(90, 160, 240),
                                    ),
                                    egui::StrokeKind::Inside,
                                );
                                let _ = payload;
                            }

                            if let Some(payload) = lane_resp.dnd_release_payload::<uuid::Uuid>() {
                                // Determine time from pointer x
                                let release_time_ms =
                                    if let Some(pos) = ui.ctx().pointer_interact_pos() {
                                        let rel_x = (pos.x - lane_rect.left()).max(0.0);
                                        (rel_x / px_per_ms) as u64
                                    } else {
                                        0
                                    };
                                pending_drop = Some((*payload, idx, release_time_ms));
                            }

                            if header_changed && updated_tracks.get(idx).is_some() {
                                updated_tracks[idx] = track;
                            }
                        }

                        if header_changed {
                            self.project.tracks = updated_tracks;
                        }

                        // Apply any pending drop
                        if let Some((media_id, track_idx, time_ms)) = pending_drop {
                            let media = self
                                .project
                                .media
                                .items
                                .iter()
                                .find(|m| m.id == media_id)
                                .cloned();
                            if let Some(item) = media {
                                use caprust_core::{Clip, MediaKind};
                                let dur = if item.duration_ms > 0 {
                                    item.duration_ms
                                } else {
                                    3000
                                };
                                let clip = match item.kind {
                                    MediaKind::Video => {
                                        Clip::new_video(&item.path, track_idx, time_ms, dur)
                                    }
                                    MediaKind::Audio => {
                                        Clip::new_audio(&item.path, track_idx, time_ms, dur)
                                    }
                                    MediaKind::Image => {
                                        Clip::new_image(&item.path, track_idx, time_ms, dur)
                                    }
                                };
                                let cmd =
                                    caprust_core::commands::ripple::RippleInsertCommand::new(clip);
                                let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                            }
                        }
                    });
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
                let _dragging =
                    crate::panels::media_bin::show(ui, &mut self.project, &mut self.media_bin);
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

        // Central preview (frame + transport)
        egui::CentralPanel::default().show(ctx, |ui| {
            let total_ms = self.total_duration_ms();

            // ---- Frame area ----
            let avail = ui.available_size();
            let frame_h = (avail.y - 60.0).max(120.0);
            let frame_rect_size = egui::vec2(avail.x, frame_h);
            let (rect, _) = ui.allocate_exact_size(frame_rect_size, egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, 6.0, egui::Color32::from_gray(12));
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "🎬 Preview",
                egui::FontId::proportional(22.0),
                egui::Color32::from_gray(90),
            );

            ui.add_space(6.0);

            // ---- Transport bar ----
            let ev = crate::panels::preview_window::show_transport(
                ui,
                &mut self.preview,
                self.playhead_ms,
                total_ms,
                &mut self.project.aspect_ratio,
            );
            self.handle_preview_events(ev, total_ms);

            // Auto-advance when playing
            if self.preview.playing && total_ms > 0 {
                let dt_ms = (ui.input(|i| i.stable_dt) * 1000.0) as u64;
                self.playhead_ms = self.playhead_ms.saturating_add(dt_ms.max(16));
                if self.playhead_ms >= total_ms {
                    if self.preview.loop_playback {
                        self.playhead_ms = 0;
                    } else {
                        self.playhead_ms = total_ms;
                        self.preview.playing = false;
                    }
                }
                ui.ctx().request_repaint();
            }
        });
    }

    fn show_export_window(&mut self, ctx: &egui::Context) {
        let total_ms = self.total_duration_ms();
        let mut open = self.export_open;
        egui::Window::new("⬆  Export video")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                if crate::panels::export_window::show(ui, &mut self.export_state, total_ms) {
                    tracing::info!(
                        "Export → dest={}, res={:?}, fps={:?}, codec={:?}, q={:?}",
                        self.export_state.destination,
                        self.export_state.resolution,
                        self.export_state.frame_rate,
                        self.export_state.codec,
                        self.export_state.quality,
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
