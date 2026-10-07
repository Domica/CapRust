use crate::i18n_helper::tr;
use crate::media_jobs::JobRunner;
use crate::panels::asset_browser::AssetBrowserState;
use crate::panels::clip_properties::{PendingEdit, PropertiesState};
use crate::panels::export_window::ExportState;
use crate::panels::media_bin::{MediaBinState, PreviewSize};
use crate::panels::preview_window::{PreviewEvents, PreviewState};
use crate::preview_player::PreviewPlayer;
use crate::theme::tokens::{anim, elev, radius, space, text};
use crate::theme::Theme;
use crate::timeline::{TimelineToolEvents, TimelineToolState};
use crate::widgets::{button, empty, section, segmented_control};
use caprust_core::commands::delete_clip::DeleteClipCommand;
use caprust_core::commands::move_clip::MoveClipCommand;
use caprust_core::commands::split_clip::SplitClipCommand;
use caprust_core::recent::{RecentList, RecentProject};
use caprust_core::settings::AppSettings;
use caprust_core::ClipType;
use caprust_core::{AspectRatio, Clip, FrameRate, ProjectState, UndoStack};
use caprust_media_io::audio_player::AudioPlayer;
use caprust_media_io::export::RateMode;
use caprust_media_io::exporter::ExportEvent;
use caprust_media_io::preview_render::PreviewRenderer;
use eframe::egui;
use egui_phosphor::regular as ph;

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

/// Severity of a toast. Error toasts get a red accent and read as
/// failures; Info is the default for confirmations and status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Error,
}

/// Lightweight transient notification shown in the top-right corner.
/// Auto-dismisses after `duration`; the user can also dismiss early
/// via the ✕ button. Multiple toasts stack vertically.
pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub created_at: std::time::Instant,
    pub duration: std::time::Duration,
}

pub struct AudioPendingRender {
    /// Hash the pending render was spawned for.
    pub hash: u64,
    /// Destination path (`<hash>.pcm`). The job writes to `.part`
    /// and renames on success.
    pub path: std::path::PathBuf,
    pub job: caprust_media_io::audio_render::AudioRenderJob,
}

/// State for the pre-rendered audio PCM cache.
///
/// `active_*` describe the currently playable file (may be an older
/// hash while a newer render is in flight). `pending` is the render
/// for a newer hash; when it finishes, it replaces `active_*`.
#[derive(Default)]
pub struct AudioCacheState {
    /// Hash of the project state the active file was rendered
    /// against. 0 means "no active file yet".
    pub active_hash: u64,
    /// Playable PCM file path, or `None` when no render has succeeded.
    pub active_path: Option<std::path::PathBuf>,
    /// In-flight render for a newer hash.
    pub pending: Option<AudioPendingRender>,
    /// Set when `audio_render_hash()` diverges from `active_hash`.
    /// Cleared when the debounce window elapses and a render fires.
    pub debounce_at: Option<std::time::Instant>,
    /// Hash of a render that failed. Prevents the poll loop from
    /// retrying the same state every 500 ms. Cleared when the hash
    /// changes (user edit) so a fixed render can be tried again.
    pub last_failed_hash: u64,
}

impl Toast {
    /// Info toast: confirmations, status updates.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: ToastKind::Info,
            created_at: std::time::Instant::now(),
            duration: std::time::Duration::from_secs(4),
        }
    }

    /// Error toast: a user-visible failure. Lingers 1.5x longer
    /// than Info so the user has a chance to read it.
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: ToastKind::Error,
            created_at: std::time::Instant::now(),
            duration: std::time::Duration::from_secs(6),
        }
    }

    /// Animation state at `now`: `(alpha, slide_dx)`.
    ///
    /// `alpha` ramps 0..1 over `anim::FAST` at the start and 1..0 over
    /// `anim::SLOW` before the toast expires. `slide_dx` is a horizontal
    /// offset that starts at `space::M` and eases to 0 as the toast
    /// fades in.
    pub fn anim_at(&self, now: std::time::Instant) -> (f32, f32) {
        let t = now.duration_since(self.created_at).as_secs_f32();
        let life = self.duration.as_secs_f32();
        let fade_in = (t / anim::FAST).clamp(0.0, 1.0);
        let fade_out = ((life - t) / anim::SLOW).clamp(0.0, 1.0);
        let alpha = fade_in.min(fade_out);
        let dx = (1.0 - ease_out_cubic(fade_in)) * space::M;
        (alpha, dx)
    }
}

/// Cubic ease-out: fast start, slow end. `t` clamped to `0.0..=1.0`.
fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    1.0 - inv * inv * inv
}

#[cfg(test)]
mod toast_anim_tests {
    use super::ease_out_cubic;

    #[test]
    fn ease_out_cubic_endpoints() {
        assert!((ease_out_cubic(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((ease_out_cubic(1.0) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn ease_out_cubic_midpoint() {
        assert!((ease_out_cubic(0.5) - 0.875).abs() < 1e-6);
    }

    #[test]
    fn ease_out_cubic_monotonic() {
        let mut prev = ease_out_cubic(0.0);
        for i in 1..=10 {
            let v = ease_out_cubic(i as f32 / 10.0);
            assert!(v >= prev, "not monotonic at {i}: {prev} -> {v}");
            prev = v;
        }
    }
}

/// Kinds of long-running jobs the jobs bar can display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Caption,
    Narration,
    Export,
    /// Auto-reframe analysis (Phase P2c-3b): frame extraction + YuNet
    /// detection + keypoint computation.
    Reframe,
    /// Background removal (Phase P3c): per-frame u2netp inference,
    /// mask written as FFV1 MKV into the project cache.
    BgRemoval,
    /// Beat analysis: onset-flux BPM + markers for an audio clip.
    Beat,
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

/// Cached audio-envelope overlay data for one clip.
/// Invalidated when `ProjectState::render_hash()` changes.
pub struct EnvelopeCacheEntry {
    pub hash: u64,
    pub samples_db: std::sync::Arc<Vec<crate::timeline::EnvelopeSample>>,
    pub duck_zones: std::sync::Arc<Vec<crate::timeline::DuckZone>>,
}

pub struct CapRustApp {
    pub mode: AppMode,
    pub project: ProjectState,
    pub undo_stack: UndoStack,
    pub draft: NewProjectDraft,
    pub theme: Theme,
    pub settings_open: bool,
    /// Start-screen New Project modal. Opened by the sidebar + button;
    /// closed on Create or Cancel.
    pub new_project_modal_open: bool,
    pub export_open: bool,
    pub export_state: ExportState,
    /// Hardware encoders confirmed to work at runtime.
    pub available_encoders: Option<Vec<caprust_core::project::VideoEncoder>>,
    pub media_bin: MediaBinState,
    pub preview_size: PreviewSize,
    pub timeline_tools: TimelineToolState,
    pub playhead_ms: u64,
    pub timeline_zoom: f32,
    pub settings_tab: crate::panels::settings_dialog::SettingsTab,
    /// First-run FFmpeg prompt state. Opened on startup when no
    /// ffmpeg/ffprobe is on PATH and not previously dismissed. See
    /// `show_ffmpeg_prompt_window`.
    pub ffmpeg_prompt: crate::panels::ffmpeg_prompt::FfmpegPromptState,
    pub ffmpeg_prompt_open: bool,
    /// True after the auto-open check has run once this session, so
    /// the modal is only offered at most once per launch.
    pub ffmpeg_prompt_checked: bool,
    /// Some((path, exported_at)) when the sync folder holds a newer
    /// snapshot than this machine has acknowledged. The prompt modal
    /// offers Load / Keep local. Set on startup and cleared when the
    /// user picks one.
    pub settings_sync_prompt: Option<(std::path::PathBuf, u64)>,
    /// True after the sync-newer check has run once this session.
    pub settings_sync_prompt_checked: bool,
    /// Hash of the last settings snapshot written to the sync folder.
    /// Guards against redundant writes when eframe calls save()
    /// repeatedly with identical state.
    pub last_sync_hash: Option<u64>,
    /// The font family currently pushed into egui. Compared each
    /// frame so a change in Settings -> Appearance rebuilds the font
    /// map exactly once.
    pub last_applied_font_family: Option<crate::theme::UiFontFamily>,
    /// Missing-media relink dialog. Opened by `check_missing_media_on_load`
    /// when the media library references files that are not on disk.
    pub relink_dialog: crate::panels::relink_dialog::RelinkDialogState,
    pub relink_dialog_open: bool,
    /// Set when the user dismisses the missing-media banner. Reset
    /// whenever the banner would show and there is nothing missing,
    /// so a future regression re-opens it.
    pub missing_media_dismissed: bool,
    #[cfg(windows)]
    pub screen_record: crate::panels::screen_record::ScreenRecordState,
    #[cfg(windows)]
    pub screen_record_open: bool,
    /// Dock tree that owns every editor panel. Replaces the fixed
    /// SidePanel / CentralPanel layout. See `crate::dock`.
    pub dock_state: egui_dock::DockState<crate::dock::Tab>,
    pub preview: PreviewState,
    pub selected_clips: Vec<uuid::Uuid>,
    pub clip_drag: Option<ClipDrag>,
    /// Rubber-band selection in progress, if any.
    pub marquee: Option<MarqueeState>,
    /// In-progress fade handle drag, if any.
    pub fade_drag: Option<FadeDrag>,
    pub settings: AppSettings,
    pub ffmpeg_status: caprust_core::FfmpegStatus,
    pub last_dnd_payload: Option<Vec<uuid::Uuid>>,
    pub recent: RecentList,
    /// Cache of decoded recent-project thumbnails, keyed by project
    /// path. Lazily populated by `recent_thumb_texture`.
    pub recent_thumb_cache: std::collections::HashMap<String, egui::TextureHandle>,
    /// Cache of waveform peak arrays, keyed by media_id. Loaded
    /// lazily from `<project>/cache/waveforms/<id>.bin` the first
    /// time a clip referencing that item needs to draw.
    pub waveform_cache: std::collections::HashMap<uuid::Uuid, std::sync::Arc<Vec<f32>>>,
    /// Cached per-clip audio envelope samples + duck zones.
    /// Keyed by clip id, invalidated when `render_hash()` changes.
    pub envelope_cache: std::collections::HashMap<uuid::Uuid, EnvelopeCacheEntry>,
    pub last_pointer: Option<egui::Pos2>,
    /// Cached track row geometry from the last frame: (top_y, [(track_idx, height)]).
    pub timeline_row_layout: (f32, Vec<(usize, f32)>),
    pub properties: PropertiesState,
    pub master_chain: crate::panels::master_chain::MasterChainState,
    pub multicam: crate::panels::multicam::MultiCamState,
    pub model_prompt: Option<caprust_core::ModelKind>,
    /// Active tab inside the model prompt window. Lets the user switch
    /// between Caption and Narration model lists without closing and
    /// re-opening the window (they are otherwise a single combined
    /// dialog). Not persisted; reset to the initial kind every time the
    /// prompt is opened.
    pub model_prompt_tab: caprust_core::ModelKind,
    /// Active model download: (model_id, receiver). Only one download
    /// at a time for now; starting a second cancels the first by
    /// dropping the receiver.
    pub model_download: Option<(
        String,
        std::sync::mpsc::Receiver<caprust_core::models::DownloadEvent>,
    )>,
    /// Receiver for an in-flight caption transcription. When Some, the
    /// timeline toolbar shows a "Transcribing…" label and the toolbar
    /// click is ignored until the job finishes.
    pub caption_rx:
        Option<std::sync::mpsc::Receiver<Result<crate::media_jobs::CaptionResult, String>>>,
    /// Receiver for an in-flight captions translation. Only one at a
    /// time. Cleared on Done or Failed.
    pub translate_rx: Option<std::sync::mpsc::Receiver<caprust_core::translate::TranslateEvent>>,
    /// The source clip id that translate_rx is working on. Needed at
    /// Done time to build the TranslateCaptionsCommand.
    pub translate_source_clip: Option<uuid::Uuid>,
    /// Last progress value emitted by translate_rx (done, total).
    /// Used to throttle the toast so we do not spam once per segment.
    pub translate_last_progress: Option<(usize, usize)>,
    /// Wall-clock time the current caption job started, for the elapsed
    /// seconds indicator.
    pub caption_job_started: Option<std::time::Instant>,
    /// Receiver for an in-flight narration synthesis. Same lifecycle as
    /// caption_rx: Some while the job runs, None otherwise.
    pub narration_rx:
        Option<std::sync::mpsc::Receiver<Result<crate::media_jobs::NarrationResult, String>>>,
    /// Receiver for an in-flight auto-reframe analysis (Phase P2c-3b).
    pub reframe_rx:
        Option<std::sync::mpsc::Receiver<Result<crate::media_jobs::ReframeResult, String>>>,
    /// Job id for the auto-reframe bar entry.
    pub reframe_job_id: Option<u64>,
    /// Receiver for an in-flight background-removal job (Phase P3c).
    /// Emits Started / Progress / Finished / Failed events.
    pub bg_removal_rx: Option<std::sync::mpsc::Receiver<crate::media_jobs::BgRemovalEvent>>,
    /// Job id for the background-removal bar entry.
    pub bg_removal_job_id: Option<u64>,
    /// Receiver for an in-flight beat analysis. Single result message.
    pub beat_rx: Option<std::sync::mpsc::Receiver<Result<crate::media_jobs::BeatResult, String>>>,
    /// Job id for the beat bar entry.
    pub beat_job_id: Option<u64>,
    /// Relative mask path (e.g. "cache/masks/<clip>.mkv") for the
    /// in-flight background-removal job. Stored here so the drain can
    /// write it into the clip without recomputing or consulting the
    /// filesystem.
    pub bg_removal_rel_path: Option<String>,
    /// Modal state for entering narration text.
    pub narration_input: crate::panels::narration_input::NarrationInputState,
    /// Result of the last update check, if a newer version was found.
    /// Some(..) => show the toast; None => nothing to notify.
    pub update_available: Option<caprust_core::update_checker::UpdateInfo>,
    /// Transient notifications shown top-right. Expired entries are
    /// pruned each frame; the user can dismiss early with the ✕ button.
    pub toasts: Vec<Toast>,
    /// Last count of clips the render planner skipped because their
    /// source file was missing. Debounces the toast so the same value
    /// does not spam every frame the hash changes.
    pub last_skipped_missing: usize,
    /// In-flight background jobs (caption, narration, export). Rendered
    /// by show_jobs_bar above the timeline while any are live.
    pub jobs: Vec<BackgroundJob>,
    pub next_job_id: u64,
    /// Pre-rendered audio PCM cache. Lives in %TEMP%, invalidated
    /// by ProjectState::audio_render_hash. See audio_render.rs.
    pub audio_cache: AudioCacheState,
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
    /// Deferred audio start. Preview with a transition takes 1-3 s
    /// to deliver the first frame (xfade forces ffmpeg to decode both
    /// clips from their beginning). If audio starts on Play click it
    /// is already that far ahead by the time the playhead anchors,
    /// producing drift equal to the render delay. Store the pending
    /// (pcm path, offset, using_cache) here on Play and start it from
    /// the re-anchor block instead.
    pub pending_audio_start: Option<(std::path::PathBuf, u64, bool)>,
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
    /// Wall-clock tracker for the in-flight export. Some while an
    /// export is running; None otherwise. Powers the ETA and elapsed
    /// labels under the progress bar.
    pub export_tracker: Option<caprust_media_io::export_progress::ExportProgressTracker>,
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
    /// Hash of the render-relevant project state at the moment the
    /// current PreviewRenderer was spawned. When the live project
    /// hashes differently, the renderer is respawned at the current
    /// playhead so effect / transition / speed / volume edits are
    /// visible without a manual seek. See ProjectState::render_hash.
    pub preview_plan_hash: u64,
    /// Set when the render hash changed while the preview was paused.
    /// The paused branch consumes it: it runs a one-shot full-plan
    /// render at the current playhead so the user immediately sees
    /// their edit instead of the last decoded frame (K1c).
    pub paused_frame_dirty: bool,
    /// Deadline for the paused one-shot renderer. When now() passes it
    /// without a frame arriving, the renderer is killed and the fallback
    /// direct-extract path takes over. None = no one-shot in flight.
    pub paused_renderer_deadline: Option<std::time::Instant>,
    /// In-flight text overlay drag from the preview pane. Some while
    /// the pointer is down over a TextOverlay bounding box; dropped on
    /// release after pushing a single SetClipCommand::text_motion so
    /// the whole gesture is one undoable step.
    pub text_overlay_drag: Option<TextOverlayDrag>,
    /// Last hash seen during a burst of render-relevant changes. Used
    /// with `pending_respawn_at` to debounce slider drags so that the
    /// renderer is not respawned once per frame.
    pub pending_hash: u64,
    /// Instant when `pending_hash` last changed. Cleared when the
    /// debounce window elapses and the renderer is respawned.
    pub pending_respawn_at: Option<std::time::Instant>,
    pub job_runner: JobRunner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FadeEdge {
    In,
    Out,
}

/// In-progress fade handle drag. Handles are small circles at the top
/// corners of any clip that carries audio (Audio and Video). Dragging
/// horizontally changes fade_in_ms (left handle) or fade_out_ms (right
/// handle). The change is committed as a SetClipCommand on release, so
/// undo/redo works normally.
#[derive(Debug, Clone)]
pub struct FadeDrag {
    pub clip_id: uuid::Uuid,
    pub edge: FadeEdge,
    pub origin_ms: u64,
    pub current_ms: u64,
    pub origin_ptr_x: f32,
    /// Clip duration, for the upper clamp.
    pub duration_ms: u64,
    /// The opposite edge's existing fade duration, for combined-clamp.
    pub other_fade_ms: u64,
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
    /// Every clip in the multi-select group this drag belongs to,
    /// together with its original (start_ms, track_index). Populated
    /// at DragStart when the dragged clip is part of a multi-select.
    /// Empty for a single-clip drag.
    pub group: Vec<(uuid::Uuid, u64, usize)>,
    pub origin_ms: u64,
    pub current_ms: i64,
    /// Track the drag started on. `track_index` is updated every
    /// frame to the track under the pointer; this one stays fixed so
    /// the release handler can compute the track delta for the
    /// whole group.
    pub origin_track: usize,
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
    /// Raw pointer delta in ms, unbounded. `current_ms` is clamped
    /// to >= 0 (so the LEFT trim cannot push the clip before the
    /// timeline start), which is wrong for the RIGHT trim: a long
    /// drag past 0 on the timeline shrinks by more than the clip's
    /// start offset, and using the clamped value caps the trim at
    /// `origin_ms`. Store the raw delta so the RIGHT trim math can
    /// use the actual pointer movement.
    pub raw_delta_ms: i64,
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
        let theme: Theme = cc
            .storage
            .and_then(|s| s.get_string("theme"))
            .and_then(|s| serde_json::from_str::<Theme>(&s).ok())
            .unwrap_or_default();
        setup_phosphor_fonts(&cc.egui_ctx, theme.font_family);
        cc.egui_ctx.set_pixels_per_point(theme.font_scale);
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
            new_project_modal_open: false,
            export_open: false,
            export_state: ExportState::default(),
            available_encoders: None,
            media_bin: MediaBinState::default(),
            preview_size: PreviewSize::Medium,
            timeline_tools: TimelineToolState::default(),
            playhead_ms: 0,
            timeline_zoom: 1.0,
            settings_tab: Default::default(),
            ffmpeg_prompt: Default::default(),
            ffmpeg_prompt_open: false,
            ffmpeg_prompt_checked: false,
            settings_sync_prompt: None,
            settings_sync_prompt_checked: false,
            last_sync_hash: None,
            last_applied_font_family: None,
            relink_dialog: Default::default(),
            relink_dialog_open: false,
            missing_media_dismissed: false,
            #[cfg(windows)]
            screen_record: crate::panels::screen_record::ScreenRecordState::new_defaults(),
            #[cfg(windows)]
            screen_record_open: false,
            dock_state: settings
                .dock_layout
                .as_ref()
                .and_then(|json| {
                    serde_json::from_str::<egui_dock::DockState<crate::dock::Tab>>(json).ok()
                })
                .unwrap_or_else(crate::dock::default_dock_state),
            preview: PreviewState::default(),
            selected_clips: Vec::new(),
            clip_drag: None,
            marquee: None,
            fade_drag: None,
            settings,
            ffmpeg_status: ffmpeg_status.clone(),
            last_dnd_payload: None,
            recent,
            recent_thumb_cache: std::collections::HashMap::new(),
            waveform_cache: std::collections::HashMap::new(),
            envelope_cache: std::collections::HashMap::new(),
            last_pointer: None,
            timeline_row_layout: (0.0, Vec::new()),
            properties: PropertiesState::default(),
            master_chain: Default::default(),
            multicam: Default::default(),
            model_prompt: None,
            model_prompt_tab: caprust_core::ModelKind::Caption,
            model_download: None,
            translate_rx: None,
            translate_source_clip: None,
            translate_last_progress: None,
            caption_rx: None,
            caption_job_started: None,
            narration_rx: None,
            reframe_rx: None,
            reframe_job_id: None,
            bg_removal_rx: None,
            bg_removal_job_id: None,
            beat_rx: None,
            beat_job_id: None,
            bg_removal_rel_path: None,
            narration_input: Default::default(),
            update_available: None,
            toasts: Vec::new(),
            last_skipped_missing: 0,
            jobs: Vec::new(),
            next_job_id: 1,
            audio_cache: AudioCacheState::default(),
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
            pending_audio_start: None,
            play_anchor_set: false,
            audio_baseline_ms: 0,
            asset_browser: AssetBrowserState::new(),
            export_in_progress: false,
            export_rx: None,
            export_progress: 0.0,
            export_tracker: None,
            export_finished_path: None,
            preview_renderer: None,
            last_frame_instant: None,
            last_streamed_clip: None,
            stream_needs_restart: false,
            explicit_seek_ms: None,
            playback_started_at: None,
            playback_started_ms: 0,
            preview_plan_hash: 0,
            paused_frame_dirty: false,
            paused_renderer_deadline: None,
            text_overlay_drag: None,
            pending_hash: 0,
            pending_respawn_at: None,
            job_runner: JobRunner::new(),
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
            Err(e) => {
                tracing::error!("Save failed: {e}");
                self.toast_error(format!("{}: {e}", tr("toast-save-failed")));
            }
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
                // Backfill any model URLs / SHA-256 that were missing in
                // the saved snapshot (e.g. projects created before F1b
                // added Piper URLs). Also adds brand-new models from
                // later builds.
                state.models.merge_missing_defaults();
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
                // Re-link clips whose media_id no longer resolves
                // to a media item (media library was re-imported →
                // new UUIDs). Runs before regen/backfill so those
                // passes see the corrected state and don't skip
                // cache files that exist under different ids.
                let (relinked, created) = self.project.relink_orphan_media_refs();
                if relinked > 0 || created > 0 {
                    tracing::info!(
                        "load: relinked {relinked} orphan refs, created {created} missing media"
                    );
                }
                // Probe any item whose probe_done is still false. This
                // catches the freshly-created items from relink above, and
                // any audio item that was missed because the older
                // thumbnail regen pass filters to Video|Image only.
                self.backfill_missing_probes();
                // Auto-regenerate thumbnails for older projects or after cache clear.
                self.regen_missing_thumbnails();
                // Reset audio cache and start the initial render.
                // Must run after every project mutation above so the
                // hash matches the final state.
                self.audio_cache = AudioCacheState::default();
                if self.ffmpeg_status.ffmpeg.is_some() {
                    let h = self.project.audio_render_hash();
                    self.spawn_audio_cache_render(h);
                }
                // Scan for missing source files. If any are found, open
                // the relink dialog on the next update() frame.
                self.check_missing_media_on_load();
                self.prune_stale_beat_grids();
                // Waveform cache may be missing on a freshly-opened project
                // (different machine) or for media imported before the
                // waveform pipeline existed. Enqueue what is missing.
                self.backfill_waveforms();
            }
            Err(e) => {
                tracing::error!("Load failed: {e}");
                self.toast_error(format!("{}: {e}", tr("toast-load-failed")));
            }
        }
    }

    /// Drop beat grids whose source file changed on disk (size or
    /// mtime mismatch) or vanished. They recompute on demand via the
    /// Sound panel button; dropping keeps stale markers off the ruler.
    /// Same metadata-only spirit as the probe backfills below.
    fn prune_stale_beat_grids(&mut self) {
        let stale: Vec<uuid::Uuid> = self
            .project
            .beat_grids
            .iter()
            .filter(|(id, e)| {
                let path = match self.project.media.items.iter().find(|m| &m.id == *id) {
                    Some(m) => m.path.clone(),
                    None => return true,
                };
                match std::fs::metadata(&path) {
                    Ok(md) => {
                        let mtime = md
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        md.len() != e.source_len_bytes || mtime != e.source_mtime_secs
                    }
                    Err(_) => true,
                }
            })
            .map(|(id, _)| *id)
            .collect();
        for id in &stale {
            self.project.beat_grids.remove(id);
        }
        if !stale.is_empty() {
            tracing::info!("beat: pruned {} stale grids", stale.len());
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
    /// Return a texture for the recent project's first-media thumbnail,
    /// or None if none is cached on disk. Loads lazily and stores the
    /// handle in `recent_thumb_cache` so we do not re-read the JPEG
    /// every frame.
    /// Return the waveform peaks for `media_id`, loading them from
    /// `<project>/cache/waveforms/<id>.bin` on first use. Returns
    /// None when the cache file does not exist yet (job in flight
    /// or item has no audio).
    /// Return the JPEG thumbnail for `media_id` as an egui texture,
    /// loading it from `<project>/cache/thumbnails/<id>.jpg` on
    /// first use. Same lazy-load pattern as `waveform_peaks_for`.
    ///
    /// Returns `None` when the cache file is not on disk yet
    /// (thumbnail job in flight, or the media has no thumbnail).
    fn thumbnail_texture_for(
        &mut self,
        ctx: &egui::Context,
        media_id: uuid::Uuid,
    ) -> Option<egui::TextureHandle> {
        if let Some(tex) = self.clip_textures.get(&media_id) {
            return Some(tex.clone());
        }
        let proj_path = self.project.project_path.as_deref()?;
        let path = caprust_core::cache::thumbnail_path(std::path::Path::new(proj_path), media_id);
        let bytes = std::fs::read(&path).ok()?;
        let img = image::load_from_memory(&bytes).ok()?;
        let rgba = img.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let color_img = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
        let handle = ctx.load_texture(
            format!("clip-{media_id}"),
            color_img,
            egui::TextureOptions::LINEAR,
        );
        self.clip_textures.insert(media_id, handle.clone());
        Some(handle)
    }

    fn waveform_peaks_for(&mut self, media_id: uuid::Uuid) -> Option<std::sync::Arc<Vec<f32>>> {
        if let Some(p) = self.waveform_cache.get(&media_id) {
            return Some(p.clone());
        }
        let proj_path = self.project.project_path.as_deref()?;
        let path = caprust_core::cache::waveform_path(std::path::Path::new(proj_path), media_id);
        let bytes = std::fs::read(&path).ok()?;
        if bytes.len() < 4 {
            return None;
        }
        let n = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        if bytes.len() < 4 + n * 4 {
            return None;
        }
        let mut peaks = Vec::with_capacity(n);
        for i in 0..n {
            let off = 4 + i * 4;
            peaks.push(f32::from_le_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ]));
        }
        let arc = std::sync::Arc::new(peaks);
        self.waveform_cache.insert(media_id, arc.clone());
        Some(arc)
    }

    /// Return cached envelope samples + duck zones for `clip`.
    /// Recomputes when `ProjectState::render_hash()` changed since the
    /// last call. Samples are geometry-independent (fractions of the
    /// clip duration, not pixels), so scroll / zoom do not invalidate
    /// the cache — only an actual project mutation does.
    fn envelope_for(
        &mut self,
        clip: &caprust_core::Clip,
    ) -> (
        std::sync::Arc<Vec<crate::timeline::EnvelopeSample>>,
        std::sync::Arc<Vec<crate::timeline::DuckZone>>,
    ) {
        let hash = self.project.render_hash();
        if let Some(entry) = self.envelope_cache.get(&clip.id) {
            if entry.hash == hash {
                return (entry.samples_db.clone(), entry.duck_zones.clone());
            }
        }
        let samples = crate::timeline::sample_envelope_db(clip);
        let zones = crate::timeline::compute_duck_zones(clip, &self.project);
        let entry = EnvelopeCacheEntry {
            hash,
            samples_db: std::sync::Arc::new(samples),
            duck_zones: std::sync::Arc::new(zones),
        };
        let result = (entry.samples_db.clone(), entry.duck_zones.clone());
        self.envelope_cache.insert(clip.id, entry);
        result
    }

    fn recent_thumb_texture(
        &mut self,
        ctx: &egui::Context,
        path: &str,
        media_id: Option<uuid::Uuid>,
    ) -> Option<egui::TextureHandle> {
        if let Some(t) = self.recent_thumb_cache.get(path) {
            return Some(t.clone());
        }
        // The .caprust file lives directly under the project dir, so
        // parent() gives us the dir that contains cache/.
        let project_dir = std::path::Path::new(path).parent()?;
        // Prefer the exact media id; fall back to the first .jpg
        // in cache/thumbnails/ for old recent entries that predate
        // the first_media_id field.
        let jpg = match media_id {
            Some(id) => {
                let p = caprust_core::cache::thumbnail_path(project_dir, id);
                if p.is_file() {
                    p
                } else {
                    first_thumb_in_dir(project_dir)?
                }
            }
            None => first_thumb_in_dir(project_dir)?,
        };
        let bytes = std::fs::read(&jpg).ok()?;
        let img = image::load_from_memory(&bytes).ok()?;
        let rgba = img.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let color_img = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
        let handle = ctx.load_texture(
            format!(
                "recent-thumb-{}",
                jpg.file_name().and_then(|s| s.to_str()).unwrap_or("thumb")
            ),
            color_img,
            egui::TextureOptions::LINEAR,
        );
        self.recent_thumb_cache
            .insert(path.to_string(), handle.clone());
        Some(handle)
    }

    fn show_start_screen(&mut self, ctx: &egui::Context) {
        let mut load_path: Option<String> = None;

        // Drag & drop a .caprust file onto the start screen.
        ctx.input(|i| {
            for f in &i.raw.dropped_files {
                if let Some(p) = &f.path {
                    if p.extension().is_some_and(|e| e == "caprust") && load_path.is_none() {
                        load_path = Some(p.to_string_lossy().to_string());
                    }
                }
            }
        });

        // LEFT: sidebar with primary actions.
        egui::SidePanel::left("start_sidebar")
            .exact_width(220.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(space::XXL);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{} CapRust", ph::FILM_STRIP))
                            .size(22.0)
                            .strong(),
                    );
                    ui.add_space(space::XS);
                    ui.label(
                        egui::RichText::new(tr("new-tagline"))
                            .small()
                            .color(egui::Color32::from_gray(140)),
                    );
                });
                ui.add_space(space::XXL);

                let btn_w = ui.available_width() - space::XL;

                if ui
                    .add_sized(
                        [btn_w, 36.0],
                        egui::Button::new(
                            egui::RichText::new(format!("{}  {}", ph::PLUS, tr("new-title")))
                                .size(text::M),
                        )
                        .fill(ui.visuals().selection.bg_fill),
                    )
                    .clicked()
                {
                    self.new_project_modal_open = true;
                }
                ui.add_space(space::S);
                if ui
                    .add_sized(
                        [btn_w, 32.0],
                        egui::Button::new(format!(
                            "{}  {}",
                            ph::FOLDER_OPEN,
                            tr("new-button-open")
                        )),
                    )
                    .clicked()
                {
                    if let Some(f) = rfd::FileDialog::new()
                        .add_filter("CapRust Project", &["caprust"])
                        .pick_file()
                    {
                        load_path = Some(f.to_string_lossy().to_string());
                    }
                }
                ui.add_space(space::S);
                if ui
                    .add_sized(
                        [btn_w, 32.0],
                        egui::Button::new(format!("{}  {}", ph::GEAR, tr("menu-file-settings"))),
                    )
                    .clicked()
                {
                    self.settings_open = true;
                }
                ui.add_space(space::S);
                if ui
                    .add_sized(
                        [btn_w, 32.0],
                        egui::Button::new(format!("{}  {}", ph::X, tr("menu-file-quit"))),
                    )
                    .clicked()
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }

                // Version pinned to the bottom.
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(space::M);
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {}",
                            tr("start-version-label"),
                            env!("CARGO_PKG_VERSION")
                        ))
                        .small()
                        .color(egui::Color32::from_gray(90)),
                    );
                });
            });

        // CENTRAL: heading + subtitle + recent grid.
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(space::XXL);
            ui.horizontal(|ui| {
                ui.add_space(space::XXL);
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(tr("start-projects-heading"))
                            .size(text::XXL)
                            .strong(),
                    );
                    ui.add_space(space::XS);
                    ui.label(
                        egui::RichText::new(tr("start-projects-subtitle"))
                            .color(egui::Color32::from_gray(140)),
                    );
                });
            });
            ui.add_space(space::XL);

            if self.recent.items.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(space::XXL);
                    ui.vertical(|ui| {
                        empty::placeholder(ui, tr("new-recent-empty"));
                    });
                });
                return;
            }

            let entries: Vec<RecentProject> = self.recent.items.clone();
            let mut forget_path: Option<String> = None;
            let mut delete_from_disk: Option<String> = None;

            let card_w = 300.0;
            let gap = space::L;

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let avail = (ui.available_width() - space::XXL).max(card_w);
                    let cols = (((avail + gap) / (card_w + gap)).floor() as usize).max(1);

                    for chunk in entries.chunks(cols) {
                        ui.horizontal_top(|ui| {
                            ui.add_space(space::XXL);
                            for rp in chunk {
                                let frame = egui::Frame::group(ui.style())
                                    .inner_margin(space::M)
                                    .fill(ui.visuals().faint_bg_color);
                                let card = frame.show(ui, |ui| {
                                    ui.set_width(card_w);
                                    ui.vertical(|ui| {
                                        let (rect, _) = ui.allocate_exact_size(
                                            egui::vec2(card_w, card_w * 9.0 / 16.0),
                                            egui::Sense::hover(),
                                        );
                                        ui.painter().rect_filled(
                                            rect,
                                            radius::cr(radius::SM),
                                            egui::Color32::from_gray(40),
                                        );
                                        let tex = self.recent_thumb_texture(
                                            ui.ctx(),
                                            &rp.path,
                                            rp.first_media_id,
                                        );
                                        if let Some(tex) = tex {
                                            let tex_size = tex.size_vec2();
                                            let scale = (rect.width() / tex_size.x)
                                                .max(rect.height() / tex_size.y);
                                            let draw_size = tex_size * scale;
                                            let frac_x = (rect.width() / draw_size.x).min(1.0);
                                            let frac_y = (rect.height() / draw_size.y).min(1.0);
                                            let uv_min = egui::Pos2::new(
                                                (1.0 - frac_x) / 2.0,
                                                (1.0 - frac_y) / 2.0,
                                            );
                                            let uv_max =
                                                egui::Pos2::new(1.0 - uv_min.x, 1.0 - uv_min.y);
                                            ui.painter().image(
                                                tex.id(),
                                                rect,
                                                egui::Rect::from_min_max(uv_min, uv_max),
                                                egui::Color32::WHITE,
                                            );
                                        } else {
                                            ui.painter().text(
                                                rect.center(),
                                                egui::Align2::CENTER_CENTER,
                                                ph::FILM_STRIP,
                                                egui::FontId::proportional(42.0),
                                                egui::Color32::from_gray(140),
                                            );
                                        }

                                        ui.add_space(space::S);
                                        ui.label(egui::RichText::new(&rp.name).strong().size(14.0));

                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{} \u{00b7} {} clips \u{00b7} {}",
                                                format_duration(rp.duration_ms),
                                                rp.clip_count,
                                                format_age(rp.last_opened),
                                            ))
                                            .small()
                                            .color(egui::Color32::from_gray(150)),
                                        );

                                        let mut bits: Vec<String> = Vec::new();
                                        if rp.base_resolution > 0 {
                                            bits.push(format!("{}p", rp.base_resolution));
                                        }
                                        if !rp.frame_rate_label.is_empty() {
                                            bits.push(format!("{} fps", rp.frame_rate_label));
                                        }
                                        if !bits.is_empty() {
                                            ui.label(
                                                egui::RichText::new(bits.join(" \u{00b7} "))
                                                    .small()
                                                    .color(egui::Color32::from_gray(120)),
                                            );
                                        }

                                        ui.add_space(space::XS);
                                        ui.horizontal(|ui| {
                                            if ui.small_button(tr("new-recent-open")).clicked() {
                                                load_path = Some(rp.path.clone());
                                            }
                                            if ui.small_button(tr("new-recent-forget")).clicked() {
                                                forget_path = Some(rp.path.clone());
                                            }
                                            if ui
                                                .small_button(ph::TRASH)
                                                .on_hover_text(tr("new-recent-delete-hint"))
                                                .clicked()
                                            {
                                                delete_from_disk = Some(rp.path.clone());
                                            }
                                        });
                                    });
                                });
                                if ui.rect_contains_pointer(card.response.rect) {
                                    ui.painter().rect_stroke(
                                        card.response.rect,
                                        radius::cr(radius::SM),
                                        egui::Stroke::new(
                                            elev::STROKE_HAIRLINE,
                                            ui.visuals().selection.bg_fill,
                                        ),
                                        egui::StrokeKind::Inside,
                                    );
                                }
                                ui.add_space(gap);
                            }
                        });
                        ui.add_space(space::M);
                    }
                });

            if let Some(p) = forget_path {
                self.recent.forget(&p);
            }
            if let Some(p) = delete_from_disk {
                let _ = std::fs::remove_file(&p);
                self.recent.forget(&p);
            }
        });

        if let Some(p) = load_path {
            self.load_project_from(&p);
        }

        self.show_new_project_modal(ctx);
    }

    /// New Project modal, opened from the start-screen sidebar. Reuses
    /// the draft fields stored on CapRustApp; the Create button runs
    /// the same path as before (create_project).
    fn show_new_project_modal(&mut self, ctx: &egui::Context) {
        if !self.new_project_modal_open {
            return;
        }
        let mut open = self.new_project_modal_open;
        let mut created = false;

        egui::Window::new(tr("new-title"))
            .id(egui::Id::new("new_project_modal"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_min_width(460.0);
                egui::Grid::new("new_project_modal_grid")
                    .num_columns(2)
                    .spacing([space::L, space::M_PLUS])
                    .show(ui, |ui| {
                        ui.label(tr("new-field-name"));
                        ui.text_edit_singleline(&mut self.draft.name);
                        ui.end_row();

                        ui.label(tr("new-field-location"));
                        ui.horizontal(|ui| {
                            ui.text_edit_singleline(&mut self.draft.location);
                            if ui.button(tr("new-button-browse")).clicked() {
                                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                    self.draft.location = dir.to_string_lossy().to_string();
                                }
                            }
                        });
                        ui.end_row();

                        ui.label(tr("new-field-format"));
                        egui::ComboBox::from_id_salt("modal_draft_aspect")
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
                            egui::Slider::new(&mut self.draft.base_resolution, 480..=2160)
                                .suffix(" px"),
                        );
                        ui.end_row();

                        ui.label(tr("new-field-fps"));
                        egui::ComboBox::from_id_salt("modal_draft_fps")
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

                ui.add_space(space::XL);
                ui.horizontal(|ui| {
                    let can_create = !self.draft.name.trim().is_empty()
                        && !self.draft.location.trim().is_empty();
                    let resp = button::primary_enabled(ui, can_create, tr("new-button-create"));
                    let resp = if can_create {
                        resp
                    } else {
                        resp.on_disabled_hover_text(tr("new-button-create-hint"))
                    };
                    if resp.clicked() {
                        self.create_project();
                        created = true;
                    }
                    if ui.button(tr("menu-file-quit")).clicked() {
                        created = true;
                    }
                });
            });

        // Apply open/close after the borrow of `self` inside `.show()`
        // has ended (DIRECTIVES section 5, egui::Window::open trap).
        self.new_project_modal_open = open && !created;
    }

    fn show_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button(tr("menu-file"), |ui| {
                    if ui.button(tr("menu-file-new")).clicked() {
                        self.mode = AppMode::StartScreen;
                        ui.close_menu();
                    }
                    ui.separator();
                    let open_lbl = if self.settings.enable_shortcuts {
                        format!("{}  (Ctrl+O)", tr("menu-file-open"))
                    } else {
                        tr("menu-file-open")
                    };
                    if ui.button(open_lbl).clicked() {
                        if let Some(f) = rfd::FileDialog::new()
                            .add_filter("CapRust Project", &["caprust"])
                            .pick_file()
                        {
                            let p = f.to_string_lossy().to_string();
                            self.load_project_from(&p);
                        }
                        ui.close_menu();
                    }
                    let save_lbl = if self.settings.enable_shortcuts {
                        format!("{}  (Ctrl+S)", tr("menu-file-save"))
                    } else {
                        tr("menu-file-save")
                    };
                    if ui.button(save_lbl).clicked() {
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
                    if ui.button(tr("menu-file-export-subtitles")).clicked() {
                        if let Some(f) = rfd::FileDialog::new()
                            .add_filter("SubRip subtitles", &["srt"])
                            .set_file_name(format!("{}.srt", self.project.name))
                            .save_file()
                        {
                            let mut p = f.clone();
                            if p.extension().is_none() {
                                p.set_extension("srt");
                            }
                            match Self::write_srt_file(&self.project.clips, &p) {
                                Ok(n) => self.toast(format!("{} ({n})", tr("exp-srt-done"))),
                                Err(e) => {
                                    tracing::error!("srt export failed: {e}");
                                    self.toast_error(format!("{}: {e}", tr("toast-export-failed")));
                                }
                            }
                        }
                        ui.close_menu();
                    }
                    #[cfg(windows)]
                    if ui.button(tr("menu-file-record-screen")).clicked() {
                        self.screen_record_open = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button(tr("menu-file-clear-cache")).clicked() {
                        if let Some(path) = self.project.project_path.clone() {
                            let _ = caprust_core::cache::clear_cache(std::path::Path::new(&path));
                            // Textures in memory must be dropped too.
                            self.clip_textures.clear();
                            self.media_bin.thumb_cache = Default::default();
                            // Re-generate in background. Both passes:
                            // thumbnails for video/image, waveforms for
                            // audio/video. Each checks the on-disk cache,
                            // so this is idempotent.
                            self.regen_missing_thumbnails();
                            self.backfill_waveforms();
                            self.backfill_missing_probes();
                        }
                        ui.close_menu();
                    }
                    if ui.button(tr("menu-file-regen-thumbs")).clicked() {
                        self.regen_missing_thumbnails();
                        self.backfill_waveforms();
                        self.backfill_missing_probes();
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
                    let undo_lbl = if self.settings.enable_shortcuts {
                        format!("{}  (Ctrl+Z)", tr("menu-edit-undo"))
                    } else {
                        tr("menu-edit-undo")
                    };
                    if ui
                        .add_enabled(can_undo, egui::Button::new(undo_lbl))
                        .clicked()
                    {
                        let _ = self.undo_stack.undo(&mut self.project);
                        ui.close_menu();
                    }
                    let can_redo = self.undo_stack.can_redo();
                    let redo_lbl = if self.settings.enable_shortcuts {
                        format!("{}  (Ctrl+Y)", tr("menu-edit-redo"))
                    } else {
                        tr("menu-edit-redo")
                    };
                    if ui
                        .add_enabled(can_redo, egui::Button::new(redo_lbl))
                        .clicked()
                    {
                        let _ = self.undo_stack.redo(&mut self.project);
                        ui.close_menu();
                    }
                    ui.separator();
                    let split_lbl = if self.settings.enable_shortcuts {
                        format!("{}  (S)", tr("menu-edit-split"))
                    } else {
                        tr("menu-edit-split")
                    };
                    if ui.button(split_lbl).clicked() {
                        let at = self.playhead_ms;
                        for id in self.selected_clips.clone() {
                            let cmd = SplitClipCommand::new(id, at);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        ui.close_menu();
                    }
                    let del_lbl = if self.settings.enable_shortcuts {
                        format!("{}  (Del)", tr("menu-edit-delete"))
                    } else {
                        tr("menu-edit-delete")
                    };
                    if ui.button(del_lbl).clicked() {
                        for id in self.selected_clips.clone() {
                            let cmd = DeleteClipCommand::new(id, false);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        self.selected_clips.clear();
                        ui.close_menu();
                    }
                    if ui.button(tr("menu-edit-ripple-delete")).clicked() {
                        for id in self.selected_clips.clone() {
                            let cmd = caprust_core::commands::delete_clip::DeleteClipCommand::new(
                                id, true,
                            );
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        self.selected_clips.clear();
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
                    ui.menu_button(tr("menu-view-layout"), |ui| {
                        for p in crate::dock::LayoutPreset::all() {
                            if ui.button(tr(p.label_key())).clicked() {
                                self.dock_state = crate::dock::preset_dock_state(p);
                                tracing::info!("dock layout preset applied: {:?}", p);
                                ui.close_menu();
                            }
                        }
                        ui.separator();
                        if ui.button(tr("menu-view-layout-reset")).clicked() {
                            self.dock_state = crate::dock::default_dock_state();
                            tracing::info!("dock layout reset to default");
                            ui.close_menu();
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
                if ui.button(format!("{} Add Text", ph::PLUS)).clicked() {
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

                // Screen recording indicator (Windows only).
                #[cfg(windows)]
                {
                    use std::sync::atomic::Ordering;
                    if self.screen_record.in_progress {
                        ui.separator();
                        let elapsed = self
                            .screen_record
                            .started_at
                            .map(|t| t.elapsed().as_secs())
                            .unwrap_or(0);
                        let mm = elapsed / 60;
                        let ss = elapsed % 60;
                        ui.label(
                            egui::RichText::new(format!(
                                "● REC {mm:02}:{ss:02} ({}/{})",
                                elapsed, self.screen_record.duration_sec
                            ))
                            .color(egui::Color32::from_rgb(220, 60, 60))
                            .strong(),
                        );
                        if ui.button(tr("screen-record-toolbar-stop")).clicked() {
                            if let Some(flag) = &self.screen_record.stop_flag {
                                flag.store(true, Ordering::Relaxed);
                            }
                        }
                        // Repaint so the timer advances.
                        ctx.request_repaint_after(std::time::Duration::from_millis(500));
                    }
                }
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
                .filter(|(_, c)| {
                    c.track_index == t
                        && !matches!(
                            c.clip_type,
                            caprust_core::ClipType::TextOverlay { .. }
                                | caprust_core::ClipType::Captions { .. }
                        )
                })
                .map(|(i, _)| i)
                .collect();
            if indices.is_empty() {
                continue;
            }
            indices.sort_by_key(|&i| self.project.clips[i].start_time_ms);
            // Pack in place: the earliest clip on the track anchors the
            // sequence, gaps between later clips close. Anchoring at 0
            // would slide every clip to the timeline start, which is
            // not what "remove gaps" means (DIRECTIVES 7.4).
            let mut cursor: u64 = self.project.clips[indices[0]].start_time_ms;
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
                    && !matches!(
                        c.clip_type,
                        caprust_core::ClipType::TextOverlay { .. }
                            | caprust_core::ClipType::Captions { .. }
                    )
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
                    && !matches!(
                        c.clip_type,
                        caprust_core::ClipType::TextOverlay { .. }
                            | caprust_core::ClipType::Captions { .. }
                    )
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
                // Follower = anything that is not an anchor. Includes
                // text overlays / captions even on Video or Overlay
                // tracks: they ride the video clip they overlap or are
                // closest to, they do not anchor the pack.
                let on_video_track = self
                    .project
                    .tracks
                    .get(c.track_index)
                    .map(|t| {
                        matches!(
                            t.kind,
                            caprust_core::TrackKind::Video | caprust_core::TrackKind::Overlay
                        )
                    })
                    .unwrap_or(false);
                let text_like = matches!(
                    c.clip_type,
                    caprust_core::ClipType::TextOverlay { .. }
                        | caprust_core::ClipType::Captions { .. }
                );
                !on_video_track || text_like
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
            // Score every video clip. Overlap is preferred (positive
            // score, larger wins). When the follower only touches a
            // clip at its edge, or sits in a gap right next to it
            // (which is how Separate Audio and Captions lay out), score
            // is negative and the closest one wins. This keeps audio /
            // text / captions in lockstep with the video they are
            // adjacent to, not only the ones they strictly overlap.
            let mut best: Option<(i64, uuid::Uuid)> = None;
            for v in &video_clip_ids {
                let Some(&(vs, ve)) = old_spans.get(v) else {
                    continue;
                };
                let ov_start = cs.max(vs);
                let ov_end = ce.min(ve);
                let score: i64 = if ov_start < ov_end {
                    (ov_end - ov_start) as i64
                } else {
                    let gap = if ce <= vs { vs - ce } else { cs - ve };
                    -(gap as i64)
                };
                if best.is_none_or(|(bs, _)| score > bs) {
                    best = Some((score, *v));
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
        if ev.trim_follow_toggled {
            self.settings.trim_follow = self.timeline_tools.trim_follow;
            tracing::info!("trim_follow = {}", self.settings.trim_follow);
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
            self.model_prompt_tab = kind;
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
                self.model_prompt_tab = caprust_core::ModelKind::Narration;
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
            self.model_prompt_tab = caprust_core::ModelKind::Narration;
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

    /// Kick off an auto-reframe analysis for the given clip. Requires
    /// the YuNet model on disk and ffmpeg + ffprobe. Silently no-ops
    /// with a toast when any prerequisite is missing.
    /// Pick the SCRFD model if it is on disk, else fall back to YuNet.
    /// Returns (path, is_scrfd). None when neither is present.
    fn resolve_face_model(&mut self) -> Option<(std::path::PathBuf, bool)> {
        // Pull in any registry entries added since the project was
        // last saved (e.g. SCRFD in a newer build) before scanning.
        self.project.models.merge_missing_defaults();
        self.project
            .models
            .scan_local(&self.settings.effective_models_dir());
        let models_dir = self.settings.effective_models_dir();
        let all = &self.project.models.models;
        let resolve =
            |id: &str| caprust_core::models::ModelRegistry::local_path_static(all, &models_dir, id);
        let scrfd = all
            .iter()
            .find(|m| m.kind == caprust_core::ModelKind::ScrfdDetector);
        if let Some(m) = scrfd {
            let p = resolve(&m.id);
            if p.is_file() {
                return Some((p, true));
            }
        }
        let yunet = all
            .iter()
            .find(|m| m.kind == caprust_core::ModelKind::FaceDetector);
        if let Some(m) = yunet {
            let p = resolve(&m.id);
            if p.is_file() {
                return Some((p, false));
            }
        }
        None
    }

    fn start_beat_job(&mut self, clip_id: uuid::Uuid) {
        if self.beat_rx.is_some() {
            self.toast(tr("toast-beat-busy"));
            return;
        }
        let Some(clip) = self.project.clips.iter().find(|c| c.id == clip_id) else {
            return;
        };
        let source_path = match &clip.clip_type {
            caprust_core::clip::ClipType::Audio { path, .. } => std::path::PathBuf::from(path),
            _ => {
                self.toast(tr("toast-beat-needs-audio"));
                return;
            }
        };
        let Some(ffmpeg) = self.ffmpeg_status.ffmpeg.clone() else {
            self.toast(tr("toast-beat-needs-ffmpeg"));
            return;
        };
        let key = clip.media_id.unwrap_or(clip.id);
        let req = crate::media_jobs::BeatRequest { key, source_path };
        let rx = crate::media_jobs::spawn_beat_job(std::path::PathBuf::from(ffmpeg), req);
        let job_id = self.begin_job(JobKind::Beat, tr("job-beat"));
        self.beat_job_id = Some(job_id);
        self.beat_rx = Some(rx);
        self.toast(tr("toast-beat-started"));
        tracing::info!("beat: job spawned for clip {clip_id}");
    }

    fn start_reframe_job(&mut self, clip_id: uuid::Uuid) {
        if self.reframe_rx.is_some() {
            self.toast(tr("toast-reframe-busy"));
            return;
        }
        let Some(clip) = self.project.clips.iter().find(|c| c.id == clip_id) else {
            return;
        };
        let source_path = match &clip.clip_type {
            caprust_core::ClipType::Video { path, .. }
            | caprust_core::ClipType::Image { path, .. } => std::path::PathBuf::from(path),
            _ => {
                self.toast(tr("toast-reframe-needs-video"));
                return;
            }
        };
        let t_start_ms = clip.start_time_ms;
        let duration_ms = clip.duration_ms;
        if duration_ms == 0 {
            self.toast(tr("toast-reframe-needs-video"));
            return;
        }

        // Model: prefer SCRFD, fall back to YuNet.
        let Some((model_path, model_is_scrfd)) = self.resolve_face_model() else {
            self.toast(tr("toast-reframe-needs-model"));
            return;
        };

        let Some(ffmpeg) = self.ffmpeg_status.ffmpeg.clone() else {
            self.toast(tr("toast-reframe-needs-ffmpeg"));
            return;
        };
        let Some(ffprobe) = self.ffmpeg_status.ffprobe.clone() else {
            self.toast(tr("toast-reframe-needs-ffmpeg"));
            return;
        };

        let target_aspect = {
            let (pw, ph) = self.project.project_dimensions();
            if ph == 0 {
                16.0 / 9.0
            } else {
                pw as f64 / ph as f64
            }
        };

        let req = crate::media_jobs::ReframeRequest {
            clip_id,
            source_path,
            t_start_ms,
            duration_ms,
            target_aspect,
            model_path,
            model_is_scrfd,
            sample_fps: 4.0,
            max_side: 480,
        };
        let rx = crate::media_jobs::spawn_reframe_job(
            std::path::PathBuf::from(ffmpeg),
            std::path::PathBuf::from(ffprobe),
            req,
        );
        let job_id = self.begin_job(JobKind::Reframe, tr("job-reframe"));
        self.reframe_job_id = Some(job_id);
        self.reframe_rx = Some(rx);
        self.toast(tr("toast-reframe-started"));
        tracing::info!("reframe: job spawned for clip {clip_id}");
    }

    /// Poll the in-flight background-removal job. Events arrive as
    /// Started / Progress / Finished / Failed. On Finished, write the
    /// mask path into the clip through SetClipCommand (undoable). On
    /// Failed, toast and clear state.
    fn drain_bg_removal_job(&mut self) {
        // Take the receiver so we can process every queued event and
        // put it back only while the stream is still live. Same shape
        // as drain_model_download.
        let Some(rx) = self.bg_removal_rx.take() else {
            return;
        };
        let mut still_live = true;
        loop {
            match rx.try_recv() {
                Ok(crate::media_jobs::BgRemovalEvent::Started { total_frames }) => {
                    tracing::info!("bg-removal: started, {total_frames} frames to process");
                    if let Some(id) = self.bg_removal_job_id {
                        // Stay indeterminate until the first Progress
                        // event: in debug builds frame 1/N can take
                        // minutes, and a 0 percent bar reads as stuck.
                        self.update_job_progress(id, BackgroundJob::INDETERMINATE);
                    }
                }
                Ok(crate::media_jobs::BgRemovalEvent::Progress { done, total }) => {
                    let p = if total == 0 {
                        0.0
                    } else {
                        done as f32 / total as f32
                    };
                    if let Some(id) = self.bg_removal_job_id {
                        self.update_job_progress(id, p);
                    }
                }
                Ok(crate::media_jobs::BgRemovalEvent::Finished {
                    clip_id,
                    mask_path,
                    frame_count,
                }) => {
                    let rel = self.bg_removal_rel_path.take();
                    let stored = match rel {
                        Some(r) => Some(r),
                        None => {
                            // Fallback: derive from the absolute path
                            // relative to the project dir. Should not
                            // happen, but a missing key would silently
                            // drop the mask and waste the whole job.
                            let proj = self.project.project_path.clone();
                            proj.and_then(|p| {
                                mask_path
                                    .strip_prefix(&p)
                                    .ok()
                                    .map(|r| r.to_string_lossy().into_owned())
                            })
                        }
                    };
                    if let Some(rel) = stored {
                        let cmd = caprust_core::commands::set_clip::SetClipCommand::new(clip_id)
                            .bg_removal(Some(rel));
                        if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                            tracing::error!("bg-removal: apply failed: {e}");
                            self.toast_error(format!("{}: {e}", tr("toast-bg-removal-failed")));
                        } else {
                            tracing::info!(
                                "bg-removal: clip {} mask applied ({frame_count} frames)",
                                clip_id
                            );
                            self.toast(tr("toast-bg-removal-done"));
                        }
                    } else {
                        tracing::warn!("bg-removal: finished but no relative path available");
                        self.toast_error(tr("toast-bg-removal-failed"));
                    }
                    if let Some(id) = self.bg_removal_job_id.take() {
                        self.finish_job(id);
                    }
                    still_live = false;
                }
                Ok(crate::media_jobs::BgRemovalEvent::Failed(msg)) => {
                    tracing::error!("bg-removal: job failed: {msg}");
                    self.toast_error(format!("{}: {msg}", tr("toast-bg-removal-failed")));
                    if let Some(id) = self.bg_removal_job_id.take() {
                        self.finish_job(id);
                    }
                    self.bg_removal_rel_path = None;
                    still_live = false;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    tracing::warn!("bg-removal: receiver disconnected unexpectedly");
                    if let Some(id) = self.bg_removal_job_id.take() {
                        self.finish_job(id);
                    }
                    self.bg_removal_rel_path = None;
                    still_live = false;
                    break;
                }
            }
        }
        if still_live {
            self.bg_removal_rx = Some(rx);
        }
    }

    /// Kick off a background-removal job for the given clip. Requires
    /// the u2netp model on disk, ffmpeg + ffprobe, and a saved project
    /// (so the mask cache has somewhere to live). Silently no-ops with
    /// a toast on any missing prerequisite.
    fn start_bg_removal_job(&mut self, clip_id: uuid::Uuid) {
        if self.bg_removal_rx.is_some() {
            self.toast(tr("toast-bg-removal-busy"));
            return;
        }
        let Some(project_path) = self.project.project_path.clone() else {
            self.toast(tr("toast-bg-removal-needs-project"));
            return;
        };
        let Some(clip) = self.project.clips.iter().find(|c| c.id == clip_id) else {
            return;
        };
        let source_path = match &clip.clip_type {
            caprust_core::ClipType::Video { path, .. }
            | caprust_core::ClipType::Image { path, .. } => std::path::PathBuf::from(path),
            _ => {
                self.toast(tr("toast-bg-removal-needs-video"));
                return;
            }
        };
        let t_start_ms = clip.start_time_ms;
        let duration_ms = clip.duration_ms;
        if duration_ms == 0 {
            self.toast(tr("toast-bg-removal-needs-video"));
            return;
        }

        // Model: resolve u2netp path from the registry.
        self.project
            .models
            .scan_local(&self.settings.effective_models_dir());
        let models_dir = self.settings.effective_models_dir();
        let model_path = self
            .project
            .models
            .models
            .iter()
            .find(|m| m.kind == caprust_core::ModelKind::BackgroundRemover)
            .map(|m| {
                caprust_core::models::ModelRegistry::local_path_static(
                    &self.project.models.models,
                    &models_dir,
                    &m.id,
                )
            });
        let Some(model_path) = model_path.filter(|p| p.is_file()) else {
            self.toast(tr("toast-bg-removal-needs-model"));
            return;
        };

        let Some(ffmpeg) = self.ffmpeg_status.ffmpeg.clone() else {
            self.toast(tr("toast-bg-removal-needs-ffmpeg"));
            return;
        };
        let Some(ffprobe) = self.ffmpeg_status.ffprobe.clone() else {
            self.toast(tr("toast-bg-removal-needs-ffmpeg"));
            return;
        };

        let fps = {
            let fr = &self.project.frame_rate;
            if fr.den == 0 {
                30.0
            } else {
                fr.num as f64 / fr.den as f64
            }
        };

        // Mask is stored at a path relative to the project so the
        // .caprust file stays portable across machines. The worker
        // gets the absolute path.
        let rel = format!("cache/masks/{clip_id}.mkv");
        let abs = caprust_core::cache::mask_path(std::path::Path::new(&project_path), clip_id);

        let req = crate::media_jobs::BgRemovalRequest {
            clip_id,
            source_path,
            t_start_ms,
            duration_ms,
            fps,
            model_path,
            mask_output: abs,
            max_side: 480,
        };
        let rx = crate::media_jobs::spawn_bg_removal_job(
            std::path::PathBuf::from(ffmpeg),
            std::path::PathBuf::from(ffprobe),
            req,
        );
        let job_id = self.begin_job(JobKind::BgRemoval, tr("job-bg-removal"));
        self.bg_removal_job_id = Some(job_id);
        self.bg_removal_rx = Some(rx);
        // Remember the relative path so drain can store it in the clip
        // without recomputing. Keyed on the job id because at most one
        // job runs at a time and clip_id is recoverable from Finished.
        self.bg_removal_rel_path = Some(rel);
        self.toast(tr("toast-bg-removal-started"));
        tracing::info!("bg-removal: job spawned for clip {clip_id}");
    }

    /// Poll the in-flight reframe job. On success, write the keypoints
    /// into the clip through SetClipCommand (undoable). On failure,
    /// toast and clear state.
    fn drain_reframe_job(&mut self) {
        let Some(rx) = self.reframe_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(result)) => {
                if result.keypoints.is_empty() {
                    tracing::warn!(
                        "reframe: {} frames analysed, {} had a face, no keypoints produced",
                        result.frames_analyzed,
                        result.frames_with_face
                    );
                    self.toast(tr("toast-reframe-no-faces"));
                } else {
                    let cmd = caprust_core::commands::set_clip::SetClipCommand::new(result.clip_id)
                        .auto_reframe(result.keypoints.clone());
                    if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                        tracing::error!("reframe: apply failed: {e}");
                        self.toast_error(format!("{}: {e}", tr("toast-reframe-failed")));
                    } else {
                        tracing::info!(
                            "reframe: applied {} keypoints to clip {}",
                            result.keypoints.len(),
                            result.clip_id
                        );
                        self.toast(tr("toast-reframe-done"));
                    }
                }
                if let Some(id) = self.reframe_job_id.take() {
                    self.finish_job(id);
                }
                self.reframe_rx = None;
            }
            Ok(Err(msg)) => {
                tracing::error!("reframe: job failed: {msg}");
                self.toast_error(format!("{}: {msg}", tr("toast-reframe-failed")));
                if let Some(id) = self.reframe_job_id.take() {
                    self.finish_job(id);
                }
                self.reframe_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                tracing::warn!("reframe: receiver disconnected unexpectedly");
                if let Some(id) = self.reframe_job_id.take() {
                    self.finish_job(id);
                }
                self.reframe_rx = None;
            }
        }
    }

    fn drain_beat_job(&mut self) {
        let Some(rx) = self.beat_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(result)) => {
                if result.bpm <= 0.0 || result.beats_ms.is_empty() {
                    tracing::warn!("beat: no usable onsets");
                    self.toast(tr("toast-beat-no-beats"));
                } else {
                    self.project.beat_grids.insert(
                        result.key,
                        caprust_core::beat::BeatEntry {
                            bpm: result.bpm,
                            beats_ms: result.beats_ms.clone(),
                            source_len_bytes: result.len_bytes,
                            source_mtime_secs: result.mtime_secs,
                        },
                    );
                    tracing::info!(
                        "beat: {:.1} BPM, {} markers",
                        result.bpm,
                        result.beats_ms.len()
                    );
                    self.toast(format!("{}: {:.0} BPM", tr("toast-beat-done"), result.bpm));
                }
                if let Some(id) = self.beat_job_id.take() {
                    self.finish_job(id);
                }
                self.beat_rx = None;
            }
            Ok(Err(msg)) => {
                tracing::error!("beat: job failed: {msg}");
                self.toast_error(format!("{}: {msg}", tr("toast-beat-failed")));
                if let Some(id) = self.beat_job_id.take() {
                    self.finish_job(id);
                }
                self.beat_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                tracing::warn!("beat: receiver disconnected unexpectedly");
                if let Some(id) = self.beat_job_id.take() {
                    self.finish_job(id);
                }
                self.beat_rx = None;
            }
        }
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
                self.toast_error(format!("{}: {msg}", tr("toast-narration-failed")));
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
            // Refresh the registry from the built-in defaults before
            // scanning. A project saved before a new model was added
            // (e.g. whisper-tiny appearing in a later release, or a
            // manually-placed file for an entry the snapshot did not
            // have) would otherwise never see the entry. Idempotent:
            // existing rows keep status, missing rows are cloned in.
            self.project.models.merge_missing_defaults();

            // scan_local() marks a model Ready when its file exists and
            // is non-empty, so manually-placed weights (and downloads
            // that completed after the last startup) are picked up
            // without a restart.
            self.project.models.scan_local(&models_dir);

            let ready = self.project.models.ready_captions();
            let Some(model) = ready.first() else {
                tracing::info!("no caption model ready — opening prompt");
                self.model_prompt = Some(caprust_core::ModelKind::Caption);
                self.model_prompt_tab = caprust_core::ModelKind::Caption;
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
                self.model_prompt_tab = caprust_core::ModelKind::Caption;
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
                self.toast_error(format!("{}: {msg}", tr("toast-caption-failed")));
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

    /// Poll the active model download. Updates ModelInfo progress and
    /// status. Clears `model_download` on any terminal event.
    /// Kick off a MyMemory translation of the given Captions clip's
    /// segments. Runs on a background thread; the receiver is polled
    /// by `drain_translate_job` on every frame.
    fn start_translate_job(&mut self, clip_id: uuid::Uuid) {
        if self.translate_rx.is_some() {
            self.toast(tr("translate-toast-busy"));
            return;
        }
        let Some(clip) = self.project.clips.iter().find(|c| c.id == clip_id) else {
            return;
        };
        let caprust_core::ClipType::Captions { segments, .. } = &clip.clip_type else {
            tracing::warn!("translate: clip {clip_id} is not a Captions clip");
            return;
        };
        if segments.is_empty() {
            self.toast(tr("translate-toast-empty"));
            return;
        }
        let texts: Vec<String> = segments.iter().map(|s| s.text.clone()).collect();
        let req = caprust_core::translate::TranslateRequest {
            texts,
            source: self.settings.translate_source_lang.clone(),
            target: self.settings.translate_target_lang.clone(),
            email: self.settings.translate_email.clone(),
        };
        tracing::info!(
            "translate: starting {} segment(s) {} -> {}",
            req.texts.len(),
            req.source,
            req.target
        );
        self.translate_rx = Some(caprust_core::translate::spawn_translate(req));
        self.translate_source_clip = Some(clip_id);
        self.translate_last_progress = None;
        self.toast(tr("translate-toast-started"));
    }

    /// Poll the in-flight translation. Emits a "starting" toast once,
    /// then a done/failed toast. On success, applies
    /// `TranslateCaptionsCommand` so the new captions clip is
    /// undoable.
    fn drain_translate_job(&mut self) {
        use caprust_core::translate::TranslateEvent;
        let Some(rx) = self.translate_rx.as_ref() else {
            return;
        };
        let mut terminal: Option<TranslateEvent> = None;
        loop {
            match rx.try_recv() {
                Ok(TranslateEvent::Progress { done, total }) => {
                    // Throttle: only note the latest; no per-segment toast.
                    self.translate_last_progress = Some((done, total));
                }
                Ok(ev @ TranslateEvent::Done { .. }) => {
                    terminal = Some(ev);
                    break;
                }
                Ok(ev @ TranslateEvent::Failed(_)) => {
                    terminal = Some(ev);
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.translate_rx = None;
                    self.translate_source_clip = None;
                    self.translate_last_progress = None;
                    return;
                }
            }
        }
        let Some(ev) = terminal else {
            return;
        };
        self.translate_rx = None;
        self.translate_last_progress = None;
        let Some(source_id) = self.translate_source_clip.take() else {
            return;
        };
        match ev {
            TranslateEvent::Done {
                translations,
                skipped,
            } => {
                let target = self.settings.translate_target_lang.clone();
                let cmd = caprust_core::commands::translate_captions::TranslateCaptionsCommand::new(
                    source_id,
                    &target,
                    translations,
                );
                match self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                    Ok(()) => {
                        if skipped.is_empty() {
                            self.toast(format!("{} {target}", tr("translate-toast-done")));
                        } else {
                            self.toast(format!(
                                "{} ({} skipped)",
                                tr("translate-toast-done"),
                                skipped.len()
                            ));
                        }
                        tracing::info!("translate: added {target} captions for clip {source_id}");
                    }
                    Err(e) => {
                        tracing::error!("translate: command failed: {e}");
                        self.toast_error(format!("{} {e}", tr("translate-toast-failed")));
                    }
                }
            }
            TranslateEvent::Failed(msg) => {
                tracing::error!("translate: failed: {msg}");
                self.toast_error(format!("{} {msg}", tr("translate-toast-failed")));
            }
            TranslateEvent::Progress { .. } => {}
        }
    }

    fn drain_model_download(&mut self) {
        use caprust_core::models::DownloadEvent;
        // Take the receiver out so we can mutate self freely inside the
        // loop. Put it back if the stream is still active; drop it on
        // any terminal event (Done / Failed / Disconnected) so the next
        // download can start.
        let Some((model_id, rx)) = self.model_download.take() else {
            return;
        };
        let mut keep = true;
        loop {
            match rx.try_recv() {
                Ok(DownloadEvent::Started { total_bytes }) => {
                    tracing::info!(
                        "model download {}: started (total={:?})",
                        model_id,
                        total_bytes
                    );
                    if let Some(m) = self.project.models.get_mut(&model_id) {
                        m.status = caprust_core::ModelStatus::Downloading;
                        m.progress = 0.0;
                    }
                }
                Ok(DownloadEvent::Progress { downloaded, total }) => {
                    let pct = match total {
                        Some(t) if t > 0 => downloaded as f32 / t as f32,
                        _ => 0.0,
                    };
                    if let Some(m) = self.project.models.get_mut(&model_id) {
                        m.status = caprust_core::ModelStatus::Downloading;
                        m.progress = pct.clamp(0.0, 1.0);
                    }
                }
                Ok(DownloadEvent::Verifying) => {
                    tracing::info!("model download {}: verifying SHA-256", model_id);
                }
                Ok(DownloadEvent::Done) => {
                    tracing::info!("model download {}: done", model_id);
                    if let Some(m) = self.project.models.get_mut(&model_id) {
                        m.status = caprust_core::ModelStatus::Ready;
                        m.progress = 1.0;
                        m.enabled = true;
                    }
                    self.toast(format!("{}: {model_id}", tr("model-toast-ready")));
                    keep = false;
                    break;
                }
                Ok(DownloadEvent::Failed(msg)) => {
                    tracing::error!("model download {}: failed: {msg}", model_id);
                    if let Some(m) = self.project.models.get_mut(&model_id) {
                        m.status = caprust_core::ModelStatus::Error;
                        m.progress = 0.0;
                    }
                    self.toast_error(format!("{}: {msg}", tr("model-toast-failed")));
                    keep = false;
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    tracing::warn!("model download thread vanished");
                    keep = false;
                    break;
                }
            }
        }
        if keep {
            self.model_download = Some((model_id, rx));
        }
    }

    /// Start a background download for the given model id. Uses the
    /// registry's URL and SHA-256 fields, and the effective models
    /// directory. No-op if a download is already in flight or the
    /// model has no URL set.
    fn start_model_download(&mut self, model_id: &str) {
        if self.model_download.is_some() {
            tracing::info!("model download already in flight, ignoring");
            return;
        }
        let Some(m) = self.project.models.models.iter().find(|m| m.id == model_id) else {
            return;
        };
        if m.url.is_empty() {
            tracing::warn!("model {} has no URL set — cannot download", model_id);
            self.toast_error(format!("{}: {}", tr("model-toast-no-url"), m.name));
            return;
        }
        let url = m.url.clone();
        let sha = m.sha256.clone();
        let dir = self.settings.effective_models_dir();
        let (filename, aux_url) = match m.kind {
            // Caption = Whisper GGML .bin; everything else on the
            // registry is ONNX.
            caprust_core::ModelKind::Caption => (format!("{model_id}.bin"), None),
            caprust_core::ModelKind::Narration => (
                format!("{model_id}.onnx"),
                // Piper voices need the sibling .onnx.json config at
                // <url>.json. The runtime (piper.rs) looks for
                // <voice>.onnx.json next to <voice>.onnx.
                Some(format!("{url}.json")),
            ),
            caprust_core::ModelKind::FaceDetector => (format!("{model_id}.onnx"), None),
            caprust_core::ModelKind::ScrfdDetector => (format!("{model_id}.onnx"), None),
            caprust_core::ModelKind::BackgroundRemover => (format!("{model_id}.onnx"), None),
        };
        let target = dir.join(filename);
        tracing::info!(
            "starting model download: {} -> {}",
            model_id,
            target.display()
        );
        let rx = caprust_core::models::spawn_model_download_with_aux(
            model_id.to_string(),
            url,
            target,
            sha,
            aux_url,
        );
        self.model_download = Some((model_id.to_string(), rx));
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
            // Negative values opt into the indeterminate bar; anything
            // else is a percentage and gets clamped to 1.0.
            j.progress = if progress < 0.0 {
                BackgroundJob::INDETERMINATE
            } else {
                progress.min(1.0)
            };
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
                    ui.add_space(space::M);
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
                        ui.add_space(space::L);
                    }
                    ctx.request_repaint();
                });
            });
    }

    /// Report a count of clips skipped by the render planner. Only
    /// toasts when the count changes to a nonzero value, so we do not
    /// spam on every frame while the project sits in a bad state.
    fn report_skipped(&mut self, count: usize) {
        if count == self.last_skipped_missing {
            return;
        }
        self.last_skipped_missing = count;
        if count == 0 {
            return;
        }
        let msg = format!(
            "{count} clip{} skipped — source file missing",
            if count == 1 { "" } else { "s" }
        );
        self.toast(msg);
    }

    /// Push a transient notification. Auto-dismisses after 4s.
    fn toast(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast::new(text));
    }

    /// Push an error toast. Renders with a red accent and stays up
    /// longer than a plain Info toast (see Toast::error).
    ///
    fn toast_error(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast::error(text));
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
            let (alpha, dx) = t.anim_at(now);
            let accent = match t.kind {
                ToastKind::Info => self.theme.accent_color(),
                ToastKind::Error => egui::Color32::from_rgb(220, 80, 80),
            };
            // Slightly brighter than the window fill so the toast
            // reads as elevated against panels of the same hue.
            let toast_fill = ctx
                .style()
                .visuals
                .window_fill
                .linear_multiply(1.9)
                .gamma_multiply(alpha);
            let border =
                egui::Stroke::new(elev::STROKE_HAIRLINE, accent.gamma_multiply(0.55 * alpha));
            let highlight = accent.gamma_multiply(0.7 * alpha);

            egui::Window::new(format!("caprust-toast-{i}"))
                .id(id)
                .resizable(false)
                .collapsible(false)
                .title_bar(false)
                .frame(egui::Frame::NONE)
                .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0 + dx, y))
                .default_width(320.0)
                .show(ctx, |ui| {
                    let inner = egui::Frame::new()
                        .fill(toast_fill)
                        .stroke(border)
                        .corner_radius(radius::cr(radius::LG))
                        .inner_margin(egui::Margin::symmetric(space::L as i8, space::M_PLUS as i8))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(&t.text)
                                        .size(13.0)
                                        .color(egui::Color32::from_gray(235).gamma_multiply(alpha)),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.button(ph::X).clicked() {
                                            dismiss = Some(i);
                                        }
                                    },
                                );
                            });
                        });

                    // 1px accent line along the top edge: a cheap
                    // fake drop shadow that reads as "this is on
                    // top of the editor".
                    let rect = inner.response.rect;
                    ui.painter().line_segment(
                        [
                            egui::pos2(rect.left() + 8.0, rect.top() + 0.5),
                            egui::pos2(rect.right() - 8.0, rect.top() + 0.5),
                        ],
                        egui::Stroke::new(elev::STROKE_HAIRLINE, highlight),
                    );
                });

            y += 62.0;
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
                        ui.add_space(space::XXS);
                        ui.label(
                            egui::RichText::new(format!(
                                "{} → {}",
                                info.current_version, info.latest_version
                            ))
                            .small()
                            .color(egui::Color32::from_gray(200)),
                        );
                        ui.add_space(space::S);
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
        if seeked {
            if self.preview.playing {
                self.explicit_seek_ms = Some(self.playhead_ms);
                self.playback_started_at = Some(std::time::Instant::now());
                self.playback_started_ms = self.playhead_ms;
            } else {
                self.paused_frame_dirty = true;
            }
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
    /// Full-width warning banner above the timeline when media files
    /// referenced by the project are missing on disk. The Relink
    /// button opens the standard relink dialog with the current list
    /// of missing entries.
    /// Enqueue a waveform job for every Audio/Video media item that
    /// does not already have a `.bin` in the project cache. Called
    /// once per project load. Catches media imported before the
    /// waveform pipeline existed (B.2), and projects opened on a
    /// different machine where the cache is empty.
    fn backfill_waveforms(&mut self) {
        let Some(proj_path) = self.project.project_path.clone() else {
            return;
        };
        let mut enqueued = 0usize;
        let mut marked = 0usize;
        let items: Vec<(uuid::Uuid, caprust_core::MediaKind, String, bool)> = self
            .project
            .media
            .items
            .iter()
            .map(|m| (m.id, m.kind, m.path.clone(), m.waveform_done))
            .collect();
        let proj = std::path::Path::new(&proj_path);
        for (id, kind, path, done) in items {
            if !matches!(
                kind,
                caprust_core::MediaKind::Audio | caprust_core::MediaKind::Video
            ) {
                continue;
            }
            if caprust_core::cache::waveform_exists(proj, id) {
                if !done {
                    if let Some(m) = self.project.media.items.iter_mut().find(|m| m.id == id) {
                        m.waveform_done = true;
                        marked += 1;
                    }
                }
                continue;
            }
            // Cache file missing. Clear the stale flag so the UI treats
            // this item as pending again (thumbnail / waveform strip
            // draws nothing while `*_done == false`).
            if done {
                if let Some(m) = self.project.media.items.iter_mut().find(|m| m.id == id) {
                    m.waveform_done = false;
                }
            }
            let _ = self.job_runner.tx.send(crate::media_jobs::Job::Waveform {
                media_id: id,
                path: std::path::PathBuf::from(&path),
                ffmpeg: self
                    .ffmpeg_status
                    .ffmpeg
                    .clone()
                    .map(std::path::PathBuf::from),
            });
            enqueued += 1;
        }
        if marked > 0 {
            tracing::info!("backfill: {marked} media items already had waveforms on disk");
        }
        if enqueued > 0 {
            tracing::info!("backfill: enqueued {enqueued} waveform jobs");
        }
    }

    fn show_missing_media_banner(&mut self, ui: &mut egui::Ui) {
        let missing = caprust_core::commands::relink_many::find_missing_media_items(&self.project);
        if missing.is_empty() {
            // Nothing missing: forget any past dismissal so a future
            // regression (user moves a file out from under us, project
            // reloaded) re-opens the banner.
            self.missing_media_dismissed = false;
            return;
        }
        if self.missing_media_dismissed {
            return;
        }

        let msg = format!("{} ({})", tr("missing-media-banner"), missing.len());
        let mut open_relink = false;
        let dismissed = crate::widgets::banner::show_with_actions(
            ui,
            crate::widgets::banner::BannerKind::Warning,
            &msg,
            true,
            |ui| {
                if ui.button(tr("missing-media-relink")).clicked() {
                    open_relink = true;
                }
            },
        );

        if dismissed {
            self.missing_media_dismissed = true;
        }
        if open_relink {
            self.relink_dialog = crate::panels::relink_dialog::RelinkDialogState {
                missing,
                ..Default::default()
            };
            self.relink_dialog_open = true;
        }
    }

    fn show_timeline(&mut self, ctx: &egui::Context) {
        let screen_h = ctx.screen_rect().height();
        egui::TopBottomPanel::bottom("timeline")
            .resizable(true)
            .default_height(280.0)
            .min_height(180.0)
            .max_height(screen_h * 0.8)
            .show(ctx, |ui| {
                self.render_timeline_panel(ui);
            });
    }
    /// Timeline panel body. Migrated out of the TopBottomPanel
    /// closure so the dock viewer can render it inside a Tab::Timeline
    /// zone. `ctx` stays a `&Context` (shadowing the cloned owned
    /// value) so the extracted body compiles unchanged.
    pub(crate) fn render_timeline_panel(&mut self, ui: &mut egui::Ui) {
        let ctx_owned = ui.ctx().clone();
        let ctx: &egui::Context = &ctx_owned;
        ui.set_min_height(180.0);
        self.project.models.tick_downloads(1.0 / 60.0);
        self.last_pointer = ctx.input(|i| i.pointer.hover_pos());

        // Persistent banner when media files referenced by the
        // project are not on disk. Auto-hides when everything is
        // present again; the Relink CTA opens the relink dialog.
        self.show_missing_media_banner(ui);

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
        let mut pending_drop: Option<(Vec<uuid::Uuid>, usize, u64)> = None;

        // DnD
        let dnd_active = egui::DragAndDrop::payload::<Vec<uuid::Uuid>>(ctx).map(|a| (*a).clone());
        if let Some(ref ids) = dnd_active {
            self.last_dnd_payload = Some(ids.clone());
        }
        let pointer_hover = ctx.input(|i| i.pointer.hover_pos());
        let pointer_released = ctx.input(|i| i.pointer.any_released());
        let pointer_down = ctx.input(|i| i.pointer.primary_down());
        let dnd_drop = if pointer_released {
            self.last_dnd_payload.clone()
        } else {
            None
        };
        let clip_drag_snapshot = self.clip_drag.clone();
        let pan_mode = self.timeline_tools.pan_mode;

        // Time/px
        let total_ms = self.total_duration_ms();
        let content_ms = (total_ms + 20_000).max(30_000);
        // The horizontal layout adds item_spacing between the header
        // and lane columns; without subtracting it the lanes overflow
        // by a few px and the right edge clips.
        let viewport_w = (ui.available_width() - header_w - ui.spacing().item_spacing.x).max(120.0);
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
        let tl_resp = ui.horizontal_top(|ui| {
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
                        let h_resp = ui.allocate_ui_with_layout(
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
                        // Row-height drag divider: bottom 4px of the header
                        // row. Lanes read the same row_h, so both columns
                        // stay in sync without layout changes. Direct write
                        // like the header chips (not undoable).
                        let div_rect = egui::Rect::from_min_size(
                            egui::pos2(
                                h_resp.response.rect.left(),
                                h_resp.response.rect.bottom() - 4.0,
                            ),
                            egui::vec2(header_w, 4.0),
                        );
                        let div = ui.interact(
                            div_rect,
                            egui::Id::new(("track_height_div", idx)),
                            egui::Sense::drag(),
                        );
                        if div.hovered() || div.dragged() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                        } else if pan_mode {
                            let _ = h_resp.response
                                .on_hover_cursor(egui::CursorIcon::Grab);
                        }
                        ui.painter().line_segment(
                            [
                                egui::pos2(div_rect.left(), div_rect.bottom() - 0.5),
                                egui::pos2(div_rect.right(), div_rect.bottom() - 0.5),
                            ],
                            egui::Stroke::new(
                                1.0,
                                if div.hovered() || div.dragged() {
                                    egui::Color32::from_gray(120)
                                } else {
                                    egui::Color32::from_gray(45)
                                },
                            ),
                        );
                        if div.dragged() {
                            let dy = div.drag_delta().y;
                            if dy != 0.0 {
                                track.height = (row_h + dy).clamp(40.0, 160.0);
                                header_changed = true;
                            }
                        }
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
                    let (ruler_rect, _) = ui
                        .allocate_exact_size(egui::vec2(viewport_w, ruler_h), egui::Sense::click());
                    let rp = ui.painter_at(ruler_rect);
                    rp.rect_filled(ruler_rect, 0.0, egui::Color32::from_gray(28));
                    rp.line_segment(
                        [
                            egui::Pos2::new(ruler_rect.left(), ruler_rect.bottom() - 0.5),
                            egui::Pos2::new(ruler_rect.right(), ruler_rect.bottom() - 0.5),
                        ],
                        egui::Stroke::new(elev::STROKE_HAIRLINE, egui::Color32::from_gray(50)),
                    );
                    let interval_ms: u64 = {
                        let cands: &[u64] = &[
                            100, 250, 500, 1000, 2000, 5000, 10_000, 15_000, 30_000, 60_000,
                            120_000, 300_000, 600_000,
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
                    let first_tick = ((scroll_x / px_per_ms) as u64 / interval_ms) * interval_ms;
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
                                egui::Stroke::new(
                                    elev::STROKE_HAIRLINE,
                                    egui::Color32::from_gray(90),
                                ),
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
                    // Beat markers for analysed audio clips. Media-time
                    // beats map onto the timeline through the clip offset;
                    // speed-ramped clips are skipped (timing no longer
                    // matches). Skipped when nothing is stored: no grid,
                    // no markers.
                    {
                        let accent = theme_snapshot.accent_color();
                        for clip in &self.project.clips {
                            let mid = match clip.media_id {
                                Some(id) => id,
                                None => continue,
                            };
                            let entry = match self.project.beat_grids.get(&mid) {
                                Some(e) => e,
                                None => continue,
                            };
                            if (clip.speed - 1.0).abs() > 0.01 {
                                continue;
                            }
                            let clip_end =
                                clip.start_time_ms.saturating_add(clip.duration_ms);
                            for &b in &entry.beats_ms {
                                let t = clip
                                    .start_time_ms
                                    .saturating_add(b)
                                    .saturating_sub(clip.source_offset_ms);
                                if t < clip.start_time_ms || t > clip_end {
                                    continue;
                                }
                                let x = ruler_rect.left() + t as f32 * px_per_ms - scroll_x;
                                if x < ruler_rect.left() || x > ruler_rect.right() {
                                    continue;
                                }
                                rp.line_segment(
                                    [
                                        egui::Pos2::new(x, ruler_rect.top() + 2.0),
                                        egui::Pos2::new(x, ruler_rect.top() + 9.0),
                                    ],
                                    egui::Stroke::new(1.5_f32, accent),
                                );
                            }
                        }
                    }
                    let ph_x = ruler_rect.left() + self.playhead_ms as f32 * px_per_ms - scroll_x;
                    let ph_visible = ph_x >= ruler_rect.left() && ph_x <= ruler_rect.right();
                    // Playhead line is drawn after the lane loop:
                    // Full mode needs the bottom of the last lane,
                    // which is only known once every row has been
                    // allocated. Compact mode will use ruler_rect
                    // alone, but we defer both for a single code
                    // path.
                    if ui.input(|i| i.pointer.primary_clicked()) {
                        if let Some(p) = ui.ctx().pointer_interact_pos() {
                            if ruler_rect.contains(p) {
                                let ms = ((p.x - ruler_rect.left() + scroll_x) / px_per_ms).max(0.0)
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
                        let (lane_rect, lane_resp) = ui.allocate_exact_size(
                            egui::vec2(viewport_w, row_h),
                            egui::Sense::hover(),
                        );
                        if pan_mode {
                            let _ = lane_resp
                                .on_hover_cursor(egui::CursorIcon::Grab);
                        }
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
                                    egui::Pos2::new(lane_rect.left(), lane_rect.top() + 2.0),
                                    egui::Pos2::new(lane_rect.left(), lane_rect.bottom() - 2.0),
                                ],
                                egui::Stroke::new(3.0_f32, egui::Color32::from_rgb(80, 200, 120)),
                            );
                        }
                        p.line_segment(
                            [
                                egui::Pos2::new(lane_rect.left(), lane_rect.bottom() - 0.5),
                                egui::Pos2::new(lane_rect.right(), lane_rect.bottom() - 0.5),
                            ],
                            egui::Stroke::new(elev::STROKE_HAIRLINE, egui::Color32::from_gray(35)),
                        );

                        let clips_here: Vec<(uuid::Uuid, u64, u64, caprust_core::ClipType, bool)> =
                            self.project
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
                                    let (is_dragged, in_group, group_orig_ms) = clip_drag_snapshot
                                        .as_ref()
                                        .map(|d| {
                                            let dragged = d.clip_id == c.id;
                                            let orig = d
                                                .group
                                                .iter()
                                                .find(|(id, _, _)| *id == c.id)
                                                .map(|(_, ms, _)| *ms);
                                            (dragged, orig.is_some(), orig)
                                        })
                                        .unwrap_or((false, false, None));
                                    let (s, d) = if let Some(drag) = clip_drag_snapshot.as_ref() {
                                        if is_dragged {
                                            // Trim drags mutate the clip's
                                            // start_time_ms / duration_ms
                                            // directly (live feedback). Use
                                            // those, not drag.current_ms,
                                            // which tracks the pointer and
                                            // for a RIGHT trim would
                                            // visually slide the clip right
                                            // even though its start is
                                            // fixed. Body drags use
                                            // current_ms as the visual
                                            // position.
                                            if drag.trim_edge.is_some() {
                                                (c.start_time_ms, c.duration_ms)
                                            } else {
                                                (drag.current_ms.max(0) as u64, c.duration_ms)
                                            }
                                        } else if in_group {
                                            // Shift by the same delta as
                                            // the dragged clip so the whole
                                            // selection follows the cursor
                                            // visually.
                                            let delta = drag.current_ms - drag.origin_ms as i64;
                                            let orig = group_orig_ms.unwrap_or(c.start_time_ms);
                                            ((orig as i64 + delta).max(0) as u64, c.duration_ms)
                                        } else {
                                            (c.start_time_ms, c.duration_ms)
                                        }
                                    } else {
                                        (c.start_time_ms, c.duration_ms)
                                    };
                                    (c.id, s, d, c.clip_type.clone(), is_dragged)
                                })
                                .collect();

                        for (clip_id, start_ms, dur_ms, ctype, is_dragged) in clips_here {
                            let x0 = lane_rect.left() - scroll_x + start_ms as f32 * px_per_ms;
                            let x1 = lane_rect.left() - scroll_x
                                + (start_ms + dur_ms) as f32 * px_per_ms;
                            if x1 < lane_rect.left() - 30.0 || x0 > lane_rect.right() + 30.0 {
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
                            let border_color = theme_snapshot.clip_border_color(border_kind);
                            p.rect_stroke(
                                clip_rect,
                                4.0,
                                egui::Stroke::new(1.5_f32, border_color),
                                egui::StrokeKind::Inside,
                            );

                            // Missing-source hatch: diagonal red lines over the clip
                            // fill so a broken clip is visible even when its
                            // clip-type color would otherwise look normal.
                            let path_missing = match &ctype {
                                caprust_core::ClipType::Video { path, .. }
                                | caprust_core::ClipType::Audio { path, .. }
                                | caprust_core::ClipType::Image { path, .. } => {
                                    !std::path::Path::new(path).exists()
                                }
                                _ => false,
                            };
                            if path_missing {
                                let hp = p.with_clip_rect(clip_rect);
                                let stroke = egui::Stroke::new(
                                    1.5_f32,
                                    egui::Color32::from_rgba_unmultiplied(220, 60, 60, 130),
                                );
                                let h = clip_rect.height();
                                let step = 10.0_f32;
                                let mut x = clip_rect.left() - h;
                                while x < clip_rect.right() {
                                    hp.line_segment(
                                        [
                                            egui::pos2(x, clip_rect.bottom()),
                                            egui::pos2(x + h, clip_rect.top()),
                                        ],
                                        stroke,
                                    );
                                    x += step;
                                }
                            }

                            // Thumbnail strip: lookup clip's media_id → texture, tile across clip width.
                            let thumb_tex = {
                                // Lazy-load from <project>/cache/thumbnails/<id>.jpg
                                // on first use. ThumbDone only fires during the
                                // session that generated the JPEG; on a fresh
                                // project open the file is on disk but the
                                // texture handle is not in `clip_textures` yet.
                                // Same pattern as `waveform_peaks_for`.
                                let mid_opt = self
                                    .project
                                    .clips
                                    .iter()
                                    .find(|cc| cc.id == clip_id)
                                    .and_then(|cc| cc.media_id);
                                match mid_opt {
                                    Some(mid) => self.thumbnail_texture_for(ctx, mid),
                                    None => None,
                                }
                            };

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

                            // Crossfade overlap shading. When the
                            // clip carries an xfade on its in-edge
                            // the model shifted it left by D ms so
                            // it overlaps the predecessor. Tint the
                            // overlapping slice with the accent so
                            // the user sees the crossfade footprint
                            // directly on the timeline. Drawn after
                            // the dim overlay, before the selection
                            // stroke, so a selected clip still
                            // reads as selected.
                            let overlap_ms = self
                                .project
                                .clips
                                .iter()
                                .find(|cc| cc.id == clip_id)
                                .map(|cc| cc.applied_xfade_shift_ms)
                                .unwrap_or(0);
                            if overlap_ms > 0 {
                                let overlap_px = (overlap_ms as f32 * px_per_ms).max(2.0);
                                let overlap_rect = egui::Rect::from_min_size(
                                    clip_rect.min,
                                    egui::vec2(
                                        overlap_px.min(clip_rect.width()),
                                        clip_rect.height(),
                                    ),
                                );
                                // Overlap shading, user-tunable in
                                // Settings -> Appearance. Alpha is part
                                // of the theme value.
                                p.rect_filled(
                                    overlap_rect,
                                    4.0,
                                    theme_snapshot.overlap_shading_color(),
                                );
                                // Thin diagonal hatching for
                                // unambiguous "this slice is a
                                // crossfade" read at a glance.
                                let hatch_stroke = egui::Stroke::new(
                                    1.0_f32,
                                    theme_snapshot.accent_color().gamma_multiply(0.9),
                                );
                                let h_ov = overlap_rect.height();
                                let step_ov = 8.0_f32;
                                let mut hx = overlap_rect.left() - h_ov;
                                while hx < overlap_rect.right() {
                                    let x0 = hx.max(overlap_rect.left());
                                    let x1 = (hx + h_ov).min(overlap_rect.right());
                                    if x1 > x0 {
                                        // Clip the line to the
                                        // overlap slice by lerping
                                        // along the diagonal.
                                        let t0 = (x0 - hx) / h_ov;
                                        let t1 = (x1 - hx) / h_ov;
                                        let y0 = overlap_rect.bottom() - t0 * h_ov;
                                        let y1 = overlap_rect.bottom() - t1 * h_ov;
                                        p.line_segment(
                                            [egui::pos2(x0, y0), egui::pos2(x1, y1)],
                                            hatch_stroke,
                                        );
                                    }
                                    hx += step_ov;
                                }
                            }

                            if self.selected_clips.contains(&clip_id) || is_dragged {
                                p.rect_stroke(
                                    clip_rect,
                                    4.0,
                                    egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
                                    egui::StrokeKind::Inside,
                                );
                            }

                            // ---- Fade handles + curve ----
                            // Only on clips that carry audio: Audio,
                            // Narration, and Video whose audio has
                            // not been detached.
                            let carries_audio = match &ctype {
                                caprust_core::ClipType::Audio { .. }
                                | caprust_core::ClipType::Narration { .. } => true,
                                caprust_core::ClipType::Video { .. } => self
                                    .project
                                    .clips
                                    .iter()
                                    .find(|cc| cc.id == clip_id)
                                    .map(|cc| !cc.audio_detached)
                                    .unwrap_or(false),
                                _ => false,
                            };

                            if carries_audio && clip_rect.width() > 24.0 {
                                // Waveform peaks drawn behind the fade
                                // curve, over the clip background.
                                // Resolution is capped at one bar per
                                // 2 px so the segment count stays
                                // small regardless of clip width.
                                if let Some(media_id) = self
                                    .project
                                    .clips
                                    .iter()
                                    .find(|cc| cc.id == clip_id)
                                    .and_then(|cc| cc.media_id)
                                {
                                    if let Some(peaks) = self.waveform_peaks_for(media_id) {
                                        let bar_w = 2.0_f32;
                                        let n_bars = ((clip_rect.width() / bar_w) as usize).max(1);
                                        let center_y = clip_rect.center().y;
                                        let amp = clip_rect.height() * 0.35;
                                        let wf = theme_snapshot.waveform_color();
                                        let stroke = egui::Stroke::new(
                                            bar_w * 0.75,
                                            egui::Color32::from_rgba_unmultiplied(
                                                wf.r(),
                                                wf.g(),
                                                wf.b(),
                                                200,
                                            ),
                                        );
                                        // The peaks span the whole clip. When the
                                        // clip is wider than the viewport we must
                                        // start drawing at the peak index that
                                        // corresponds to `clip_rect.left()`, and
                                        // stop at the one for `clip_rect.right()`,
                                        // so the waveform slides with the scroll
                                        // instead of being glued to the viewport.
                                        let clip_full_left = lane_rect.left() - scroll_x
                                            + start_ms as f32 * px_per_ms;
                                        let clip_full_width = (dur_ms as f32 * px_per_ms).max(1.0);
                                        let left_off = clip_rect.left() - clip_full_left;
                                        let right_off = clip_rect.right() - clip_full_left;
                                        let start_frac =
                                            (left_off / clip_full_width).clamp(0.0, 1.0);
                                        let end_frac =
                                            (right_off / clip_full_width).clamp(0.0, 1.0);
                                        let start_idx =
                                            (peaks.len() as f32 * start_frac) as usize;
                                        let end_idx =
                                            ((peaks.len() as f32 * end_frac).ceil() as usize)
                                                .min(peaks.len());
                                        let visible = end_idx.saturating_sub(start_idx).max(1);
                                        let stride = (visible / n_bars).max(1);
                                        let clip = ui.painter().with_clip_rect(clip_rect);
                                        let mut i = start_idx;
                                        let mut x = clip_rect.left();
                                        while i < end_idx && x < clip_rect.right() {
                                            let v = peaks[i];
                                            let h = v * amp;
                                            if h >= 0.5 {
                                                clip.line_segment(
                                                    [
                                                        egui::pos2(x, center_y - h),
                                                        egui::pos2(x, center_y + h),
                                                    ],
                                                    stroke,
                                                );
                                            }
                                            x += bar_w;
                                            i += stride;
                                        }
                                    }
                                }
                                let (fi_ms, fo_ms) = self
                                    .project
                                    .clips
                                    .iter()
                                    .find(|cc| cc.id == clip_id)
                                    .map(|cc| (cc.fade_in_ms, cc.fade_out_ms))
                                    .unwrap_or((0, 0));
                                // Use live drag values when this clip
                                // is being dragged, so the curve
                                // follows the pointer without a
                                // round-trip through the project.
                                let (fi_show, fo_show) = if let Some(fd) = &self.fade_drag {
                                    if fd.clip_id == clip_id {
                                        match fd.edge {
                                            FadeEdge::In => (fd.current_ms, fo_ms),
                                            FadeEdge::Out => (fi_ms, fd.current_ms),
                                        }
                                    } else {
                                        (fi_ms, fo_ms)
                                    }
                                } else {
                                    (fi_ms, fo_ms)
                                };
                                let fi_px =
                                    (fi_show as f32 / dur_ms.max(1) as f32) * clip_rect.width();
                                let fo_px =
                                    (fo_show as f32 / dur_ms.max(1) as f32) * clip_rect.width();

                                let curve_color =
                                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 180);
                                let curve_stroke = egui::Stroke::new(2.0_f32, curve_color);

                                // Fade-in curve: diagonal from
                                // top-left down to the top of the
                                // waveform at fi_px.
                                if fi_px > 0.5 {
                                    p.line_segment(
                                        [
                                            clip_rect.left_top(),
                                            egui::pos2(clip_rect.left() + fi_px, clip_rect.top()),
                                        ],
                                        curve_stroke,
                                    );
                                    // Triangle fill under curve
                                    p.add(egui::Shape::convex_polygon(
                                        vec![
                                            clip_rect.left_top(),
                                            egui::pos2(clip_rect.left() + fi_px, clip_rect.top()),
                                            clip_rect.left_bottom(),
                                        ],
                                        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 60),
                                        egui::Stroke::NONE,
                                    ));
                                }
                                // Fade-out curve (mirror).
                                if fo_px > 0.5 {
                                    p.line_segment(
                                        [
                                            egui::pos2(clip_rect.right() - fo_px, clip_rect.top()),
                                            clip_rect.right_top(),
                                        ],
                                        curve_stroke,
                                    );
                                    p.add(egui::Shape::convex_polygon(
                                        vec![
                                            egui::pos2(clip_rect.right() - fo_px, clip_rect.top()),
                                            clip_rect.right_top(),
                                            clip_rect.right_bottom(),
                                        ],
                                        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 60),
                                        egui::Stroke::NONE,
                                    ));
                                }

                                // Handles: small filled circles at
                                // top corners, offset horizontally
                                // by the fade amount.
                                let hr = 5.0_f32;
                                let h_in_pos = egui::pos2(
                                    clip_rect.left() + fi_px.max(hr),
                                    clip_rect.top() + hr * 0.6,
                                );
                                let h_out_pos = egui::pos2(
                                    clip_rect.right() - fo_px.max(hr),
                                    clip_rect.top() + hr * 0.6,
                                );
                                let hovered_this_clip = pointer_hover
                                    .map(|pp| clip_rect.contains(pp))
                                    .unwrap_or(false);
                                let in_hover = hovered_this_clip
                                    && pointer_hover
                                        .map(|pp| (pp - h_in_pos).length() < hr * 1.8)
                                        .unwrap_or(false);
                                let out_hover = hovered_this_clip
                                    && pointer_hover
                                        .map(|pp| (pp - h_out_pos).length() < hr * 1.8)
                                        .unwrap_or(false);
                                let active_in = self
                                    .fade_drag
                                    .as_ref()
                                    .map(|fd| fd.clip_id == clip_id && fd.edge == FadeEdge::In)
                                    .unwrap_or(false);
                                let active_out = self
                                    .fade_drag
                                    .as_ref()
                                    .map(|fd| fd.clip_id == clip_id && fd.edge == FadeEdge::Out)
                                    .unwrap_or(false);
                                let fill_in = if active_in || in_hover {
                                    egui::Color32::from_rgb(255, 220, 90)
                                } else {
                                    egui::Color32::from_white_alpha(200)
                                };
                                let fill_out = if active_out || out_hover {
                                    egui::Color32::from_rgb(255, 220, 90)
                                } else {
                                    egui::Color32::from_white_alpha(200)
                                };
                                p.circle_filled(h_in_pos, hr, fill_in);
                                p.circle_stroke(
                                    h_in_pos,
                                    hr,
                                    egui::Stroke::new(elev::STROKE_HAIRLINE, egui::Color32::BLACK),
                                );
                                p.circle_filled(h_out_pos, hr, fill_out);
                                p.circle_stroke(
                                    h_out_pos,
                                    hr,
                                    egui::Stroke::new(elev::STROKE_HAIRLINE, egui::Color32::BLACK),
                                );

                                // Cursor affordance.
                                if in_hover || out_hover {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                                }

                                // Start a fade drag on press over a
                                // handle. Precedence over clip
                                // drag-select.
                                let track_locked_fh = self
                                    .project
                                    .tracks
                                    .get(idx)
                                    .map(|t| t.locked)
                                    .unwrap_or(false);
                                if !track_locked_fh
                                    && !pan_mode
                                    && pointer_down
                                    && self.fade_drag.is_none()
                                    && clip_drag_snapshot.is_none()
                                {
                                    if in_hover {
                                        pending_actions.push(ClipAction::FadeDragStart(
                                            clip_id,
                                            FadeEdge::In,
                                            fi_ms as f32,
                                        ));
                                    } else if out_hover {
                                        pending_actions.push(ClipAction::FadeDragStart(
                                            clip_id,
                                            FadeEdge::Out,
                                            fo_ms as f32,
                                        ));
                                    }
                                }
                            }
                            // Audio envelope overlay — volume automation
                            // polyline + ducking zones. Drawn after the
                            // waveform + fade curves, before the clip
                            // label so it sits on top of the waveform
                            // but under the text. See DIRECTIVES §14,
                            // issue #14.
                            if carries_audio {
                                if let Some(clip_data) = self
                                    .project
                                    .clips
                                    .iter()
                                    .find(|c| c.id == clip_id)
                                    .cloned()
                                {
                                    let (samples, zones) = self.envelope_for(&clip_data);
                                    let line_color = theme_snapshot.waveform_color();
                                    // Per-zone opacity comes from the DuckZone
                                    // itself (derived from duck_reduction_db).
                                    let duck_color = line_color;
                                    crate::timeline::draw_audio_envelope(
                                        &p,
                                        full_rect,
                                        clip_rect,
                                        &samples,
                                        &zones,
                                        line_color,
                                        duck_color,
                                    );
                                }
                            }

                            let (label_full, label_short) = {
                                let full = self
                                    .project
                                    .clips
                                    .iter()
                                    .find(|cc| cc.id == clip_id)
                                    .and_then(|cc| cc.name.clone())
                                    .unwrap_or_else(|| match &ctype {
                                        caprust_core::ClipType::TextOverlay { content, .. } => {
                                            content.clone()
                                        }
                                        caprust_core::ClipType::Captions { .. } => {
                                            format!("{} Captions", ph::CHAT_TEXT)
                                        }
                                        caprust_core::ClipType::Narration { .. } => {
                                            format!("{} Narration", ph::MICROPHONE)
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
                                egui::Sense::click_and_drag(),
                            );
                            // Double-click on a clip → focus its
                            // TextOverlay content editor in the
                            // Properties panel. For non-text clips
                            // the dispatcher clears any stale flag.
                            if resp.double_clicked() {
                                pending_actions.push(ClipAction::FocusTextContent(clip_id));
                            }
                            // Selection and drag are driven by egui's
                            // interaction result, not by geometric
                            // rect containment. rect_contains_pointer
                            // is true for EVERY widget under the
                            // pointer, so on overlapping clips (e.g.
                            // Overlay over V1 at the same x) the last
                            // one in the loop always won the drag and
                            // the click selected the wrong clip.
                            // egui arbitrates z-order for these
                            // callbacks so only the topmost fires.
                            // Selection is driven from the drag-start
                            // path below (see pointer_down block).
                            // Firing Select here as well would toggle
                            // twice on a Ctrl+click (once on press,
                            // once on release), cancelling out.

                            let pointer_on_clip = ui.rect_contains_pointer(clip_rect);
                            const TRIM_ZONE: f32 = 8.0;
                            let hovered_edge: Option<TrimEdge> = if pointer_on_clip
                                && clip_rect.width() > TRIM_ZONE * 3.0
                            {
                                if let Some(pp) = pointer_hover {
                                    let lz = egui::Rect::from_min_size(
                                        clip_rect.min,
                                        egui::vec2(TRIM_ZONE, clip_rect.height()),
                                    );
                                    let rz = egui::Rect::from_min_size(
                                        egui::pos2(clip_rect.max.x - TRIM_ZONE, clip_rect.min.y),
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
                                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                            }
                            // Trim edge computed from the PRESS
                            // position (resp.interact_pointer_pos),
                            // not the live pointer. drag_started()
                            // fires after 3-4 px of movement, by
                            // which point the live pointer has
                            // already left the 8 px trim zone and
                            // hovered_edge would be None. The press
                            // position is stable for the whole drag.
                            let trim_edge_at_press: Option<TrimEdge> = {
                                // Test rect containment against the
                                // PRESS position, not the live pointer.
                                // drag_started() fires after 3-4 px of
                                // movement; by then the live pointer
                                // has already left the clip rect and
                                // pointer_on_clip would be false,
                                // demoting the gesture to a body drag.
                                let press_pos = ui.input(|i| i.pointer.press_origin());
                                let inside_at_press = press_pos
                                    .map(|pp| clip_rect.contains(pp))
                                    .unwrap_or(false);
                                if inside_at_press
                                    && clip_rect.width() > TRIM_ZONE * 3.0
                                {
                                    if let Some(pp) = press_pos {
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
                                }
                            };
                            // Selection and drag are driven by
                            // egui's interaction result, not by
                            // geometric rect containment.
                            // rect_contains_pointer is true for
                            // EVERY widget under the pointer, so on
                            // overlapping clips (Overlay over V1 at
                            // the same x) the last one in the loop
                            // always won the drag and the click
                            // selected the wrong clip. egui
                            // arbitrates z-order for these
                            // callbacks so only the topmost fires.
                            if resp.clicked() {
                                pending_actions.push(ClipAction::Select(clip_id));
                            }
                            if resp.drag_started() {
                                tracing::info!(
                                    "drag_started clip={} pointer_on_clip={} trim_edge_at_press={:?} clip_w={:.1}",
                                    clip_id,
                                    pointer_on_clip,
                                    trim_edge_at_press,
                                    clip_rect.width(),
                                );
                                let track_locked_here = self
                                    .project
                                    .tracks
                                    .get(idx)
                                    .map(|t| t.locked)
                                    .unwrap_or(false);
                                if !track_locked_here && !pan_mode {
                                    if !self.selected_clips.contains(&clip_id) {
                                        pending_actions.push(ClipAction::Select(clip_id));
                                    }
                                    pending_actions
                                        .push(ClipAction::DragStart(clip_id, idx, start_ms));
                                    if let Some(e) = trim_edge_at_press {
                                        pending_actions
                                            .push(ClipAction::SetTrimEdge(clip_id, Some(e)));
                                    }
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
                                // --- Create multicam group ---
                                // Shown only when two or more clips are
                                // selected: a multicam group is meaningless
                                // for a single angle. Cloned ids because the
                                // action queue is drained after the closure.
                                if self.selected_clips.len() >= 2 {
                                    let lbl = tr("clip-ctx-create-multicam");
                                    if ui.button(lbl).clicked() {
                                        pending_actions.push(
                                            ClipAction::CreateMultiCamGroup(
                                                self.selected_clips.clone(),
                                            ),
                                        );
                                        ui.close_menu();
                                    }
                                }
                                // --- Speed submenu ---
                                ui.menu_button(tr("clip-ctx-speed"), |ui| {
                                    for v in [0.25_f32, 0.5, 1.0, 1.5, 2.0, 4.0] {
                                        let label = format!("{v:.2}x");
                                        if ui.button(label).clicked() {
                                            pending_actions.push(ClipAction::SetSpeed(clip_id, v));
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
                                    pending_actions.push(ClipAction::GenerateCaptions(clip_id));
                                    ui.close_menu();
                                }
                                // Translate captions only makes sense on a
                                // clip that already has segments.
                                let has_captions = self
                                    .project
                                    .clips
                                    .iter()
                                    .find(|c| c.id == clip_id)
                                    .map(|c| {
                                        matches!(
                                            &c.clip_type,
                                            caprust_core::ClipType::Captions { segments, .. }
                                                if !segments.is_empty()
                                        )
                                    })
                                    .unwrap_or(false);
                                if has_captions
                                    && ui.button(tr("clip-ctx-translate-captions")).clicked()
                                {
                                    pending_actions.push(ClipAction::TranslateCaptions(clip_id));
                                    ui.close_menu();
                                }
                                // Audio detach / reattach. Three
                                // cases:
                                // 1. Video with embedded audio ->
                                //    Separate audio.
                                // 2. Video whose audio has been
                                //    detached -> Reattach audio.
                                // 3. Audio clip that was detached
                                //    from some video -> Reattach to
                                //    that video (reverse lookup
                                //    through the back-reference).
                                let menu_clip =
                                    self.project.clips.iter().find(|c| c.id == clip_id).cloned();
                                if let Some(c) = menu_clip.as_ref() {
                                    let is_video =
                                        matches!(c.clip_type, caprust_core::ClipType::Video { .. });
                                    let is_audio =
                                        matches!(c.clip_type, caprust_core::ClipType::Audio { .. });
                                    if is_video
                                        && !c.audio_detached
                                        && ui.button(tr("clip-ctx-separate-audio")).clicked()
                                    {
                                        pending_actions.push(ClipAction::SeparateAudio(clip_id));
                                        ui.close_menu();
                                    }
                                    if is_video
                                        && c.audio_detached
                                        && ui.button(tr("clip-ctx-reattach-audio")).clicked()
                                    {
                                        pending_actions.push(ClipAction::ReattachAudio(clip_id));
                                        ui.close_menu();
                                    }
                                    if is_audio {
                                        // Find the video whose back-ref
                                        // points at this audio clip.
                                        let parent_video = self
                                            .project
                                            .clips
                                            .iter()
                                            .find(|cc| cc.detached_audio_clip_id == Some(clip_id))
                                            .map(|cc| cc.id);
                                        if let Some(parent) = parent_video {
                                            if ui.button(tr("clip-ctx-reattach-to-video")).clicked()
                                            {
                                                pending_actions
                                                    .push(ClipAction::ReattachAudio(parent));
                                                ui.close_menu();
                                            }
                                        }
                                    }
                                }
                                if self.settings.enable_shortcuts {
                                    ui.separator();
                                    if ui
                                        .button(format!("{}  (R)", tr("clip-ctx-reverse")))
                                        .clicked()
                                    {
                                        pending_actions.push(ClipAction::ToggleReverse(clip_id));
                                        ui.close_menu();
                                    }
                                    if ui
                                        .button(format!("{}  (H)", tr("clip-ctx-mirror-h")))
                                        .clicked()
                                    {
                                        pending_actions.push(ClipAction::ToggleFlipH(clip_id));
                                        ui.close_menu();
                                    }
                                    if ui
                                        .button(format!("{}  (V)", tr("clip-ctx-mirror-v")))
                                        .clicked()
                                    {
                                        pending_actions.push(ClipAction::ToggleFlipV(clip_id));
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
                                egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(90, 160, 240)),
                                egui::StrokeKind::Inside,
                            );
                        }
                        if let (Some(ids), Some(pp)) = (&dnd_drop, pointer_hover) {
                            if lane_rect.contains(pp) && !ids.is_empty() {
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
                                    .find(|m| ids.first() == Some(&m.id))
                                    .map(|m| m.duration_ms)
                                    .unwrap_or(0);
                                let snapped_ms = self
                                    .snap_ms(uuid::Uuid::nil(), idx, raw_ms as i64, dur, px_per_ms)
                                    .max(0) as u64;
                                pending_drop = Some((ids.clone(), idx, snapped_ms));

                                // Drop ghost: green edges when
                                // snapped to a neighbour, neutral
                                // blue otherwise. Disappears once
                                // the clip lands; the real clip
                                // renders in the standard style.
                                let ghost_x =
                                    lane_rect.left() + (snapped_ms as f32 * px_per_ms) - scroll_x;
                                let ghost_w = (dur as f32 * px_per_ms).max(4.0);
                                let ghost_rect = egui::Rect::from_min_size(
                                    egui::Pos2::new(ghost_x, lane_rect.top() + 3.0),
                                    egui::vec2(ghost_w, (lane_rect.height() - 6.0).max(4.0)),
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
                                    egui::Stroke::new(elev::STROKE_HAIRLINE, edge_color),
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

                    // Fade handle drag: emit Delta every frame
                    // while a drag is active, End on release.
                    if let Some(fd) = &self.fade_drag {
                        if let Some(pp) = pointer_hover {
                            pending_actions.push(ClipAction::FadeDragDelta(
                                fd.clip_id,
                                pp.x - fd.origin_ptr_x,
                                px_per_ms,
                            ));
                        }
                        if pointer_released {
                            pending_actions.push(ClipAction::FadeDragEnd(fd.clip_id));
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
                                let on_clip = all_clip_rects.iter().any(|(_, r)| r.contains(pp));
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

                        // Cancel on Esc; with no marquee active it also
                        // exits pan mode (the sticky hand-tool trap: every
                        // left-drag pans while it is on).
                        if esc {
                            self.marquee = None;
                            self.timeline_tools.pan_mode = false;
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
                                    if rect.intersects(*cr) && !self.selected_clips.contains(id) {
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
                                egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(120, 180, 240)),
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

        // Wheel/touchpad scroll pans only while hovering the timeline
        // itself; unguarded it stole scrolls from every other panel and
        // made the whole lane stack slide under the pointer.
        let sd = ctx.input(|i| i.raw_scroll_delta);
        let over_timeline = ctx
            .input(|i| i.pointer.hover_pos())
            .map(|p| tl_resp.response.rect.contains(p))
            .unwrap_or(false);
        if over_timeline && (sd.y.abs() > 0.0 || sd.x.abs() > 0.0) {
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
            let cmd = caprust_core::commands::duplicate_track::DuplicateTrackCommand::new(idx);
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
                    if self.preview.playing && (target as i64 - self.playhead_ms as i64).abs() > 500
                    {
                        self.explicit_seek_ms = Some(target);
                        // Re-anchor wall clock so playhead stays
                        // in sync with the new position.
                        self.playback_started_at = Some(std::time::Instant::now());
                        self.playback_started_ms = target;
                    } else if !self.preview.playing {
                        // Paused seek: force the paused one-shot to
                        // re-render at the new playhead. Without this,
                        // clicking the timeline while paused moves the
                        // playhead but the preview shows a stale frame
                        // and no renderer is ever spawned.
                        self.paused_frame_dirty = true;
                    }
                    self.playhead_ms = target;
                }
                ClipAction::FocusTextContent(id) => {
                    // Select the clip (plain click semantics) and
                    // ask the Properties panel to focus the content
                    // editor on the next frame. If the clip is not
                    // a TextOverlay, the panel clears the request
                    // on its next render.
                    self.selected_clips = vec![id];
                    self.properties.focus_content_for = Some(id);
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
                    // If the dragged clip is part of a multi-select,
                    // snapshot the whole group's original positions
                    // so DragEnd can move all of them together.
                    let group: Vec<(uuid::Uuid, u64, usize)> =
                        if self.selected_clips.contains(&id) && self.selected_clips.len() > 1 {
                            self.selected_clips
                                .iter()
                                .filter_map(|cid| {
                                    self.project
                                        .clips
                                        .iter()
                                        .find(|c| c.id == *cid)
                                        .map(|c| (*cid, c.start_time_ms, c.track_index))
                                })
                                .collect()
                        } else {
                            Vec::new()
                        };
                    self.clip_drag = Some(ClipDrag {
                        clip_id: id,
                        group,
                        origin_ms: o,
                        current_ms: o as i64,
                        origin_track: ti,
                        track_index: ti,
                        clip_duration_ms: dur,
                        origin_ptr: ptr,
                        last_ptr: ptr,
                        trim_edge: None,
                        source_duration_ms: sdr,
                        origin_duration_ms: dur,
                        raw_delta_ms: 0,
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
                            let raw_delta = (dx / ppm) as i64;
                            if let Some(cur) = &mut self.clip_drag {
                                cur.current_ms = snapped;
                                cur.raw_delta_ms = raw_delta;
                                if let Some(t) = new_track {
                                    cur.track_index = t;
                                }
                            }
                            // Live trim feedback (DIRECTIVES 6):
                            // mutate the clip directly so the user
                            // sees the clip shrink/grow frame by
                            // frame. Discarded on DragEnd, which
                            // restores the originals before running
                            // SetClipCommand so undo captures the
                            // pre-drag state.
                            if let Some(edge) = d.trim_edge {
                                if let Some(c) = self.project.clips.iter_mut().find(|c| c.id == id)
                                {
                                    match edge {
                                        TrimEdge::Left => {
                                            let ns = (d.origin_ms as i64 + raw_delta).max(0) as u64;
                                            let nd = (d.origin_duration_ms as i64 - raw_delta)
                                                .max(100)
                                                as u64;
                                            c.start_time_ms = ns;
                                            c.duration_ms = nd;
                                        }
                                        TrimEdge::Right => {
                                            let mut nd = (d.origin_duration_ms as i64 + raw_delta)
                                                .max(100)
                                                as u64;
                                            if d.source_duration_ms > 0 {
                                                nd = nd.min(d.source_duration_ms);
                                            }
                                            c.duration_ms = nd;
                                        }
                                    }
                                }
                            }
                            // Trim-follow: while trimming an edge
                            // and the toggle is on, park the
                            // playhead on the edge so the preview
                            // shows the exact frame being set.
                            if self.settings.trim_follow && d.trim_edge.is_some() {
                                let target = snapped.max(0) as u64;
                                if target != self.playhead_ms {
                                    if self.preview.playing {
                                        self.explicit_seek_ms = Some(target);
                                        self.playback_started_at = Some(std::time::Instant::now());
                                        self.playback_started_ms = target;
                                    } else {
                                        self.paused_frame_dirty = true;
                                    }
                                    self.playhead_ms = target;
                                }
                            }
                        }
                    }
                }
                ClipAction::DragEnd(id) => {
                    if let Some(d) = self.clip_drag.take() {
                        tracing::info!(
                            "drag_end clip={} trim_edge={:?} dm={} origin_ms={} current_ms={} origin_dur={} source_dur={}",
                            d.clip_id,
                            d.trim_edge,
                            d.current_ms - d.origin_ms as i64,
                            d.origin_ms,
                            d.current_ms,
                            d.origin_duration_ms,
                            d.source_duration_ms,
                        );
                        if d.clip_id == id {
                            // Trim-follow: with magnetic ON the
                            // pack slides the clip back into the
                            // gap, so re-anchor the playhead on
                            // the FINAL edge after the command
                            // below runs.
                            let was_trim = d.trim_edge.is_some();
                            let follow = self.settings.trim_follow;
                            if let Some(edge) = d.trim_edge {
                                // Restore pre-drag values first so
                                // SetClipCommand's `before` snapshot
                                // (used by undo) captures the state
                                // the user saw before the drag.
                                // Without this, undo would restore to
                                // whatever intermediate value the
                                // live mutation left behind on the
                                // last frame.
                                if let Some(c) = self.project.clips.iter_mut().find(|c| c.id == id)
                                {
                                    c.start_time_ms = d.origin_ms;
                                    c.duration_ms = d.origin_duration_ms;
                                }
                                let dm = d.current_ms - d.origin_ms as i64;
                                let (ns, nd) = match edge {
                                    TrimEdge::Left => (
                                        (d.origin_ms as i64 + dm).max(0) as u64,
                                        (d.origin_duration_ms as i64 - dm).max(100) as u64,
                                    ),
                                    TrimEdge::Right => {
                                        // Use the RAW pointer delta, not
                                        // the clamped dm. The clamp in
                                        // DragDelta exists for the LEFT
                                        // trim (cannot push the clip
                                        // before the timeline start), but
                                        // it caps the RIGHT trim at
                                        // origin_ms of shrinkage when a
                                        // long drag passes 0 on the
                                        // timeline.
                                        let mut nd = (d.origin_duration_ms as i64 + d.raw_delta_ms)
                                            .max(100)
                                            as u64;
                                        if d.source_duration_ms > 0 {
                                            nd = nd.min(d.source_duration_ms);
                                        }
                                        (d.origin_ms, nd)
                                    }
                                };
                                let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                    .start_time_ms(ns)
                                    .duration_ms(nd);
                                let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                                if follow {
                                    // Re-anchor on the final edge so
                                    // magnetic slide-backs land on
                                    // the same frame the user saw
                                    // while dragging.
                                    let target = match edge {
                                        TrimEdge::Left => ns,
                                        TrimEdge::Right => ns + nd,
                                    };
                                    if self.preview.playing {
                                        self.explicit_seek_ms = Some(target);
                                        self.playback_started_at = Some(std::time::Instant::now());
                                        self.playback_started_ms = target;
                                    }
                                    self.playhead_ms = target;
                                }
                                let _ = (was_trim, follow);
                            } else {
                                let nm = d.current_ms.max(0) as u64;
                                let nt = d.track_index;
                                let time_changed = nm != d.origin_ms;
                                let track_changed = self
                                    .project
                                    .clips
                                    .iter()
                                    .find(|c| c.id == id)
                                    .map(|c| c.track_index)
                                    != Some(nt);

                                // Xfade cleanup on move. If the user
                                // drags a clip that carries an xfade,
                                // its `applied_xfade_shift_ms` is no
                                // longer accurate: the predecessor is
                                // at its old spot but the clip just
                                // moved. Drop the transition (which
                                // also restores every follower to its
                                // pre-xfade position) AND apply the
                                // move in one MacroCommand so a single
                                // Ctrl+Z restores both the transition
                                // and the position, and the user sees
                                // the clip actually move instead of
                                // snapping back.
                                let had_xfade = self
                                    .project
                                    .clips
                                    .iter()
                                    .find(|c| c.id == id)
                                    .map(|c| c.applied_xfade_shift_ms > 0)
                                    .unwrap_or(false);
                                if had_xfade && (time_changed || track_changed) {
                                    // Capture the clip's pre-clear
                                    // position and its xfade shift so
                                    // we know where it lands AFTER
                                    // SetTransitionCommand::clear runs.
                                    // MoveClipCommand::from_ms must be
                                    // that post-clear position, not
                                    // d.origin_ms, or undo drifts.
                                    let (visual_start, pre_shift) = self
                                        .project
                                        .clips
                                        .iter()
                                        .find(|c| c.id == id)
                                        .map(|c| (c.start_time_ms, c.applied_xfade_shift_ms))
                                        .unwrap_or((d.origin_ms, 0));
                                    let post_clear_start = visual_start + pre_shift;
                                    let ripple = self.timeline_tools.ripple_move;

                                    let mut cmds: Vec<Box<dyn caprust_core::commands::Command>> =
                                        Vec::new();
                                    cmds.push(Box::new(
                                        caprust_core::commands::set_effect::SetTransitionCommand::new(
                                            id, true, None,
                                        ),
                                    ));
                                    if track_changed {
                                        cmds.push(Box::new(
                                            caprust_core::commands::set_clip::SetClipCommand::new(
                                                id,
                                            )
                                            .track_index(nt),
                                        ));
                                        if time_changed {
                                            cmds.push(Box::new(
                                                caprust_core::commands::move_clip::MoveClipCommand {
                                                    clip_id: id,
                                                    from_ms: post_clear_start,
                                                    to_ms: nm,
                                                },
                                            ));
                                        }
                                    } else if ripple && time_changed {
                                        // Ripple move: same track, clear
                                        // xfade, then shift clip plus
                                        // every later clip by the delta.
                                        cmds.push(Box::new(
                                            caprust_core::commands::ripple_move::RippleMoveCommand::new(
                                                id,
                                                post_clear_start,
                                                nm,
                                            ),
                                        ));
                                    } else if time_changed {
                                        cmds.push(Box::new(
                                            caprust_core::commands::move_clip::MoveClipCommand {
                                                clip_id: id,
                                                from_ms: post_clear_start,
                                                to_ms: nm,
                                            },
                                        ));
                                    }
                                    let cmd =
                                        caprust_core::commands::macro_command::MacroCommand::new(
                                            "Move clip (xfade cleared)",
                                            cmds,
                                        );
                                    let _ =
                                        self.undo_stack.execute(Box::new(cmd), &mut self.project);
                                    self.toast(tr("toast-xfade-removed-on-move"));
                                } else if d.group.len() > 1 && (time_changed || track_changed) {
                                    // Multi-select drag: one command
                                    // moves every member by the same
                                    // delta. Undo restores the whole
                                    // batch in one step.
                                    let delta_ms = nm as i64 - d.origin_ms as i64;
                                    let delta_track = nt as i64 - d.origin_track as i64;
                                    let max_track = self.project.tracks.len().saturating_sub(1);
                                    let moves: Vec<caprust_core::commands::move_many::ClipMove> = d
                                        .group
                                        .iter()
                                        .map(|(cid, orig_ms, orig_track)| {
                                            let new_ms = (*orig_ms as i64 + delta_ms).max(0) as u64;
                                            let new_track = ((*orig_track as i64 + delta_track)
                                                .max(0)
                                                as usize)
                                                .min(max_track);
                                            caprust_core::commands::move_many::ClipMove {
                                                clip_id: *cid,
                                                from_ms: *orig_ms,
                                                to_ms: new_ms,
                                                from_track: *orig_track,
                                                to_track: new_track,
                                            }
                                        })
                                        .collect();
                                    let cmd =
                                        caprust_core::commands::move_many::MoveManyCommand::new(
                                            moves,
                                        );
                                    let _ =
                                        self.undo_stack.execute(Box::new(cmd), &mut self.project);
                                } else if time_changed || track_changed {
                                    // Single-clip fallback. If the
                                    // track changed, chain a
                                    // SetClipCommand so a track move
                                    // and a time move undo together.
                                    if track_changed {
                                        let mut cmds: Vec<
                                            Box<dyn caprust_core::commands::Command>,
                                        > = Vec::new();
                                        cmds.push(Box::new(
                                            caprust_core::commands::set_clip::SetClipCommand::new(
                                                id,
                                            )
                                            .track_index(nt),
                                        ));
                                        if time_changed {
                                            cmds.push(Box::new(
                                                caprust_core::commands::move_clip::MoveClipCommand {
                                                    clip_id: id,
                                                    from_ms: d.origin_ms,
                                                    to_ms: nm,
                                                },
                                            ));
                                        }
                                        let cmd = caprust_core::commands::macro_command::MacroCommand::new(
                                            "Move clip to another track",
                                            cmds,
                                        );
                                        let _ = self
                                            .undo_stack
                                            .execute(Box::new(cmd), &mut self.project);
                                    } else if self.timeline_tools.ripple_move && time_changed {
                                        // Same track, ripple toggle ON:
                                        // move this clip and shift every
                                        // later clip on the track by the
                                        // same delta in one undo step.
                                        let cmd =
                                            caprust_core::commands::ripple_move::RippleMoveCommand::new(
                                                id, d.origin_ms, nm,
                                            );
                                        let _ = self
                                            .undo_stack
                                            .execute(Box::new(cmd), &mut self.project);
                                    } else {
                                        let cmd = MoveClipCommand {
                                            clip_id: id,
                                            from_ms: d.origin_ms,
                                            to_ms: nm,
                                        };
                                        let _ = self
                                            .undo_stack
                                            .execute(Box::new(cmd), &mut self.project);
                                    }
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
                ClipAction::TranslateCaptions(id) => {
                    self.start_translate_job(id);
                }
                ClipAction::SeparateAudio(id) => {
                    let cmd = caprust_core::commands::separate_audio::SeparateAudioCommand::new(id);
                    if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                        tracing::error!("separate audio failed: {e}");
                        self.toast_error(tr("toast-separate-audio-failed"));
                    } else {
                        tracing::info!("separate audio: created Audio clip from {id}");
                        self.toast(tr("toast-separate-audio-done"));
                    }
                }
                ClipAction::CreateMultiCamGroup(ids) => {
                    let n = self.project.multicam_groups.len() + 1;
                    let name = format!("MultiCam {n}");
                    let cmd =
                        caprust_core::commands::create_multicam_group::CreateMultiCamGroupCommand::new(
                            name, ids,
                        );
                    if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                        tracing::error!("create multicam group failed: {e}");
                        self.toast_error(tr("toast-multicam-create-failed"));
                    } else {
                        tracing::info!("created multicam group");
                        self.toast(tr("toast-multicam-created"));
                    }
                }
                ClipAction::ReattachAudio(id) => {
                    // `id` is either a video whose audio was detached,
                    // or an audio clip that was detached from some
                    // video. Resolve to the video id first.
                    let video_id = if self.project.clips.iter().any(|c| {
                        c.id == id && matches!(c.clip_type, caprust_core::ClipType::Video { .. })
                    }) {
                        Some(id)
                    } else {
                        self.project
                            .clips
                            .iter()
                            .find(|c| c.detached_audio_clip_id == Some(id))
                            .map(|c| c.id)
                    };
                    if let Some(vid) = video_id {
                        let cmd =
                            caprust_core::commands::reattach_audio::ReattachAudioCommand::new(vid);
                        if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                            tracing::error!("reattach audio failed: {e}");
                        } else {
                            tracing::info!("reattach audio: video {vid}");
                            self.toast(tr("toast-reattach-audio-done"));
                        }
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
                        let cmd = caprust_core::commands::ripple::RippleInsertCommand::new(c);
                        if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                            tracing::error!("paste failed: {e}");
                        } else {
                            self.toast(tr("toast-clip-pasted"));
                        }
                    }
                }
                ClipAction::Duplicate(id) => {
                    if let Some(orig) = self.project.clips.iter().find(|c| c.id == id).cloned() {
                        let mut c = orig;
                        c.id = uuid::Uuid::new_v4();
                        c.start_time_ms += c.duration_ms;
                        let cmd = caprust_core::commands::ripple::RippleInsertCommand::new(c);
                        if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                            tracing::error!("duplicate failed: {e}");
                        } else {
                            self.toast(tr("toast-clip-duplicated"));
                        }
                    }
                }
                ClipAction::RippleDelete(id) => {
                    let cmd = caprust_core::commands::delete_clip::DeleteClipCommand::new(id, true);
                    if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                        tracing::error!("ripple delete failed: {e}");
                    }
                }
                ClipAction::SetSpeed(id, v) => {
                    let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id).speed(v);
                    if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
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
                    let cmd =
                        caprust_core::commands::set_clip::SetClipCommand::new(id).volume_db(target);
                    if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                        tracing::error!("mute clip failed: {e}");
                    }
                }
                ClipAction::FadeDragStart(id, edge, current_ms) => {
                    let (dur, fi, fo, ptr_x) = self
                        .project
                        .clips
                        .iter()
                        .find(|c| c.id == id)
                        .map(|c| {
                            (
                                c.duration_ms,
                                c.fade_in_ms,
                                c.fade_out_ms,
                                self.last_pointer.map(|p| p.x).unwrap_or(0.0),
                            )
                        })
                        .unwrap_or((0, 0, 0, 0.0));
                    let other = match edge {
                        FadeEdge::In => fo,
                        FadeEdge::Out => fi,
                    };
                    self.fade_drag = Some(FadeDrag {
                        clip_id: id,
                        edge,
                        origin_ms: current_ms as u64,
                        current_ms: current_ms as u64,
                        origin_ptr_x: ptr_x,
                        duration_ms: dur,
                        other_fade_ms: other,
                    });
                }
                ClipAction::FadeDragDelta(id, dx, ppm) => {
                    if ppm > 0.0 {
                        if let Some(fd) = self.fade_drag.clone() {
                            if fd.clip_id == id {
                                let delta_ms = (dx / ppm) as i64;
                                let cand = fd.origin_ms as i64 + delta_ms;
                                // Clamp: fade can't be negative,
                                // can't overlap the opposite
                                // edge's existing fade, and
                                // can't exceed the clip length.
                                let cap = fd.duration_ms.saturating_sub(fd.other_fade_ms);
                                let new_ms = cand.clamp(0, cap as i64) as u64;
                                if let Some(cur) = self.fade_drag.as_mut() {
                                    cur.current_ms = new_ms;
                                }
                            }
                        }
                    }
                }
                ClipAction::FadeDragEnd(id) => {
                    if let Some(fd) = self.fade_drag.take() {
                        if fd.clip_id == id && fd.current_ms != fd.origin_ms {
                            let new_val = fd.current_ms;
                            let cmd = match fd.edge {
                                FadeEdge::In => {
                                    caprust_core::commands::set_clip::SetClipCommand::new(id)
                                        .fade_in_ms(new_val)
                                }
                                FadeEdge::Out => {
                                    caprust_core::commands::set_clip::SetClipCommand::new(id)
                                        .fade_out_ms(new_val)
                                }
                            };
                            if let Err(e) =
                                self.undo_stack.execute(Box::new(cmd), &mut self.project)
                            {
                                tracing::error!("fade commit failed: {e}");
                            } else {
                                tracing::info!("fade {:?} = {}ms on {}", fd.edge, new_val, id);
                            }
                        }
                    }
                }
            }
        }

        if let Some((mids, ti, t)) = pending_drop {
            use caprust_core::{Clip, MediaKind};
            // Build every clip first, then wrap in either a single
            // RippleInsertCommand or a MacroCommand. Wrapping in a
            // macro means Ctrl+Z restores the whole batch in one
            // step instead of popping one clip at a time.
            let mut clips: Vec<Clip> = Vec::new();
            let mut added: Vec<uuid::Uuid> = Vec::new();
            let mut cursor_ms = t;
            for mid in mids {
                let item = self
                    .project
                    .media
                    .items
                    .iter()
                    .find(|m| m.id == mid)
                    .cloned();
                let Some(item) = item else { continue };
                let dur = if item.duration_ms > 0 {
                    item.duration_ms
                } else {
                    3000
                };
                let mut clip = match item.kind {
                    MediaKind::Video => Clip::new_video(&item.path, ti, cursor_ms, dur),
                    MediaKind::Audio => Clip::new_audio(&item.path, ti, cursor_ms, dur),
                    MediaKind::Image => Clip::new_image(&item.path, ti, cursor_ms, dur),
                };
                clip.media_id = Some(item.id);
                // Cap growth at the true source length. When ffprobe has
                // not finished yet, item.duration_ms is 0: keep the cap
                // at 0 (unlimited) and let the probe handler tighten it
                // once the real duration lands.
                clip.source_duration_ms = item.duration_ms;
                added.push(clip.id);
                cursor_ms = cursor_ms.saturating_add(dur);
                clips.push(clip);
            }
            match clips.len() {
                0 => {}
                1 => {
                    let cmd = caprust_core::commands::ripple::RippleInsertCommand::new(
                        clips.into_iter().next().unwrap(),
                    );
                    let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                }
                n => {
                    let inner: Vec<Box<dyn caprust_core::Command>> = clips
                        .into_iter()
                        .map(|c| {
                            Box::new(caprust_core::commands::ripple::RippleInsertCommand::new(c))
                                as Box<dyn caprust_core::Command>
                        })
                        .collect();
                    let cmd = caprust_core::commands::macro_command::MacroCommand::new(
                        format!("Insert {n} clips"),
                        inner,
                    );
                    let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                }
            }
            if !added.is_empty() {
                self.selected_clips = added;
            }
            self.last_dnd_payload = None;
        }
    }

    /// New dock-based layout. Fallback to the classic layout via
    /// `show_editor_classic` while the migration is in progress; flip
    /// `USE_DOCK_LAYOUT` to compare them.
    fn show_editor(&mut self, ctx: &egui::Context) {
        const USE_DOCK_LAYOUT: bool = true;
        if USE_DOCK_LAYOUT {
            self.show_editor_dock(ctx);
        } else {
            self.show_editor_classic(ctx);
        }
    }

    fn show_editor_dock(&mut self, ctx: &egui::Context) {
        self.show_menu_bar(ctx);
        self.show_toolbar(ctx);
        // Jobs bar sits above the timeline so it is visible from any
        // panel. In classic mode show_editor_classic calls this; in
        // docked mode we need our own call or the bar never renders.
        self.show_jobs_bar(ctx);

        // Move the dock state out so the viewer can borrow `self`
        // mutably without aliasing `self.dock_state`.
        let mut ds = std::mem::replace(&mut self.dock_state, egui_dock::DockState::new(Vec::new()));
        {
            // Compute the dock style BEFORE we move `self` into the
            // viewer; material_style borrows the theme immutably.
            let style = crate::dock::material_style(ctx, &self.theme);
            let mut viewer = crate::dock::AppTabViewer { app: self };
            egui_dock::DockArea::new(&mut ds)
                .style(style)
                .show(ctx, &mut viewer);
        }
        self.dock_state = ds;
    }

    #[allow(dead_code)]
    fn show_editor_classic(&mut self, ctx: &egui::Context) {
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
                self.handle_asset_browser_output(out);
            });

        egui::SidePanel::right("right_panel")
            .resizable(true)
            .default_width(280.0)
            .min_width(240.0)
            .show(ctx, |ui| {
                self.render_properties_panel(ui);
            });

        // Jobs bar sits above the timeline so it's visible from any
        // panel. It auto-hides when no jobs are live.
        self.show_jobs_bar(ctx);

        self.show_timeline(ctx);

        // Central preview (frame + transport)
        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_preview_panel(ui);
        });
    }

    /// Properties panel body. Migrated out of the SidePanel::right
    /// closure so the dock viewer can render it inside a Tab::Properties
    /// zone.
    pub(crate) fn render_master_chain_panel(&mut self, ui: &mut egui::Ui) {
        let out = crate::panels::master_chain::show(ui, &self.project, &mut self.master_chain);

        if let Some(id) = out.remove_instance {
            let cmd =
                caprust_core::commands::remove_master_plugin::RemoveMasterPluginCommand::new(id);
            if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                tracing::warn!("remove master plugin failed: {e:#}");
            }
        }
        if let Some((id, bypassed)) = out.set_bypass {
            // Reuse AddMasterPluginCommand is wrong; we need a direct
            // field edit. For MVP the bypass toggle goes through the
            // existing per-instance edit path.
            if let Some(inst) = self.project.master_plugins.iter_mut().find(|p| p.id == id) {
                inst.bypassed = bypassed;
                // Intentionally not undoable yet (follow-up: a proper
                // SetMasterPluginBypassCommand). Preview still respawns
                // because render_hash includes `bypassed`.
            }
        }
    }
    /// Render the multicam groups panel inside a dock zone.
    pub(crate) fn render_multicam_panel(&mut self, ui: &mut egui::Ui) {
        let out = crate::panels::multicam::show(ui, &self.project, &mut self.multicam);

        if let Some((gid, angle)) = out.set_active_angle {
            let cmd =
                caprust_core::commands::set_active_angle::SetActiveAngleCommand::new(gid, angle);
            if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                tracing::error!("set active angle failed: {e}");
            }
        }

        if let Some(gid) = out.remove_group {
            let cmd =
                caprust_core::commands::remove_multicam_group::RemoveMultiCamGroupCommand::new(gid);
            if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                tracing::error!("remove multicam group failed: {e}");
                self.toast_error(tr("toast-multicam-remove-failed"));
            } else {
                self.toast(tr("toast-multicam-removed"));
            }
        }

        if let Some(gid) = out.sync_requested {
            self.start_multicam_sync(gid);
        }

        // Drain the sync job each frame. Progress updates the panel
        // state; terminal states commit the offsets or report a
        // failure.
        let mut finished: Option<(uuid::Uuid, Result<Vec<i64>, String>)> = None;
        if let Some(rx) = &self.multicam.sync_rx {
            let mut is_done = false;
            while let Ok(ev) = rx.try_recv() {
                use caprust_media_io::multicam_sync::SyncEvent;
                match ev {
                    SyncEvent::Progress { current, total } => {
                        self.multicam.sync_progress = Some((current, total));
                    }
                    SyncEvent::Done { offsets_ms } => {
                        let gid = self.multicam.sync_group.unwrap_or_default();
                        finished = Some((gid, Ok(offsets_ms)));
                        is_done = true;
                        break;
                    }
                    SyncEvent::Failed(e) => {
                        let gid = self.multicam.sync_group.unwrap_or_default();
                        finished = Some((gid, Err(e)));
                        is_done = true;
                        break;
                    }
                }
            }
            if is_done {
                self.multicam.sync_rx = None;
                self.multicam.sync_progress = None;
            }
        }
        if let Some((gid, result)) = finished {
            self.multicam.sync_group = None;
            match result {
                Ok(offsets) => {
                    let cmd =
                        caprust_core::commands::set_multicam_sync::SetMultiCamSyncCommand::new(
                            gid, offsets,
                        );
                    if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                        tracing::error!("apply multicam sync: {e}");
                        self.toast_error(tr("toast-multicam-sync-failed"));
                    } else {
                        self.toast(tr("toast-multicam-synced"));
                    }
                }
                Err(e) => {
                    tracing::error!("multicam sync job failed: {e}");
                    self.toast_error(tr("toast-multicam-sync-failed"));
                }
            }
        }

        // Keep the timer / progress label fresh while a job runs.
        if self.multicam.is_syncing() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }
    }

    /// Spawn the audio-envelope sync job for a multicam group.
    /// Collects the source paths of the group's angle clips, hands
    /// them to media-io::multicam_sync, and stashes the receiver in
    /// self.multicam.sync_rx for the panel render loop to drain.
    fn start_multicam_sync(&mut self, group_id: uuid::Uuid) {
        if self.multicam.is_syncing() {
            tracing::warn!("multicam sync already running");
            return;
        }
        let Some(group) = self
            .project
            .multicam_groups
            .iter()
            .find(|g| g.id == group_id)
        else {
            return;
        };

        // Collect source paths in angle order.
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        for cid in &group.angle_clip_ids {
            let Some(clip) = self.project.clips.iter().find(|c| c.id == *cid) else {
                tracing::warn!("sync: angle clip {cid} missing from project");
                return;
            };
            let path = match &clip.clip_type {
                caprust_core::ClipType::Video { path, .. }
                | caprust_core::ClipType::Audio { path, .. } => path.clone(),
                _ => {
                    tracing::warn!("sync: angle clip {cid} has no source file");
                    return;
                }
            };
            paths.push(std::path::PathBuf::from(path));
        }

        let Some(ffmpeg) = caprust_core::ffmpeg::find_ffmpeg(&self.settings) else {
            self.toast_error(tr("toast-multicam-sync-no-ffmpeg"));
            return;
        };

        let rx = caprust_media_io::multicam_sync::spawn_multicam_sync(ffmpeg, paths);
        self.multicam.sync_rx = Some(rx);
        self.multicam.sync_group = Some(group_id);
        self.multicam.sync_progress = None;
        tracing::info!("multicam sync started for group {group_id}");
    }
    pub(crate) fn render_properties_panel(&mut self, ui: &mut egui::Ui) {
        section::header(ui, tr("props-heading"));

        let selected = self.selected_clips.first().copied();
        crate::panels::clip_properties::show(
            ui,
            &self.project,
            selected,
            &mut self.properties,
            self.playhead_ms,
        );

        // Consume any pending edits → commands
        if !self.properties.pending.is_empty() {
            let edits = std::mem::take(&mut self.properties.pending);
            if let Some(id) = selected {
                // Field edits collapse into one SetClipCommand; effect
                // and transition edits run as separate commands so each
                // is individually undoable.
                let mut field_cmd: Option<caprust_core::commands::set_clip::SetClipCommand> = None;
                for e in edits {
                    match e {
                        PendingEdit::RemoveEffect(effect_id) => {
                            let c = caprust_core::commands::set_effect::RemoveEffectCommand::new(
                                id, effect_id,
                            );
                            let _ = self.undo_stack.execute(Box::new(c), &mut self.project);
                        }
                        PendingEdit::ClearTransitionIn => {
                            let c = caprust_core::commands::set_effect::SetTransitionCommand::new(
                                id, true, None,
                            );
                            let _ = self.undo_stack.execute(Box::new(c), &mut self.project);
                        }
                        PendingEdit::ClearTransitionOut => {
                            let c = caprust_core::commands::set_effect::SetTransitionCommand::new(
                                id, false, None,
                            );
                            let _ = self.undo_stack.execute(Box::new(c), &mut self.project);
                        }
                        PendingEdit::CaptionSegmentText { idx, text } => {
                            let c = caprust_core::commands::edit_caption_segment::EditCaptionSegmentCommand::new(
                                        id, idx, text,
                                    );
                            let _ = self.undo_stack.execute(Box::new(c), &mut self.project);
                        }
                        PendingEdit::TextStyle(v) => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .text_style(v);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::CaptionStyle(v) => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .caption_style(v);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::TextContent(v) => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .text_content(v);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::TextMotion(m) => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .text_motion(m);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::TextEffect(e) => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .text_effect(e);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::VolumeKeyframes(v) => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .volume_keyframes(v);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::SpeedEnd(v) => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .speed_end(v);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::SpeedEase(v) => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .speed_ease(v);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::TransitionInEasing(v) => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .transition_in_easing(v);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::TransitionOutEasing(v) => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .transition_out_easing(v);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::TransitionDuration(v) => {
                            // Uses the transition-specific command so the
                            // follower shift is recomputed when an xfade
                            // is active. The generic SetClipCommand would
                            // change the duration without moving anything,
                            // leaving the overlap wrong.
                            let cmd = caprust_core::commands::set_effect::
                                SetTransitionDurationCommand::new(id, v);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::SpeedRange(v) => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .speed_range(v);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::DuckAgainst(v) => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .duck_against(v);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::DuckReductionDb(v) => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .duck_reduction_db(v);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::StartReframe => {
                            self.start_reframe_job(id);
                        }
                        PendingEdit::StartBeatAnalysis => {
                            self.start_beat_job(id);
                        }
                        PendingEdit::StartBgRemoval => {
                            self.start_bg_removal_job(id);
                        }
                        PendingEdit::ClearBgRemoval => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .bg_removal(None);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::ChromaKey(v) => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .chroma_key(v);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::AutoReframe(kps) => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .auto_reframe(kps);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::Name(v) => {
                            let trimmed = v.trim().to_string();
                            let new_name = if trimmed.is_empty() {
                                None
                            } else {
                                Some(trimmed)
                            };
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
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
                                PendingEdit::FadeInSec(v) => {
                                    cmd.fade_in_ms((v.max(0.0) * 1000.0).round() as u64)
                                }
                                PendingEdit::FadeOutSec(v) => {
                                    cmd.fade_out_ms((v.max(0.0) * 1000.0).round() as u64)
                                }
                                PendingEdit::AudioNormalize(v) => cmd.audio_normalize(v),
                                PendingEdit::AudioDenoise(v) => cmd.audio_denoise(v),
                                PendingEdit::AudioVoiceBoost(v) => cmd.audio_voice_boost(v),
                                PendingEdit::AudioTelephone(v) => cmd.audio_telephone(v),
                                PendingEdit::AudioBass(v) => cmd.audio_bass(v),
                                PendingEdit::AudioEcho(v) => cmd.audio_echo(v),
                                PendingEdit::AudioChipmunk(v) => cmd.audio_chipmunk(v),
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
    }

    /// Handle a batch of asset-browser output: media jobs, imports,
    /// removals, preset clicks. Shared between the classic
    /// SidePanel::left and every docked AssetX tab.
    fn handle_asset_browser_output(
        &mut self,
        out: crate::panels::asset_browser::AssetBrowserOutput,
    ) {
        // Master-chain plugin add request from the browser.
        if let Some(info) = out.plugin_add_requested {
            let inst = caprust_core::plugin::PluginInstance::new(
                info.id.clone(),
                info.path.clone(),
                info.name.clone(),
            );
            let cmd = caprust_core::commands::add_master_plugin::AddMasterPluginCommand::new(inst);
            if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                tracing::warn!("add master plugin failed: {e:#}");
            } else {
                tracing::info!("added master plugin: {}", info.name);
            }
        }

        if out.media.skipped_duplicates > 0 {
            self.toast(tr("media-import-duplicates-skipped"));
        }
        // Enqueue background probe + thumbnail jobs for new imports.
        for id in out.media.newly_imported {
            if let Some(item) = self.project.media.items.iter().find(|m| m.id == id) {
                tracing::info!("enqueueing probe for {}", item.path);
                let item_clone = item.clone();
                self.job_runner.enqueue(
                    &item_clone,
                    self.ffmpeg_status
                        .ffmpeg
                        .clone()
                        .map(std::path::PathBuf::from),
                    self.ffmpeg_status
                        .ffprobe
                        .clone()
                        .map(std::path::PathBuf::from),
                );
            }
        }

        // Remove requested items from library. Any clip that
        // references the media item via media_id is deleted through
        // DeleteClipCommand first, so Ctrl+Z restores both the clips
        // and (in the same step, thanks to the batch) the media
        // entry. Files on disk are kept.
        for id in out.media.remove_requested {
            let clip_ids: Vec<uuid::Uuid> = self
                .project
                .clips
                .iter()
                .filter(|c| c.media_id == Some(id))
                .map(|c| c.id)
                .collect();
            let rip = self.timeline_tools.magnetic;
            for cid in &clip_ids {
                let cmd = DeleteClipCommand::new(*cid, rip);
                let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                self.selected_clips.retain(|x| x != cid);
            }
            self.project.media.remove(id);
            self.clip_textures.remove(&id);
            tracing::info!(
                "media removed: {id} ({} clip(s) dropped from timeline)",
                clip_ids.len()
            );
        }

        // Preset clicked (transitions/effects/filters/text).
        // Wiring to selected timeline clip is a follow-up PR.
        // Apply clicked preset to the selected clip.
        if let Some((preset_id, tab)) = out.preset_clicked {
            if let Some(clip_id) = self.selected_clips.first().copied() {
                use crate::panels::asset_browser::AssetTab;
                use caprust_core::commands::set_effect::{AddEffectCommand, SetTransitionCommand};
                match tab {
                    AssetTab::Effects | AssetTab::Filters => {
                        if preset_id != "none" {
                            let cmd = AddEffectCommand::new(clip_id, preset_id);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
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
                                matches!(c.clip_type, caprust_core::ClipType::TextOverlay { .. })
                            })
                            .unwrap_or(false);
                        if is_text {
                            let cmd =
                                caprust_core::commands::set_clip::SetClipCommand::new(clip_id)
                                    .text_style(preset_id);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
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
                                caprust_core::commands::ripple::RippleInsertCommand::new(clip);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                            tracing::info!("inserted TextOverlay clip with style '{preset_id}'");
                        }
                    }
                    AssetTab::Media | AssetTab::Templates | AssetTab::Plugins => {}
                }
            } else {
                tracing::warn!("preset '{preset_id}' clicked but no clip selected");
            }
        }
    }

    /// Assets panel body for the dock layout: renders a single tab
    /// without the tab strip (egui_dock draws its own) and routes the
    /// output through the shared handler.
    pub(crate) fn render_assets_panel(
        &mut self,
        ui: &mut egui::Ui,
        tab: crate::panels::asset_browser::AssetTab,
    ) {
        let out = crate::panels::asset_browser::render_tab_content(
            ui,
            tab,
            &mut self.project,
            &mut self.asset_browser,
            &mut self.media_bin,
        );
        self.handle_asset_browser_output(out);
    }

    /// Preview panel body. Migrated out of the CentralPanel closure
    /// so the dock viewer can render it inside a Tab::Preview zone.
    pub(crate) fn render_preview_panel(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
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
                    "preview state: playhead={}ms playing={} clips={} clip_info={} ffmpeg={} audio_ms={:?} delta={:?}",
                    playhead,
                    self.preview.playing,
                    self.project.clips.len(),
                    clip_info.is_some(),
                    self.ffmpeg_status.ffmpeg.is_some(),
                    self.audio_player
                        .as_ref()
                        .map(|ap| ap.samples_played() * 1000 / 48_000),
                    self.audio_player
                        .as_ref()
                        .map(|ap| playhead as i64 - (ap.samples_played() * 1000 / 48_000) as i64),

                );
            }
        }

        // ---- Phase K (K1): auto-respawn on render-relevant change ----
        // Hash-based detection instead of hooking every
        // undo_stack.execute site. Undo/redo, paste, load, and any
        // future command automatically invalidate. Guarded by
        // `playing && has_frame` so a paused preview does not
        // thrash and the very first spawn (has_frame == false) is
        // not double-triggered.
        //
        // Limitation: a slider drag respawns once per change, not
        // seamlessly. Seamless double-buffered swap is Phase K2,
        // a separate PR if this proves jittery in practice.
        //
        // K1b: debounce the respawn so a DragValue slider does not
        // spawn ffmpeg once per frame. We track the newest hash and
        // the instant it last changed; the respawn fires only when
        // the value has been stable for RESPAWN_DEBOUNCE_MS.
        {
            const RESPAWN_DEBOUNCE_MS: u128 = 250;
            let live = self.project.render_hash();
            if live != self.pending_hash {
                self.pending_hash = live;
                self.pending_respawn_at = Some(std::time::Instant::now());
            }
            if live != self.preview_plan_hash {
                let ready = self
                    .pending_respawn_at
                    .map(|t| t.elapsed().as_millis() >= RESPAWN_DEBOUNCE_MS)
                    .unwrap_or(true);
                if ready {
                    self.preview_plan_hash = live;
                    self.pending_respawn_at = None;
                    if self.preview.playing && self.preview_player.has_frame {
                        self.explicit_seek_ms = Some(self.playhead_ms);
                    } else if !self.preview.playing {
                        // K1c: paused preview. Mark dirty so the paused
                        // branch re-renders exactly one frame through
                        // the full filtergraph.
                        self.paused_frame_dirty = true;
                    }
                }
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
                        RateMode::Vbr,
                        8000,
                        "veryfast",
                        &models_dir,
                        start_from,
                        caprust_core::project::VideoEncoder::H264Cpu,
                    ) {
                        Ok(plan) => {
                            self.report_skipped(plan.skipped.missing_source);
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
                                    // Prefer the pre-rendered audio cache when it is
                                    // ready; fall back to the preview's own PCM file
                                    // during the first few seconds after project load.
                                    let using_cache = self.audio_cache.active_path.is_some();
                                    let pcm_path = self
                                        .audio_cache
                                        .active_path
                                        .clone()
                                        .or_else(|| renderer.pcm_path.clone());
                                    // Cache PCM starts at t=0; offset is
                                    // the playhead. Preview PCM inherits
                                    // the plan's input-side -ss when
                                    // seek_optimized is set, so its file
                                    // already starts at the seek point.
                                    let pcm_offset = if using_cache || !renderer.seek_optimized {
                                        start_from
                                    } else {
                                        0
                                    };
                                    // Deferred audio start (see field docs).
                                    // Preview with a transition takes 1-3 s
                                    // to render the first frame; if audio
                                    // started now it would already be that
                                    // far ahead when the playhead anchors,
                                    // producing the exact xfade drift we
                                    // chased for a day. Queue it here and
                                    // start from the re-anchor block.
                                    self.pending_audio_start = pcm_path
                                        .as_ref()
                                        .map(|p| (p.clone(), pcm_offset, using_cache));

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
                        let handle =
                            ctx.load_texture("preview-timeline", img, egui::TextureOptions::LINEAR);
                        self.preview_player.texture = Some(handle);
                        self.preview_player.has_frame = true;

                        // Re-anchor wall clock to the first consumed
                        // frame of this session. Without this, wall_ms
                        // includes the ~1 s of ffmpeg audio priming and
                        // every sync log line shows a bogus drift.
                        if !self.play_anchor_set {
                            self.playback_started_at = Some(std::time::Instant::now());
                            self.play_anchor_set = true;

                            // Start deferred audio HERE, synchronised with
                            // the playhead anchor. Audio and video now both
                            // begin from self.playhead_ms, so the first
                            // sync measurement reads ~0 ms instead of the
                            // 2+ seconds the xfade chain takes to deliver
                            // the first frame.
                            if let Some((path, offset, using_cache)) =
                                self.pending_audio_start.take()
                            {
                                match AudioPlayer::play_pcm_file(&path, offset) {
                                    Ok(p) => {
                                        p.set_volume(self.settings.master_volume);
                                        p.set_muted(self.settings.muted);
                                        tracing::info!(
                                            "preview: audio started from {}ms (vol={:.2} muted={}) [{}]",
                                            self.playhead_ms,
                                            self.settings.master_volume,
                                            self.settings.muted,
                                            if using_cache { "cache" } else { "preview-pcm" },
                                        );
                                        self.audio_player = Some(p);
                                    }
                                    Err(e) => {
                                        tracing::warn!("preview: audio unavailable: {e}");
                                        self.audio_player = None;
                                    }
                                }
                            } else {
                                self.audio_player = None;
                            }

                            // Capture audio position now, AFTER any
                            // deferred player was just created. With the
                            // deferral above this baseline should always
                            // be near zero.
                            self.audio_baseline_ms =
                                self.audio_player.as_ref().map_or(0, |ap| ap.playhead_ms());
                            tracing::info!(
                                "playhead: re-anchored at {}ms (audio_baseline={}ms)",
                                self.playhead_ms,
                                self.audio_baseline_ms
                            );
                        }
                    }
                }
            }

            // Advance playhead every frame while playing, regardless
            // of whether a preview frame arrived. The video renderer
            // EOFs when the last video clip ends, but audio may run
            // longer (m4a tail); without this, the playhead freezes
            // at video end and the timeline stops scrolling while
            // cpal keeps playing. (§21: never accumulate dt;
            // anchor is wall-clock, audio sample counter only slows
            // us down when audio is behind.)
            if self.play_anchor_set {
                let wall_ms = self
                    .playback_started_at
                    .map(|t0| self.playback_started_ms + t0.elapsed().as_millis() as u64)
                    .unwrap_or(self.playhead_ms);

                let (new_ph, src_tag) = match self.audio_player.as_ref() {
                    Some(ap) => {
                        let audio_ms = self.playback_started_ms
                            + ap.playhead_ms().saturating_sub(self.audio_baseline_ms);
                        if audio_ms < wall_ms {
                            (audio_ms, "audio")
                        } else {
                            (wall_ms, "wall")
                        }
                    }
                    None => (wall_ms, "wall"),
                };

                let underruns = self.audio_player.as_ref().map_or(0, |ap| ap.underruns());
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

            ctx.request_repaint();
        } else {
            // Paused: stop audio/streaming, keep a one-shot renderer
            // alive across frames so the UI thread never blocks.
            self.preview_player.stop_stream();
            self.preview_player.cancel_pending();

            // K1c: fire a new one-shot when the render hash changed.
            // The renderer is left running until it produces its first
            // frame (or the deadline expires). That makes filtered
            // edits (text content, caption style, motion, effects,
            // transitions) visible without pressing play.
            if std::mem::take(&mut self.paused_frame_dirty) {
                if let Some(mut r) = self.preview_renderer.take() {
                    r.kill();
                }
                self.paused_renderer_deadline = None;
                if let Some(ffmpeg) = self.ffmpeg_status.ffmpeg.clone() {
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
                    let models_dir = self.settings.effective_models_dir();
                    match caprust_media_io::export_graph::plan_from_project(
                        &self.project,
                        rw,
                        rh,
                        fps_num,
                        fps_den,
                        23,
                        RateMode::Vbr,
                        8000,
                        "veryfast",
                        &models_dir,
                        playhead,
                        caprust_core::project::VideoEncoder::H264Cpu,
                    ) {
                        Ok(plan) => {
                            self.report_skipped(plan.skipped.missing_source);
                            match PreviewRenderer::spawn(
                                std::path::Path::new(&ffmpeg),
                                &plan,
                                playhead,
                                rw,
                                rh,
                                fps_f,
                            ) {
                                Ok(r) => {
                                    tracing::info!(
                                        "preview: paused one-shot render from {}ms",
                                        playhead
                                    );
                                    self.preview_renderer = Some(r);
                                    self.paused_renderer_deadline = Some(
                                        std::time::Instant::now()
                                            + std::time::Duration::from_secs(8),
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!("preview: paused spawn failed: {e}");
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("preview: paused plan failed: {e}");
                        }
                    }
                }
            }

            // Poll the paused one-shot, if one is in flight.
            let mut got_frame = false;
            if let Some(r) = self.preview_renderer.as_ref() {
                if let Some(buf) = r.try_next() {
                    let w = r.width as usize;
                    let h = r.height as usize;
                    let expected = w * h * 4;
                    if buf.len() >= expected {
                        let img =
                            egui::ColorImage::from_rgba_unmultiplied([w, h], &buf[..expected]);
                        let handle =
                            ctx.load_texture("preview-timeline", img, egui::TextureOptions::LINEAR);
                        self.preview_player.texture = Some(handle);
                        self.preview_player.has_frame = true;
                        got_frame = true;
                    }
                }
            }
            if got_frame {
                if let Some(mut r) = self.preview_renderer.take() {
                    r.kill();
                }
                self.paused_renderer_deadline = None;
                tracing::info!("preview: paused one-shot delivered frame");
            } else if self.preview_renderer.is_some() {
                let expired = self
                    .paused_renderer_deadline
                    .map(|t| std::time::Instant::now() >= t)
                    .unwrap_or(false);
                if expired {
                    tracing::warn!("preview: paused one-shot timed out");
                    if let Some(mut r) = self.preview_renderer.take() {
                        r.kill();
                    }
                    self.paused_renderer_deadline = None;
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }

            // Fallback: no frame ever uploaded (first open, or after
            // a failed one-shot). Direct source extract without
            // filtergraph.
            if self.preview_renderer.is_none()
                && !got_frame
                && self.preview_player.texture.is_none()
            {
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
                    self.preview_player.poll(&ctx);
                    if self.preview_player.pending.is_some() {
                        ctx.request_repaint_after(std::time::Duration::from_millis(50));
                    }
                }
            }
        }

        // ---- Render frame or placeholder ----
        // Capture the draw_rect / tex_size so the overlay below can
        // use them after the texture borrow ends.
        let mut overlay_ctx: Option<(egui::Rect, egui::Vec2)> = None;
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
            overlay_ctx = Some((draw_rect, tex_size));
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

        // ---- TextOverlay bounding box + drag ----
        // Approximate where the drawtext sits inside the frame using
        // the same math build_drawtext_body uses for the x/y exprs:
        //   left = (w - text_w)/2 + mx * w
        //   top  = h*0.08 + my*h          (above == true)
        //   top  = h*0.82 + my*h          (above == false)
        // text_w is a monospace-ish approximation; exact metrics
        // would require the font file, not worth it here. Only the
        // single selected clip is handled; multi-select drags stay
        // timeline-only.
        if let Some((draw_rect, tex_size)) = overlay_ctx {
            let active_id = self.selected_clips.first().copied();
            let info = active_id.and_then(|id| {
                self.project
                    .clips
                    .iter()
                    .find(|c| c.id == id)
                    .and_then(|c| match &c.clip_type {
                        ClipType::TextOverlay {
                            content,
                            font_size,
                            above,
                            motion,
                            ..
                        } => Some((
                            id,
                            content.chars().count() as f32,
                            *font_size,
                            *above,
                            motion.x,
                            motion.y,
                            motion.scale,
                        )),
                        _ => None,
                    })
            });

            if let Some((id, chars, font_size, above, mx, my, m_scale)) = info {
                let rw = tex_size.x.max(1.0);
                let rh = tex_size.y.max(1.0);
                let sx = draw_rect.width() / rw;
                let sy = draw_rect.height() / rh;
                let fs = font_size * m_scale.max(0.01);

                // Approximate text extents in frame pixels.
                let text_w = (chars * fs * 0.55).max(8.0);
                let text_h = (fs * 1.2).max(8.0);

                let frame_left = rw * 0.5 + mx * rw - text_w * 0.5;
                let frame_top = if above {
                    rh * 0.08 + my * rh
                } else {
                    rh * 0.82 + my * rh
                };

                let s_left = draw_rect.left() + frame_left * sx;
                let s_top = draw_rect.top() + frame_top * sy;
                let s_w = text_w * sx;
                let s_h = text_h * sy;
                let box_rect =
                    egui::Rect::from_min_size(egui::pos2(s_left, s_top), egui::vec2(s_w, s_h));

                let dragging_this = self
                    .text_overlay_drag
                    .as_ref()
                    .map(|d| d.clip_id == id)
                    .unwrap_or(false);

                // Accent-coloured outline; brighter while dragging.
                let stroke_color = if dragging_this {
                    egui::Color32::from_rgb(120, 220, 255)
                } else {
                    egui::Color32::from_rgb(0, 170, 220)
                };
                ui.painter().rect_stroke(
                    box_rect,
                    2.0,
                    egui::Stroke::new(1.5_f32, stroke_color),
                    egui::StrokeKind::Inside,
                );

                // Corner squares: the bottom-right one is the
                // resize handle; the other three are decorative.
                let corner = 6.0;
                let handle_center = box_rect.right_bottom();
                for pos in [
                    box_rect.left_top(),
                    box_rect.right_top(),
                    box_rect.left_bottom(),
                ] {
                    let r = egui::Rect::from_center_size(pos, egui::vec2(corner, corner));
                    ui.painter().rect_filled(r, 0.0, stroke_color);
                }
                let handle_size = 14.0;
                let handle_rect = egui::Rect::from_center_size(
                    handle_center,
                    egui::vec2(handle_size, handle_size),
                );
                ui.painter().rect_filled(
                    egui::Rect::from_center_size(handle_center, egui::vec2(corner, corner)),
                    0.0,
                    stroke_color,
                );

                // Body drag: move. Hit-test should exclude the
                // handle so a click on the handle does not start a
                // move instead.
                let body_rect = egui::Rect::from_min_max(
                    box_rect.min,
                    egui::pos2(box_rect.max.x - handle_size * 0.5, box_rect.max.y),
                );
                let body_resp = ui.interact(
                    body_rect,
                    egui::Id::new(("text_overlay_body", id)),
                    egui::Sense::click_and_drag(),
                );
                let handle_resp = ui.interact(
                    handle_rect,
                    egui::Id::new(("text_overlay_handle", id)),
                    egui::Sense::click_and_drag(),
                );

                let over_handle = handle_resp.hovered();
                let over_body = body_resp.hovered();
                if over_handle
                    || dragging_this
                        && self
                            .text_overlay_drag
                            .as_ref()
                            .map(|d| d.mode == TextOverlayDragMode::Scale)
                            .unwrap_or(false)
                {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
                } else if over_body || dragging_this {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
                }

                if handle_resp.drag_started() {
                    let ptr = handle_resp.interact_pointer_pos().unwrap_or(handle_center);
                    let anchor_screen = box_rect.center();
                    let origin_dist = (ptr - anchor_screen).length().max(1.0);
                    self.text_overlay_drag = Some(TextOverlayDrag {
                        clip_id: id,
                        mode: TextOverlayDragMode::Scale,
                        start_ptr: ptr,
                        origin_x: mx,
                        origin_y: my,
                        origin_scale: m_scale,
                        anchor_screen,
                        origin_dist,
                        rw,
                        rh,
                        sx,
                        sy,
                    });
                } else if body_resp.drag_started() {
                    self.text_overlay_drag = Some(TextOverlayDrag {
                        clip_id: id,
                        mode: TextOverlayDragMode::Move,
                        start_ptr: body_resp.interact_pointer_pos().unwrap_or(egui::Pos2::ZERO),
                        origin_x: mx,
                        origin_y: my,
                        origin_scale: m_scale,
                        anchor_screen: box_rect.center(),
                        origin_dist: 1.0,
                        rw,
                        rh,
                        sx,
                        sy,
                    });
                }

                if let Some(drag) = self.text_overlay_drag.as_ref() {
                    if drag.clip_id == id {
                        if let Some(ptr) = ui.ctx().input(|i| i.pointer.interact_pos()) {
                            match drag.mode {
                                TextOverlayDragMode::Move => {
                                    let dx_screen = ptr.x - drag.start_ptr.x;
                                    let dy_screen = ptr.y - drag.start_ptr.y;
                                    let dx_frame = dx_screen / drag.sx;
                                    let dy_frame = dy_screen / drag.sy;
                                    let new_mx =
                                        (drag.origin_x + dx_frame / drag.rw).clamp(-1.0, 1.0);
                                    let new_my =
                                        (drag.origin_y + dy_frame / drag.rh).clamp(-1.0, 1.0);
                                    if let Some(c) =
                                        self.project.clips.iter_mut().find(|c| c.id == id)
                                    {
                                        if let ClipType::TextOverlay { motion, .. } =
                                            &mut c.clip_type
                                        {
                                            motion.x = new_mx;
                                            motion.y = new_my;
                                        }
                                    }
                                }
                                TextOverlayDragMode::Scale => {
                                    let dist = (ptr - drag.anchor_screen).length();
                                    let ratio = dist / drag.origin_dist;
                                    let new_scale = (drag.origin_scale * ratio).clamp(0.3, 3.0);
                                    if let Some(c) =
                                        self.project.clips.iter_mut().find(|c| c.id == id)
                                    {
                                        if let ClipType::TextOverlay { motion, .. } =
                                            &mut c.clip_type
                                        {
                                            motion.scale = new_scale;
                                        }
                                    }
                                }
                            }
                        }

                        let stopped = match drag.mode {
                            TextOverlayDragMode::Move => body_resp.drag_stopped(),
                            TextOverlayDragMode::Scale => handle_resp.drag_stopped(),
                        };
                        if stopped {
                            let drag = self.text_overlay_drag.take().unwrap();
                            let final_motion =
                                self.project.clips.iter().find(|c| c.id == id).and_then(
                                    |c| match &c.clip_type {
                                        ClipType::TextOverlay { motion, .. } => Some(*motion),
                                        _ => None,
                                    },
                                );
                            if let Some(final_motion) = final_motion {
                                // Restore origin in-place so the
                                // command's `before` snapshot is the
                                // pre-drag state.
                                if let Some(c) = self.project.clips.iter_mut().find(|c| c.id == id)
                                {
                                    if let ClipType::TextOverlay { motion, .. } = &mut c.clip_type {
                                        motion.x = drag.origin_x;
                                        motion.y = drag.origin_y;
                                        motion.scale = drag.origin_scale;
                                    }
                                }
                                let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                    .text_motion(final_motion);
                                let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                            }
                        }
                    }
                }
            }
        }

        ui.add_space(space::S);

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
                let project_dims = self.project.project_dimensions();
                if self.available_encoders.is_none() {
                    use caprust_core::project::VideoEncoder;
                    let mut available = vec![
                        VideoEncoder::H264Cpu,
                        VideoEncoder::H265Cpu,
                        VideoEncoder::Av1Cpu,
                    ];
                    if let Some(ffmpeg) = self.ffmpeg_status.ffmpeg.clone() {
                        let path = std::path::Path::new(&ffmpeg);
                        for enc in [
                            VideoEncoder::H264Nvenc,
                            VideoEncoder::H265Nvenc,
                            VideoEncoder::Av1Nvenc,
                            VideoEncoder::H264Amf,
                            VideoEncoder::H265Amf,
                            VideoEncoder::Av1Amf,
                        ] {
                            if enc.probe(path) {
                                available.push(enc);
                            }
                        }
                    }
                    tracing::info!("export: probed encoders available: {available:?}");
                    self.available_encoders = Some(available);
                }
                let available = self.available_encoders.clone().unwrap_or_default();

                start_clicked = crate::panels::export_window::show(
                    ui,
                    &mut self.export_state,
                    &available,
                    total_ms,
                    clip_count,
                    project_dims,
                );

                // Progress section
                if self.export_in_progress {
                    ui.add_space(space::M);
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
                    if let Some(tracker) = &self.export_tracker {
                        let elapsed_s = tracker.elapsed_secs();
                        let eta_s = tracker.eta_string(self.export_progress);
                        let mut line =
                            format!("{}: {}", tr("exp-elapsed"), format_short_time(elapsed_s));
                        if let Some(eta) = eta_s {
                            line.push_str(&format!(" · {}: {eta}", tr("exp-remaining")));
                        }
                        ui.label(
                            egui::RichText::new(line)
                                .small()
                                .color(egui::Color32::from_gray(180)),
                        );
                    }
                    ctx.request_repaint();
                }

                // Finished section
                if let Some(path) = self.export_finished_path.clone() {
                    ui.add_space(space::M);
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!("{} Done: {path}", ph::CHECK))
                            .color(egui::Color32::from_rgb(120, 220, 120)),
                    );
                    if button::primary(ui, format!("{} {}", ph::FOLDER_OPEN, tr("exp-open-folder")))
                        .clicked()
                    {
                        caprust_media_io::exporter::reveal_in_folder(std::path::Path::new(&path));
                    }
                    if button::secondary(ui, tr("exp-dismiss")).clicked() {
                        self.export_finished_path = None;
                    }
                }
            });
        self.export_open = open;

        if start_clicked && !self.export_in_progress {
            self.start_export();
        }
    }

    #[cfg(windows)]
    fn show_screen_record_modal(&mut self, ctx: &egui::Context) {
        use crate::panels::screen_record::{show_modal, ScreenRecordAction};

        // Load monitors the first time the modal opens.
        if !self.screen_record.monitors_loaded {
            match caprust_screen_record::enumerate_monitors() {
                Ok(list) => {
                    self.screen_record.monitors = list;
                    self.screen_record.monitors_loaded = true;
                }
                Err(e) => {
                    tracing::error!("enumerate monitors: {e}");
                    self.screen_record.error = Some(e.to_string());
                    self.screen_record.monitors_loaded = true;
                }
            }
        }

        // Poll an in-flight recording for completion.
        let mut finished: Option<Result<std::path::PathBuf, String>> = None;
        if let Some(rx) = &self.screen_record.result_rx {
            if let Ok(r) = rx.try_recv() {
                finished = Some(r);
            }
        }
        if let Some(r) = finished {
            self.screen_record.in_progress = false;
            self.screen_record.result_rx = None;
            self.screen_record.stop_flag = None;
            match r {
                Ok(path) => {
                    // Auto-import the recording into the media bin so it
                    // shows up without a second click. Same pipeline as
                    // the media-bin import button: AddMediaCommand on
                    // the undo stack, then enqueue probe + thumbnail.
                    self.import_recording(&path);
                    self.toast(format!("{} {}", tr("screen-record-saved"), path.display()));
                    self.screen_record.error = None;
                }
                Err(e) => {
                    tracing::error!("screen record failed: {e}");
                    self.screen_record.error = Some(format!("{} {e}", tr("screen-record-failed")));
                }
            }
        }

        // Poll the in-flight recording so the progress bar
        // advances. The receiver lives in the state and is
        // checked above; nothing else to do here but repaint.
        if self.screen_record.in_progress {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        let mut open = self.screen_record_open;
        let action = show_modal(ctx, &mut self.screen_record, &mut open);
        self.screen_record_open = open;

        match action {
            ScreenRecordAction::None => {}
            ScreenRecordAction::Close => {
                self.screen_record_open = false;
                self.screen_record.error = None;
            }
            ScreenRecordAction::Stop => {
                if let Some(flag) = &self.screen_record.stop_flag {
                    flag.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            ScreenRecordAction::Start {
                monitor,
                duration_sec,
                fps,
            } => {
                self.start_screen_record(monitor, duration_sec, fps);
            }
        }
    }

    #[cfg(windows)]
    fn start_screen_record(&mut self, monitor: usize, duration_sec: u32, fps: u32) {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::mpsc::channel;
        use std::sync::Arc;
        use std::time::Duration;

        let Some(ffmpeg) = caprust_core::ffmpeg::find_ffmpeg(&self.settings) else {
            self.screen_record.error = Some(tr("screen-record-no-ffmpeg"));
            return;
        };

        let dir = recordings_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.screen_record.error = Some(format!("mkdir: {e}"));
            return;
        }
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let out = dir.join(format!("record-{secs}.mp4"));

        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = stop.clone();
        let handle = caprust_screen_record::record::RecordHandle {
            stop: stop_worker,
            frames_written: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };
        let (tx, rx) = channel();
        let out_for_worker = out.clone();
        std::thread::spawn(move || {
            let r = caprust_screen_record::record::record_to_file(
                &ffmpeg,
                monitor,
                &out_for_worker,
                Duration::from_secs(duration_sec as u64),
                fps,
                handle,
            )
            .map(|s| s.output_path)
            .map_err(|e| e.to_string());
            let _ = tx.send(r);
        });

        self.screen_record.in_progress = true;
        self.screen_record.started_at = Some(std::time::Instant::now());
        self.screen_record.stop_flag = Some(stop);
        self.screen_record.result_rx = Some(rx);
        self.screen_record.error = None;
        let _ = Ordering::Relaxed; // silence unused on non-mutating paths
    }

    #[cfg(windows)]
    fn import_recording(&mut self, path: &std::path::Path) {
        use caprust_core::commands::add_media::AddMediaCommand;
        use caprust_core::MediaKind;

        if !path.is_file() {
            tracing::warn!("recording file missing: {}", path.display());
            return;
        }
        let path_str = path.to_string_lossy().into_owned();
        let cmd = AddMediaCommand::new(path_str.clone(), MediaKind::Video);
        if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
            tracing::error!("import recording: {e}");
            return;
        }

        let Some(item) = self
            .project
            .media
            .items
            .iter()
            .find(|m| m.path == path_str)
            .cloned()
        else {
            return;
        };
        self.job_runner.enqueue(
            &item,
            self.ffmpeg_status
                .ffmpeg
                .clone()
                .map(std::path::PathBuf::from),
            self.ffmpeg_status
                .ffprobe
                .clone()
                .map(std::path::PathBuf::from),
        );
        tracing::info!("recording imported into media bin: {}", path.display());
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
                ui.add_space(space::M);
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
        if self.model_prompt.is_none() {
            return;
        }

        // First frame after opening (or after any change to model_prompt)
        // resets the active tab to the kind the caller requested. We
        // track this by comparing the tab's "family" with the incoming
        // kind only on prompt open — since model_prompt is Some for the
        // whole lifetime, we simply initialise the tab at the same time
        // the caller sets model_prompt. To avoid drift we sync here on
        // the first frame: if the tab's family does not match `kind`,
        // nothing to do (user already switched), so we only force-sync
        // when the prompt was just opened. That signal is the `kind`
        // value itself; the simplest robust approach is to reset the
        // tab whenever the caller assigns model_prompt.
        //
        // Implementation detail: callers assign self.model_prompt
        // together with self.model_prompt_tab. See start_*_job below.

        // Advance fake downloads while this dialog is up.
        self.project.models.tick_downloads(1.0 / 60.0);

        let tab = self.model_prompt_tab;
        let title = match tab {
            caprust_core::ModelKind::Caption => tr("mp-captions-title"),
            caprust_core::ModelKind::Narration => tr("mp-narration-title"),
            // FaceDetector is not reachable from the prompt tabs (only
            // Caption / Narration are listed), but the match must stay
            // exhaustive so a future tab addition compiles.
            caprust_core::ModelKind::FaceDetector => tr("mp-face-title"),
            caprust_core::ModelKind::ScrfdDetector => tr("mp-scrfd-title"),
            caprust_core::ModelKind::BackgroundRemover => tr("mp-bg-title"),
        };

        let mut open = true;
        let mut chosen: Option<(String, String)> = None;
        let mut chosen_download: Option<String> = None;
        let mut cancel = false;

        egui::Window::new(title)
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_width(560.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                // Tab strip: switch between Caption and Narration
                // model lists without closing the window. Needed
                // because the toolbar's download icon opens the prompt
                // for only one family at a time (the one with more
                // missing entries), which left the other family
                // unreachable from the UI once a tie occurred.
                ui.horizontal(|ui| {
                    let mut active = self.model_prompt_tab;
                    let labels = [tr("mp-tab-captions"), tr("mp-tab-narration")];
                    let selected = if active == caprust_core::ModelKind::Caption {
                        0
                    } else {
                        1
                    };
                    let (_, clicked) = segmented_control::segmented_control(ui, &labels, selected);
                    if let Some(i) = clicked {
                        if i != selected {
                            active = if i == 0 {
                                caprust_core::ModelKind::Caption
                            } else {
                                caprust_core::ModelKind::Narration
                            };
                        }
                    }
                    if active != self.model_prompt_tab {
                        self.model_prompt_tab = active;
                    }
                });
                ui.add_space(space::S);
                ui.separator();
                ui.add_space(space::XS);

                ui.label(
                    egui::RichText::new("This action needs a model. Download one, then click Use.")
                        .color(egui::Color32::from_gray(180)),
                );
                ui.add_space(space::S);
                ui.separator();

                let tab = self.model_prompt_tab;
                let ids: Vec<String> = self
                    .project
                    .models
                    .models
                    .iter()
                    .filter(|m| m.kind == tab)
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
                                        let has_url = !m.url.is_empty();
                                        let btn = ui.add_enabled(
                                            has_url,
                                            egui::Button::new(format!(
                                                "{} Download",
                                                ph::DOWNLOAD_SIMPLE
                                            )),
                                        );
                                        if !has_url {
                                            btn.on_hover_text(tr("model-no-url-hint"));
                                        } else if btn.clicked() {
                                            // Defer the actual spawn until
                                            // after this borrow of
                                            // self.project.models ends.
                                            chosen_download = Some(m.id.clone());
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

                ui.add_space(space::M);
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

        // A "Download" click deferred the spawn until after the
        // mutable borrow of self.project.models inside the window's
        // closure ended. Fire it now.
        if let Some(id) = chosen_download.take() {
            self.start_model_download(&id);
            return;
        }

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
            // Use the current tab, not the kind the prompt was originally
            // opened with — the user may have switched tabs to pick a
            // model from the other family.
            match tab {
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
                        self.model_prompt_tab = caprust_core::ModelKind::Narration;
                    } else {
                        self.narration_input.open = true;
                    }
                }
                caprust_core::ModelKind::FaceDetector => {
                    // The P2 auto-reframe path picks the model up from
                    // the registry on demand; there is no user-facing
                    // action to trigger here beyond the success toast.
                }
                caprust_core::ModelKind::ScrfdDetector => {
                    // Same as FaceDetector: resolved on demand by
                    // resolve_face_model when an auto-reframe job
                    // starts.
                }
                caprust_core::ModelKind::BackgroundRemover => {
                    // P3 path resolves the model from the registry on
                    // demand; same as FaceDetector.
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
        // Snapshot the candidate items first: the loop body may reset
        // `thumb_done` on the same vector, and holding an immutable
        // borrow across that mutation trips the borrow checker.
        let items: Vec<(uuid::Uuid, bool, caprust_core::MediaItem)> = self
            .project
            .media
            .items
            .iter()
            .filter(|i| {
                matches!(
                    i.kind,
                    caprust_core::MediaKind::Video | caprust_core::MediaKind::Image
                )
            })
            .map(|i| (i.id, i.thumb_done, i.clone()))
            .collect();

        let mut count = 0usize;
        for (id, was_done, item_clone) in items {
            let jpg = caprust_core::cache::thumbnail_path(std::path::Path::new(&proj_path), id);
            if !jpg.is_file() {
                // Cache was cleared (or the project moved machines).
                // Clear the stale flag so any consumer that gates on
                // `thumb_done` sees the item as pending again.
                if was_done {
                    if let Some(m) = self.project.media.items.iter_mut().find(|m| m.id == id) {
                        m.thumb_done = false;
                    }
                }
                self.job_runner.enqueue(
                    &item_clone,
                    self.ffmpeg_status
                        .ffmpeg
                        .clone()
                        .map(std::path::PathBuf::from),
                    self.ffmpeg_status
                        .ffprobe
                        .clone()
                        .map(std::path::PathBuf::from),
                );
                count += 1;
            }
        }
        if count > 0 {
            tracing::info!("regen: enqueued {count} missing thumbnails");
        }
    }

    /// Enqueue probe+thumbnail jobs for every media item whose
    /// `probe_done` flag is still false. Covers items that were
    /// auto-created on load by `relink_orphan_media_refs` and any
    /// item whose probe previously failed. Runs alongside the
    /// waveform backfill and the thumbnail regen.
    fn backfill_missing_probes(&mut self) {
        if !self.ffmpeg_status.is_available() {
            tracing::warn!("probe backfill skipped: ffmpeg/ffprobe not detected");
            return;
        }
        let items: Vec<caprust_core::MediaItem> = self
            .project
            .media
            .items
            .iter()
            .filter(|m| !m.probe_done)
            .cloned()
            .collect();
        let mut count = 0usize;
        for item in items {
            self.job_runner.enqueue(
                &item,
                self.ffmpeg_status
                    .ffmpeg
                    .clone()
                    .map(std::path::PathBuf::from),
                self.ffmpeg_status
                    .ffprobe
                    .clone()
                    .map(std::path::PathBuf::from),
            );
            count += 1;
        }
        if count > 0 {
            tracing::info!("backfill: enqueued {count} probe jobs");
        }
    }

    /// Collect normalized caption cues from the project and write
    /// them as SubRip. Returns the cue count. Used by File → Export
    /// subtitles and by the export dialog switch.
    fn write_srt_file(clips: &[Clip], dest: &std::path::Path) -> std::io::Result<usize> {
        let cues = caprust_core::clip::collect_caption_cues(clips);
        let srt = caprust_core::clip::cues_to_srt(&cues);
        std::fs::write(dest, srt)?;
        Ok(cues.len())
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

        let encoder = self.export_state.codec;
        tracing::info!("export: using encoder {encoder:?}");
        let models_dir = self.settings.effective_models_dir();
        let plan = match caprust_media_io::export_graph::plan_from_project(
            &self.project,
            w,
            h,
            fps_num,
            fps_den,
            crf,
            self.export_state.rate_mode,
            self.export_state.bitrate_kbps,
            "veryfast",
            &models_dir,
            0,
            encoder,
        ) {
            Ok(p) => {
                self.report_skipped(p.skipped.missing_source);
                p
            }
            Err(e) => {
                tracing::error!("export plan failed: {e}");
                return;
            }
        };

        // Output path: <destination>/<project-name>.mp4, never relative
        // (a relative output made Explorer fall back to Documents on reveal).
        let out = crate::panels::export_window::resolve_export_output(
            &self.export_state.destination,
            &self.project.name,
            std::path::Path::new(&crate::panels::export_window::default_videos_dir()),
        );
        if let Some(parent) = out.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::error!("export: cannot create output dir {}: {e}", parent.display());
                self.toast_error(format!("{}: {e}", tr("toast-export-failed")));
                return;
            }
        }

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
        self.export_tracker =
            Some(caprust_media_io::export_progress::ExportProgressTracker::start());
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
                    self.export_tracker = None;
                    self.export_finished_path = Some(output.to_string_lossy().to_string());
                    if self.export_state.subtitles_srt {
                        let srt_path = std::path::PathBuf::from(&output).with_extension("srt");
                        match Self::write_srt_file(&self.project.clips, &srt_path) {
                            Ok(n) => self.toast(format!("{} ({n})", tr("exp-srt-done"))),
                            Err(e) => {
                                tracing::error!("srt sidecar failed: {e}");
                                self.toast_error(format!("{}: {e}", tr("toast-export-failed")));
                            }
                        }
                    }
                }
                ExportEvent::Failed(msg) => {
                    tracing::error!("export failed: {msg}");
                    self.toast_error(format!("{}: {msg}", tr("toast-export-failed")));
                    if let Some(id) = self.export_job_id.take() {
                        self.finish_job(id);
                    }
                    self.export_in_progress = false;
                    self.export_tracker = None;
                }
            }
        }
    }

    // ---------------------------------------------------------------
    // Missing-media relink
    // ---------------------------------------------------------------

    // ---------------------------------------------------------------
    // Audio PCM cache
    // ---------------------------------------------------------------

    /// Kill any pending render, then start a fresh one for `hash`.
    /// The render is asynchronous; `poll_audio_cache` promotes it to
    /// active when it completes.
    fn spawn_audio_cache_render(&mut self, hash: u64) {
        self.audio_cache.pending = None;

        let Some(ffmpeg) = self.ffmpeg_status.ffmpeg.clone() else {
            tracing::warn!("audio cache: no ffmpeg, skipping render");
            return;
        };

        let (pw, ph) = self.project.project_dimensions();
        let (rw, rh) = caprust_media_io::player::preview_size(pw, ph, 320);
        let (fps_num, fps_den) = self.export_state.frame_rate.fraction(
            self.project.frame_rate.num as i64,
            self.project.frame_rate.den as i64,
        );
        let models_dir = self.settings.effective_models_dir();

        let plan = match caprust_media_io::export_graph::plan_from_project(
            &self.project,
            rw,
            rh,
            fps_num,
            fps_den,
            23,
            RateMode::Vbr,
            8000,
            "veryfast",
            &models_dir,
            0,
            caprust_core::project::VideoEncoder::H264Cpu,
        ) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("audio cache: plan failed: {e}");
                return;
            }
        };

        let path = caprust_media_io::audio_render::cache_path(hash);
        match caprust_media_io::audio_render::spawn_audio_render(
            std::path::Path::new(&ffmpeg),
            &plan,
            path.clone(),
        ) {
            Ok(job) => {
                tracing::info!("audio cache: spawned render for hash {hash:016x}");
                self.audio_cache.pending = Some(AudioPendingRender { hash, path, job });
            }
            Err(e) => {
                tracing::warn!("audio cache: spawn failed: {e}");
            }
        }
    }

    /// Called once per frame from `update()`. Polls the pending job
    /// and, on completion, promotes it to active. Also watches the
    /// project's `audio_render_hash` and schedules a re-render when
    /// it diverges from the active file, debounced 500 ms.
    fn poll_audio_cache(&mut self) {
        const DEBOUNCE_MS: u128 = 500;

        let mut pending_terminal: Option<(u64, std::path::PathBuf, bool)> = None;
        if let Some(pending) = self.audio_cache.pending.as_mut() {
            if let Some(ev) = pending.job.poll() {
                match ev {
                    caprust_media_io::audio_render::AudioRenderEvent::Done { target, .. } => {
                        pending_terminal = Some((pending.hash, target, true));
                    }
                    caprust_media_io::audio_render::AudioRenderEvent::Failed(msg) => {
                        tracing::warn!("audio cache: render failed: {msg}");
                        pending_terminal = Some((pending.hash, pending.path.clone(), false));
                    }
                    caprust_media_io::audio_render::AudioRenderEvent::Started { .. } => {}
                }
            }
        }
        if let Some((hash, path, ok)) = pending_terminal {
            self.audio_cache.pending = None;
            if ok {
                tracing::info!("audio cache: ready hash {hash:016x} at {}", path.display());
                self.audio_cache.active_hash = hash;
                self.audio_cache.active_path = Some(path);
                self.audio_cache.last_failed_hash = 0;
            } else {
                // Record so we do not retry the same hash in a loop.
                // A subsequent project edit changes the hash, which
                // clears the mark implicitly.
                self.audio_cache.last_failed_hash = hash;
            }
        }

        let live = self.project.audio_render_hash();
        if live != self.audio_cache.active_hash
            && live != self.audio_cache.last_failed_hash
            && self.audio_cache.pending.is_none()
        {
            if self.audio_cache.debounce_at.is_none() {
                self.audio_cache.debounce_at = Some(std::time::Instant::now());
            }
            let ready = self
                .audio_cache
                .debounce_at
                .map(|t| t.elapsed().as_millis() >= DEBOUNCE_MS)
                .unwrap_or(false);
            if ready {
                self.audio_cache.debounce_at = None;
                self.spawn_audio_cache_render(live);
            }
        } else {
            self.audio_cache.debounce_at = None;
        }
    }

    /// Called at the end of every successful `load_project_from`.
    /// Synchronous scan of the media library; opens the dialog when
    /// any file is missing. No-op on clean projects.
    fn check_missing_media_on_load(&mut self) {
        let missing = caprust_core::commands::relink_many::find_missing_media_items(&self.project);
        if missing.is_empty() {
            return;
        }
        tracing::info!("relink: {} media file(s) missing on load", missing.len());
        self.relink_dialog = crate::panels::relink_dialog::RelinkDialogState {
            missing,
            ..Default::default()
        };
        self.relink_dialog_open = true;
        // Fresh load with missing entries: un-dismiss the banner so
        // the user sees it again even if they dismissed it before
        // reloading the same project.
        self.missing_media_dismissed = false;
    }

    fn show_relink_dialog_window(&mut self, ctx: &egui::Context) {
        let mut open = self.relink_dialog_open;
        let mut user_wants_close = false;
        let mut locate_requested = false;
        egui::Window::new(tr("relink-dialog-title"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_width(480.0)
            .show(ctx, |ui| {
                let ev = crate::panels::relink_dialog::show(ui, &mut self.relink_dialog);
                if ev.locate_folder {
                    locate_requested = true;
                }
                if ev.close {
                    user_wants_close = true;
                }
            });
        // Same borrow-trap pattern as the ffmpeg prompt: apply the
        // close decision AFTER the closure returns.
        if user_wants_close {
            open = false;
        }
        self.relink_dialog_open = open;

        if locate_requested {
            self.run_relink_folder_pick();
        }
    }

    /// rfd folder picker -> scan -> RelinkManyCommand. Runs on the UI
    /// thread; scan is depth-capped and typically completes in a few
    /// hundred milliseconds. If it turns out slow on network drives,
    /// move to a background thread in a follow-up commit.
    fn run_relink_folder_pick(&mut self) {
        let Some(dir) = rfd::FileDialog::new()
            .set_title(tr("relink-dialog-locate"))
            .pick_folder()
        else {
            return;
        };

        self.relink_dialog.tried_folder = true;

        let matches = caprust_core::commands::relink_many::scan_folder_for_missing(
            &dir,
            &self.relink_dialog.missing,
        );

        if matches.is_empty() {
            tracing::info!(
                "relink: folder {} matched 0 of {} missing",
                dir.display(),
                self.relink_dialog.missing.len()
            );
            self.relink_dialog.phase =
                crate::panels::relink_dialog::RelinkPhase::Failed(tr("relink-dialog-none-matched"));
            return;
        }

        let n = matches.len();
        let mappings: Vec<_> = matches
            .into_iter()
            .map(|m| caprust_core::commands::relink_many::RelinkMapping {
                media_id: m.media_id,
                old_path: m.old_path,
                new_path: m.new_path,
            })
            .collect();

        let cmd = caprust_core::commands::relink_many::RelinkManyCommand::new(mappings);
        match self.undo_stack.execute(Box::new(cmd), &mut self.project) {
            Ok(()) => {
                tracing::info!("relink: applied {n} mapping(s)");
                self.toast(format!("{} · {}", tr("toast-relinked"), n));
                let still =
                    caprust_core::commands::relink_many::find_missing_media_items(&self.project);
                let still_n = still.len();
                self.relink_dialog.missing = still;
                self.relink_dialog.phase = crate::panels::relink_dialog::RelinkPhase::Done {
                    matched: n,
                    still_missing: still_n,
                };
            }
            Err(e) => {
                tracing::error!("relink: command failed: {e}");
                self.relink_dialog.phase =
                    crate::panels::relink_dialog::RelinkPhase::Failed(e.to_string());
            }
        }
    }

    fn show_ffmpeg_prompt_window(&mut self, ctx: &egui::Context) {
        // Poll the download receiver first; terminal states flip the
        // phase and re-run detection once the binary is on disk.
        let terminal = crate::panels::ffmpeg_prompt::poll(&mut self.ffmpeg_prompt);
        let mut open = self.ffmpeg_prompt_open;
        let mut user_wants_close = false;
        egui::Window::new(tr("ffmpeg-prompt-title"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_width(480.0)
            .show(ctx, |ui| {
                let ev = crate::panels::ffmpeg_prompt::show(
                    ui,
                    &mut self.ffmpeg_prompt,
                    &mut self.settings,
                );
                // Any close path (button or X) must be applied to the
                // local `open` AFTER the closure, since `open` is
                // already borrowed mutably by .open(&mut open).
                // Using a local flag avoids the double-borrow.
                if ev.start_download {
                    // Resolve the target dir: user input, else default.
                    let dir = if self.ffmpeg_prompt.install_dir.trim().is_empty() {
                        caprust_core::ffmpeg::managed_dir(&self.settings)
                    } else {
                        std::path::PathBuf::from(&self.ffmpeg_prompt.install_dir)
                    };
                    self.settings.managed_ffmpeg_dir = Some(dir.to_string_lossy().to_string());
                    let rx = caprust_core::ffmpeg::spawn_ffmpeg_download(
                        dir,
                        caprust_core::ffmpeg::FFMPEG_URL.to_string(),
                    );
                    self.ffmpeg_prompt.receiver = Some(rx);
                    self.ffmpeg_prompt.phase =
                        crate::panels::ffmpeg_prompt::PromptPhase::Downloading {
                            done: 0,
                            total: None,
                        };
                }
                if ev.browse_existing {
                    if let Some(f) = rfd::FileDialog::new()
                        .add_filter("ffmpeg", &["exe"])
                        .pick_file()
                    {
                        self.settings.ffmpeg_path = Some(f.to_string_lossy().to_string());
                        if let Some(parent) = f.parent() {
                            let probe = parent.join("ffprobe.exe");
                            if probe.is_file() {
                                self.settings.ffprobe_path =
                                    Some(probe.to_string_lossy().to_string());
                            }
                        }
                        self.ffmpeg_status = caprust_core::detect_ffmpeg(&self.settings);
                        self.ffmpeg_prompt_open = false;
                    }
                }
                if ev.close {
                    if self.ffmpeg_prompt.dont_ask_again {
                        self.settings.ffmpeg_prompt_dismissed = true;
                    }
                    user_wants_close = true;
                }
            });
        if user_wants_close {
            open = false;
        }
        self.ffmpeg_prompt_open = open;
        if let Some(crate::panels::ffmpeg_prompt::PromptPhase::Done) = terminal {
            self.ffmpeg_status = caprust_core::detect_ffmpeg(&self.settings);
            tracing::info!(
                "ffmpeg: managed binary installed and detected ({} -> {:?})",
                self.settings.managed_ffmpeg_dir.clone().unwrap_or_default(),
                self.ffmpeg_status.ffmpeg
            );
        }
    }

    fn show_settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        // Universal settings window: sized for the tallest tab so the
        // layout does not jump when the user switches tabs. Pinned to
        // the screen centre on first open; the user can still drag it.
        egui::Window::new(tr("set-title"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_width(640.0)
            .min_width(640.0)
            .min_height(700.0)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                let ev = crate::panels::settings_dialog::show(
                    ui,
                    &mut self.theme,
                    &mut self.project.models,
                    &mut self.settings,
                    &mut self.settings_tab,
                    &mut self.ffmpeg_status,
                );
                if let Some(id) = ev.download_requested.clone() {
                    self.start_model_download(&id);
                }
                if ev.export_requested {
                    self.handle_settings_export();
                }
                if ev.import_requested {
                    self.handle_settings_import();
                }
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

    /// Export AppSettings to a user-chosen JSON file. Called from the
    /// Paths -> Backup section. Success and failure both surface as a
    /// toast; errors also log at warn level.
    fn handle_settings_export(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(tr("set-backup-export"))
            .add_filter("JSON", &["json"])
            .set_file_name("caprust-settings.json")
            .save_file()
        else {
            return;
        };

        match self.settings.export_to_file(&path) {
            Ok(()) => {
                tracing::info!("settings exported to {}", path.display());
                self.toast(tr("toast-settings-exported"));
            }
            Err(e) => {
                tracing::warn!("settings export failed: {e:#}");
                self.toast_error(tr("toast-settings-export-failed"));
            }
        }
    }

    /// Load AppSettings from a user-chosen JSON file and replace the
    /// current one, except the ffmpeg/ffprobe paths, which stay local.
    /// Re-detects ffmpeg because models_dir may have changed. Language is picked up on the next update()
    /// via caprust_i18n::set_current_lang, theme is separate storage
    /// and untouched.
    fn handle_settings_import(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(tr("set-backup-import"))
            .add_filter("JSON", &["json"])
            .pick_file()
        else {
            return;
        };

        match caprust_core::settings::AppSettings::import_from_file(&path) {
            Ok(new_settings) => {
                self.settings.replace_from_untrusted(new_settings);
                self.ffmpeg_status = caprust_core::detect_ffmpeg(&self.settings);
                tracing::info!("settings imported from {}", path.display());
                self.toast(tr("toast-settings-imported"));
            }
            Err(e) => {
                tracing::warn!("settings import failed: {e:#}");
                self.toast_error(tr("toast-settings-import-failed"));
            }
        }
    }

    /// Modal shown once on startup when the sync folder holds a
    /// snapshot newer than what this machine has acknowledged.
    /// Load replaces settings and stamps last_synced_at. Keep local
    /// only stamps, so the prompt does not return until the other
    /// machine writes again.
    fn show_settings_sync_prompt_window(&mut self, ctx: &egui::Context) {
        let Some((path, stamp)) = self.settings_sync_prompt.clone() else {
            return;
        };
        let mut open = true;

        egui::Window::new(tr("sync-prompt-title"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(tr("sync-prompt-body"));
                ui.add_space(space::S);
                ui.label(
                    egui::RichText::new(path.display().to_string())
                        .small()
                        .monospace()
                        .color(egui::Color32::from_gray(150)),
                );
                ui.add_space(space::M_PLUS);
                ui.horizontal(|ui| {
                    if ui.button(tr("sync-prompt-load")).clicked() {
                        match caprust_core::settings::AppSettings::load_sync_file(&path) {
                            Ok((loaded, loaded_stamp)) => {
                                self.settings.replace_from_untrusted(loaded);
                                self.settings.last_synced_at = Some(loaded_stamp);
                                self.ffmpeg_status = caprust_core::detect_ffmpeg(&self.settings);
                                self.settings_sync_prompt = None;
                                tracing::info!(
                                    "settings sync: loaded snapshot from {}",
                                    path.display()
                                );
                                self.toast(tr("toast-settings-imported"));
                                return;
                            }
                            Err(e) => {
                                tracing::warn!("settings sync: load failed: {e:#}");
                                self.toast_error(tr("toast-settings-import-failed"));
                                // Treat as acknowledged so the modal
                                // does not reappear every frame.
                                self.settings.last_synced_at = Some(stamp);
                                self.settings_sync_prompt = None;
                                return;
                            }
                        }
                    }
                    if ui.button(tr("sync-prompt-keep")).clicked() {
                        // Acknowledge the file so the modal does not
                        // return until the other machine writes again.
                        self.settings.last_synced_at = Some(stamp);
                        self.settings_sync_prompt = None;
                        tracing::info!("settings sync: kept local, acknowledged stamp {stamp}");
                    }
                });
            });

        if !open {
            self.settings_sync_prompt = None;
        }
    }
}

/// Short "3m 12s" / "45s" / "1h 05m" formatter for the export ETA
/// label. Zero-valued leading units are omitted.
fn format_short_time(seconds: f32) -> String {
    let total = seconds.max(0.0).round() as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
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

/// Fallback for old recent entries: first .jpg in
/// `<project_dir>/cache/thumbnails/`, sorted by filename for
/// determinism across runs. Returns None when the directory is
/// missing or empty.
fn first_thumb_in_dir(project_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let dir = project_dir.join("cache").join("thumbnails");
    let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jpg"))
        .collect();
    entries.sort();
    entries.into_iter().next()
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
    /// Reattach a separated audio clip back into its source video.
    /// `id` is the video id when the action is triggered from a
    /// video's context menu, and the audio id when triggered from
    /// the audio clip (the dispatcher resolves both cases).
    ReattachAudio(uuid::Uuid),
    Copy(uuid::Uuid),
    Paste,
    Duplicate(uuid::Uuid),
    RippleDelete(uuid::Uuid),
    SetSpeed(uuid::Uuid, f32),
    MuteClip(uuid::Uuid),
    FadeDragStart(uuid::Uuid, FadeEdge, f32),
    FadeDragDelta(uuid::Uuid, f32, f32),
    FadeDragEnd(uuid::Uuid),
    /// Timeline double-click on a clip. Selects it and asks the
    /// Properties panel to focus the TextOverlay content editor.
    /// No-op on non-TextOverlay clips.
    FocusTextContent(uuid::Uuid),
    /// Translate a Captions clip's segments into the target language
    /// configured in Settings, on a new Captions track. Only valid on
    /// Captions clips.
    TranslateCaptions(uuid::Uuid),
    /// Create a new multicam group from the currently selected
    /// clips. Only offered when 2+ clips are selected.
    CreateMultiCamGroup(Vec<uuid::Uuid>),
}

impl eframe::App for CapRustApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Live-apply the UI scale and font family. Both are cheap
        // for egui to re-evaluate; the font rebuild only fires when
        // the family actually changes.
        ctx.set_pixels_per_point(self.theme.font_scale);
        if self.last_applied_font_family != Some(self.theme.font_family) {
            setup_phosphor_fonts(ctx, self.theme.font_family);
            self.last_applied_font_family = Some(self.theme.font_family);
        }

        // Refresh the model registry from the built-in defaults once
        // per frame. Cheap (a dozen entries), idempotent, and makes
        // every downstream read site automatically aware of any entry
        // added in a newer build -- whisper-tiny being the case that
        // surfaced this. Existing rows keep status/progress; missing
        // rows are cloned in.
        self.project.models.merge_missing_defaults();
        caprust_i18n::set_current_lang(&self.settings.language);
        self.theme.apply(ctx);

        // Poll export events.
        self.poll_export();

        // Drain background jobs (ffprobe results, thumbnails ready).
        self.drain_update_check();
        self.drain_model_download();
        self.drain_translate_job();
        self.drain_caption_job();
        self.drain_narration_job();
        self.drain_reframe_job();
        self.drain_beat_job();
        self.drain_bg_removal_job();
        self.poll_audio_cache();
        let thumbs_ready = self.job_runner.drain(&mut self.project);

        // Load any newly-ready thumbnails into the timeline texture cache.
        if let (Some(proj_path), false) =
            (self.project.project_path.clone(), thumbs_ready.is_empty())
        {
            for media_id in thumbs_ready {
                // Always reload: the on-disk file was just (re)written.
                // The old TextureHandle would show stale bytes after a
                // cache clear / regen cycle.
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
            let shift = ctx.input(|i| i.modifiers.shift);
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
                    egui::Key::S if ctrl => {
                        self.save_project_to_disk();
                    }
                    egui::Key::O if ctrl => {
                        if let Some(f) = rfd::FileDialog::new()
                            .add_filter("CapRust Project", &["caprust"])
                            .pick_file()
                        {
                            let p = f.to_string_lossy().to_string();
                            self.load_project_from(&p);
                        }
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
                    // Undo / redo. Windows convention is Ctrl+Z / Ctrl+Y;
                    // macOS and some editors also accept Ctrl+Shift+Z for
                    // redo. The !shift guard must come before the shift
                    // variant because match arms are evaluated in order.
                    egui::Key::Z if ctrl && !shift => {
                        if let Err(e) = self.undo_stack.undo(&mut self.project) {
                            tracing::error!("undo failed: {e}");
                            self.toast_error(format!("{}: {e}", tr("toast-undo-failed")));
                        }
                    }
                    egui::Key::Z if ctrl && shift => {
                        if let Err(e) = self.undo_stack.redo(&mut self.project) {
                            tracing::error!("redo failed: {e}");
                            self.toast_error(format!("{}: {e}", tr("toast-redo-failed")));
                        }
                    }
                    egui::Key::Y if ctrl => {
                        if let Err(e) = self.undo_stack.redo(&mut self.project) {
                            tracing::error!("redo failed: {e}");
                            self.toast_error(format!("{}: {e}", tr("toast-redo-failed")));
                        }
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

        // First-run FFmpeg prompt: auto-open once per session when
        // neither a PATH nor a managed install is present and the
        // user has not dismissed the dialog before.
        if !self.ffmpeg_prompt_checked {
            self.ffmpeg_prompt_checked = true;
            if !self.ffmpeg_status.is_available() && !self.settings.ffmpeg_prompt_dismissed {
                self.ffmpeg_prompt.install_dir = caprust_core::ffmpeg::managed_dir(&self.settings)
                    .to_string_lossy()
                    .to_string();
                self.ffmpeg_prompt_open = true;
            }
        }
        if self.ffmpeg_prompt_open {
            self.show_ffmpeg_prompt_window(ctx);
        }

        // Settings sync prompt. One check per session, like ffmpeg.
        if !self.settings_sync_prompt_checked {
            self.settings_sync_prompt_checked = true;
            if let Some((path, stamp)) = self.settings.check_sync_newer() {
                tracing::info!(
                    "settings sync: newer snapshot in {} (stamp {stamp})",
                    path.display()
                );
                self.settings_sync_prompt = Some((path, stamp));
            }
        }
        if self.settings_sync_prompt.is_some() {
            self.show_settings_sync_prompt_window(ctx);
        }
        if self.relink_dialog_open {
            self.show_relink_dialog_window(ctx);
        }
        if self.export_open {
            self.show_export_window(ctx);
        }

        #[cfg(windows)]
        if self.screen_record_open {
            self.show_screen_record_modal(ctx);
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
        // Snapshot the current dock tree into settings before
        // serializing, so a layout change made in this session is
        // persisted with the next save() tick.
        if let Ok(json) = serde_json::to_string(&self.dock_state) {
            self.settings.dock_layout = Some(json);
        }

        // Cloud-folder sync. Hash-guarded so identical settings do
        // not spam the cloud client with writes on every save tick.
        if self.settings.sync_path().is_some() {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            if let Ok(json) = serde_json::to_string(&self.settings) {
                json.hash(&mut h);
            }
            let digest = h.finish();
            if Some(digest) != self.last_sync_hash {
                match self.settings.sync_now() {
                    Ok(Some(stamp)) => {
                        self.last_sync_hash = Some(digest);
                        tracing::info!("settings: sync snapshot written (stamp {stamp})");
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!("settings: sync write failed: {e:#}");
                    }
                }
            }
        }
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
fn setup_phosphor_fonts(ctx: &egui::Context, family: crate::theme::UiFontFamily) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);

    // Optional system font as the highest-priority face in the
    // Proportional family. The named key is inserted at position 0
    // of the Proportional family's list so every proportional
    // request uses it first, falling back to egui's bundled font
    // for glyphs the system font is missing.
    if let Some(path) = family.system_path() {
        match std::fs::read(&path) {
            Ok(bytes) => {
                let key = format!("caprust-system-{family:?}").to_lowercase();
                fonts.font_data.insert(
                    key.clone(),
                    std::sync::Arc::new(egui::FontData::from_owned(bytes)),
                );
                fonts
                    .families
                    .entry(egui::FontFamily::Proportional)
                    .or_default()
                    .insert(0, key.clone());
                tracing::info!(
                    "font: loaded system family {} from {}",
                    family.label(),
                    path.display()
                );
            }
            Err(e) => {
                tracing::warn!(
                    "font: could not read {} for family {}: {e}",
                    path.display(),
                    family.label()
                );
            }
        }
    } else if family != crate::theme::UiFontFamily::EguiDefault {
        tracing::warn!(
            "font: family {} not available on this machine, using egui default",
            family.label()
        );
    }

    ctx.set_fonts(fonts);
}

/// Which operation the active TextOverlay drag is performing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOverlayDragMode {
    /// Reposition the text: motion.x/y.
    Move,
    /// Resize the text: motion.scale, computed from the pointer
    /// distance to the box anchor captured at drag start.
    Scale,
}

/// State for an in-progress text overlay drag from the preview pane.
#[derive(Debug, Clone, Copy)]
pub struct TextOverlayDrag {
    pub clip_id: uuid::Uuid,
    pub mode: TextOverlayDragMode,
    /// Pointer position when the drag started (screen space).
    pub start_ptr: egui::Pos2,
    /// motion.x/y at drag start, so the live delta is computed from
    /// the origin every frame and never accumulates float drift.
    pub origin_x: f32,
    pub origin_y: f32,
    /// motion.scale at drag start; only used in Scale mode.
    pub origin_scale: f32,
    /// Screen-space anchor (box centre) at drag start; only used in
    /// Scale mode. Distance ratio pointer/anchor drives the new scale.
    pub anchor_screen: egui::Pos2,
    /// Screen-space distance pointer-to-anchor at drag start; guards
    /// the ratio against a zero denominator.
    pub origin_dist: f32,
    /// Frame-space pixel size at drag start, so a mid-drag preview
    /// respawn (different rw/rh) does not change the mapping.
    pub rw: f32,
    pub rh: f32,
    /// Scale factors from frame-space to screen-space at drag start.
    pub sx: f32,
    pub sy: f32,
}

#[cfg(windows)]
fn recordings_dir() -> std::path::PathBuf {
    let base = std::env::var("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("CapRust").join("recordings")
}
