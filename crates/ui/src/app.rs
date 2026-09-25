use crate::i18n_helper::tr;
use crate::media_jobs::JobRunner;
use crate::panels::asset_browser::AssetBrowserState;
use crate::panels::clip_properties::{PendingEdit, PropertiesState};
use crate::panels::export_window::ExportState;
use crate::panels::media_bin::{MediaBinState, PreviewSize};
use crate::panels::preview_window::{PreviewEvents, PreviewState};
use crate::preview_player::PreviewPlayer;
use crate::theme::Theme;
use crate::timeline::{TimelineToolEvents, TimelineToolState};
use caprust_core::commands::delete_clip::DeleteClipCommand;
use caprust_core::commands::move_clip::MoveClipCommand;
use caprust_core::commands::split_clip::SplitClipCommand;
use caprust_core::recent::{RecentList, RecentProject};
use caprust_core::settings::AppSettings;
use caprust_core::{AspectRatio, Clip, FrameRate, ProjectState, UndoStack};
use caprust_media_io::audio_player::AudioPlayer;
use caprust_media_io::exporter::ExportEvent;
use caprust_media_io::preview_render::PreviewRenderer;
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

/// Lightweight transient notification shown in the top-right corner.
/// Auto-dismisses after `duration`; the user can also dismiss early
/// via the ✕ button. Multiple toasts stack vertically.
pub struct Toast {
    pub text: String,
    pub created_at: std::time::Instant,
    pub duration: std::time::Duration,
}

impl Toast {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            created_at: std::time::Instant::now(),
            duration: std::time::Duration::from_secs(4),
        }
    }
}

/// Kinds of long-running jobs the jobs bar can display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Caption,
    Narration,
    Export,
}

/// One in-flight background job. `progress` < 0.0 means indeterminate
/// (no reliable signal from the producer).
#[derive(Debug, Clone)]
pub struct BackgroundJob {
    pub id: u64,
    pub kind: JobKind,
    pub label: String,
    pub progress: f32,
    pub started_at: std::time::Instant,
}

impl BackgroundJob {
    pub const INDETERMINATE: f32 = -1.0;
    pub fn is_indeterminate(&self) -> bool {
        self.progress < 0.0
    }
    pub fn elapsed(&self) -> std::time::Duration {
        self.started_at.elapsed()
    }
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
    /// Rubber-band selection in progress, if any.
    pub marquee: Option<MarqueeState>,
    pub settings: AppSettings,
    pub ffmpeg_status: caprust_core::FfmpegStatus,
    pub last_dnd_payload: Option<uuid::Uuid>,
    pub recent: RecentList,
    pub last_pointer: Option<egui::Pos2>,
    /// Cached track row geometry from the last frame: (top_y, [(track_idx, height)]).
    pub timeline_row_layout: (f32, Vec<(usize, f32)>),
    pub properties: PropertiesState,
    pub model_prompt: Option<caprust_core::ModelKind>,
    /// Receiver for an in-flight caption transcription. When Some, the
    /// timeline toolbar shows a "Transcribing…" label and the toolbar
    /// click is ignored until the job finishes.
    pub caption_rx:
        Option<std::sync::mpsc::Receiver<Result<crate::media_jobs::CaptionResult, String>>>,
    /// Wall-clock time the current caption job started, for the elapsed
    /// seconds indicator.
    pub caption_job_started: Option<std::time::Instant>,
    /// Receiver for an in-flight narration synthesis. Same lifecycle as
    /// caption_rx: Some while the job runs, None otherwise.
    pub narration_rx:
        Option<std::sync::mpsc::Receiver<Result<crate::media_jobs::NarrationResult, String>>>,
    /// Modal state for entering narration text.
    pub narration_input: crate::panels::narration_input::NarrationInputState,
    /// Result of the last update check, if a newer version was found.
    /// Some(..) => show the toast; None => nothing to notify.
    pub update_available: Option<caprust_core::update_checker::UpdateInfo>,
    /// Transient notifications shown top-right. Expired entries are
    /// pruned each frame; the user can dismiss early with the ✕ button.
    pub toasts: Vec<Toast>,
    /// In-flight background jobs (caption, narration, export). Rendered
    /// by show_jobs_bar above the timeline while any are live.
    pub jobs: Vec<BackgroundJob>,
    pub next_job_id: u64,
    /// Active job id per pipeline. Some while the corresponding job
    /// runs; None otherwise. Used to update/finish the matching
    /// BackgroundJob without a lookup by kind.
    pub caption_job_id: Option<u64>,
    /// Clipboard for Copy/Paste on the timeline. Holds a full clip
    /// snapshot; Paste assigns a new id and inserts at the playhead.
    pub clip_clipboard: Option<caprust_core::Clip>,
    /// Pending track rename: (track_index, edit_buffer). Some while the
    /// rename modal is open.
    pub track_rename: Option<(usize, String)>,
    /// Clips waiting to be transcribed, in order. Populated by
    /// "caption all in track"; drained sequentially because only one
    /// caption_rx slot exists at a time.
    pub caption_queue: std::collections::VecDeque<uuid::Uuid>,
    /// Total number of clips in the current batch. 0 or 1 means no
    /// batch is running and the job label stays generic.
    pub caption_batch_total: usize,
    /// Index of the currently running job within the batch (1-based).
    pub caption_batch_current: usize,
    pub narration_job_id: Option<u64>,
    pub export_job_id: Option<u64>,
    /// Receiver for the background update-check thread. Cleared after
    /// first successful receive.
    pub update_rx: Option<
        std::sync::mpsc::Receiver<Result<Option<caprust_core::update_checker::UpdateInfo>, String>>,
    >,
    pub timeline_scroll_x: f32,
    pub clip_textures: std::collections::HashMap<uuid::Uuid, egui::TextureHandle>,
    pub preview_player: PreviewPlayer,
    /// Audio playback for the current preview session. None = no audio.
    pub audio_player: Option<AudioPlayer>,
    /// True after we have re-anchored playback_started_at to the first
    /// consumed video frame of the current play session. Reset on every
    /// toggle_play(true). See comments at the anchor site.
    pub play_anchor_set: bool,
    /// AudioPlayer::playhead_ms() value at the moment the wall clock was
    /// re-anchored. Subtracted from ap.playhead_ms() in the playhead
    /// formula so that at re-anchor time playhead == wall. Without this
    /// the ~300-400 ms during which cpal was already running but we had
    /// not yet re-anchored remains baked into every subsequent sample,
    /// producing a stable constant offset between video and audio.
    pub audio_baseline_ms: u64,
    pub asset_browser: AssetBrowserState,
    pub export_in_progress: bool,
    pub export_rx: Option<std::sync::mpsc::Receiver<ExportEvent>>,
    pub export_progress: f32,
    pub export_finished_path: Option<String>,
    /// Clip currently being streamed in preview (None = no stream).
    pub preview_renderer: Option<PreviewRenderer>,
    pub last_frame_instant: Option<std::time::Instant>,
    pub last_streamed_clip: Option<uuid::Uuid>,
    /// Set to true when the user seeks; forces the preview stream to restart.
    pub stream_needs_restart: bool,
    /// When Some, preview should restart from here regardless of delta.
    pub explicit_seek_ms: Option<u64>,
    /// Wall-clock instant when playback started (for playhead derivation).
    pub playback_started_at: Option<std::time::Instant>,
    /// Playhead value (ms) at the moment playback started.
    pub playback_started_ms: u64,
    pub job_runner: JobRunner,
}

/// Rubber-band selection state. `start` and `current` are in screen
/// coordinates; the marquee becomes a rect between them and clips whose
/// rects intersect it on release are added to `selected_clips`.
#[derive(Debug, Clone, Copy)]
pub struct MarqueeState {
    pub start: egui::Pos2,
    pub current: egui::Pos2,
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

/// Spawn a background update-check thread. Returns None if the user
/// has disabled update checks in Settings; the receiver is polled by
/// `drain_update_check` during update ticks.
fn spawn_update_check(
    settings: &AppSettings,
) -> Option<
    std::sync::mpsc::Receiver<Result<Option<caprust_core::update_checker::UpdateInfo>, String>>,
> {
    if !settings.check_for_updates {
        return None;
    }
    let current = env!("CARGO_PKG_VERSION").to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("caprust-update-check".into())
        .spawn(move || {
            let res =
                caprust_core::update_checker::check(&current, true).map_err(|e| e.to_string());
            let _ = tx.send(res);
        })
        .ok()?;
    Some(rx)
}

impl CapRustApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_phosphor_fonts(&cc.egui_ctx);
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
        let update_rx = spawn_update_check(&settings);
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
            marquee: None,
            settings,
            ffmpeg_status: ffmpeg_status.clone(),
            last_dnd_payload: None,
            recent,
            last_pointer: None,
            timeline_row_layout: (0.0, Vec::new()),
            properties: PropertiesState::default(),
            model_prompt: None,
            caption_rx: None,
            caption_job_started: None,
            narration_rx: None,
            narration_input: Default::default(),
            update_available: None,
            toasts: Vec::new(),
            jobs: Vec::new(),
            next_job_id: 1,
            caption_job_id: None,
            clip_clipboard: None,
            track_rename: None,
            caption_queue: std::collections::VecDeque::new(),
            caption_batch_total: 0,
            caption_batch_current: 0,
            narration_job_id: None,
            export_job_id: None,
            update_rx,
            timeline_scroll_x: 0.0,
            clip_textures: std::collections::HashMap::new(),
            preview_player: PreviewPlayer::new(),
            audio_player: None,
            play_anchor_set: false,
            audio_baseline_ms: 0,
            asset_browser: AssetBrowserState::new(),
            export_in_progress: false,
            export_rx: None,
            export_progress: 0.0,
            export_finished_path: None,
            preview_renderer: None,
            last_frame_instant: None,
            last_streamed_clip: None,
            stream_needs_restart: false,
            explicit_seek_ms: None,
            playback_started_at: None,
            playback_started_ms: 0,
            job_runner: JobRunner::new(
                ffmpeg_status.ffmpeg.clone().map(std::path::PathBuf::from),
                ffmpeg_status.ffprobe.clone().map(std::path::PathBuf::from),
            ),
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
                // Prune empty Captions clips left behind by older builds
                // that inserted a placeholder whenever a model was picked
                // in the prompt.
                self.cleanup_phantom_captions();
                // Auto-regenerate thumbnails for older projects or after cache clear.
                self.regen_missing_thumbnails();
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
                            ui.heading(tr("new-title"));
                            ui.separator();
                            egui::Grid::new("new_project_grid")
                                .num_columns(2)
                                .spacing([12.0, 10.0])
                                .show(ui, |ui| {
                                    ui.label(tr("new-field-name"));
                                    ui.text_edit_singleline(&mut self.draft.name);
                                    ui.end_row();
                                    ui.label(tr("new-field-location"));
                                    ui.horizontal(|ui| {
                                        ui.text_edit_singleline(&mut self.draft.location);
                                        if ui.button(tr("new-button-browse")).clicked() {
                                            if let Some(dir) = rfd::FileDialog::new().pick_folder()
                                            {
                                                self.draft.location =
                                                    dir.to_string_lossy().to_string();
                                            }
                                        }
                                    });
                                    ui.end_row();
                                    ui.label(tr("new-field-format"));
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
                                    ui.label(tr("new-field-resolution"));
                                    ui.add(
                                        egui::Slider::new(
                                            &mut self.draft.base_resolution,
                                            480..=2160,
                                        )
                                        .suffix(" px"),
                                    );
                                    ui.end_row();
                                    ui.label(tr("new-field-fps"));
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
                                if ui.button(tr("new-button-create")).clicked() {
                                    self.create_project();
                                }
                                if ui.button(tr("menu-file-quit")).clicked() {
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                }
                            });
                        });
                });

                // RIGHT: Recent Projects
                ui.vertical(|ui| {
                    ui.set_min_width(320.0);
                    ui.set_max_width(360.0);
                    ui.heading(tr("new-recent-heading"));
                    ui.separator();

                    if self.recent.items.is_empty() {
                        ui.label(
                            egui::RichText::new(tr("new-recent-empty"))
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
                                                    if ui
                                                        .small_button(tr("new-recent-open"))
                                                        .clicked()
                                                    {
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
                ui.menu_button(tr("menu-file"), |ui| {
                    if ui.button(tr("menu-file-new")).clicked() {
                        self.mode = AppMode::StartScreen;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button(tr("menu-file-open")).clicked() {
                        if let Some(f) = rfd::FileDialog::new()
                            .add_filter("CapRust Project", &["caprust"])
                            .pick_file()
                        {
                            let p = f.to_string_lossy().to_string();
                            self.load_project_from(&p);
                        }
                        ui.close_menu();
                    }
                    if ui.button(tr("menu-file-save")).clicked() {
                        self.save_project_to_disk();
                        ui.close_menu();
                    }
                    if ui.button(tr("menu-file-save-as")).clicked() {
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
                    if ui.button(tr("menu-file-export")).clicked() {
                        self.export_open = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button(tr("menu-file-clear-cache")).clicked() {
                        if let Some(path) = self.project.project_path.clone() {
                            let _ = caprust_core::cache::clear_cache(std::path::Path::new(&path));
                            // Textures in memory must be dropped too.
                            self.clip_textures.clear();
                            self.media_bin.thumb_cache = Default::default();
                            // Re-generate in background.
                            self.regen_missing_thumbnails();
                        }
                        ui.close_menu();
                    }
                    if ui.button(tr("menu-file-regen-thumbs")).clicked() {
                        self.regen_missing_thumbnails();
                        ui.close_menu();
                    }
                    if ui.button(tr("menu-file-settings")).clicked() {
                        self.settings_open = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button(tr("menu-file-close")).clicked() {
                        self.mode = AppMode::StartScreen;
                        ui.close_menu();
                    }
                    if ui.button(tr("menu-file-quit")).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button(tr("menu-edit"), |ui| {
                    let can_undo = self.undo_stack.can_undo();
                    if ui
                        .add_enabled(can_undo, egui::Button::new(tr("menu-edit-undo")))
                        .clicked()
                    {
                        let _ = self.undo_stack.undo(&mut self.project);
                        ui.close_menu();
                    }
                    let can_redo = self.undo_stack.can_redo();
                    if ui
                        .add_enabled(can_redo, egui::Button::new(tr("menu-edit-redo")))
                        .clicked()
                    {
                        let _ = self.undo_stack.redo(&mut self.project);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button(tr("menu-edit-split")).clicked() {
                        ui.close_menu();
                    }
                    if ui.button(tr("menu-edit-delete")).clicked() {
                        ui.close_menu();
                    }
                    if ui.button(tr("menu-edit-ripple-delete")).clicked() {
                        ui.close_menu();
                    }
                });

                ui.menu_button(tr("menu-view"), |ui| {
                    ui.menu_button(tr("menu-view-sort"), |ui| {
                        for s in crate::panels::media_bin::MediaSort::all() {
                            let sel = self.media_bin.sort == s;
                            if ui.selectable_label(sel, s.label()).clicked() {
                                self.media_bin.sort = s;
                                ui.close_menu();
                            }
                        }
                    });
                    ui.menu_button(tr("menu-view-size"), |ui| {
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
                ui.separator();
                let (kb_txt, kb_col) = if self.settings.enable_shortcuts {
                    ("⌨ ON", egui::Color32::from_rgb(80, 200, 120))
                } else {
                    ("⌨ OFF", egui::Color32::from_rgb(220, 120, 80))
                };
                ui.label(egui::RichText::new(kb_txt).color(kb_col).strong());
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

    /// Magnetic pack. Video and Overlay tracks pack end-to-end (their
    /// own clips shoulder to shoulder). Audio, Captions and Text tracks
    /// do NOT pack on their own: they follow the video clip they were
    /// attached to, shifting by the same delta that video clip shifted.
    ///
    /// "Attached to" = maximum overlap in the original timeline. A clip
    /// with no overlapping video anywhere keeps its position (it sits
    /// in a gap that the video pack did not move).
    ///
    /// Only runs on toggle ON; drag/drop/delete do not auto-repack
    /// (see DIRECTIVES §7.4).
    fn apply_magnetic(&mut self) {
        use std::collections::HashMap;

        if !self.timeline_tools.magnetic {
            return;
        }

        // Snapshot: old start of every clip, keyed by id.
        let old_starts: HashMap<uuid::Uuid, u64> = self
            .project
            .clips
            .iter()
            .map(|c| (c.id, c.start_time_ms))
            .collect();

        // Which tracks pack on their own?
        let video_tracks: Vec<usize> = self
            .project
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                matches!(
                    t.kind,
                    caprust_core::TrackKind::Video | caprust_core::TrackKind::Overlay
                )
            })
            .map(|(i, _)| i)
            .collect();

        // Pack those, per track.
        for t in video_tracks {
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

        // Delta per video clip: new_start - old_start (signed).
        let deltas: HashMap<uuid::Uuid, i64> = self
            .project
            .clips
            .iter()
            .filter(|c| {
                self.project
                    .tracks
                    .get(c.track_index)
                    .map(|t| {
                        matches!(
                            t.kind,
                            caprust_core::TrackKind::Video | caprust_core::TrackKind::Overlay
                        )
                    })
                    .unwrap_or(false)
            })
            .map(|c| {
                let old = *old_starts.get(&c.id).unwrap_or(&c.start_time_ms) as i64;
                (c.id, c.start_time_ms as i64 - old)
            })
            .collect();

        // Build a (child -> parent video id) map by maximum overlap in
        // the ORIGINAL timeline.
        let video_clip_ids: Vec<uuid::Uuid> = self
            .project
            .clips
            .iter()
            .filter(|c| {
                self.project
                    .tracks
                    .get(c.track_index)
                    .map(|t| {
                        matches!(
                            t.kind,
                            caprust_core::TrackKind::Video | caprust_core::TrackKind::Overlay
                        )
                    })
                    .unwrap_or(false)
            })
            .map(|c| c.id)
            .collect();

        // Snapshot enough info about the follower clips and video clips
        // to compute overlap without holding a borrow.
        let follower_ids: Vec<uuid::Uuid> = self
            .project
            .clips
            .iter()
            .filter(|c| {
                self.project
                    .tracks
                    .get(c.track_index)
                    .map(|t| {
                        !matches!(
                            t.kind,
                            caprust_core::TrackKind::Video | caprust_core::TrackKind::Overlay
                        )
                    })
                    .unwrap_or(false)
            })
            .map(|c| c.id)
            .collect();

        // (id, old_start, old_end) for follower and video clips.
        let old_spans: HashMap<uuid::Uuid, (u64, u64)> = self
            .project
            .clips
            .iter()
            .map(|c| {
                let s = *old_starts.get(&c.id).unwrap_or(&c.start_time_ms);
                (c.id, (s, s + c.duration_ms))
            })
            .collect();

        let mut parent_of: HashMap<uuid::Uuid, uuid::Uuid> = HashMap::new();
        for child in &follower_ids {
            let Some(&(cs, ce)) = old_spans.get(child) else {
                continue;
            };
            let mut best: Option<(u64, uuid::Uuid)> = None;
            for v in &video_clip_ids {
                let Some(&(vs, ve)) = old_spans.get(v) else {
                    continue;
                };
                let ov_start = cs.max(vs);
                let ov_end = ce.min(ve);
                if ov_start < ov_end {
                    let ov = ov_end - ov_start;
                    if best.is_none_or(|(bo, _)| ov > bo) {
                        best = Some((ov, *v));
                    }
                }
            }
            if let Some((_, parent)) = best {
                parent_of.insert(*child, parent);
            }
        }

        // Apply the parent's delta to each follower.
        for c in self.project.clips.iter_mut() {
            let Some(parent) = parent_of.get(&c.id) else {
                continue;
            };
            let Some(&delta) = deltas.get(parent) else {
                continue;
            };
            let new_start = (c.start_time_ms as i64 + delta).max(0) as u64;
            c.start_time_ms = new_start;
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
                .push(caprust_core::Track::new(&format!("V{idx}"), kind));
        }
        if ev.captions_clicked {
            if self.caption_rx.is_some() {
                tracing::info!("caption job already in flight, ignoring click");
            } else {
                self.start_caption_job(None);
            }
        }
        if ev.download_models_clicked {
            // Pick the family with more missing entries so the first
            // screen the user sees is the one that needs attention.
            let models_dir = self.settings.effective_models_dir();
            self.project.models.scan_local(&models_dir);
            let captions_missing = self
                .project
                .models
                .models
                .iter()
                .filter(|m| m.kind == caprust_core::ModelKind::Caption)
                .filter(|m| m.status != caprust_core::ModelStatus::Ready)
                .count();
            let narration_missing = self
                .project
                .models
                .models
                .iter()
                .filter(|m| m.kind == caprust_core::ModelKind::Narration)
                .filter(|m| m.status != caprust_core::ModelStatus::Ready)
                .count();
            let kind = if narration_missing > captions_missing {
                caprust_core::ModelKind::Narration
            } else {
                caprust_core::ModelKind::Caption
            };
            tracing::info!(
                "models: download icon clicked (captions missing {captions_missing}, narration missing {narration_missing}) — opening {kind:?}"
            );
            self.model_prompt = Some(kind);
        }
        if ev.captions_all_clicked {
            let track_idx = self
                .selected_clips
                .first()
                .and_then(|id| self.project.clips.iter().find(|c| c.id == *id))
                .map(|c| c.track_index)
                .or_else(|| {
                    self.project
                        .tracks
                        .iter()
                        .position(|t| t.kind == caprust_core::TrackKind::Video)
                        .filter(|idx| self.project.clips.iter().any(|c| c.track_index == *idx))
                });
            match track_idx {
                Some(t) => self.start_caption_jobs_for_track(t),
                None => self.toast(tr("toast-caption-no-selection")),
            }
        }
        if ev.narration_clicked {
            // Refresh from disk so manually-placed Piper voices (and
            // ones added since startup) are seen without a restart.
            let models_dir = self.settings.effective_models_dir();
            self.project.models.scan_local(&models_dir);
            let any_ready = !self.project.models.ready_narration().is_empty();
            if any_ready {
                if self.narration_rx.is_some() {
                    tracing::info!("narration job already in flight, ignoring click");
                } else {
                    self.narration_input.open = true;
                }
            } else {
                self.model_prompt = Some(caprust_core::ModelKind::Narration);
            }
        }
    }

    /// Spawn a background Piper synthesis job. See `NarrationRequest`.
    fn start_narration_job(&mut self, voice_id: String, text: String) {
        let models_dir = self.settings.effective_models_dir();
        let voice_onnx_path = self.project.models.local_path(&models_dir, &voice_id);
        if !voice_onnx_path.is_file() {
            tracing::warn!(
                "narration: voice {} not on disk at {}",
                voice_id,
                voice_onnx_path.display()
            );
            self.model_prompt = Some(caprust_core::ModelKind::Narration);
            return;
        }

        let ffprobe = match self.ffmpeg_status.ffprobe.clone() {
            Some(p) => std::path::PathBuf::from(p),
            None => {
                tracing::warn!("narration: ffprobe unavailable");
                return;
            }
        };

        let req = crate::media_jobs::NarrationRequest {
            voice_id,
            voice_onnx_path,
            models_dir,
            text,
        };
        let rx = crate::media_jobs::spawn_narration_job(ffprobe, req);
        let job_id = self.begin_job(JobKind::Narration, tr("job-narration"));
        self.narration_job_id = Some(job_id);
        self.narration_rx = Some(rx);
        self.toast(tr("toast-narration-started"));
        tracing::info!("narration: job spawned");
    }

    fn drain_narration_job(&mut self) {
        let Some(rx) = self.narration_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(result)) => {
                let track_idx = self.ensure_audio_track();
                let clip = caprust_core::Clip::new_narration(
                    track_idx,
                    self.playhead_ms,
                    result.duration_ms.max(500),
                    &result.voice_id,
                    &result.voice_id,
                    &result.text,
                );
                let cmd = caprust_core::commands::ripple::RippleInsertCommand::new(clip);
                if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                    tracing::error!("narration: insert failed: {e}");
                } else {
                    tracing::info!(
                        "narration: inserted clip ({}ms) at {}",
                        result.duration_ms,
                        self.playhead_ms
                    );
                }
                if let Some(id) = self.narration_job_id.take() {
                    self.finish_job(id);
                }
                self.narration_rx = None;
            }
            Ok(Err(msg)) => {
                tracing::error!("narration: job failed: {msg}");
                self.toast(format!("{}: {msg}", tr("toast-narration-failed")));
                if let Some(id) = self.narration_job_id.take() {
                    self.finish_job(id);
                }
                self.narration_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                tracing::error!("narration: job thread vanished");
                if let Some(id) = self.narration_job_id.take() {
                    self.finish_job(id);
                }
                self.narration_rx = None;
            }
        }
    }

    fn ensure_audio_track(&mut self) -> usize {
        if let Some((i, _)) = self
            .project
            .tracks
            .iter()
            .enumerate()
            .find(|(_, t)| t.kind == caprust_core::TrackKind::Audio)
        {
            return i;
        }
        let idx = self.project.tracks.len() + 1;
        self.project.tracks.push(caprust_core::Track::new(
            &format!("A{idx}"),
            caprust_core::TrackKind::Audio,
        ));
        tracing::info!("narration: auto-created Audio track A{idx}");
        self.project.tracks.len() - 1
    }

    /// Return the index of the dedicated Captions track, creating it
    /// if missing. §7.1: the Captions lane is a singleton, so we never
    /// spawn a second one.
    ///
    /// Captions clips belong here — not on the currently-selected track
    /// and not on the Overlay lane. Putting them elsewhere pollutes
    /// `audio_track_clip_count` (an Audio track that contained only a
    /// stray Captions clip used to disable the video-embedded audio
    /// fallback in the export plan) and hides the captions lane in the
    /// timeline.
    fn ensure_captions_track(&mut self) -> usize {
        if let Some((i, _)) = self
            .project
            .tracks
            .iter()
            .enumerate()
            .find(|(_, t)| t.kind == caprust_core::TrackKind::Captions)
        {
            return i;
        }
        self.project.tracks.push(caprust_core::Track::new(
            "Captions",
            caprust_core::TrackKind::Captions,
        ));
        tracing::info!("caption: auto-created Captions track");
        self.project.tracks.len() - 1
    }

    /// Drop Captions clips that carry no segments. These are leftovers
    /// from an older build that inserted an empty clip whenever a model
    /// was picked in the prompt, and would otherwise accumulate every
    /// time the project is opened and re-saved.
    fn cleanup_phantom_captions(&mut self) {
        let before = self.project.clips.len();
        self.project.clips.retain(|c| {
            !matches!(
                &c.clip_type,
                caprust_core::ClipType::Captions { segments, .. } if segments.is_empty()
            )
        });
        let removed = before - self.project.clips.len();
        if removed > 0 {
            tracing::info!("caption: pruned {removed} empty captions clip(s) on load");
        }
    }

    /// Queue every audio-bearing clip on `track_index` for
    /// transcription, in timeline order. Starts the first immediately;
    /// the rest run as each job completes.
    fn start_caption_jobs_for_track(&mut self, track_index: usize) {
        let mut sources: Vec<(uuid::Uuid, u64)> = self
            .project
            .clips
            .iter()
            .filter(|c| c.track_index == track_index)
            .filter(|c| c.duration_ms > 0)
            .filter(|c| {
                matches!(
                    &c.clip_type,
                    caprust_core::ClipType::Audio { .. } | caprust_core::ClipType::Video { .. }
                )
            })
            .map(|c| (c.id, c.start_time_ms))
            .collect();
        sources.sort_by_key(|(_, start)| *start);

        if sources.is_empty() {
            self.toast(tr("toast-caption-no-audio"));
            tracing::info!("caption: no audio-bearing clips on track {track_index}");
            return;
        }

        let n = sources.len();
        self.caption_queue.clear();
        self.caption_queue
            .extend(sources.into_iter().map(|(id, _)| id));
        self.caption_batch_total = n;
        self.caption_batch_current = 0;
        tracing::info!("caption: queued {n} clips for transcription");
        self.toast(format!("{} · {}", tr("toast-caption-queued"), n));

        self.pump_caption_queue();
    }

    /// Start the next queued caption job if one is waiting and no
    /// other caption job is in flight.
    fn pump_caption_queue(&mut self) {
        if self.caption_rx.is_some() {
            return;
        }
        let Some(next) = self.caption_queue.pop_front() else {
            // Nothing left — clear the batch so the next single-clip
            // caption job shows the generic label again.
            self.caption_batch_total = 0;
            self.caption_batch_current = 0;
            return;
        };
        self.caption_batch_current += 1;
        tracing::info!(
            "caption: starting next queued clip ({} remaining)",
            self.caption_queue.len()
        );
        self.start_caption_job(Some(next));
    }

    /// Handle a 💬 click. If a model is ready and no job is in flight,
    /// Spawn a Whisper transcription over the requested clip and
    /// remember the receiver.
    ///
    /// `source_id` is the clip to transcribe. When `None`, the current
    /// selection is used: exactly one clip → that clip; more than one
    /// → the first (with a toast noting the choice, since multi-clip
    /// batch is a follow-up); nothing selected → a warning toast and
    /// no job.
    ///
    /// If no model is ready, opens the model-prompt dialog instead.
    fn start_caption_job(&mut self, source_id: Option<uuid::Uuid>) {
        // Resolve the source clip up front so a bad selection fails
        // before we do any model work.
        let resolved_source: Option<uuid::Uuid> = match source_id {
            Some(id) => Some(id),
            None => {
                // Copy selection out before any &mut self call so the
                // borrow checker is happy.
                let selected = self.selected_clips.clone();
                match selected.as_slice() {
                    [] => {
                        self.toast(tr("toast-caption-no-selection"));
                        tracing::info!("caption: no clip selected");
                        return;
                    }
                    [single] => Some(*single),
                    [first, rest @ ..] => {
                        tracing::info!(
                            "caption: {} clips selected — using the first",
                            rest.len() + 1
                        );
                        self.toast(tr("toast-caption-multi-first"));
                        Some(*first)
                    }
                }
            }
        };

        // Demo bypass: a zero-byte `DEMO` file in the models folder
        // activates a fake transcription path. Lets the caption pipeline
        // be exercised end to end (insert, track routing, save) without
        // a real Whisper model. Delete the file to go back to normal.
        let models_dir = self.settings.effective_models_dir();
        let demo_mode = models_dir.join("DEMO").exists();
        if demo_mode {
            tracing::info!("caption: DEMO marker present — bypassing model check");
        }

        let (model_id, language, model_path) = if demo_mode {
            (
                "demo".to_string(),
                self.settings.language.clone(),
                std::path::PathBuf::new(),
            )
        } else {
            // Refresh model status from the filesystem before deciding
            // whether a caption model is available. scan_local() marks a
            // model Ready when its file exists and is non-empty, so
            // manually-placed weights (and downloads that completed
            // after the last startup) are picked up without a restart.
            self.project.models.scan_local(&models_dir);

            let ready = self.project.models.ready_captions();
            let Some(model) = ready.first() else {
                tracing::info!("no caption model ready — opening prompt");
                self.model_prompt = Some(caprust_core::ModelKind::Caption);
                return;
            };
            let model_id = model.id.clone();
            let language = model.language.clone();

            let model_path = self.project.models.local_path(&models_dir, &model_id);
            if !model_path.is_file() {
                tracing::warn!(
                    "caption: model {} not on disk at {}",
                    model_id,
                    model_path.display()
                );
                self.model_prompt = Some(caprust_core::ModelKind::Caption);
                return;
            }
            (model_id, language, model_path)
        };

        // Look up the resolved source. If it's missing, is a type
        // without a file, or has zero duration, bail with a clear toast
        // instead of silently doing nothing.
        let Some(source_clip) = self
            .project
            .clips
            .iter()
            .find(|c| Some(c.id) == resolved_source)
            .cloned()
        else {
            self.toast(tr("toast-caption-no-selection"));
            tracing::warn!("caption: selected clip vanished");
            return;
        };

        let path_str = match &source_clip.clip_type {
            caprust_core::ClipType::Audio { path, .. } => path.clone(),
            caprust_core::ClipType::Video { path, .. } => path.clone(),
            _ => {
                self.toast(tr("toast-caption-no-audio"));
                tracing::warn!("caption: selected clip has no audio source");
                return;
            }
        };
        if source_clip.duration_ms == 0 {
            self.toast(tr("toast-caption-no-audio"));
            tracing::warn!("caption: selected clip has zero duration");
            return;
        }
        let source_path = std::path::PathBuf::from(path_str);
        let source_start_ms = 0u64;
        let duration_ms = source_clip.duration_ms;
        // Captions clip lands under its source: same timeline start and
        // duration, so the user can see the transcript directly below
        // the video/audio it came from. `source_start_ms` stays 0 until
        // clip trimming is wired through to the extraction step.
        let insert_at_ms = source_clip.start_time_ms;

        let req = crate::media_jobs::CaptionRequest {
            model_id,
            model_path,
            language,
            source_path,
            source_start_ms,
            duration_ms,
            insert_at_ms,
        };

        // Demo path does not touch ffmpeg or whisper; it returns fake
        // segments from a background thread that mimics the real job's
        // channel shape, so drain_caption_job needs no changes.
        let rx = if demo_mode {
            crate::media_jobs::spawn_demo_caption_job(req)
        } else {
            let ffmpeg = match self.ffmpeg_status.ffmpeg.clone() {
                Some(p) => std::path::PathBuf::from(p),
                None => {
                    tracing::warn!("caption: ffmpeg not available");
                    return;
                }
            };
            crate::media_jobs::spawn_caption_job(ffmpeg, req)
        };
        let label = if self.caption_batch_total > 1 {
            format!(
                "{}  {}/{}",
                tr("job-caption"),
                self.caption_batch_current,
                self.caption_batch_total
            )
        } else {
            tr("job-caption")
        };
        let job_id = self.begin_job(JobKind::Caption, label);
        self.caption_job_id = Some(job_id);
        self.caption_rx = Some(rx);
        self.caption_job_started = Some(std::time::Instant::now());
        self.toast(tr("toast-caption-started"));
        tracing::info!("caption: job spawned ({duration_ms}ms source)");
    }

    /// Poll the in-flight caption job. On success, insert a populated
    /// Captions clip at the source's timeline position. On failure, log
    /// and clear state so the user can retry.
    fn drain_caption_job(&mut self) {
        let Some(rx) = self.caption_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(result)) => {
                if result.segments.is_empty() {
                    // No speech detected (or a stub job returned nothing).
                    // Do not insert an empty Captions clip — it renders as
                    // a phantom block on the timeline and pollutes the
                    // project file across saves.
                    tracing::warn!("caption: job returned 0 segments — not inserting a clip");
                    self.toast(tr("toast-caption-empty"));
                    if let Some(id) = self.caption_job_id.take() {
                        self.finish_job(id);
                    }
                    self.caption_rx = None;
                    self.caption_job_started = None;
                    self.pump_caption_queue();
                    return;
                }
                let captions_track = self.ensure_captions_track();
                let clip = caprust_core::Clip::new_captions(
                    captions_track,
                    result.insert_at_ms,
                    result.duration_ms.max(1000),
                    &result.model_id,
                    &result.language,
                );
                let mut clip = clip;
                if let caprust_core::ClipType::Captions { segments, .. } = &mut clip.clip_type {
                    *segments = result.segments.clone();
                }
                let cmd = caprust_core::commands::ripple::RippleInsertCommand::new(clip);
                if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                    tracing::error!("caption: insert failed: {e}");
                } else {
                    tracing::info!(
                        "caption: inserted clip with {} segments at {}ms",
                        result.segments.len(),
                        result.insert_at_ms
                    );
                    self.toast(tr("toast-caption-added"));
                }
                if let Some(id) = self.caption_job_id.take() {
                    self.finish_job(id);
                }
                self.caption_rx = None;
                self.caption_job_started = None;
                self.pump_caption_queue();
            }
            Ok(Err(msg)) => {
                tracing::error!("caption: job failed: {msg}");
                self.toast(format!("{}: {msg}", tr("toast-caption-failed")));
                if let Some(id) = self.caption_job_id.take() {
                    self.finish_job(id);
                }
                self.caption_rx = None;
                self.caption_job_started = None;
                self.pump_caption_queue();
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                tracing::error!("caption: job thread vanished");
                if let Some(id) = self.caption_job_id.take() {
                    self.finish_job(id);
                }
                self.caption_rx = None;
                self.caption_job_started = None;
                self.pump_caption_queue();
            }
        }
    }

    /// Poll the update-check thread. On success with Some(info), store
    /// it and let the toast render. On None or Err, do nothing.
    fn drain_update_check(&mut self) {
        let Some(rx) = self.update_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(Some(info))) => {
                // Respect snooze / skip preferences persisted in the
                // update-checker cache from a previous session.
                if caprust_core::update_checker::should_notify(&info) {
                    tracing::info!(
                        "update: {} available (current {})",
                        info.latest_version,
                        info.current_version
                    );
                    self.update_available = Some(info);
                } else {
                    tracing::info!(
                        "update: {} available but snoozed/skipped",
                        info.latest_version
                    );
                }
                self.update_rx = None;
            }
            Ok(Ok(None)) => {
                tracing::debug!("update: already current");
                self.update_rx = None;
            }
            Ok(Err(e)) => {
                tracing::debug!("update: check failed: {e}");
                self.update_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.update_rx = None;
            }
        }
    }

    /// Register a new background job and return its id. Callers store
    /// the id so drain paths can update progress or finish it.
    fn begin_job(&mut self, kind: JobKind, label: impl Into<String>) -> u64 {
        let id = self.next_job_id;
        self.next_job_id += 1;
        self.jobs.push(BackgroundJob {
            id,
            kind,
            label: label.into(),
            progress: BackgroundJob::INDETERMINATE,
            started_at: std::time::Instant::now(),
        });
        id
    }

    /// Update a job's progress (0.0..=1.0). No-op if the id is gone.
    fn update_job_progress(&mut self, id: u64, progress: f32) {
        if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            j.progress = progress.clamp(0.0, 1.0);
        }
    }

    /// Remove a job from the list. No-op if already gone.
    fn finish_job(&mut self, id: u64) {
        self.jobs.retain(|j| j.id != id);
    }

    /// Thin bar above the timeline listing live background jobs.
    /// Auto-hides when the list is empty.
    fn show_jobs_bar(&mut self, ctx: &egui::Context) {
        if self.jobs.is_empty() {
            return;
        }

        egui::TopBottomPanel::bottom("jobs_bar")
            .exact_height(34.0)
            .resizable(false)
            .show_separator_line(false)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(8.0);
                    let n = self.jobs.len();
                    ui.label(
                        egui::RichText::new(format!(
                            "{} job{} running",
                            n,
                            if n == 1 { "" } else { "s" }
                        ))
                        .small()
                        .color(egui::Color32::from_gray(170)),
                    );
                    ui.separator();

                    for j in &self.jobs {
                        ui.label(
                            egui::RichText::new(&j.label)
                                .small()
                                .color(egui::Color32::from_gray(220)),
                        );
                        let bar_w = 120.0;
                        if j.is_indeterminate() {
                            // Spinner substitute: animated dots via
                            // progress bar in a spinning style is not
                            // built into egui. Use a low-alpha bar that
                            // repaints; the caller requests repaint so
                            // the pulse is visible.
                            let pulse = 0.15 + 0.15 * (j.elapsed().as_secs_f32() * 3.0).sin();
                            ui.add(
                                egui::ProgressBar::new(pulse)
                                    .desired_width(bar_w)
                                    .desired_height(8.0),
                            );
                        } else {
                            ui.add(
                                egui::ProgressBar::new(j.progress)
                                    .desired_width(bar_w)
                                    .desired_height(8.0),
                            );
                        }
                        ui.label(
                            egui::RichText::new(format!("{:.1}s", j.elapsed().as_secs_f32()))
                                .small()
                                .monospace()
                                .color(egui::Color32::from_gray(150)),
                        );
                        ui.add_space(12.0);
                    }
                    ctx.request_repaint();
                });
            });
    }

    /// Push a transient notification. Auto-dismisses after 4s.
    fn toast(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast::new(text));
    }

    /// Render live toasts top-right. Prunes expired entries each frame.
    fn show_toasts(&mut self, ctx: &egui::Context) {
        let now = std::time::Instant::now();
        self.toasts
            .retain(|t| now.duration_since(t.created_at) < t.duration);
        if self.toasts.is_empty() {
            return;
        }

        let mut dismiss: Option<usize> = None;
        let base_y = 48.0
            + if self.update_available.is_some() {
                140.0
            } else {
                0.0
            };
        let mut y = base_y;

        for (i, t) in self.toasts.iter().enumerate() {
            let id = egui::Id::new(("caprust-toast", i, t.created_at));
            egui::Window::new(format!("caprust-toast-{i}"))
                .id(id)
                .resizable(false)
                .collapsible(false)
                .title_bar(false)
                .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, y))
                .default_width(320.0)
                .show(ctx, |ui| {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&t.text).size(13.0));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("✕").clicked() {
                                        dismiss = Some(i);
                                    }
                                },
                            );
                        });
                    });
                });

            y += 56.0;
        }

        if let Some(i) = dismiss {
            self.toasts.remove(i);
        }
    }

    /// Render the update toast in the top-right corner when an update
    /// is available. Three actions: Download (open browser), Remind me
    /// later (snooze 7 days), Skip this version (never notify again
    /// for this exact version).
    fn show_update_toast(&mut self, ctx: &egui::Context) {
        let Some(info) = self.update_available.clone() else {
            return;
        };

        let mut dismiss = false;
        let mut open_browser = false;
        let mut snooze = false;
        let mut skip = false;

        egui::Window::new(tr("update-toast-title"))
            .id(egui::Id::new("update_toast"))
            .resizable(false)
            .collapsible(false)
            .title_bar(false)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 48.0))
            .default_width(320.0)
            .show(ctx, |ui| {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new(tr("update-toast-title"))
                                .strong()
                                .size(14.0),
                        );
                        ui.add_space(2.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "{} → {}",
                                info.current_version, info.latest_version
                            ))
                            .small()
                            .color(egui::Color32::from_gray(200)),
                        );
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            let dl = egui::Button::new(
                                egui::RichText::new(tr("update-toast-download"))
                                    .color(egui::Color32::WHITE)
                                    .strong(),
                            )
                            .fill(egui::Color32::from_rgb(34, 139, 230));
                            if ui.add(dl).clicked() {
                                open_browser = true;
                                dismiss = true;
                            }
                            if ui.button(tr("update-toast-later")).clicked() {
                                snooze = true;
                                dismiss = true;
                            }
                            if ui.button(tr("update-toast-skip")).clicked() {
                                skip = true;
                                dismiss = true;
                            }
                        });
                    });
                });
            });

        if open_browser {
            if let Err(e) = open::that(&info.release_url) {
                tracing::warn!("update: open browser failed: {e}");
            }
        }
        if snooze {
            if let Err(e) = caprust_core::update_checker::snooze_default(
                &info.latest_version,
                &info.release_url,
            ) {
                tracing::warn!("update: snooze write failed: {e}");
            }
        }
        if skip {
            if let Err(e) = caprust_core::update_checker::skip_version(&info.latest_version) {
                tracing::warn!("update: skip write failed: {e}");
            }
        }
        if dismiss {
            self.update_available = None;
        }
    }

    fn handle_preview_events(&mut self, ev: PreviewEvents, total_ms: u64) {
        let mut seeked = false;
        if ev.seek_back_30 {
            self.playhead_ms = self.playhead_ms.saturating_sub(30_000);
            seeked = true;
        }
        if ev.seek_back_5 {
            self.playhead_ms = self.playhead_ms.saturating_sub(5_000);
            seeked = true;
        }
        if ev.seek_fwd_5 {
            self.playhead_ms = (self.playhead_ms + 5_000).min(total_ms);
            seeked = true;
        }
        if ev.seek_fwd_30 {
            self.playhead_ms = (self.playhead_ms + 30_000).min(total_ms);
            seeked = true;
        }
        if seeked && self.preview.playing {
            self.explicit_seek_ms = Some(self.playhead_ms);
            self.playback_started_at = Some(std::time::Instant::now());
            self.playback_started_ms = self.playhead_ms;
        }
        if ev.toggle_mute {
            self.settings.muted = !self.settings.muted;
            if let Some(ap) = self.audio_player.as_ref() {
                ap.set_muted(self.settings.muted);
            }
            tracing::info!("preview: muted={}", self.settings.muted);
        }
        if let Some(v) = ev.volume_changed {
            self.settings.master_volume = v.clamp(0.0, 1.0);
            if let Some(ap) = self.audio_player.as_ref() {
                ap.set_volume(self.settings.master_volume);
            }
            tracing::info!("preview: volume={:.2}", self.settings.master_volume);
        }
        if ev.toggle_play {
            self.preview.playing = !self.preview.playing;
            if !self.preview.playing {
                self.preview_player.cancel_pending();
                self.preview_player.stop_stream();
                self.audio_player = None; // Drop zaustavlja cpal stream
                if let Some(mut r) = self.preview_renderer.take() {
                    r.kill();
                }
                self.last_streamed_clip = None;
                self.playback_started_at = None;
            } else {
                // Starting play: use current playhead as the render start.
                self.explicit_seek_ms = Some(self.playhead_ms);
                self.play_anchor_set = false;
                self.audio_baseline_ms = 0;
            }
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

                // Toolbar
                let can_undo = self.undo_stack.can_undo();
                let can_redo = self.undo_stack.can_redo();
                let mut tools = self.timeline_tools;

                // Compute model availability once per frame so the
                // download icon can tint itself and explain its state
                // in a tooltip.
                let models_dir = self.settings.effective_models_dir();
                let _ = models_dir; // reserved for a future filesystem scan inside this frame
                let captions_total = self
                    .project
                    .models
                    .models
                    .iter()
                    .filter(|m| m.kind == caprust_core::ModelKind::Caption)
                    .count();
                let narration_total = self
                    .project
                    .models
                    .models
                    .iter()
                    .filter(|m| m.kind == caprust_core::ModelKind::Narration)
                    .count();
                let captions_ready = self.project.models.ready_captions().len();
                let narration_ready = self.project.models.ready_narration().len();
                let availability = crate::timeline::toolbar::ModelAvailability::Status {
                    captions_ready,
                    narration_ready,
                    captions_total,
                    narration_total,
                };

                let ev = crate::timeline::toolbar::show(
                    ui,
                    &mut tools,
                    can_undo,
                    can_redo,
                    self.playhead_ms,
                    availability,
                );
                self.timeline_tools = tools;
                self.handle_timeline_events(ev);
                ui.separator();

                let header_w = crate::timeline::track_header::HEADER_WIDTH;
                let ruler_h = crate::timeline::ruler::RULER_HEIGHT;
                let full_h = ui.available_height().max(150.0);

                let order = caprust_core::track::display_order(&self.project.tracks);
                let mut updated_tracks = self.project.tracks.clone();
                let mut header_changed = false;
                let mut pending_delete_track: Option<usize> = None;
                // (clip_id, screen rect) for marquee hit test on release.
                let mut all_clip_rects: Vec<(uuid::Uuid, egui::Rect)> = Vec::new();
                let mut all_lane_rects: Vec<egui::Rect> = Vec::new();
                let mut pending_duplicate_track: Option<usize> = None;
                let mut pending_rename_track: Option<usize> = None;
                let mut pending_actions: Vec<ClipAction> = Vec::new();
                let mut pending_drop: Option<(uuid::Uuid, usize, u64)> = None;

                // DnD
                let dnd_active = egui::DragAndDrop::payload::<uuid::Uuid>(ctx).map(|a| *a);
                if let Some(id) = dnd_active {
                    self.last_dnd_payload = Some(id);
                }
                let pointer_hover = ctx.input(|i| i.pointer.hover_pos());
                let pointer_released = ctx.input(|i| i.pointer.any_released());
                let pointer_down = ctx.input(|i| i.pointer.primary_down());
                let dnd_drop = if pointer_released {
                    self.last_dnd_payload
                } else {
                    None
                };
                let clip_drag_snapshot = self.clip_drag.clone();
                let pan_mode = self.timeline_tools.pan_mode;

                // Time/px
                let total_ms = self.total_duration_ms();
                let content_ms = (total_ms + 20_000).max(30_000);
                let viewport_w = (ui.available_width() - header_w).max(120.0);
                let px_per_ms = (viewport_w * 0.9 * self.timeline_zoom) / content_ms as f32;
                let px_per_ms = px_per_ms.max(0.002);
                let content_width = (content_ms as f32 * px_per_ms).max(viewport_w);
                let max_scroll = (content_width - viewport_w).max(0.0);
                self.timeline_scroll_x = self.timeline_scroll_x.clamp(0.0, max_scroll);

                // Follow playhead: nudge scroll so playhead stays centered
                let follow = self.timeline_tools.follow_playhead;
                let playing = self.preview.playing;
                if follow && playing {
                    let ph_px = self.playhead_ms as f32 * px_per_ms;
                    let target = ph_px - viewport_w * 0.5;
                    self.timeline_scroll_x = target.clamp(0.0, max_scroll);
                    ctx.request_repaint();
                }
                let scroll_x = self.timeline_scroll_x;

                // Pan mode: left-drag inside the timeline scrolls horizontally.
                if pan_mode {
                    let drag_delta = ctx.input(|i| i.pointer.delta());
                    if ctx.input(|i| i.pointer.primary_down()) && drag_delta.x != 0.0 {
                        self.timeline_scroll_x =
                            (self.timeline_scroll_x - drag_delta.x).clamp(0.0, max_scroll);
                        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                    }
                }

                let theme_snapshot = self.theme.clone();
                ui.horizontal_top(|ui| {
                    // LEFT: headers
                    ui.allocate_ui_with_layout(
                        egui::vec2(header_w, full_h),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(header_w);
                            ui.set_min_height(full_h);
                            ui.allocate_space(egui::vec2(header_w, ruler_h));
                            for &idx in &order {
                                let mut track = updated_tracks[idx].clone();
                                let row_h = track.height;
                                ui.allocate_ui_with_layout(
                                    egui::vec2(header_w, row_h),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui.set_width(header_w);
                                        // Force the child UI to occupy the
                                        // full row height. allocate_ui_with_layout
                                        // shrinks the reserved space to the
                                        // child's actual content (two header
                                        // rows, ~36px) rather than the
                                        // requested row_h. The lanes on the
                                        // right use allocate_exact_size, which
                                        // does honor row_h. That mismatch
                                        // accumulated ~14px per track and left
                                        // the header column ~1 track short of
                                        // the lane column after 4-5 tracks.
                                        ui.set_min_height(row_h);
                                        let hev = crate::timeline::track_header::show(
                                            ui,
                                            &mut track,
                                            idx,
                                            &theme_snapshot,
                                            row_h,
                                        );
                                        if hev.changed {
                                            header_changed = true;
                                        }
                                        if hev.delete_requested {
                                            pending_delete_track = Some(idx);
                                        }
                                        if hev.duplicate_requested {
                                            pending_duplicate_track = Some(idx);
                                        }
                                        if hev.rename_requested {
                                            pending_rename_track = Some(idx);
                                        }
                                    },
                                );
                                updated_tracks[idx] = track;
                            }
                        },
                    );

                    // RIGHT: ruler + lanes with manual scroll
                    ui.allocate_ui_with_layout(
                        egui::vec2(viewport_w, full_h),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_min_width(viewport_w);
                            ui.set_min_height(full_h);

                            // Ruler
                            let (ruler_rect, _) = ui.allocate_exact_size(
                                egui::vec2(viewport_w, ruler_h),
                                egui::Sense::click(),
                            );
                            let rp = ui.painter_at(ruler_rect);
                            rp.rect_filled(ruler_rect, 0.0, egui::Color32::from_gray(28));
                            rp.line_segment(
                                [
                                    egui::Pos2::new(ruler_rect.left(), ruler_rect.bottom() - 0.5),
                                    egui::Pos2::new(ruler_rect.right(), ruler_rect.bottom() - 0.5),
                                ],
                                egui::Stroke::new(1.0_f32, egui::Color32::from_gray(50)),
                            );
                            let interval_ms: u64 = {
                                let cands: &[u64] = &[
                                    100, 250, 500, 1000, 2000, 5000, 10_000, 15_000, 30_000,
                                    60_000, 120_000, 300_000, 600_000,
                                ];
                                let mut c = *cands.last().unwrap();
                                for &x in cands {
                                    if (x as f32) * px_per_ms >= 70.0 {
                                        c = x;
                                        break;
                                    }
                                }
                                c
                            };
                            let first_tick =
                                ((scroll_x / px_per_ms) as u64 / interval_ms) * interval_ms;
                            let last_tick = ((scroll_x + viewport_w) / px_per_ms) as u64;
                            let mut t = first_tick;
                            while t <= last_tick + interval_ms {
                                let x = ruler_rect.left() + (t as f32) * px_per_ms - scroll_x;
                                if x > ruler_rect.right() + 5.0 {
                                    break;
                                }
                                if x >= ruler_rect.left() - 5.0 {
                                    let th = if t.is_multiple_of(interval_ms * 5) {
                                        10.0
                                    } else if t.is_multiple_of(interval_ms * 2) {
                                        7.0
                                    } else {
                                        5.0
                                    };
                                    rp.line_segment(
                                        [
                                            egui::Pos2::new(x, ruler_rect.bottom() - th),
                                            egui::Pos2::new(x, ruler_rect.bottom()),
                                        ],
                                        egui::Stroke::new(1.0_f32, egui::Color32::from_gray(90)),
                                    );
                                    if th >= 10.0 {
                                        let s = t / 1000;
                                        let mm = (s % 3600) / 60;
                                        let ss = s % 60;
                                        rp.text(
                                            egui::Pos2::new(x + 3.0, ruler_rect.top() + 2.0),
                                            egui::Align2::LEFT_TOP,
                                            format!("{mm:02}:{ss:02}"),
                                            egui::FontId::proportional(10.0),
                                            egui::Color32::from_gray(170),
                                        );
                                    }
                                }
                                t += interval_ms;
                            }
                            let ph_x =
                                ruler_rect.left() + self.playhead_ms as f32 * px_per_ms - scroll_x;
                            let ph_visible =
                                ph_x >= ruler_rect.left() && ph_x <= ruler_rect.right();
                            // Playhead line is drawn after the lane loop:
                            // Full mode needs the bottom of the last lane,
                            // which is only known once every row has been
                            // allocated. Compact mode will use ruler_rect
                            // alone, but we defer both for a single code
                            // path.
                            if ui.input(|i| i.pointer.primary_clicked()) {
                                if let Some(p) = ui.ctx().pointer_interact_pos() {
                                    if ruler_rect.contains(p) {
                                        let ms = ((p.x - ruler_rect.left() + scroll_x) / px_per_ms)
                                            .max(0.0)
                                            as u64;
                                        pending_actions.push(ClipAction::SetPlayhead(ms));
                                    }
                                }
                            }

                            // Lanes
                            let mut top_y_opt: Option<f32> = None;
                            let mut rows_actual: Vec<(usize, f32)> = Vec::new();
                            // Tracked so the playhead overlay can span
                            // the entire stack in Full mode.
                            let mut lane_stack_bottom: Option<f32> = None;
                            for &idx in &order {
                                let track = &updated_tracks[idx];
                                let row_h = track.height;
                                let (lane_rect, _) = ui.allocate_exact_size(
                                    egui::vec2(viewport_w, row_h),
                                    egui::Sense::hover(),
                                );
                                if top_y_opt.is_none() {
                                    top_y_opt = Some(lane_rect.top());
                                }
                                lane_stack_bottom = Some(lane_rect.bottom());
                                rows_actual.push((idx, row_h));
                                all_lane_rects.push(lane_rect);

                                let p = ui.painter_at(lane_rect);
                                let bg = theme_snapshot.track_lane_bg(track.kind, track.visible);
                                p.rect_filled(lane_rect, 0.0, bg);
                                if track.pinned {
                                    p.line_segment(
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
                                p.line_segment(
                                    [
                                        egui::Pos2::new(lane_rect.left(), lane_rect.bottom() - 0.5),
                                        egui::Pos2::new(
                                            lane_rect.right(),
                                            lane_rect.bottom() - 0.5,
                                        ),
                                    ],
                                    egui::Stroke::new(1.0_f32, egui::Color32::from_gray(35)),
                                );

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
                                        let dt = clip_drag_snapshot
                                            .as_ref()
                                            .filter(|d| d.clip_id == c.id)
                                            .map(|d| d.track_index);
                                        match dt {
                                            Some(tt) => tt == idx,
                                            None => c.track_index == idx,
                                        }
                                    })
                                    .map(|c| {
                                        let is_dragged = clip_drag_snapshot
                                            .as_ref()
                                            .map(|d| d.clip_id == c.id)
                                            .unwrap_or(false);
                                        let (s, d) = if is_dragged {
                                            let d = clip_drag_snapshot.as_ref().unwrap();
                                            (d.current_ms.max(0) as u64, c.duration_ms)
                                        } else {
                                            (c.start_time_ms, c.duration_ms)
                                        };
                                        (c.id, s, d, c.clip_type.clone(), is_dragged)
                                    })
                                    .collect();

                                for (clip_id, start_ms, dur_ms, ctype, is_dragged) in clips_here {
                                    let x0 =
                                        lane_rect.left() - scroll_x + start_ms as f32 * px_per_ms;
                                    let x1 = lane_rect.left() - scroll_x
                                        + (start_ms + dur_ms) as f32 * px_per_ms;
                                    if x1 < lane_rect.left() - 30.0 || x0 > lane_rect.right() + 30.0
                                    {
                                        continue;
                                    }

                                    let full_rect = egui::Rect::from_min_max(
                                        egui::pos2(x0, lane_rect.top() + 3.0),
                                        egui::pos2(x1.max(x0 + 8.0), lane_rect.bottom() - 3.0),
                                    );
                                    let clip_rect = full_rect.intersect(lane_rect);
                                    if clip_rect.width() < 2.0 {
                                        continue;
                                    }
                                    all_clip_rects.push((clip_id, clip_rect));

                                    let color = match &ctype {
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
                                    let c = if is_dragged {
                                        color.gamma_multiply(1.3)
                                    } else {
                                        color
                                    };
                                    p.rect_filled(clip_rect, 4.0, c);

                                    // Border: pastel tint of the track
                                    // colour, distinct from the clip's own
                                    // fill so adjacent clips stay readable
                                    // when they butt up against each other.
                                    let border_kind = self
                                        .project
                                        .tracks
                                        .get(idx)
                                        .map(|t| t.kind)
                                        .unwrap_or(caprust_core::TrackKind::Video);
                                    let border_color =
                                        theme_snapshot.clip_border_color(border_kind);
                                    p.rect_stroke(
                                        clip_rect,
                                        4.0,
                                        egui::Stroke::new(1.5_f32, border_color),
                                        egui::StrokeKind::Inside,
                                    );

                                    // Thumbnail strip: lookup clip's media_id → texture, tile across clip width.
                                    let thumb_tex = self
                                        .project
                                        .clips
                                        .iter()
                                        .find(|cc| cc.id == clip_id)
                                        .and_then(|cc| cc.media_id)
                                        .and_then(|mid| self.clip_textures.get(&mid).cloned());

                                    if let Some(tex) = thumb_tex {
                                        let tex_size = tex.size_vec2();
                                        let aspect = tex_size.x / tex_size.y.max(1.0);
                                        let tile_h = clip_rect.height();
                                        let tile_w = (tile_h * aspect).max(8.0);
                                        let mut x = clip_rect.left();
                                        let right = clip_rect.right();
                                        let mut guard = 0;
                                        while x < right - 1.0 && guard < 200 {
                                            let w = (right - x).min(tile_w);
                                            let tile_rect = egui::Rect::from_min_size(
                                                egui::pos2(x, clip_rect.top()),
                                                egui::vec2(w, tile_h),
                                            );
                                            let frac = (w / tile_w).min(1.0);
                                            p.image(
                                                tex.id(),
                                                tile_rect,
                                                egui::Rect::from_min_max(
                                                    egui::pos2(0.0, 0.0),
                                                    egui::pos2(frac, 1.0),
                                                ),
                                                egui::Color32::from_white_alpha(220),
                                            );
                                            x += tile_w;
                                            guard += 1;
                                        }
                                        // Dim overlay so clip-type color stays readable.
                                        p.rect_filled(clip_rect, 4.0, c.gamma_multiply(0.35));
                                    }
                                    if self.selected_clips.contains(&clip_id) || is_dragged {
                                        p.rect_stroke(
                                            clip_rect,
                                            4.0,
                                            egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                    let (label_full, label_short) = {
                                        let full = self
                                            .project
                                            .clips
                                            .iter()
                                            .find(|cc| cc.id == clip_id)
                                            .and_then(|cc| cc.name.clone())
                                            .unwrap_or_else(|| match &ctype {
                                                caprust_core::ClipType::TextOverlay {
                                                    content,
                                                    ..
                                                } => content.clone(),
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
                                            });
                                        // Truncate to 12 chars + ellipsis. Count
                                        // in chars so emoji-heavy names don't
                                        // overflow the visual budget.
                                        let short = if full.chars().count() > 12 {
                                            let mut s: String = full.chars().take(12).collect();
                                            s.push('…');
                                            s
                                        } else {
                                            full.clone()
                                        };
                                        (full, short)
                                    };
                                    p.text(
                                        clip_rect.left_top() + egui::vec2(6.0, 4.0),
                                        egui::Align2::LEFT_TOP,
                                        &label_short,
                                        egui::FontId::proportional(11.0),
                                        egui::Color32::WHITE,
                                    );
                                    // Hover tooltip with full name — only when
                                    // the name was truncated. Uses the pointer
                                    // position (ui.rect_contains_pointer)
                                    // rather than a Response, since the clip
                                    // painter has no interactive Response here.
                                    if label_full != label_short
                                        && pointer_hover
                                            .map(|pp| clip_rect.contains(pp))
                                            .unwrap_or(false)
                                        && self.clip_drag.is_none()
                                    {
                                        egui::show_tooltip_at_pointer(
                                            ui.ctx(),
                                            ui.layer_id(),
                                            egui::Id::new(("clip_name_tip", clip_id)),
                                            |ui| {
                                                ui.label(&label_full);
                                            },
                                        );
                                    }

                                    // Effect/transition badge (top-right of clip)
                                    if let Some(fx_clip) =
                                        self.project.clips.iter().find(|c| c.id == clip_id)
                                    {
                                        let has_fx = !fx_clip.effects.is_empty();
                                        let has_tr = fx_clip.transition_in.is_some()
                                            || fx_clip.transition_out.is_some();
                                        if has_fx || has_tr {
                                            let badge = if has_fx && has_tr {
                                                "✨⇄"
                                            } else if has_fx {
                                                "✨"
                                            } else {
                                                "⇄"
                                            };
                                            p.text(
                                                clip_rect.right_top() + egui::vec2(-6.0, 4.0),
                                                egui::Align2::RIGHT_TOP,
                                                badge,
                                                egui::FontId::proportional(11.0),
                                                egui::Color32::from_rgb(255, 240, 130),
                                            );
                                        }
                                    }

                                    let resp = ui.interact(
                                        clip_rect,
                                        egui::Id::new(("clip", clip_id)),
                                        egui::Sense::click(),
                                    );
                                    // Selection is driven from the drag-start
                                    // path below (see pointer_down block).
                                    // Firing Select here as well would toggle
                                    // twice on a Ctrl+click (once on press,
                                    // once on release), cancelling out.

                                    let pointer_on_clip = ui.rect_contains_pointer(clip_rect);
                                    const TRIM_ZONE: f32 = 8.0;
                                    let hovered_edge: Option<TrimEdge> =
                                        if pointer_on_clip && clip_rect.width() > TRIM_ZONE * 3.0 {
                                            if let Some(pp) = pointer_hover {
                                                let lz = egui::Rect::from_min_size(
                                                    clip_rect.min,
                                                    egui::vec2(TRIM_ZONE, clip_rect.height()),
                                                );
                                                let rz = egui::Rect::from_min_size(
                                                    egui::pos2(
                                                        clip_rect.max.x - TRIM_ZONE,
                                                        clip_rect.min.y,
                                                    ),
                                                    egui::vec2(TRIM_ZONE, clip_rect.height()),
                                                );
                                                if lz.contains(pp) {
                                                    Some(TrimEdge::Left)
                                                } else if rz.contains(pp) {
                                                    Some(TrimEdge::Right)
                                                } else {
                                                    None
                                                }
                                            } else {
                                                None
                                            }
                                        } else {
                                            None
                                        };
                                    if hovered_edge.is_some() {
                                        ui.ctx()
                                            .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                                    }
                                    let track_locked = self
                                        .project
                                        .tracks
                                        .get(idx)
                                        .map(|t| t.locked)
                                        .unwrap_or(false);
                                    if !track_locked
                                        && !pan_mode
                                        && pointer_on_clip
                                        && pointer_down
                                        && clip_drag_snapshot.is_none()
                                    {
                                        // Ctrl held → let the modifier logic
                                        // in the Select handler decide
                                        // (toggle). Plain click on an
                                        // already-selected clip is a no-op
                                        // so the whole multi-selection can
                                        // be dragged without collapsing to
                                        // one clip.
                                        let ctrl =
                                            ctx.input(|i| i.modifiers.ctrl || i.modifiers.command);
                                        let already = self.selected_clips.contains(&clip_id);
                                        if ctrl || !already {
                                            pending_actions.push(ClipAction::Select(clip_id));
                                        }
                                        pending_actions
                                            .push(ClipAction::DragStart(clip_id, idx, start_ms));
                                        if let Some(e) = hovered_edge {
                                            pending_actions
                                                .push(ClipAction::SetTrimEdge(clip_id, Some(e)));
                                        }
                                    }

                                    resp.context_menu(|ui| {
                                        // --- Copy / Paste / Duplicate ---
                                        let copy_lbl = if self.settings.enable_shortcuts {
                                            format!("{}  (Ctrl+C)", tr("clip-ctx-copy"))
                                        } else {
                                            tr("clip-ctx-copy")
                                        };
                                        if ui.button(copy_lbl).clicked() {
                                            pending_actions.push(ClipAction::Copy(clip_id));
                                            ui.close_menu();
                                        }
                                        let paste_lbl = if self.settings.enable_shortcuts {
                                            format!("{}  (Ctrl+V)", tr("clip-ctx-paste"))
                                        } else {
                                            tr("clip-ctx-paste")
                                        };
                                        if ui
                                            .add_enabled(
                                                self.clip_clipboard.is_some(),
                                                egui::Button::new(paste_lbl),
                                            )
                                            .clicked()
                                        {
                                            pending_actions.push(ClipAction::Paste);
                                            ui.close_menu();
                                        }
                                        let dup_lbl = if self.settings.enable_shortcuts {
                                            format!("{}  (Ctrl+D)", tr("clip-ctx-duplicate"))
                                        } else {
                                            tr("clip-ctx-duplicate")
                                        };
                                        if ui.button(dup_lbl).clicked() {
                                            pending_actions.push(ClipAction::Duplicate(clip_id));
                                            ui.close_menu();
                                        }
                                        ui.separator();

                                        // --- Delete (hard) ---
                                        let del = if self.settings.enable_shortcuts {
                                            format!("{}  (Del)", tr("clip-ctx-delete"))
                                        } else {
                                            tr("clip-ctx-delete")
                                        };
                                        if ui.button(del).clicked() {
                                            pending_actions.push(ClipAction::Delete(clip_id));
                                            ui.close_menu();
                                        }
                                        // --- Ripple delete ---
                                        let rip_lbl = tr("clip-ctx-ripple-delete");
                                        if ui.button(rip_lbl).clicked() {
                                            pending_actions.push(ClipAction::RippleDelete(clip_id));
                                            ui.close_menu();
                                        }
                                        // --- Split at playhead ---
                                        let spl = if self.settings.enable_shortcuts {
                                            format!("{}  (S)", tr("clip-ctx-split"))
                                        } else {
                                            tr("clip-ctx-split")
                                        };
                                        if ui.button(spl).clicked() {
                                            pending_actions
                                                .push(ClipAction::Split(clip_id, self.playhead_ms));
                                            ui.close_menu();
                                        }
                                        // --- Speed submenu ---
                                        ui.menu_button(tr("clip-ctx-speed"), |ui| {
                                            for v in [0.25_f32, 0.5, 1.0, 1.5, 2.0, 4.0] {
                                                let label = format!("{v:.2}x");
                                                if ui.button(label).clicked() {
                                                    pending_actions
                                                        .push(ClipAction::SetSpeed(clip_id, v));
                                                    ui.close_menu();
                                                }
                                            }
                                        });
                                        // --- Mute clip ---
                                        let muted = self
                                            .project
                                            .clips
                                            .iter()
                                            .find(|c| c.id == clip_id)
                                            .map(|c| c.volume_db <= -59.0)
                                            .unwrap_or(false);
                                        let mute_lbl = if muted {
                                            tr("clip-ctx-unmute")
                                        } else {
                                            tr("clip-ctx-mute")
                                        };
                                        if ui.button(mute_lbl).clicked() {
                                            pending_actions.push(ClipAction::MuteClip(clip_id));
                                            ui.close_menu();
                                        }
                                        ui.separator();
                                        if ui.button(tr("clip-ctx-generate-captions")).clicked() {
                                            pending_actions
                                                .push(ClipAction::GenerateCaptions(clip_id));
                                            ui.close_menu();
                                        }
                                        // Separate audio only applies to clips that
                                        // actually have an embedded audio track and
                                        // have not already been detached.
                                        let can_detach = self
                                            .project
                                            .clips
                                            .iter()
                                            .find(|c| c.id == clip_id)
                                            .map(|c| {
                                                matches!(
                                                    c.clip_type,
                                                    caprust_core::ClipType::Video { .. }
                                                ) && !c.audio_detached
                                            })
                                            .unwrap_or(false);
                                        if can_detach
                                            && ui.button(tr("clip-ctx-separate-audio")).clicked()
                                        {
                                            pending_actions
                                                .push(ClipAction::SeparateAudio(clip_id));
                                            ui.close_menu();
                                        }
                                        if self.settings.enable_shortcuts {
                                            ui.separator();
                                            if ui
                                                .button(format!("{}  (R)", tr("clip-ctx-reverse")))
                                                .clicked()
                                            {
                                                pending_actions
                                                    .push(ClipAction::ToggleReverse(clip_id));
                                                ui.close_menu();
                                            }
                                            if ui
                                                .button(format!("{}  (H)", tr("clip-ctx-mirror-h")))
                                                .clicked()
                                            {
                                                pending_actions
                                                    .push(ClipAction::ToggleFlipH(clip_id));
                                                ui.close_menu();
                                            }
                                            if ui
                                                .button(format!("{}  (V)", tr("clip-ctx-mirror-v")))
                                                .clicked()
                                            {
                                                pending_actions
                                                    .push(ClipAction::ToggleFlipV(clip_id));
                                                ui.close_menu();
                                            }
                                        }
                                    });
                                }

                                let hovering = pointer_hover
                                    .map(|pp| lane_rect.contains(pp))
                                    .unwrap_or(false);
                                if dnd_active.is_some() && hovering {
                                    p.rect_stroke(
                                        lane_rect.shrink(2.0),
                                        4.0,
                                        egui::Stroke::new(
                                            2.0_f32,
                                            egui::Color32::from_rgb(90, 160, 240),
                                        ),
                                        egui::StrokeKind::Inside,
                                    );
                                }
                                if let (Some(id), Some(pp)) = (dnd_drop, pointer_hover) {
                                    if lane_rect.contains(pp) {
                                        let rel = (pp.x - lane_rect.left() + scroll_x).max(0.0);
                                        let raw_ms = (rel / px_per_ms) as u64;

                                        // Snap the drop position against
                                        // neighbouring clip edges and the
                                        // playhead (same rules as clip
                                        // drag). Durations come from the
                                        // media item being dropped.
                                        let dur = self
                                            .project
                                            .media
                                            .items
                                            .iter()
                                            .find(|m| m.id == id)
                                            .map(|m| m.duration_ms)
                                            .unwrap_or(0);
                                        let snapped_ms = self
                                            .snap_ms(
                                                uuid::Uuid::nil(),
                                                idx,
                                                raw_ms as i64,
                                                dur,
                                                px_per_ms,
                                            )
                                            .max(0)
                                            as u64;
                                        pending_drop = Some((id, idx, snapped_ms));

                                        // Drop ghost: green edges when
                                        // snapped to a neighbour, neutral
                                        // blue otherwise. Disappears once
                                        // the clip lands; the real clip
                                        // renders in the standard style.
                                        let ghost_x = lane_rect.left()
                                            + (snapped_ms as f32 * px_per_ms)
                                            - scroll_x;
                                        let ghost_w = (dur as f32 * px_per_ms).max(4.0);
                                        let ghost_rect = egui::Rect::from_min_size(
                                            egui::Pos2::new(ghost_x, lane_rect.top() + 3.0),
                                            egui::vec2(
                                                ghost_w,
                                                (lane_rect.height() - 6.0).max(4.0),
                                            ),
                                        );
                                        let is_snapped = snapped_ms != raw_ms;
                                        let edge_color = if is_snapped {
                                            egui::Color32::from_rgb(120, 220, 120)
                                        } else {
                                            egui::Color32::from_rgb(120, 180, 240)
                                        };
                                        p.rect_filled(
                                            ghost_rect,
                                            4.0,
                                            egui::Color32::from_rgba_unmultiplied(
                                                edge_color.r(),
                                                edge_color.g(),
                                                edge_color.b(),
                                                55,
                                            ),
                                        );
                                        p.rect_stroke(
                                            ghost_rect,
                                            4.0,
                                            egui::Stroke::new(1.0_f32, edge_color),
                                            egui::StrokeKind::Outside,
                                        );
                                        // Emphasised left / right edges
                                        p.line_segment(
                                            [ghost_rect.left_top(), ghost_rect.left_bottom()],
                                            egui::Stroke::new(3.0_f32, edge_color),
                                        );
                                        p.line_segment(
                                            [ghost_rect.right_top(), ghost_rect.right_bottom()],
                                            egui::Stroke::new(3.0_f32, edge_color),
                                        );
                                    }
                                }
                            }

                            if let Some(d) = &clip_drag_snapshot {
                                if let Some(pp) = pointer_hover {
                                    pending_actions.push(ClipAction::DragDelta(
                                        d.clip_id,
                                        pp.x - d.origin_ptr.x,
                                        px_per_ms,
                                    ));
                                }
                                if pointer_released {
                                    pending_actions.push(ClipAction::DragEnd(d.clip_id));
                                }
                            }
                            // Playhead overlay: draw once, after every
                            // lane is allocated, so Full mode can span
                            // the entire stack.
                            if ph_visible {
                                let lane_bottom = lane_stack_bottom.unwrap_or(ruler_rect.bottom());
                                let line_bottom = match theme_snapshot.playhead_size {
                                    crate::theme::PlayheadSize::Compact => ruler_rect.bottom(),
                                    crate::theme::PlayheadSize::Full => lane_bottom,
                                };
                                // Use the UI's own painter. painter_at
                                // with a zero-width rect (ph_x..=ph_x)
                                // produces a degenerate clip rect and
                                // silently draws nothing — the original
                                // invisible-playhead bug. The UI's clip
                                // already covers the timeline area.
                                let op = ui.painter();
                                op.line_segment(
                                    [
                                        egui::Pos2::new(ph_x, ruler_rect.top()),
                                        egui::Pos2::new(ph_x, line_bottom),
                                    ],
                                    egui::Stroke::new(2.0_f32, theme_snapshot.playhead_color()),
                                );
                                // Grab handle: small filled triangle at
                                // the top so the user can see where to
                                // click to seek.
                                let tri = vec![
                                    egui::Pos2::new(ph_x - 5.0, ruler_rect.top()),
                                    egui::Pos2::new(ph_x + 5.0, ruler_rect.top()),
                                    egui::Pos2::new(ph_x, ruler_rect.top() + 6.0),
                                ];
                                op.add(egui::Shape::convex_polygon(
                                    tri,
                                    theme_snapshot.playhead_color(),
                                    egui::Stroke::NONE,
                                ));
                            }

                            // ---- Marquee (rubber-band) selection ----
                            {
                                let pointer_pos = ui.ctx().pointer_interact_pos();
                                let pressed = ui.input(|i| i.pointer.primary_pressed());
                                let released = ui.input(|i| i.pointer.any_released());
                                let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                                let shift = ui.input(|i| i.modifiers.shift);

                                // Start: press inside a lane but not on a clip.
                                if pressed && self.marquee.is_none() && !pan_mode {
                                    if let Some(pp) = pointer_pos {
                                        let on_lane = all_lane_rects.iter().any(|r| r.contains(pp));
                                        let on_clip =
                                            all_clip_rects.iter().any(|(_, r)| r.contains(pp));
                                        if on_lane && !on_clip {
                                            self.marquee = Some(MarqueeState {
                                                start: pp,
                                                current: pp,
                                            });
                                        }
                                    }
                                }

                                // Update in-progress.
                                if let (Some(m), Some(pp)) = (self.marquee.as_mut(), pointer_pos) {
                                    m.current = pp;
                                }

                                // Cancel on Esc.
                                if esc {
                                    self.marquee = None;
                                }

                                // Finalize on release.
                                if released {
                                    if let Some(m) = self.marquee.take() {
                                        let rect = egui::Rect::from_two_pos(m.start, m.current);
                                        // Shift = additive; no modifier
                                        // replaces the selection.
                                        if !shift {
                                            self.selected_clips.clear();
                                        }
                                        for (id, cr) in &all_clip_rects {
                                            if rect.intersects(*cr)
                                                && !self.selected_clips.contains(id)
                                            {
                                                self.selected_clips.push(*id);
                                            }
                                        }
                                    }
                                }

                                // Render the marquee while active.
                                if let Some(m) = self.marquee {
                                    let rect = egui::Rect::from_two_pos(m.start, m.current);
                                    let painter = ui.ctx().layer_painter(egui::LayerId::new(
                                        egui::Order::Foreground,
                                        egui::Id::new("marquee_overlay"),
                                    ));
                                    painter.rect_filled(
                                        rect,
                                        2.0,
                                        egui::Color32::from_rgba_unmultiplied(90, 160, 240, 40),
                                    );
                                    painter.rect_stroke(
                                        rect,
                                        2.0,
                                        egui::Stroke::new(
                                            1.5_f32,
                                            egui::Color32::from_rgb(120, 180, 240),
                                        ),
                                        egui::StrokeKind::Inside,
                                    );
                                }
                            }

                            if let Some(t) = top_y_opt {
                                self.timeline_row_layout = (t, rows_actual);
                            }
                        },
                    );
                });

                let sd = ctx.input(|i| i.raw_scroll_delta);
                if sd.y.abs() > 0.0 || sd.x.abs() > 0.0 {
                    let d = if sd.x.abs() > sd.y.abs() { sd.x } else { sd.y };
                    self.timeline_scroll_x = (self.timeline_scroll_x - d).clamp(0.0, max_scroll);
                }

                if header_changed {
                    self.project.tracks = updated_tracks;
                }
                if let Some(idx) = pending_delete_track {
                    if idx < self.project.tracks.len() {
                        self.project.tracks.remove(idx);
                        self.project.clips.retain(|c| c.track_index != idx);
                        for c in self.project.clips.iter_mut() {
                            if c.track_index > idx {
                                c.track_index -= 1;
                            }
                        }
                    }
                }
                if let Some(idx) = pending_duplicate_track {
                    let cmd =
                        caprust_core::commands::duplicate_track::DuplicateTrackCommand::new(idx);
                    if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                        tracing::error!("duplicate track failed: {e}");
                    } else {
                        tracing::info!("duplicate track {idx}");
                    }
                }
                if let Some(idx) = pending_rename_track {
                    let name = self
                        .project
                        .tracks
                        .get(idx)
                        .map(|t| t.name.clone())
                        .unwrap_or_default();
                    self.track_rename = Some((idx, name));
                }

                for a in pending_actions {
                    match a {
                        ClipAction::SetPlayhead(ms) => {
                            let target = ms.min(total_ms.max(1));
                            if self.preview.playing
                                && (target as i64 - self.playhead_ms as i64).abs() > 500
                            {
                                self.explicit_seek_ms = Some(target);
                                // Re-anchor wall clock so playhead stays
                                // in sync with the new position.
                                self.playback_started_at = Some(std::time::Instant::now());
                                self.playback_started_ms = target;
                            }
                            self.playhead_ms = target;
                        }
                        ClipAction::Select(id) => {
                            let ctrl = ctx.input(|i| i.modifiers.ctrl || i.modifiers.command);
                            if ctrl {
                                // Ctrl+click toggles membership.
                                if self.selected_clips.contains(&id) {
                                    self.selected_clips.retain(|&x| x != id);
                                } else {
                                    self.selected_clips.push(id);
                                }
                            } else if !self.selected_clips.contains(&id) {
                                // Plain click on an unselected clip
                                // replaces the selection.
                                self.selected_clips = vec![id];
                            }
                            // Plain click on an already-selected clip
                            // keeps the multi-selection so the whole
                            // group can be dragged together.
                        }
                        ClipAction::SetTrimEdge(id, edge) => {
                            if let Some(d) = &mut self.clip_drag {
                                if d.clip_id == id {
                                    d.trim_edge = edge;
                                }
                            }
                        }
                        ClipAction::DragStart(id, ti, o) => {
                            let (dur, sdr) = self
                                .project
                                .clips
                                .iter()
                                .find(|c| c.id == id)
                                .map(|c| (c.duration_ms, c.source_duration_ms))
                                .unwrap_or((3000, 0));
                            let ptr = self.last_pointer.unwrap_or_else(|| egui::pos2(0.0, 0.0));
                            self.clip_drag = Some(ClipDrag {
                                clip_id: id,
                                origin_ms: o,
                                current_ms: o as i64,
                                track_index: ti,
                                clip_duration_ms: dur,
                                origin_ptr: ptr,
                                last_ptr: ptr,
                                trim_edge: None,
                                source_duration_ms: sdr,
                                origin_duration_ms: dur,
                            });
                        }
                        ClipAction::DragDelta(id, dx, ppm) => {
                            if let Some(d) = self.clip_drag.clone() {
                                if d.clip_id == id && ppm > 0.0 {
                                    let cand = d.origin_ms as i64 + (dx / ppm) as i64;
                                    let snapped = self.snap_ms(
                                        id,
                                        d.track_index,
                                        cand.max(0),
                                        d.clip_duration_ms,
                                        ppm,
                                    );
                                    let new_track = self.track_for_y(d.track_index);
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
                                    if let Some(edge) = d.trim_edge {
                                        let dm = d.current_ms - d.origin_ms as i64;
                                        let (ns, nd) = match edge {
                                            TrimEdge::Left => (
                                                (d.origin_ms as i64 + dm).max(0) as u64,
                                                (d.origin_duration_ms as i64 - dm).max(100) as u64,
                                            ),
                                            TrimEdge::Right => {
                                                let mut nd = (d.origin_duration_ms as i64 + dm)
                                                    .max(100)
                                                    as u64;
                                                if d.source_duration_ms > 0 {
                                                    nd = nd.min(d.source_duration_ms);
                                                }
                                                (d.origin_ms, nd)
                                            }
                                        };
                                        let cmd =
                                            caprust_core::commands::set_clip::SetClipCommand::new(
                                                id,
                                            )
                                            .start_time_ms(ns)
                                            .duration_ms(nd);
                                        let _ = self
                                            .undo_stack
                                            .execute(Box::new(cmd), &mut self.project);
                                    } else {
                                        let nm = d.current_ms.max(0) as u64;
                                        let nt = d.track_index;
                                        if nm != d.origin_ms
                                            || self
                                                .project
                                                .clips
                                                .iter()
                                                .find(|c| c.id == id)
                                                .map(|c| c.track_index)
                                                != Some(nt)
                                        {
                                            let cmd = MoveClipCommand {
                                                clip_id: id,
                                                from_ms: d.origin_ms,
                                                to_ms: nm,
                                            };
                                            let _ = self
                                                .undo_stack
                                                .execute(Box::new(cmd), &mut self.project);
                                            if let Some(c) =
                                                self.project.clips.iter_mut().find(|c| c.id == id)
                                            {
                                                c.track_index = nt;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        ClipAction::Delete(id) => {
                            let rip = self.timeline_tools.magnetic;
                            let cmd = DeleteClipCommand::new(id, rip);
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
                        ClipAction::GenerateCaptions(id) => {
                            self.start_caption_job(Some(id));
                        }
                        ClipAction::SeparateAudio(id) => {
                            let cmd =
                                caprust_core::commands::separate_audio::SeparateAudioCommand::new(
                                    id,
                                );
                            if let Err(e) =
                                self.undo_stack.execute(Box::new(cmd), &mut self.project)
                            {
                                tracing::error!("separate audio failed: {e}");
                                self.toast(tr("toast-separate-audio-failed"));
                            } else {
                                tracing::info!("separate audio: created Audio clip from {id}");
                                self.toast(tr("toast-separate-audio-done"));
                            }
                        }
                        ClipAction::Copy(id) => {
                            if let Some(c) = self.project.clips.iter().find(|c| c.id == id) {
                                self.clip_clipboard = Some(c.clone());
                                tracing::info!("copy: {id}");
                                self.toast(tr("toast-clip-copied"));
                            }
                        }
                        ClipAction::Paste => {
                            if let Some(mut c) = self.clip_clipboard.clone() {
                                c.id = uuid::Uuid::new_v4();
                                c.start_time_ms = self.playhead_ms;
                                if c.track_index >= self.project.tracks.len() {
                                    c.track_index = 0;
                                }
                                let cmd =
                                    caprust_core::commands::ripple::RippleInsertCommand::new(c);
                                if let Err(e) =
                                    self.undo_stack.execute(Box::new(cmd), &mut self.project)
                                {
                                    tracing::error!("paste failed: {e}");
                                } else {
                                    self.toast(tr("toast-clip-pasted"));
                                }
                            }
                        }
                        ClipAction::Duplicate(id) => {
                            if let Some(orig) =
                                self.project.clips.iter().find(|c| c.id == id).cloned()
                            {
                                let mut c = orig;
                                c.id = uuid::Uuid::new_v4();
                                c.start_time_ms += c.duration_ms;
                                let cmd =
                                    caprust_core::commands::ripple::RippleInsertCommand::new(c);
                                if let Err(e) =
                                    self.undo_stack.execute(Box::new(cmd), &mut self.project)
                                {
                                    tracing::error!("duplicate failed: {e}");
                                } else {
                                    self.toast(tr("toast-clip-duplicated"));
                                }
                            }
                        }
                        ClipAction::RippleDelete(id) => {
                            let cmd = caprust_core::commands::delete_clip::DeleteClipCommand::new(
                                id, true,
                            );
                            if let Err(e) =
                                self.undo_stack.execute(Box::new(cmd), &mut self.project)
                            {
                                tracing::error!("ripple delete failed: {e}");
                            }
                        }
                        ClipAction::SetSpeed(id, v) => {
                            let cmd =
                                caprust_core::commands::set_clip::SetClipCommand::new(id).speed(v);
                            if let Err(e) =
                                self.undo_stack.execute(Box::new(cmd), &mut self.project)
                            {
                                tracing::error!("set speed failed: {e}");
                            } else {
                                tracing::info!("set speed {v}x on {id}");
                            }
                        }
                        ClipAction::MuteClip(id) => {
                            let cur = self
                                .project
                                .clips
                                .iter()
                                .find(|c| c.id == id)
                                .map(|c| c.volume_db)
                                .unwrap_or(0.0);
                            let target = if cur <= -59.0 { 0.0 } else { -60.0 };
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .volume_db(target);
                            if let Err(e) =
                                self.undo_stack.execute(Box::new(cmd), &mut self.project)
                            {
                                tracing::error!("mute clip failed: {e}");
                            }
                        }
                    }
                }

                if let Some((mid, ti, t)) = pending_drop {
                    let media = self
                        .project
                        .media
                        .items
                        .iter()
                        .find(|m| m.id == mid)
                        .cloned();
                    if let Some(item) = media {
                        use caprust_core::{Clip, MediaKind};
                        let dur = if item.duration_ms > 0 {
                            item.duration_ms
                        } else {
                            3000
                        };
                        let mut clip = match item.kind {
                            MediaKind::Video => Clip::new_video(&item.path, ti, t, dur),
                            MediaKind::Audio => Clip::new_audio(&item.path, ti, t, dur),
                            MediaKind::Image => Clip::new_image(&item.path, ti, t, dur),
                        };
                        clip.media_id = Some(item.id);
                        let nid = clip.id;
                        let cmd = caprust_core::commands::ripple::RippleInsertCommand::new(clip);
                        let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        self.selected_clips = vec![nid];
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
            .default_width(280.0)
            .min_width(120.0)
            .show(ctx, |ui| {
                let out = crate::panels::asset_browser::show(
                    ui,
                    &mut self.project,
                    &mut self.asset_browser,
                    &mut self.media_bin,
                );

                // Enqueue background probe + thumbnail jobs for new imports.
                for id in out.media.newly_imported {
                    if let Some(item) = self.project.media.items.iter().find(|m| m.id == id) {
                        tracing::info!("enqueueing probe for {}", item.path);
                        let item_clone = item.clone();
                        self.job_runner.enqueue(&item_clone);
                    }
                }

                // Remove requested items from library (files on disk are kept).
                for id in out.media.remove_requested {
                    self.project.media.remove(id);
                    self.clip_textures.remove(&id);
                    tracing::info!("media removed from library: {id}");
                }

                // Preset clicked (transitions/effects/filters/text).
                // Wiring to selected timeline clip is a follow-up PR.
                // Apply clicked preset to the selected clip.
                if let Some((preset_id, tab)) = out.preset_clicked {
                    if let Some(clip_id) = self.selected_clips.first().copied() {
                        use crate::panels::asset_browser::AssetTab;
                        use caprust_core::commands::set_effect::{
                            AddEffectCommand, SetTransitionCommand,
                        };
                        match tab {
                            AssetTab::Effects | AssetTab::Filters => {
                                if preset_id != "none" {
                                    let cmd = AddEffectCommand::new(clip_id, preset_id);
                                    let _ =
                                        self.undo_stack.execute(Box::new(cmd), &mut self.project);
                                    tracing::info!("applied effect '{preset_id}'");
                                }
                            }
                            AssetTab::Transitions => {
                                let t = if preset_id == "none" {
                                    None
                                } else {
                                    Some(preset_id.to_string())
                                };
                                let cmd = SetTransitionCommand::new(clip_id, true, t);
                                let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                                tracing::info!("set transition in = '{preset_id}'");
                            }
                            AssetTab::Text => {
                                // Two cases:
                                //   - selected clip is a TextOverlay →
                                //     apply the preset as its style.
                                //   - anything else → create a fresh
                                //     TextOverlay clip on the playhead
                                //     with the preset style and a
                                //     sensible placeholder text.
                                let is_text = self
                                    .project
                                    .clips
                                    .iter()
                                    .find(|c| c.id == clip_id)
                                    .map(|c| {
                                        matches!(
                                            c.clip_type,
                                            caprust_core::ClipType::TextOverlay { .. }
                                        )
                                    })
                                    .unwrap_or(false);
                                if is_text {
                                    let cmd =
                                        caprust_core::commands::set_clip::SetClipCommand::new(
                                            clip_id,
                                        )
                                        .text_style(preset_id);
                                    let _ =
                                        self.undo_stack.execute(Box::new(cmd), &mut self.project);
                                    tracing::info!("set text style = '{preset_id}'");
                                } else {
                                    let mut clip = caprust_core::Clip::new_text(
                                        "Double-click to edit",
                                        0,
                                        self.playhead_ms,
                                        3000,
                                        true,
                                    );
                                    if let caprust_core::ClipType::TextOverlay { style, .. } =
                                        &mut clip.clip_type
                                    {
                                        *style = preset_id.to_string();
                                    }
                                    let cmd =
                                        caprust_core::commands::ripple::RippleInsertCommand::new(
                                            clip,
                                        );
                                    let _ =
                                        self.undo_stack.execute(Box::new(cmd), &mut self.project);
                                    tracing::info!(
                                        "inserted TextOverlay clip with style '{preset_id}'"
                                    );
                                }
                            }
                            AssetTab::Media | AssetTab::Templates => {}
                        }
                    } else {
                        tracing::warn!("preset '{preset_id}' clicked but no clip selected");
                    }
                }
            });

        egui::SidePanel::right("right_panel")
            .resizable(true)
            .default_width(280.0)
            .min_width(240.0)
            .show(ctx, |ui| {
                ui.heading(tr("props-heading"));
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
                        // Field edits collapse into one SetClipCommand; effect
                        // and transition edits run as separate commands so each
                        // is individually undoable.
                        let mut field_cmd: Option<
                            caprust_core::commands::set_clip::SetClipCommand,
                        > = None;
                        for e in edits {
                            match e {
                                PendingEdit::RemoveEffect(effect_id) => {
                                    let c = caprust_core::commands::set_effect::RemoveEffectCommand::new(id, effect_id);
                                    let _ = self.undo_stack.execute(Box::new(c), &mut self.project);
                                }
                                PendingEdit::ClearTransitionIn => {
                                    let c = caprust_core::commands::set_effect::SetTransitionCommand::new(id, true, None);
                                    let _ = self.undo_stack.execute(Box::new(c), &mut self.project);
                                }
                                PendingEdit::ClearTransitionOut => {
                                    let c = caprust_core::commands::set_effect::SetTransitionCommand::new(id, false, None);
                                    let _ = self.undo_stack.execute(Box::new(c), &mut self.project);
                                }
                                PendingEdit::CaptionSegmentText { idx, text } => {
                                    let c = caprust_core::commands::edit_caption_segment::EditCaptionSegmentCommand::new(
                                        id, idx, text,
                                    );
                                    let _ = self.undo_stack.execute(Box::new(c), &mut self.project);
                                }
                                PendingEdit::TextStyle(v) => {
                                    let cmd =
                                        caprust_core::commands::set_clip::SetClipCommand::new(id)
                                            .text_style(v);
                                    let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                                }
                                PendingEdit::Name(v) => {
                                    let trimmed = v.trim().to_string();
                                    let new_name = if trimmed.is_empty() {
                                        None
                                    } else {
                                        Some(trimmed)
                                    };
                                    let cmd =
                                        caprust_core::commands::set_clip::SetClipCommand::new(id)
                                            .name(new_name);
                                    let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                                }
                                other => {
                                    let cmd = field_cmd.take().unwrap_or_else(|| {
                                        caprust_core::commands::set_clip::SetClipCommand::new(id)
                                    });
                                    field_cmd = Some(match other {
                                        PendingEdit::Speed(v) => cmd.speed(v),
                                        PendingEdit::Reverse(v) => cmd.reversed(v),
                                        PendingEdit::FlipH(v) => cmd.flip_h(v),
                                        PendingEdit::FlipV(v) => cmd.flip_v(v),
                                        PendingEdit::VolumeDb(v) => cmd.volume_db(v),
                                        PendingEdit::TrimStart(v) => cmd.start_time_ms(v),
                                        PendingEdit::TrimDuration(v) => cmd.duration_ms(v),
                                        PendingEdit::TrackIndex(v) => cmd.track_index(v),
                                        _ => cmd,
                                    });
                                }
                            }
                        }
                        if let Some(cmd) = field_cmd {
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                    }
                }
            });

        // Jobs bar sits above the timeline so it's visible from any
        // panel. It auto-hides when no jobs are live.
        self.show_jobs_bar(ctx);

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

            // ---- Target decode size ----
            let (pw, ph) = self.project.project_dimensions();
            let max_side = match self.preview.quality {
                crate::panels::preview_window::PreviewQuality::Quarter => 320,
                crate::panels::preview_window::PreviewQuality::Half => 480,
                crate::panels::preview_window::PreviewQuality::Full => 640,
            };
            let (tw, th) = caprust_media_io::player::preview_size(pw, ph, max_side);

            // ---- Find clip under playhead on a visible track ----
            let playhead = self.playhead_ms;
            let clip_info: Option<(uuid::Uuid, String, u64, f32)> = self
                .project
                .clips
                .iter()
                .find(|c| {
                    let on_playhead =
                        playhead >= c.start_time_ms && playhead < c.start_time_ms + c.duration_ms;
                    if !on_playhead {
                        return false;
                    }
                    let track_visible = self
                        .project
                        .tracks
                        .get(c.track_index)
                        .map(|t| t.visible)
                        .unwrap_or(true);
                    if !track_visible {
                        return false;
                    }
                    matches!(
                        c.clip_type,
                        caprust_core::ClipType::Video { .. } | caprust_core::ClipType::Image { .. }
                    )
                })
                .and_then(|c| match &c.clip_type {
                    caprust_core::ClipType::Video { path, .. }
                    | caprust_core::ClipType::Image { path, .. } => {
                        Some((c.id, path.clone(), c.start_time_ms, c.speed))
                    }
                    _ => None,
                });

            // ---- Rate-limited state log ----
            {
                use std::sync::atomic::{AtomicU64, Ordering};
                static LAST: AtomicU64 = AtomicU64::new(0);
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                if now >= LAST.load(Ordering::Relaxed) + 5 {
                    LAST.store(now, Ordering::Relaxed);
                    tracing::info!(
                        "preview state: playhead={}ms playing={} clips={} clip_info={} ffmpeg={}",
                        playhead,
                        self.preview.playing,
                        self.project.clips.len(),
                        clip_info.is_some(),
                        self.ffmpeg_status.ffmpeg.is_some(),
                    );
                }
            }

            // ---- Playing vs paused ----
            let playing = self.preview.playing;

            if playing {
                // Ensure the timeline renderer is running.
                let renderer_dead = self.preview_renderer.is_none();
                let explicit_seek = self.explicit_seek_ms.is_some();
                let need_start = renderer_dead || explicit_seek;

                if need_start {
                    // Where to start the renderer from.
                    let start_from = self.explicit_seek_ms.take().unwrap_or(self.playhead_ms);

                    // Kill old one
                    if let Some(mut r) = self.preview_renderer.take() {
                        r.kill();
                    }

                    // Build the same render plan that export uses.
                    let (pw, ph) = self.project.project_dimensions();
                    let max_side = match self.preview.quality {
                        crate::panels::preview_window::PreviewQuality::Quarter => 320,
                        crate::panels::preview_window::PreviewQuality::Half => 480,
                        crate::panels::preview_window::PreviewQuality::Full => 640,
                    };
                    let (rw, rh) = caprust_media_io::player::preview_size(pw, ph, max_side);

                    let (fps_num, fps_den) = self.export_state.frame_rate.fraction(
                        self.project.frame_rate.num as i64,
                        self.project.frame_rate.den as i64,
                    );
                    let fps_f = fps_num as f64 / fps_den.max(1) as f64;

                    if let Some(ffmpeg) = self.ffmpeg_status.ffmpeg.clone() {
                        let models_dir = self.settings.effective_models_dir();
                        match caprust_media_io::export_graph::plan_from_project(
                            &self.project,
                            rw,
                            rh,
                            fps_num,
                            fps_den,
                            23,
                            "veryfast",
                            &models_dir,
                        ) {
                            Ok(plan) => {
                                match PreviewRenderer::spawn(
                                    std::path::Path::new(&ffmpeg),
                                    &plan,
                                    start_from,
                                    rw,
                                    rh,
                                    fps_f,
                                ) {
                                    Ok(renderer) => {
                                        // Re-anchor wall clock so drift during
                                        // renderer startup doesn't push playhead.
                                        self.playback_started_at = Some(std::time::Instant::now());
                                        self.playback_started_ms = start_from;

                                        // Start audio playback from the PCM file
                                        // that this renderer will write. Audio is
                                        // optional: if there's no track, or no
                                        // device, or the file can't be opened,
                                        // video keeps playing silently.
                                        self.audio_player = match renderer.pcm_path.as_ref() {
                                            Some(path) => match AudioPlayer::play_pcm_file(
                                                path, start_from,
                                            ) {
                                                Ok(p) => {
                                                    // Apply user's persisted
                                                    // volume / mute settings
                                                    // to the freshly created
                                                    // stream before playback
                                                    // starts.
                                                    p.set_volume(self.settings.master_volume);
                                                    p.set_muted(self.settings.muted);
                                                    tracing::info!(
                                                        "preview: audio started from {}ms (vol={:.2} muted={})",
                                                        start_from,
                                                        self.settings.master_volume,
                                                        self.settings.muted,
                                                    );
                                                    Some(p)
                                                }
                                                Err(e) => {
                                                    tracing::warn!(
                                                        "preview: audio unavailable: {e}"
                                                    );
                                                    None
                                                }
                                            },
                                            None => None,
                                        };

                                        tracing::info!(
                                            "preview: renderer started from {}ms",
                                            self.playhead_ms
                                        );
                                        self.preview_renderer = Some(renderer);
                                        self.stream_needs_restart = false;
                                    }
                                    Err(e) => {
                                        tracing::error!("preview renderer spawn failed: {e}");
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!("preview plan failed: {e}");
                            }
                        }
                    }
                }

                // Drain ready frames; keep only the last.
                let mut consumed = 0u32;
                let mut latest: Option<Vec<u8>> = None;
                // Audio-priming gate, moved here from the reader thread.
                //
                // ffmpeg needs ~0.5-4 s before its first audio byte hits
                // the PCM file (filtergraph priming + muxer buffering).
                // During that window we do NOT consume video frames: the
                // reader keeps filling the sync_channel at fps, but we
                // hold the display on whatever was last shown. When the
                // audio player finally produces samples, this opens and
                // we start consuming from the FRONT of the buffer, so
                // frame 0 corresponds to audio position 0 ms.
                //
                // If there is no audio track at all (audio_player is
                // None), the gate is trivially open.
                let audio_playing = self
                    .audio_player
                    .as_ref()
                    .is_none_or(|ap| ap.playhead_ms() > 0);
                if audio_playing {
                    if let Some(r) = self.preview_renderer.as_ref() {
                        while let Some(frame) = r.try_next() {
                            latest = Some(frame);
                            consumed += 1;
                            if consumed > 6 {
                                break;
                            }
                        }
                    }
                }

                if let Some(buf) = latest {
                    if let Some(r) = self.preview_renderer.as_ref() {
                        let w = r.width as usize;
                        let h = r.height as usize;
                        let expected = w * h * 4;
                        if buf.len() >= expected {
                            let img =
                                egui::ColorImage::from_rgba_unmultiplied([w, h], &buf[..expected]);
                            let handle = ctx.load_texture(
                                "preview-timeline",
                                img,
                                egui::TextureOptions::LINEAR,
                            );
                            self.preview_player.texture = Some(handle);
                            self.preview_player.has_frame = true;

                            // Re-anchor wall clock to the first consumed
                            // frame of this session. Without this, wall_ms
                            // includes the ~1 s of ffmpeg audio priming and
                            // every sync log line shows a bogus drift.
                            if !self.play_anchor_set {
                                self.playback_started_at = Some(std::time::Instant::now());
                                // Capture audio position now. See field docs.
                                self.audio_baseline_ms = self
                                    .audio_player
                                    .as_ref()
                                    .map_or(0, |ap| ap.playhead_ms());
                                self.play_anchor_set = true;
                                tracing::info!(
                                    "playhead: re-anchored at {}ms (audio_baseline={}ms)",
                                    self.playhead_ms,
                                    self.audio_baseline_ms
                                );
                            }

                        }
                        // Advance playhead — audio-master when audio is
                        // running, wall-clock fallback otherwise (§21).
                        let wall_ms = self
                            .playback_started_at
                            .map(|t0| {
                                self.playback_started_ms
                                    + t0.elapsed().as_millis() as u64
                            })
                            .unwrap_or(self.playhead_ms);

                        // Playhead source: the wall clock is the
                        // reference; the audio sample counter only serves
                        // to SLOW US DOWN when audio is falling behind.
                        //
                        // Why not let audio lead: on Windows WASAPI the
                        // cpal callback can fire slightly more often than
                        // the nominal rate. Over 60 s we measured a
                        // steady +19 ms/s error, i.e. the sample counter
                        // ran ~1.9% fast. That produced a +1157 ms
                        // playhead-ahead-of-wall over one minute, which
                        // is a real, visible desync (video led audio).
                        //
                        // Capping at wall_ms eliminates that class of
                        // drift entirely: if audio is late we freeze the
                        // playhead (correct), if audio is early we ignore
                        // it (correct). Sample counter accuracy no longer
                        // matters for absolute position, only for the
                        // relative "is audio behind" signal.
                        let (new_ph, src_tag) = match self.audio_player.as_ref() {
                            Some(ap) => {
                                let audio_ms = self.playback_started_ms
                                    + ap.playhead_ms()
                                        .saturating_sub(self.audio_baseline_ms);
                                if audio_ms < wall_ms {
                                    (audio_ms, "audio")
                                } else {
                                    (wall_ms, "wall")
                                }
                            }
                            None => (wall_ms, "wall"),
                        };

                        let underruns = self
                            .audio_player
                            .as_ref()
                            .map_or(0, |ap| ap.underruns());
                        tracing::debug!(
                            "sync: playhead={}ms audio={:?}ms wall={}ms drift={}ms src={} underruns={}",
                            new_ph,
                            self.audio_player.as_ref().map(|ap| ap.playhead_ms()),
                            wall_ms,
                            new_ph as i64 - wall_ms as i64,
                            src_tag,
                            underruns
                        );

                        self.playhead_ms = new_ph;
                    }
                }

                ctx.request_repaint();
            } else {
                // Paused: kill any renderer, do a seek-based single-frame decode
                if let Some(mut r) = self.preview_renderer.take() {
                    r.kill();
                }
                self.preview_player.stop_stream();
                if let (Some(ffmpeg), Some((clip_id, path, clip_start, speed))) =
                    (self.ffmpeg_status.ffmpeg.clone(), clip_info.clone())
                {
                    let source_ms = ((playhead.saturating_sub(clip_start)) as f32 * speed) as u64;
                    self.preview_player.request(
                        std::path::Path::new(&ffmpeg),
                        std::path::Path::new(&path),
                        clip_id,
                        source_ms,
                        tw,
                        th,
                    );
                }
                self.preview_player.poll(ctx);
                if self.preview_player.pending.is_some() {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                }
            }

            // ---- Render frame or placeholder ----
            if let Some(tex) = self.preview_player.texture.as_ref() {
                let tex_size = tex.size_vec2();
                let avail_w = rect.width() - 16.0;
                let avail_h = rect.height() - 16.0;
                let scale = (avail_w / tex_size.x).min(avail_h / tex_size.y);
                let draw_size = tex_size * scale;
                let draw_rect = egui::Rect::from_center_size(rect.center(), draw_size);
                ui.painter().image(
                    tex.id(),
                    draw_rect,
                    egui::Rect::from_min_max(egui::Pos2::new(0.0, 0.0), egui::Pos2::new(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            } else {
                let msg = if self.ffmpeg_status.ffmpeg.is_none() {
                    "FFmpeg not detected — set it in Settings → Paths"
                } else if clip_info.is_none() {
                    if self.project.clips.is_empty() {
                        "No clips on timeline"
                    } else {
                        "Playhead is not over a video clip"
                    }
                } else {
                    "Decoding…"
                };
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "🎬 Preview",
                    egui::FontId::proportional(22.0),
                    egui::Color32::from_gray(90),
                );
                ui.painter().text(
                    rect.center() + egui::vec2(0.0, 26.0),
                    egui::Align2::CENTER_CENTER,
                    msg,
                    egui::FontId::proportional(12.0),
                    egui::Color32::from_gray(120),
                );
            }

            ui.add_space(6.0);

            // ---- Transport bar ----
            let ev = crate::panels::preview_window::show_transport(
                ui,
                &mut self.preview,
                self.playhead_ms,
                total_ms,
                &mut self.project.aspect_ratio,
                self.settings.muted,
                self.settings.master_volume,
            );
            self.handle_preview_events(ev, total_ms);

            // ---- End-of-timeline ----
            if self.preview.playing && total_ms > 0 && self.playhead_ms >= total_ms {
                if self.preview.loop_playback {
                    self.playhead_ms = 0;
                    // Force renderer restart from t=0.
                    if let Some(mut r) = self.preview_renderer.take() {
                        r.kill();
                    }
                    self.explicit_seek_ms = Some(0);
                    self.playback_started_at = Some(std::time::Instant::now());
                    self.playback_started_ms = 0;
                } else {
                    self.playhead_ms = total_ms;
                    self.preview.playing = false;
                    self.preview_player.stop_stream();
                    if let Some(mut r) = self.preview_renderer.take() {
                        r.kill();
                    }
                }
            }
        });
    }

    fn show_export_window(&mut self, ctx: &egui::Context) {
        let total_ms = self.total_duration_ms();
        let mut open = self.export_open;
        let mut start_clicked = false;

        egui::Window::new(tr("exp-title"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_width(440.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                let clip_count = self.project.clips.len();
                start_clicked = crate::panels::export_window::show(
                    ui,
                    &mut self.export_state,
                    total_ms,
                    clip_count,
                );

                // Progress section
                if self.export_in_progress {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.label(
                        egui::RichText::new("Exporting…")
                            .strong()
                            .color(egui::Color32::from_rgb(120, 180, 240)),
                    );
                    ui.add(
                        egui::ProgressBar::new(self.export_progress)
                            .desired_width(ui.available_width())
                            .show_percentage(),
                    );
                    ctx.request_repaint();
                }

                // Finished section
                if let Some(path) = self.export_finished_path.clone() {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!("✓ Done: {path}"))
                            .color(egui::Color32::from_rgb(120, 220, 120)),
                    );
                    if ui.button("📂 Open folder").clicked() {
                        caprust_media_io::exporter::reveal_in_folder(std::path::Path::new(&path));
                    }
                    if ui.button("Dismiss").clicked() {
                        self.export_finished_path = None;
                    }
                }
            });
        self.export_open = open;

        if start_clicked && !self.export_in_progress {
            self.start_export();
        }
    }

    /// Modal for renaming the currently-selected track. Opened from
    /// the track header context menu.
    fn show_track_rename_window(&mut self, ctx: &egui::Context) {
        let Some((idx, mut buf)) = self.track_rename.clone() else {
            return;
        };

        let mut commit = false;
        let mut cancel = false;

        egui::Window::new(tr("tk-rename-title"))
            .id(egui::Id::new("track_rename_modal"))
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(360.0)
            .show(ctx, |ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut buf)
                        .desired_width(f32::INFINITY)
                        .hint_text("Track name"),
                );
                resp.request_focus();
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    commit = true;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let ok = egui::Button::new(
                        egui::RichText::new(tr("tk-rename-ok"))
                            .color(egui::Color32::WHITE)
                            .strong(),
                    )
                    .fill(egui::Color32::from_rgb(34, 139, 230));
                    if ui.add(ok).clicked() {
                        commit = true;
                    }
                    if ui.button(tr("tk-rename-cancel")).clicked() {
                        cancel = true;
                    }
                });
            });

        if cancel {
            self.track_rename = None;
            return;
        }
        if commit {
            let trimmed = buf.trim().to_string();
            if !trimmed.is_empty() {
                if let Some(t) = self.project.tracks.get_mut(idx) {
                    t.name = trimmed;
                }
                tracing::info!("renamed track {idx}");
            }
            self.track_rename = None;
        } else {
            self.track_rename = Some((idx, buf));
        }
    }

    fn show_model_prompt_window(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.model_prompt else {
            return;
        };

        // Advance fake downloads while this dialog is up.
        self.project.models.tick_downloads(1.0 / 60.0);

        let title = match kind {
            caprust_core::ModelKind::Caption => tr("mp-captions-title"),
            caprust_core::ModelKind::Narration => tr("mp-narration-title"),
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
                                            egui::RichText::new(tr("mp-use-this"))
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
                    if ui.button(tr("mp-cancel")).clicked() {
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

        if let Some((model_id, _lang)) = chosen {
            // Do NOT insert a placeholder clip here. Clicking "Use" on a
            // model is a signal to run the actual job; the job's drain
            // path is the only place that ever inserts a Captions or
            // Narration clip, and it inserts real content (segments or a
            // rendered WAV). Inserting an empty clip from the prompt was
            // the source of phantom zero-segment Captions clips on
            // track 0 every time a user picked a model.
            //
            // Also: give the model registry a chance to see whether the
            // weights are actually on disk before dispatching. A model
            // marked Ready in the JSON but missing from disk would
            // otherwise dispatch and fail silently.
            let models_dir = self.settings.effective_models_dir();
            self.project.models.scan_local(&models_dir);

            self.model_prompt = None;
            match kind {
                caprust_core::ModelKind::Caption => {
                    // start_caption_job re-checks readiness; if still not
                    // ready it re-opens this prompt, which is the desired
                    // behaviour when a download has not actually happened.
                    self.start_caption_job(None);
                }
                caprust_core::ModelKind::Narration => {
                    if self.project.models.ready_narration().is_empty() {
                        // Re-open prompt; nothing to narrate with yet.
                        self.model_prompt = Some(caprust_core::ModelKind::Narration);
                    } else {
                        self.narration_input.open = true;
                    }
                }
            }
            let _ = model_id; // reserved for future "pin this model" behaviour
        }
    }

    /// Enqueue probe+thumbnail jobs for any media that has no thumbnail
    /// on disk. Call after loading a project and after cache clear.
    fn regen_missing_thumbnails(&mut self) {
        let Some(proj_path) = self.project.project_path.clone() else {
            return;
        };
        if !self.ffmpeg_status.is_available() {
            tracing::warn!("regen skipped: ffmpeg/ffprobe not detected");
            return;
        }
        let mut count = 0usize;
        for item in &self.project.media.items {
            if !matches!(
                item.kind,
                caprust_core::MediaKind::Video | caprust_core::MediaKind::Image
            ) {
                continue;
            }
            let jpg =
                caprust_core::cache::thumbnail_path(std::path::Path::new(&proj_path), item.id);
            if !jpg.is_file() {
                let item_clone = item.clone();
                self.job_runner.enqueue(&item_clone);
                count += 1;
            }
        }
        if count > 0 {
            tracing::info!("regen: enqueued {count} missing thumbnails");
        }
    }

    fn start_export(&mut self) {
        let Some(ffmpeg) = self.ffmpeg_status.ffmpeg.clone() else {
            tracing::error!("export: ffmpeg not detected");
            return;
        };

        // Resolve target size from export state.
        let (pw, ph) = self.project.project_dimensions();
        let (w, h) = self.export_state.resolution.dimensions(pw, ph);
        let (fps_num, fps_den) = self.export_state.frame_rate.fraction(
            self.project.frame_rate.num as i64,
            self.project.frame_rate.den as i64,
        );

        let crf = match self.export_state.quality {
            crate::panels::export_window::QualityTier::Small => 26,
            crate::panels::export_window::QualityTier::Regular => 20,
            crate::panels::export_window::QualityTier::Large => 16,
        };

        let models_dir = self.settings.effective_models_dir();
        let plan = match caprust_media_io::export_graph::plan_from_project(
            &self.project,
            w,
            h,
            fps_num,
            fps_den,
            crf,
            "veryfast",
            &models_dir,
        ) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("export plan failed: {e}");
                return;
            }
        };

        // Output path: <destination>/<project-name>.mp4
        let mut out = std::path::PathBuf::from(&self.export_state.destination);
        out.push(format!("{}.mp4", self.project.name.replace(' ', "_")));

        tracing::info!(
            "starting export: {} → {} ({}x{} @ {}/{})",
            plan.inputs.len(),
            out.display(),
            w,
            h,
            fps_num,
            fps_den,
        );

        let rx =
            caprust_media_io::exporter::spawn_export(std::path::PathBuf::from(ffmpeg), plan, out);
        let job_id = self.begin_job(JobKind::Export, tr("job-export"));
        self.export_job_id = Some(job_id);
        self.export_rx = Some(rx);
        self.export_in_progress = true;
        self.export_progress = 0.0;
        self.export_finished_path = None;
    }

    fn poll_export(&mut self) {
        // Drain all pending events into a local Vec first, then process
        // them. Holding `&Receiver` across the loop would conflict with
        // the `&mut self` calls (finish_job) that the event handlers
        // need.
        let mut events: Vec<ExportEvent> = Vec::new();
        {
            let Some(rx) = self.export_rx.as_ref() else {
                return;
            };
            while let Ok(ev) = rx.try_recv() {
                events.push(ev);
            }
        }
        for ev in events {
            match ev {
                ExportEvent::Started => {
                    tracing::info!("export: ffmpeg started");
                }
                ExportEvent::Progress(p) => {
                    if let Some(id) = self.export_job_id {
                        self.update_job_progress(id, p);
                    }
                    self.export_progress = p;
                }
                ExportEvent::Log(line) => {
                    tracing::info!("export log: {line}");
                }
                ExportEvent::Finished { output } => {
                    tracing::info!("export: finished → {}", output.display());
                    if let Some(id) = self.export_job_id.take() {
                        self.finish_job(id);
                    }
                    self.export_in_progress = false;
                    self.export_finished_path = Some(output.to_string_lossy().to_string());
                }
                ExportEvent::Failed(msg) => {
                    tracing::error!("export failed: {msg}");
                    if let Some(id) = self.export_job_id.take() {
                        self.finish_job(id);
                    }
                    self.export_in_progress = false;
                }
            }
        }
    }

    fn show_settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new(tr("set-title"))
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
    GenerateCaptions(uuid::Uuid),
    SeparateAudio(uuid::Uuid),
    Copy(uuid::Uuid),
    Paste,
    Duplicate(uuid::Uuid),
    RippleDelete(uuid::Uuid),
    SetSpeed(uuid::Uuid, f32),
    MuteClip(uuid::Uuid),
}

impl eframe::App for CapRustApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        caprust_i18n::set_current_lang(&self.settings.language);
        self.theme.apply(ctx);

        // Poll export events.
        self.poll_export();

        // Drain background jobs (ffprobe results, thumbnails ready).
        self.drain_update_check();
        self.drain_caption_job();
        self.drain_narration_job();
        let thumbs_ready = self.job_runner.drain(&mut self.project);

        // Load any newly-ready thumbnails into the timeline texture cache.
        if let (Some(proj_path), false) =
            (self.project.project_path.clone(), thumbs_ready.is_empty())
        {
            for media_id in thumbs_ready {
                if self.clip_textures.contains_key(&media_id) {
                    continue;
                }
                let jpg =
                    caprust_core::cache::thumbnail_path(std::path::Path::new(&proj_path), media_id);
                if let Ok(bytes) = std::fs::read(&jpg) {
                    if let Ok(img) = image::load_from_memory(&bytes) {
                        let rgba = img.to_rgba8();
                        let size = [rgba.width() as usize, rgba.height() as usize];
                        let color_img =
                            egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                        let handle = ctx.load_texture(
                            format!("clip-{media_id}"),
                            color_img,
                            egui::TextureOptions::LINEAR,
                        );
                        self.clip_textures.insert(media_id, handle);
                    }
                }
            }
        }

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
            // Don't hijack keys while typing in a text field.
            let typing = ctx.wants_keyboard_input();

            for k in events {
                if typing {
                    continue;
                }
                // Re-check per key — Settings may have been toggled mid-frame.
                if !self.settings.enable_shortcuts {
                    break;
                }
                match k {
                    egui::Key::R => {
                        tracing::info!(
                            "Key R pressed (shortcuts={})",
                            self.settings.enable_shortcuts
                        );
                        for id in self.selected_clips.clone() {
                            let cur = self
                                .project
                                .clips
                                .iter()
                                .find(|c| c.id == id)
                                .map(|c| c.reversed)
                                .unwrap_or(false);
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .reversed(!cur);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                    }
                    egui::Key::H => {
                        for id in self.selected_clips.clone() {
                            let cur = self
                                .project
                                .clips
                                .iter()
                                .find(|c| c.id == id)
                                .map(|c| c.flip_h)
                                .unwrap_or(false);
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .flip_h(!cur);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                    }
                    egui::Key::V => {
                        for id in self.selected_clips.clone() {
                            let cur = self
                                .project
                                .clips
                                .iter()
                                .find(|c| c.id == id)
                                .map(|c| c.flip_v)
                                .unwrap_or(false);
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .flip_v(!cur);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
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
        if self.track_rename.is_some() {
            self.show_track_rename_window(ctx);
        }
        self.show_toasts(ctx);
        self.show_update_toast(ctx);
        if self.narration_input.open {
            let n_ev = crate::panels::narration_input::show(
                ctx,
                &mut self.narration_input,
                &self.project.models,
            );
            if let Some((voice_id, text)) = n_ev.synthesize {
                self.start_narration_job(voice_id, text);
                self.narration_input.open = false;
            }
            if n_ev.closed {
                self.narration_input.open = false;
            }
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

/// Register the Phosphor icon font family.
fn setup_phosphor_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
}
