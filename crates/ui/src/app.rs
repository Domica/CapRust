use crate::panels::clip_properties::{PendingEdit, PropertiesState};
use crate::panels::export_window::ExportState;
use crate::panels::media_bin::{MediaBinState, PreviewSize};
use crate::panels::preview_window::{PreviewEvents, PreviewState};
use crate::theme::Theme;
use crate::timeline::{TimelineToolEvents, TimelineToolState};
use caprust_core::commands::delete_clip::DeleteClipCommand;
use caprust_core::commands::move_clip::MoveClipCommand;
use caprust_core::commands::split_clip::SplitClipCommand;
use caprust_core::recent::{RecentList, RecentProject};
use caprust_core::settings::AppSettings;
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
    pub selected_clips: Vec<uuid::Uuid>,
    pub clip_drag: Option<ClipDrag>,
    pub settings: AppSettings,
    pub ffmpeg_status: caprust_core::FfmpegStatus,
    pub last_dnd_payload: Option<uuid::Uuid>,
    pub recent: RecentList,
    pub last_pointer: Option<egui::Pos2>,
    /// Cached track row geometry from the last frame: (top_y, [(track_idx, height)]).
    pub timeline_row_layout: (f32, Vec<(usize, f32)>),
    pub properties: PropertiesState,
    pub model_prompt: Option<caprust_core::ModelKind>,
}

#[derive(Debug, Clone)]
pub struct ClipDrag {
    pub clip_id: uuid::Uuid,
    pub origin_ms: u64,
    pub current_ms: i64,
    pub track_index: usize,
    pub clip_duration_ms: u64,
    /// Where the pointer was when drag began (egui space).
    pub origin_ptr: egui::Pos2,
    /// Cumulative pointer delta since drag start.
    pub last_ptr: egui::Pos2,
    /// Which edge was grabbed, if trimming.
    pub trim_edge: Option<TrimEdge>,
    /// Original duration for the duration cap.
    pub source_duration_ms: u64,
    /// Original start (for trim left math).
    pub origin_duration_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrimEdge {
    Left,
    Right,
}

impl CapRustApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let theme = cc
            .storage
            .and_then(|s| s.get_string("theme"))
            .and_then(|s| serde_json::from_str::<Theme>(&s).ok())
            .unwrap_or_default();
        let settings: AppSettings = cc
            .storage
            .and_then(|s| s.get_string("settings"))
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let ffmpeg_status = caprust_core::detect_ffmpeg(&settings);
        let recent: RecentList = cc
            .storage
            .and_then(|s| s.get_string("recent"))
            .and_then(|s| serde_json::from_str(&s).ok())
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
            selected_clips: Vec::new(),
            clip_drag: None,
            settings,
            ffmpeg_status,
            last_dnd_payload: None,
            recent,
            last_pointer: None,
            timeline_row_layout: (0.0, Vec::new()),
            properties: PropertiesState::default(),
            model_prompt: None,
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
        // Do NOT persist yet — user must hit Save (Ctrl+S) first.
        // This avoids creating a bogus empty .caprust file on every Create.
    }

    fn project_file_path(&self) -> Option<std::path::PathBuf> {
        let folder = self.project.project_path.as_ref()?;
        Some(caprust_core::project_io::project_file_for(
            std::path::Path::new(folder),
            &self.project.name,
        ))
    }

    fn save_project_to_disk(&mut self) {
        let Some(path) = self.project_file_path() else {
            return;
        };
        match caprust_core::project_io::save_project(&self.project, &path) {
            Ok(()) => {
                let entry =
                    caprust_core::recent::entry_from(&self.project, &path.to_string_lossy());
                self.recent.push(entry);
                tracing::info!("Saved project to {}", path.display());
            }
            Err(e) => tracing::error!("Save failed: {e}"),
        }
    }

    fn load_project_from(&mut self, path: &str) {
        match caprust_core::project_io::load_project(std::path::Path::new(path)) {
            Ok(state) => {
                // Ensure project_path points to the folder containing the file
                let mut state = state;
                if let Some(parent) = std::path::Path::new(path).parent() {
                    state.project_path = Some(parent.to_string_lossy().to_string());
                }
                self.project = state;
                self.undo_stack = UndoStack::new();
                self.mode = AppMode::Editor;
                let entry = caprust_core::recent::entry_from(&self.project, path);
                self.recent.push(entry);
                tracing::info!("Loaded project {path}");
            }
            Err(e) => tracing::error!("Load failed: {e}"),
        }
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
        let mut load_path: Option<String> = None;
        let mut forget_path: Option<String> = None;
        let mut delete_from_disk: Option<String> = None;

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(((ui.available_height() - 500.0) / 2.0).max(24.0));
                ui.heading(egui::RichText::new("🎬 CapRust").size(34.0));
                ui.add_space(4.0);
                ui.label("Social-first video editor");
                ui.add_space(30.0);
            });
            // Two columns: left = New Project form, right = Recent
            ui.horizontal_top(|ui| {
            // Center the two columns horizontally.
            ui.add_space(((ui.available_width() - 884.0) / 2.0).max(12.0));
                // LEFT: New Project form (existing)
                ui.vertical(|ui| {
                    ui.set_min_width(520.0);
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
                                            if let Some(dir) = rfd::FileDialog::new().pick_folder()
                                            {
                                                self.draft.location =
                                                    dir.to_string_lossy().to_string();
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
                                        egui::Slider::new(
                                            &mut self.draft.base_resolution,
                                            480..=2160,
                                        )
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

                // RIGHT: Recent Projects
                ui.vertical(|ui| {
                    ui.set_min_width(320.0);
                    ui.set_max_width(360.0);
                    ui.heading("Recent Projects");
                    ui.separator();

                    if self.recent.items.is_empty() {
                        ui.label(
                            egui::RichText::new("No recent projects yet.")
                                .italics()
                                .color(egui::Color32::from_gray(120)),
                        );
                    } else {
                        let entries: Vec<RecentProject> = self.recent.items.clone();
                        egui::ScrollArea::vertical()
                            .max_height(360.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for rp in entries {
                                    let frame = egui::Frame::group(ui.style())
                                        .inner_margin(8.0)
                                        .fill(ui.visuals().faint_bg_color);
                                    frame.show(ui, |ui| {
                                        ui.set_width(320.0);
                                        ui.horizontal(|ui| {
                                            // Thumbnail placeholder
                                            let (rect, _) = ui.allocate_exact_size(
                                                egui::vec2(72.0, 54.0),
                                                egui::Sense::hover(),
                                            );
                                            ui.painter().rect_filled(
                                                rect,
                                                4.0,
                                                egui::Color32::from_gray(40),
                                            );
                                            ui.painter().text(
                                                rect.center(),
                                                egui::Align2::CENTER_CENTER,
                                                "🎬",
                                                egui::FontId::proportional(22.0),
                                                egui::Color32::from_gray(180),
                                            );

                                            ui.vertical(|ui| {
                                                ui.label(egui::RichText::new(&rp.name).strong());
                                                ui.label(
                                                    egui::RichText::new(format!(
                                                        "{} · {} clips · {}",
                                                        format_duration(rp.duration_ms),
                                                        rp.clip_count,
                                                        format_age(rp.last_opened),
                                                    ))
                                                    .small()
                                                    .color(egui::Color32::from_gray(150)),
                                                );
                                                ui.horizontal(|ui| {
                                                    if ui.small_button("Open").clicked() {
                                                        load_path = Some(rp.path.clone());
                                                    }
                                                    if ui.small_button("Forget").clicked() {
                                                        forget_path = Some(rp.path.clone());
                                                    }
                                                    if ui
                                                        .small_button("🗑")
                                                        .on_hover_text(
                                                            "Delete project file from disk",
                                                        )
                                                        .clicked()
                                                    {
                                                        delete_from_disk = Some(rp.path.clone());
                                                    }
                                                });
                                            });
                                        });
                                    });
                                    ui.add_space(4.0);
                                }
                            });
                    }
                });
            });
        });

        if let Some(p) = load_path {
            self.load_project_from(&p);
        }
        if let Some(p) = forget_path {
            self.recent.forget(&p);
        }
        if let Some(p) = delete_from_disk {
            let _ = std::fs::remove_file(&p);
            // Also remove its folder cache? leave for later.
            self.recent.forget(&p);
        }
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
                    if ui.button("Open Project…").clicked() {
                        if let Some(f) = rfd::FileDialog::new()
                            .add_filter("CapRust Project", &["caprust"])
                            .pick_file()
                        {
                            let p = f.to_string_lossy().to_string();
                            self.load_project_from(&p);
                        }
                        ui.close_menu();
                    }
                    if ui.button("Save Project").clicked() {
                        self.save_project_to_disk();
                        ui.close_menu();
                    }
                    if ui.button("Save As…").clicked() {
                        if let Some(f) = rfd::FileDialog::new()
                            .add_filter("CapRust Project", &["caprust"])
                            .set_file_name(format!("{}.caprust", self.project.name))
                            .save_file()
                        {
                            let mut p = f.clone();
                            if p.extension().is_none() {
                                p.set_extension("caprust");
                            }
                            if let Some(parent) = p.parent() {
                                self.project.project_path =
                                    Some(parent.to_string_lossy().to_string());
                            }
                            if let Some(stem) = p.file_stem() {
                                self.project.name = stem.to_string_lossy().to_string();
                            }
                            self.save_project_to_disk();
                        }
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
    /// Snap a candidate start time to nearby clip edges or the playhead.
    fn snap_ms(
        &self,
        clip_id: uuid::Uuid,
        _track_idx: usize,
        candidate_ms: i64,
        duration_ms: u64,
        px_per_ms: f32,
    ) -> i64 {
        if !self.timeline_tools.snapping || px_per_ms <= 0.0 {
            return candidate_ms.max(0);
        }
        let threshold_ms = (10.0_f32 / px_per_ms).max(1.0) as i64;
        let cand_start = candidate_ms.max(0);
        let cand_end = cand_start + duration_ms as i64;

        let mut best_start = cand_start;
        let mut best_dist = threshold_ms + 1;

        for c in &self.project.clips {
            if c.id == clip_id {
                continue;
            }
            let s = c.start_time_ms as i64;
            let e = (c.start_time_ms + c.duration_ms) as i64;
            for target in [s, e] {
                let d = (cand_start - target).abs();
                if d < best_dist {
                    best_dist = d;
                    best_start = target;
                }
                let d2 = (cand_end - target).abs();
                if d2 < best_dist {
                    best_dist = d2;
                    best_start = (target - duration_ms as i64).max(0);
                }
            }
        }
        let ph = self.playhead_ms as i64;
        let d = (cand_start - ph).abs();
        if d < best_dist {
            best_dist = d;
            best_start = ph;
        }
        let d2 = (cand_end - ph).abs();
        if d2 < best_dist {
            best_start = (ph - duration_ms as i64).max(0);
        }

        best_start.max(0)
    }

    /// Pack clips on each track end-to-end when the magnetic timeline is on.
    fn apply_magnetic(&mut self) {
        if !self.timeline_tools.magnetic {
            return;
        }
        let track_count = self.project.tracks.len();
        for t in 0..track_count {
            let mut indices: Vec<usize> = self
                .project
                .clips
                .iter()
                .enumerate()
                .filter(|(_, c)| c.track_index == t)
                .map(|(i, _)| i)
                .collect();
            if indices.is_empty() {
                continue;
            }
            indices.sort_by_key(|&i| self.project.clips[i].start_time_ms);
            let mut cursor: u64 = 0;
            for &i in &indices {
                self.project.clips[i].start_time_ms = cursor;
                cursor += self.project.clips[i].duration_ms;
            }
        }
    }

    /// Which track row contains the current pointer Y? Uses the timeline
    /// geometry cached during the last frame.
    fn track_for_y(&self, _fallback: usize) -> Option<usize> {
        let ptr = self.last_pointer?;
        let (top_y, rows) = self.timeline_row_layout.clone();
        let mut y = top_y;
        for (idx, h) in rows {
            if ptr.y >= y && ptr.y < y + h {
                return Some(idx);
            }
            y += h;
        }
        None
    }

    fn handle_timeline_events(&mut self, ev: TimelineToolEvents) {
        if ev.magnetic_toggled && self.timeline_tools.magnetic {
            self.apply_magnetic();
        }
        if ev.snapping_toggled && self.timeline_tools.snapping {
            // nothing to do on toggle; snap only matters during drag
        }
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
            let ready: Option<(String, String)> = self
                .project
                .models
                .ready_narration()
                .first()
                .map(|m| (m.id.clone(), m.language.clone()));
            if let Some((model_id, _lang)) = ready {
                let clip = caprust_core::Clip::new_narration(
                    0,
                    self.playhead_ms,
                    3000,
                    &model_id,
                    &model_id,
                    "Narration text goes here",
                );
                let cmd = caprust_core::commands::ripple::RippleInsertCommand::new(clip);
                let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
            } else {
                self.model_prompt = Some(caprust_core::ModelKind::Narration);
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
            .default_height(280.0)
            .min_height(180.0)
            .max_height(screen_h * 0.8)
            .show(ctx, |ui| {
                ui.set_min_height(180.0);

                self.project.models.tick_downloads(1.0 / 60.0);
                self.last_pointer = ctx.input(|i| i.pointer.hover_pos());

                // ---------------- Toolbar ----------------
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
                self.handle_timeline_events(ev);

                ui.separator();

                // ---------------- State prep ----------------
                let header_w = crate::timeline::track_header::HEADER_WIDTH;
                let ruler_h = crate::timeline::ruler::RULER_HEIGHT;

                let order = caprust_core::track::display_order(&self.project.tracks);
                let mut updated_tracks = self.project.tracks.clone();
                let mut header_changed = false;
                let mut pending_delete_track: Option<usize> = None;
                let mut pending_actions: Vec<ClipAction> = Vec::new();
                let mut pending_drop: Option<(uuid::Uuid, usize, u64)> = None;

                // DnD: payload type in egui 0.31 is Arc<T>, no downcast needed.
                let dnd_active: Option<uuid::Uuid> =
                    egui::DragAndDrop::payload::<uuid::Uuid>(ctx).map(|arc| *arc);
                if let Some(id) = dnd_active {
                    self.last_dnd_payload = Some(id);
                }
                let pointer_hover = ctx.input(|i| i.pointer.hover_pos());
                let pointer_released = ctx.input(|i| i.pointer.any_released());
                let pointer_down = ctx.input(|i| i.pointer.primary_down());
                let dnd_drop: Option<uuid::Uuid> = if pointer_released {
                    self.last_dnd_payload
                } else {
                    None
                };

                let clip_drag_snapshot = self.clip_drag.clone();

                // ---------------- Dimensions ----------------
                let total_ms = self.total_duration_ms();
                let content_ms = (total_ms + 20_000).max(30_000);
                let avail_w = ui.available_width();
                let lanes_w = (avail_w - header_w).max(120.0);
                let px_per_ms = (lanes_w * 0.90 * self.timeline_zoom) / content_ms as f32;
                let px_per_ms = px_per_ms.max(0.002);
                let content_width = (content_ms as f32 * px_per_ms).max(lanes_w);

                // Cache row layout so track_for_y() can map pointer Y to a track.
                {
                    let mut rows: Vec<(usize, f32)> = Vec::new();
                    for &idx in &order {
                        rows.push((idx, updated_tracks[idx].height));
                    }
                    // Approximate top Y as pointer-y origin; refined on first frame.
                    let top_y = ctx
                        .input(|i| i.pointer.hover_pos())
                        .map(|p| p.y - 40.0)
                        .unwrap_or(0.0);
                    if self.timeline_row_layout.1.is_empty()
                        || self.timeline_row_layout.1.len() != rows.len()
                    {
                        self.timeline_row_layout = (top_y, rows);
                    }
                }

                // ---------------- Two columns side by side ----------------
                // No ScrollAreas inside the timeline — panel is resizable.
                ui.horizontal_top(|ui| {
                    // === LEFT: header column ===
                    ui.allocate_ui_with_layout(
                        egui::vec2(header_w, ui.available_height()),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(header_w);

                            // ruler spacer
                            ui.allocate_space(egui::vec2(header_w, ruler_h));

                            for &idx in &order {
                                let mut track = updated_tracks[idx].clone();
                                let row_h = track.height;
                                ui.allocate_ui_with_layout(
                                    egui::vec2(header_w, row_h),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui.set_width(header_w);
                                        let hev = crate::timeline::track_header::show(
                                            ui, &mut track, idx,
                                        );
                                        if hev.changed {
                                            header_changed = true;
                                        }
                                        if hev.delete_requested {
                                            pending_delete_track = Some(idx);
                                        }
                                    },
                                );
                                updated_tracks[idx] = track;
                            }
                        },
                    );

                    // === RIGHT: ruler + lanes ===
                    let lanes_h = ui.available_height();
                    ui.allocate_ui_with_layout(
                        egui::vec2(content_width, lanes_h),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_min_width(content_width);

                            // Ruler
                            if let Some(ms) = crate::timeline::ruler::show(
                                ui,
                                content_width,
                                px_per_ms,
                                self.playhead_ms,
                                total_ms,
                            ) {
                                pending_actions.push(ClipAction::SetPlayhead(ms));
                            }

                            // Lanes
                            let mut top_y_opt: Option<f32> = None;
                            let mut rows_actual: Vec<(usize, f32)> = Vec::new();
                            for &idx in &order {
                                let track = &updated_tracks[idx];
                                let row_h = track.height;

                                let (lane_rect, _) = ui.allocate_exact_size(
                                    egui::vec2(content_width, row_h),
                                    egui::Sense::hover(),
                                );
                                if top_y_opt.is_none() {
                                    top_y_opt = Some(lane_rect.top());
                                }
                                rows_actual.push((idx, row_h));

                                // Background
                                let lane_bg = if track.visible {
                                    egui::Color32::from_gray(22)
                                } else {
                                    egui::Color32::from_gray(16)
                                };
                                ui.painter().rect_filled(lane_rect, 0.0, lane_bg);

                                // Pinned marker
                                if track.pinned {
                                    ui.painter().line_segment(
                                        [
                                            egui::Pos2::new(
                                                lane_rect.left(),
                                                lane_rect.top() + 2.0,
                                            ),
                                            egui::Pos2::new(
                                                lane_rect.left(),
                                                lane_rect.bottom() - 2.0,
                                            ),
                                        ],
                                        egui::Stroke::new(
                                            3.0_f32,
                                            egui::Color32::from_rgb(80, 200, 120),
                                        ),
                                    );
                                }

                                // Bottom separator
                                ui.painter().line_segment(
                                    [
                                        egui::Pos2::new(lane_rect.left(), lane_rect.bottom() - 0.5),
                                        egui::Pos2::new(
                                            lane_rect.right(),
                                            lane_rect.bottom() - 0.5,
                                        ),
                                    ],
                                    egui::Stroke::new(1.0_f32, egui::Color32::from_gray(35)),
                                );

                                // Clip rectangles
                                let clips_here: Vec<(
                                    uuid::Uuid,
                                    u64,
                                    u64,
                                    caprust_core::ClipType,
                                    bool,
                                )> = self
                                    .project
                                    .clips
                                    .iter()
                                    .filter(|c| {
                                        // If this clip is being dragged, show it on
                                        // the drag's target track instead of its
                                        // stored track.
                                        let drag_track = clip_drag_snapshot
                                            .as_ref()
                                            .filter(|d| d.clip_id == c.id)
                                            .map(|d| d.track_index);
                                        match drag_track {
                                            Some(t) => t == idx,
                                            None => c.track_index == idx,
                                        }
                                    })
                                    .map(|c| {
                                        let is_dragged = clip_drag_snapshot
                                            .as_ref()
                                            .map(|d| d.clip_id == c.id)
                                            .unwrap_or(false);
                                        let (start, dur) = if is_dragged {
                                            let d = clip_drag_snapshot.as_ref().unwrap();
                                            (d.current_ms.max(0) as u64, c.duration_ms)
                                        } else {
                                            (c.start_time_ms, c.duration_ms)
                                        };
                                        (c.id, start, dur, c.clip_type.clone(), is_dragged)
                                    })
                                    .collect();

                                for (clip_id, start_ms, dur_ms, ctype, is_dragged) in clips_here {
                                    let x0 = lane_rect.left() + (start_ms as f32) * px_per_ms;
                                    let x1 =
                                        lane_rect.left() + ((start_ms + dur_ms) as f32) * px_per_ms;
                                    let clip_rect = egui::Rect::from_min_max(
                                        egui::pos2(x0, lane_rect.top() + 3.0),
                                        egui::pos2(x1.max(x0 + 8.0), lane_rect.bottom() - 3.0),
                                    );

                                    let base_color = match &ctype {
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
                                    let color = if is_dragged {
                                        base_color.gamma_multiply(1.3)
                                    } else {
                                        base_color
                                    };
                                    ui.painter().rect_filled(clip_rect, 4.0, color);

                                    let selected = self.selected_clips.contains(&clip_id);
                                    if selected || is_dragged {
                                        ui.painter().rect_stroke(
                                            clip_rect,
                                            4.0,
                                            egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
                                            egui::StrokeKind::Inside,
                                        );
                                    }

                                    let label = match &ctype {
                                        caprust_core::ClipType::TextOverlay { content, .. } => {
                                            content.clone()
                                        }
                                        caprust_core::ClipType::Captions { .. } => {
                                            "💬 Captions".into()
                                        }
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

                                    // --- Trim handles ---
                                    const HANDLE_W: f32 = 6.0;
                                    if clip_rect.width() > HANDLE_W * 3.0 {
                                        let left_rect = egui::Rect::from_min_size(
                                            clip_rect.min,
                                            egui::vec2(HANDLE_W, clip_rect.height()),
                                        );
                                        let right_rect = egui::Rect::from_min_size(
                                            egui::pos2(clip_rect.max.x - HANDLE_W, clip_rect.min.y),
                                            egui::vec2(HANDLE_W, clip_rect.height()),
                                        );
                                        // Draw subtle handle visual
                                        ui.painter().rect_filled(
                                            left_rect,
                                            2.0,
                                            egui::Color32::from_white_alpha(60),
                                        );
                                        ui.painter().rect_filled(
                                            right_rect,
                                            2.0,
                                            egui::Color32::from_white_alpha(60),
                                        );

                                        // Interact
                                        let l_resp = ui.interact(
                                            left_rect,
                                            egui::Id::new(("trim_l", clip_id)),
                                            egui::Sense::click(),
                                        );
                                        let r_resp = ui.interact(
                                            right_rect,
                                            egui::Id::new(("trim_r", clip_id)),
                                            egui::Sense::click(),
                                        );

                                        if (l_resp.hovered() || r_resp.hovered())
                                            && pointer_down
                                            && clip_drag_snapshot.is_none()
                                        {
                                            pending_actions.push(ClipAction::Select(clip_id));
                                            pending_actions.push(ClipAction::DragStart(
                                                clip_id, idx, start_ms,
                                            ));
                                            let edge = if l_resp.hovered() {
                                                Some(TrimEdge::Left)
                                            } else {
                                                Some(TrimEdge::Right)
                                            };
                                            pending_actions
                                                .push(ClipAction::SetTrimEdge(clip_id, edge));
                                        }
                                    }

                                    let resp = ui.interact(
                                        clip_rect,
                                        egui::Id::new(("clip", clip_id)),
                                        egui::Sense::click(),
                                    );

                                    if resp.clicked() {
                                        pending_actions.push(ClipAction::Select(clip_id));
                                    }

                                    // Manual drag detection. Uses explicit
                                    // rect-contains-pointer + primary-down, since
                                    // resp.hovered() is unreliable while holding
                                    // the button inside a ScrollArea.
                                    let pointer_on_clip = ui.rect_contains_pointer(clip_rect);
                                    if pointer_on_clip
                                        && pointer_down
                                        && clip_drag_snapshot.is_none()
                                    {
                                        pending_actions.push(ClipAction::Select(clip_id));
                                        pending_actions
                                            .push(ClipAction::DragStart(clip_id, idx, start_ms));
                                    }

                                    resp.context_menu(|ui| {
                                        let del_lbl = if self.settings.enable_shortcuts {
                                            "Delete  (Del)"
                                        } else {
                                            "Delete"
                                        };
                                        if ui.button(del_lbl).clicked() {
                                            pending_actions.push(ClipAction::Delete(clip_id));
                                            ui.close_menu();
                                        }
                                        let split_lbl = if self.settings.enable_shortcuts {
                                            "Split at playhead  (S)"
                                        } else {
                                            "Split at playhead"
                                        };
                                        if ui.button(split_lbl).clicked() {
                                            pending_actions
                                                .push(ClipAction::Split(clip_id, self.playhead_ms));
                                            ui.close_menu();
                                        }
                                        ui.separator();
                                        if ui.button("Reverse  (R)").clicked() {
                                            pending_actions
                                                .push(ClipAction::ToggleReverse(clip_id));
                                            ui.close_menu();
                                        }
                                        if ui.button("Mirror horizontally  (H)").clicked() {
                                            pending_actions.push(ClipAction::ToggleFlipH(clip_id));
                                            ui.close_menu();
                                        }
                                        if ui.button("Mirror vertically  (V)").clicked() {
                                            pending_actions.push(ClipAction::ToggleFlipV(clip_id));
                                            ui.close_menu();
                                        }
                                    });
                                }

                                // --- DROP TARGET ---
                                let hovering = pointer_hover
                                    .map(|p| lane_rect.contains(p))
                                    .unwrap_or(false);

                                if dnd_active.is_some() && hovering {
                                    ui.painter().rect_stroke(
                                        lane_rect.shrink(2.0),
                                        4.0,
                                        egui::Stroke::new(
                                            2.0_f32,
                                            egui::Color32::from_rgb(90, 160, 240),
                                        ),
                                        egui::StrokeKind::Inside,
                                    );
                                }

                                if let (Some(id), Some(ptr)) = (dnd_drop, pointer_hover) {
                                    if lane_rect.contains(ptr) {
                                        let rel_x = (ptr.x - lane_rect.left()).max(0.0);
                                        let t_ms = (rel_x / px_per_ms) as u64;
                                        pending_drop = Some((id, idx, t_ms));
                                    }
                                }
                            }

                            // --- Drag continuation ---
                            if let Some(d) = &clip_drag_snapshot {
                                if let Some(p) = pointer_hover {
                                    let dx_total = p.x - d.origin_ptr.x;
                                    pending_actions.push(ClipAction::DragDelta(
                                        d.clip_id, dx_total, px_per_ms,
                                    ));
                                }
                                if pointer_released {
                                    pending_actions.push(ClipAction::DragEnd(d.clip_id));
                                }
                            }

                            // Cache actual row geometry for track_for_y()
                            if let Some(top) = top_y_opt {
                                self.timeline_row_layout = (top, rows_actual);
                            }
                        },
                    );
                });

                // ---------------- Apply changes ----------------
                if header_changed {
                    self.project.tracks = updated_tracks;
                }

                if let Some(idx) = pending_delete_track {
                    if idx < self.project.tracks.len() {
                        let removed = self.project.tracks.remove(idx);
                        self.project.clips.retain(|c| c.track_index != idx);
                        for c in self.project.clips.iter_mut() {
                            if c.track_index > idx {
                                c.track_index -= 1;
                            }
                        }
                        tracing::info!("Deleted track {}", removed.name);
                    }
                }

                for a in pending_actions {
                    match a {
                        ClipAction::SetPlayhead(ms) => {
                            self.playhead_ms = ms.min(total_ms.max(1));
                        }
                        ClipAction::Select(id) => {
                            if ctx.input(|i| i.modifiers.ctrl || i.modifiers.command) {
                                if self.selected_clips.contains(&id) {
                                    self.selected_clips.retain(|&x| x != id);
                                } else {
                                    self.selected_clips.push(id);
                                }
                            } else {
                                self.selected_clips = vec![id];
                            }
                        }
                        ClipAction::SetTrimEdge(id, edge) => {
                            if let Some(d) = &mut self.clip_drag {
                                if d.clip_id == id {
                                    d.trim_edge = edge;
                                }
                            }
                        }
                        ClipAction::DragStart(id, track_idx, origin) => {
                            let (dur, src_dur) = self
                                .project
                                .clips
                                .iter()
                                .find(|c| c.id == id)
                                .map(|c| (c.duration_ms, c.source_duration_ms))
                                .unwrap_or((3000, 0));
                            let ptr = self.last_pointer.unwrap_or_else(|| egui::pos2(0.0, 0.0));
                            self.clip_drag = Some(ClipDrag {
                                clip_id: id,
                                origin_ms: origin,
                                current_ms: origin as i64,
                                track_index: track_idx,
                                clip_duration_ms: dur,
                                origin_ptr: ptr,
                                last_ptr: ptr,
                                trim_edge: None,
                                source_duration_ms: src_dur,
                                origin_duration_ms: dur,
                            });
                        }
                        ClipAction::DragDelta(id, dx_total, ppm) => {
                            if let Some(d) = self.clip_drag.clone() {
                                if d.clip_id == id && ppm > 0.0 {
                                    let candidate = d.origin_ms as i64 + (dx_total / ppm) as i64;
                                    let snapped = self.snap_ms(
                                        id,
                                        d.track_index,
                                        candidate.max(0),
                                        d.clip_duration_ms,
                                        ppm,
                                    );
                                    // Change track based on Y position of pointer.
                                    let new_track = self.track_for_y(
                                        d.track_index, // fallback if unknown
                                    );
                                    if let Some(cur) = &mut self.clip_drag {
                                        cur.current_ms = snapped;
                                        if let Some(t) = new_track {
                                            cur.track_index = t;
                                        }
                                    }
                                }
                            }
                        }
                        ClipAction::DragEnd(id) => {
                            if let Some(d) = self.clip_drag.take() {
                                if d.clip_id == id {
                                    let new_ms = d.current_ms.max(0) as u64;
                                    if new_ms != d.origin_ms {
                                        let cmd = MoveClipCommand {
                                            clip_id: id,
                                            from_ms: d.origin_ms,
                                            to_ms: new_ms,
                                        };
                                        let _ = self
                                            .undo_stack
                                            .execute(Box::new(cmd), &mut self.project);
                                    }
                                }
                            }
                        }
                        ClipAction::Delete(id) => {
                            let cmd = DeleteClipCommand::new(id, false);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                            self.selected_clips.retain(|&x| x != id);
                        }
                        ClipAction::Split(id, at) => {
                            let cmd = SplitClipCommand::new(id, at);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        ClipAction::ToggleReverse(id) => {
                            if let Some(c) = self.project.clips.iter_mut().find(|c| c.id == id) {
                                c.reversed = !c.reversed;
                            }
                        }
                        ClipAction::ToggleFlipH(id) => {
                            if let Some(c) = self.project.clips.iter_mut().find(|c| c.id == id) {
                                c.flip_h = !c.flip_h;
                            }
                        }
                        ClipAction::ToggleFlipV(id) => {
                            if let Some(c) = self.project.clips.iter_mut().find(|c| c.id == id) {
                                c.flip_v = !c.flip_v;
                            }
                        }
                    }
                }

                // ---------------- Apply drop ----------------
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
                        let new_id = clip.id;
                        let cmd = caprust_core::commands::ripple::RippleInsertCommand::new(clip);
                        let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        self.selected_clips = vec![new_id];
                        // No auto-repack: the user chose where to drop.
                        tracing::info!(
                            "Dropped media {} onto track {} at {}ms",
                            media_id,
                            track_idx,
                            time_ms
                        );
                    }
                    self.last_dnd_payload = None;
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
                let _dragging =
                    crate::panels::media_bin::show(ui, &mut self.project, &mut self.media_bin);
            });

        egui::SidePanel::right("right_panel")
            .resizable(true)
            .default_width(280.0)
            .min_width(240.0)
            .show(ctx, |ui| {
                ui.heading("Properties");
                ui.separator();

                let selected = self.selected_clips.first().copied();
                crate::panels::clip_properties::show(
                    ui,
                    &self.project,
                    selected,
                    &mut self.properties,
                );

                // Consume any pending edits → commands
                if !self.properties.pending.is_empty() {
                    let edits = std::mem::take(&mut self.properties.pending);
                    if let Some(id) = selected {
                        let mut cmd = caprust_core::commands::set_clip::SetClipCommand::new(id);
                        for e in edits {
                            cmd = match e {
                                PendingEdit::Speed(v) => cmd.speed(v),
                                PendingEdit::Reverse(v) => cmd.reversed(v),
                                PendingEdit::FlipH(v) => cmd.flip_h(v),
                                PendingEdit::FlipV(v) => cmd.flip_v(v),
                                PendingEdit::VolumeDb(v) => cmd.volume_db(v),
                                PendingEdit::TrimStart(v) => cmd.start_time_ms(v),
                                PendingEdit::TrimDuration(v) => cmd.duration_ms(v),
                                PendingEdit::TrackIndex(v) => cmd.track_index(v),
                            };
                        }
                        let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                    }
                }
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
            .default_width(440.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_pos(ctx.screen_rect().center())
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
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

    fn show_model_prompt_window(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.model_prompt else {
            return;
        };

        // Advance fake downloads while this dialog is up.
        self.project.models.tick_downloads(1.0 / 60.0);

        let title = match kind {
            caprust_core::ModelKind::Caption => "💬  Captions — choose a model",
            caprust_core::ModelKind::Narration => "🎙  Narration — choose a voice",
        };

        let mut open = true;
        let mut chosen: Option<(String, String)> = None;
        let mut cancel = false;

        egui::Window::new(title)
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_width(560.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("This action needs a model. Download one, then click Use.")
                        .color(egui::Color32::from_gray(180)),
                );
                ui.add_space(6.0);
                ui.separator();

                let ids: Vec<String> = self
                    .project
                    .models
                    .models
                    .iter()
                    .filter(|m| m.kind == kind)
                    .map(|m| m.id.clone())
                    .collect();

                for id in ids {
                    let m = self
                        .project
                        .models
                        .models
                        .iter_mut()
                        .find(|m| m.id == id)
                        .unwrap();

                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(&m.name).strong());
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "· {} MB · {}",
                                            m.size_mb, m.language
                                        ))
                                        .small()
                                        .color(egui::Color32::from_gray(140)),
                                    );
                                });
                                ui.label(
                                    egui::RichText::new(&m.description)
                                        .small()
                                        .color(egui::Color32::from_gray(170)),
                                );
                            });

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| match m.status {
                                    caprust_core::ModelStatus::NotDownloaded => {
                                        if ui.button("⬇ Download").clicked() {
                                            m.status = caprust_core::ModelStatus::Downloading;
                                            m.progress = 0.0;
                                        }
                                    }
                                    caprust_core::ModelStatus::Downloading => {
                                        ui.add(
                                            egui::ProgressBar::new(m.progress)
                                                .desired_width(120.0)
                                                .show_percentage(),
                                        );
                                    }
                                    caprust_core::ModelStatus::Ready => {
                                        let btn = egui::Button::new(
                                            egui::RichText::new("✓ Use this")
                                                .color(egui::Color32::WHITE)
                                                .strong(),
                                        )
                                        .fill(egui::Color32::from_rgb(34, 139, 230));
                                        if ui.add(btn).clicked() {
                                            chosen = Some((m.id.clone(), m.language.clone()));
                                        }
                                    }
                                    caprust_core::ModelStatus::Error => {
                                        ui.label(
                                            egui::RichText::new("Error")
                                                .color(egui::Color32::from_rgb(230, 90, 90)),
                                        );
                                    }
                                },
                            );
                        });
                    });
                }

                ui.add_space(8.0);
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    ui.label(
                        egui::RichText::new("Downloads go to Settings → Paths → AI models folder.")
                            .small()
                            .color(egui::Color32::from_gray(140)),
                    );
                });
            });

        if cancel || !open {
            self.model_prompt = None;
            return;
        }

        if let Some((model_id, lang)) = chosen {
            let clip = match kind {
                caprust_core::ModelKind::Caption => {
                    caprust_core::Clip::new_captions(0, self.playhead_ms, 4000, &model_id, &lang)
                }
                caprust_core::ModelKind::Narration => caprust_core::Clip::new_narration(
                    0,
                    self.playhead_ms,
                    3000,
                    &model_id,
                    &model_id,
                    "Narration text goes here",
                ),
            };
            let cmd = caprust_core::commands::ripple::RippleInsertCommand::new(clip);
            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
            self.model_prompt = None;
        }
    }

    fn show_settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new("Settings")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_width(520.0)
            .show(ctx, |ui| {
                let ev = crate::panels::settings_dialog::show(
                    ui,
                    &mut self.theme,
                    &mut self.project.models,
                    &mut self.settings,
                    &mut self.settings_tab,
                    &mut self.ffmpeg_status,
                );
                if ev.save {
                    // Re-detect ffmpeg with new paths
                    self.ffmpeg_status = caprust_core::detect_ffmpeg(&self.settings);
                    // Trigger a save next frame (eframe persists via save())
                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                        egui::UserAttentionType::Informational,
                    ));
                    self.settings_open = false;
                }
                if ev.close {
                    self.settings_open = false;
                }
            });
        self.settings_open = open;
    }
}

fn format_duration(ms: u64) -> String {
    let s = ms / 1000;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

fn format_age(unix_secs: u64) -> String {
    let now = caprust_core::recent::now_unix();
    if now <= unix_secs {
        return "just now".into();
    }
    let d = now - unix_secs;
    if d < 60 {
        format!("{d}s ago")
    } else if d < 3600 {
        format!("{}m ago", d / 60)
    } else if d < 86_400 {
        format!("{}h ago", d / 3600)
    } else {
        format!("{}d ago", d / 86_400)
    }
}

#[derive(Debug)]
enum ClipAction {
    SetPlayhead(u64),
    Select(uuid::Uuid),
    DragStart(uuid::Uuid, usize, u64),
    SetTrimEdge(uuid::Uuid, Option<TrimEdge>),
    DragDelta(uuid::Uuid, f32, f32),
    DragEnd(uuid::Uuid),
    Delete(uuid::Uuid),
    Split(uuid::Uuid, u64),
    ToggleReverse(uuid::Uuid),
    ToggleFlipH(uuid::Uuid),
    ToggleFlipV(uuid::Uuid),
}

impl eframe::App for CapRustApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        caprust_i18n::set_current_lang(&self.settings.language);
        self.theme.apply(ctx);

        // Keyboard shortcuts (only in Editor + when enabled in Settings)
        if self.mode == AppMode::Editor && self.settings.enable_shortcuts {
            let events: Vec<egui::Key> = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| {
                        if let egui::Event::Key {
                            key, pressed: true, ..
                        } = e
                        {
                            Some(*key)
                        } else {
                            None
                        }
                    })
                    .collect()
            });

            let ctrl = ctx.input(|i| i.modifiers.ctrl || i.modifiers.command);

            for k in events {
                match k {
                    egui::Key::R => {
                        for id in self.selected_clips.clone() {
                            if let Some(c) = self.project.clips.iter_mut().find(|c| c.id == id) {
                                c.reversed = !c.reversed;
                            }
                        }
                    }
                    egui::Key::H => {
                        for id in self.selected_clips.clone() {
                            if let Some(c) = self.project.clips.iter_mut().find(|c| c.id == id) {
                                c.flip_h = !c.flip_h;
                            }
                        }
                    }
                    egui::Key::V => {
                        for id in self.selected_clips.clone() {
                            if let Some(c) = self.project.clips.iter_mut().find(|c| c.id == id) {
                                c.flip_v = !c.flip_v;
                            }
                        }
                    }
                    egui::Key::Delete | egui::Key::Backspace => {
                        for id in self.selected_clips.clone() {
                            let cmd = DeleteClipCommand::new(id, false);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        self.selected_clips.clear();
                    }
                    egui::Key::S if !ctrl => {
                        let at = self.playhead_ms;
                        for id in self.selected_clips.clone() {
                            let cmd = SplitClipCommand::new(id, at);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                    }
                    egui::Key::A if ctrl => {
                        self.selected_clips = self.project.clips.iter().map(|c| c.id).collect();
                    }
                    _ => {}
                }
            }
        }

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
        if self.model_prompt.is_some() {
            self.show_model_prompt_window(ctx);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Ok(json) = serde_json::to_string(&self.theme) {
            storage.set_string("theme", json);
        }
        if let Ok(json) = serde_json::to_string(&self.settings) {
            storage.set_string("settings", json);
        }
        if let Ok(json) = serde_json::to_string(&self.recent) {
            storage.set_string("recent", json);
        }
    }
}
