use crate::i18n_helper::tr;
use crate::media_jobs::JobRunner;
use crate::panels::asset_browser::AssetBrowserState;
use crate::panels::clip_properties::{PendingEdit, PropertiesState};
use crate::panels::export_window::ExportState;
use crate::panels::media_bin::{MediaBinLayout, MediaBinState, PreviewSize, TabLayout};
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

/// Height of the strip left along the bottom of the Modern editor when the
/// timeline is collapsed. It carries the control that brings the timeline
/// back, so it has to stay reachable.
const TIMELINE_HANDLE_HEIGHT: f32 = 26.0;

/// Everything in the timeline above the tracks: the title strip, the
/// separator and the toolbar. Used to size the band so the lanes are never
/// cut off by the bottom of the window.
const TIMELINE_CHROME_H: f32 = 68.0;

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
    /// Stabilization pass 1 (vidstabdetect): measures camera motion
    /// over the clip's source window, transforms file written into
    /// the project cache.
    Stab,
    /// Face anonymize: YuNet boxes per frame, mask written as FFV1
    /// MKV into the project cache.
    Anon,
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
    /// Receiver for the background encoder-capability probe. `Some`
    /// while a probe is in flight. Nine ffmpeg spawns cannot run on
    /// the UI thread without freezing the window before it paints.
    pub encoder_probe_rx:
        Option<std::sync::mpsc::Receiver<Vec<caprust_core::project::VideoEncoder>>>,
    pub media_bin: MediaBinState,
    pub preview_size: PreviewSize,
    /// Media tab layout mode: "no-filter" (legacy flat list) or "filtered" (with sidebar).
    pub tab_layout: TabLayout,
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
    /// Minutes the user last applied through the Autosave tab's custom
    /// spinner. Owned here so the confirmation can outlive the frame
    /// the button was clicked on. Cleared when the dialog closes.
    /// Transient settings-panel state the app owns: the applied autosave
    /// interval and the live remote-control pairing code. Neither is
    /// persisted; both have to outlive the frame they were set on.
    pub settings_status: crate::panels::settings_dialog::SettingsStatus,
    /// Address the live listener is bound to. A socket cannot change
    /// address, so this is what tells the reconcile step to restart it.
    pub remote_bind: Option<String>,
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
    /// Wall-clock time of the last recovery autosave. `None` until
    /// the first one fires. Compared against the configured interval
    /// in `tick_autosave`.
    pub last_autosave_at: Option<std::time::Instant>,
    /// `render_hash` of the project as of the last autosave (or last
    /// manual Save). Compared each frame to decide whether anything
    /// actually changed since, so an idle session does not keep
    /// rewriting an identical snapshot.
    pub last_autosave_hash: Option<u64>,
    /// `render_hash` of the project at its last manual Save. This is
    /// the real dirty flag: if the current hash differs, the project
    /// has unsaved changes and autosave has something to capture.
    pub last_saved_hash: Option<u64>,
    /// Pending crash-recovery offer. `Some((project_file, offer))`
    /// after `load_project_from` finds a newer snapshot; cleared by
    /// the restore / discard / later buttons in
    /// `show_recovery_prompt_window`.
    pub recovery_prompt: Option<(std::path::PathBuf, caprust_core::recovery::RecoveryOffer)>,
    /// Project file the recovery snapshots belong to.
    ///
    /// `tick_autosave` used to derive this from `project_file_path()`
    /// (`<project_path>/<name>.caprust`) while `offer_recovery` was
    /// handed the path the user actually opened. Those agree for a
    /// project this app saved, but not for one whose file stem differs
    /// from its stored `name`, and the recovery slot key is a hash of
    /// the whole path. The two sides then wrote and read different
    /// slots, so a snapshot existed and was never offered. Recording
    /// the path once, at load and at save, removes the class of bug.
    pub recovery_slot_path: Option<std::path::PathBuf>,
    /// A missing-media scan that was postponed because a recovery
    /// offer was on screen. Runs as soon as that offer is answered.
    pub pending_missing_scan: bool,
    /// Timeline collapsed: the timeline is replaced by a handle strip and
    /// the columns take the whole height.
    pub timeline_collapsed: bool,
    /// Height the user dragged the Modern timeline to. `None` means the
    /// timeline follows `settings.timeline_size`; picking a size clears this,
    /// which is what makes S/M/L win over an earlier drag.
    pub timeline_drag: Option<f32>,
    /// Fraction of a side column taken by its top container, as the user
    /// dragged it. `None` means the column keeps its preset split.
    ///
    /// The sidebars could be dragged left and right but not up and down:
    /// the column was cut at a fixed fraction, so the two containers in it
    /// never moved relative to each other while the middle column could be
    /// dragged freely. These give the columns their own seam. They are not
    /// cleared when the timeline size changes -- that setting says how tall
    /// the timeline is, and has no business moving a sidebar.
    pub left_split: Option<f32>,
    pub right_split: Option<f32>,
    /// Sidebar visibility for the Modern editor. The timeline size presets
    /// these, but they can be set by hand afterwards -- at M and L the preset
    /// turns a sidebar off, and without a way to turn it back on the left
    /// column was simply gone with no way to fill it.
    pub modern_left_on: bool,
    pub modern_right_on: bool,

    // ---- Remote control (MCP over TCP) ----
    /// Protocol state for the in-app MCP server. Holds no project data;
    /// the open session is borrowed per request so the remote drives the
    /// same state and the same undo stack the window shows.
    pub mcp_server: caprust_mcp_core::server::Server,
    /// Socket listener, `None` until the user turns remote control on.
    /// Toggling it off drops the listener and closes the port.
    pub remote_bridge: Option<caprust_mcp_core::remote::RemoteBridge>,
    /// Requests handled since the last drain, for the status line.
    /// Surfaced rather than just logged so a silent failure is visible.
    pub remote_last_batch: usize,
    /// Live pairing code, `None` when none has been issued or it has
    /// expired. Not persisted; the token it produces is.

    #[cfg(windows)]
    pub screen_record: crate::panels::screen_record::ScreenRecordState,
    #[cfg(windows)]
    pub screen_record_open: bool,
    /// Dock tree that owns every editor panel. Replaces the fixed
    /// SidePanel / CentralPanel layout. See `crate::dock`.
    pub dock_state: egui_dock::DockState<crate::dock::Tab>,
    /// Zone contents for the fixed editor. Only read there -- the dock
    /// editor keeps using `dock_state`.
    pub modern: ModernState,
    pub preview: PreviewState,
    pub selected_clips: Vec<uuid::Uuid>,
    pub clip_drag: Option<ClipDrag>,
    /// Rubber-band selection in progress, if any.
    pub marquee: Option<MarqueeState>,
    /// In-progress fade handle drag, if any.
    pub fade_drag: Option<FadeDrag>,
    /// In-progress sticker move, scale or rotate on the preview. Holds
    /// both ends of the gesture so the whole drag is one undo step.
    pub sticker_drag: Option<StickerDrag>,
    /// Sticker PNGs decoded for the preview overlay. Shared with the
    /// asset browser so a mark is not decoded twice.
    pub sticker_tex: crate::panels::asset_browser::StickerTextures,
    pub settings: AppSettings,
    pub ffmpeg_status: caprust_core::FfmpegStatus,
    pub last_dnd_payload: Option<Vec<uuid::Uuid>>,
    pub recent: RecentList,
    /// A project a remote client asked to open, recorded during the
    /// MCP batch and acted on once the borrow of `self.project` is
    /// released. `None` for all but one frame.
    pub pending_remote_open: Option<String>,
    /// A destination a remote client asked to export to, recorded
    /// during the MCP batch and acted on once the borrow of
    /// `self.project` is released. Same reason as
    /// `pending_remote_open`.
    pub pending_remote_export: Option<String>,
    /// Playback state mirrored for remote clients, and where a remote
    /// transport command waits to be applied. The playhead itself is
    /// borrowed for the length of the MCP batch.
    pub remote_transport: PreviewTransport,
    /// Mirror of the export settings, for the same reason: the export
    /// window's state cannot be borrowed alongside the rest.
    pub remote_export: ExportSettingsMirror,
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
    /// Receiver for an in-flight face-anonymize job.
    /// Emits Started / Progress / Finished / Failed events.
    pub anon_rx: Option<std::sync::mpsc::Receiver<crate::media_jobs::AnonEvent>>,
    /// Job id for the anonymize bar entry.
    pub anon_job_id: Option<u64>,
    /// Relative mask path (e.g. "cache/anon/<clip>.mkv") for the
    /// in-flight anonymize job.
    pub anon_rel_path: Option<String>,
    /// Receiver for an in-flight stabilization detect job.
    /// Emits Started / Progress / Finished / Failed events.
    pub stab_rx: Option<std::sync::mpsc::Receiver<crate::media_jobs::StabEvent>>,
    /// Job id for the stabilization bar entry.
    pub stab_job_id: Option<u64>,
    /// Relative transforms path (e.g. "cache/stab/<clip>.trf") for the
    /// in-flight stab job. Stored here so the drain can write it into
    /// the clip without recomputing.
    pub stab_rel_path: Option<String>,
    /// Receiver for an in-flight save-frame worker. Single result
    /// message (Ok(path) / Err(reason)), drained per frame.
    pub save_frame_rx: Option<std::sync::mpsc::Receiver<Result<std::path::PathBuf, String>>>,
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
    /// Pending clip rename: (clip_id, edit_buffer). Some while the
    /// rename modal is open. Opened by double-clicking a non-text
    /// clip or the clip context menu "Rename" item.
    pub clip_rename: Option<(uuid::Uuid, String)>,
    /// Pending template apply (replaces the timeline). Some while the
    /// confirm modal is open.
    pub template_confirm: Option<crate::panels::asset_browser::TemplateRequest>,
    /// Pending save-as-template: name buffer. Some while the modal is open.
    pub template_save: Option<String>,
    /// In-flight SFX download: (entry id, result channel). Single
    /// download at a time in v1; the row shows a spinner meanwhile.
    pub sfx_download: Option<(
        String,
        std::sync::mpsc::Receiver<Result<std::path::PathBuf, String>>,
    )>,
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
    /// Importers found on disk, scanned once. The File menu is rebuilt
    /// every frame, so the directory is not re-read per frame.
    pub importer_cache: Option<Vec<caprust_core::importer::ImporterPlugin>>,
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

/// A sticker being moved or resized on the preview.
///
/// `from` is where the gesture started and never changes; `to` is where
/// the sticker is right now. Committing the pair as one command keeps a
/// drag to one undo step no matter how far the pointer travelled.
#[derive(Debug, Clone)]
pub struct StickerDrag {
    pub clip_id: uuid::Uuid,
    pub from: caprust_core::OverlayPlacement,
    pub to: caprust_core::OverlayPlacement,
    /// Where the box's centre sat when the gesture began, in unit-frame
    /// coordinates. Frozen for the length of the drag.
    ///
    /// Resizing reads the pointer's distance from this rather than from
    /// the current centre, which moves as the sticker grows. Measuring
    /// against a moving reference feeds the sticker's own growth back
    /// into its size and the grip runs away from the pointer.
    /// Frozen at the start of the gesture, in **frame fractions**.
    /// Not pixels: it is handed to `OverlayPlacement::at_centre`, which
    /// is in fractions, and a pixel handed over by mistake solves to a
    /// position past the end of the frame.
    pub origin_centre: (f32, f32),
    /// Degrees the pointer is asking for on top of `from.rotation`.
    /// Zero for a move or a scale.
    pub extra_degrees: f32,
    /// Which gesture is running, so the drawing can show the right one
    /// and a scale that began as a rotate does not turn into a move.
    pub mode: StickerMode,
}

/// Where the rotation handle sits: above the box, on the vertical centre
/// line, far enough out that it does not land on the picture itself.
pub fn rotation_grip_rect(box_rect: egui::Rect) -> egui::Rect {
    const SIZE: f32 = 22.0;
    // A minimum stand-off, so a sticker scaled almost to nothing still
    // has a handle that is not under the pointer by default.
    let out = (box_rect.height() * 0.45).max(18.0);
    egui::Rect::from_center_size(
        egui::pos2(box_rect.center().x, box_rect.top() - out),
        egui::vec2(SIZE, SIZE),
    )
}

/// A sticker's four corners on screen: top-left, top-right,
/// bottom-right, bottom-left, already turned.
///
/// The turns of the rectangle rather than the box around it. The export
/// composites the turned rectangle, and that is also what the preview
/// paints, so the handles have to sit on these: on the corners of the
/// bounding box they end up in empty space once the sticker is turned,
/// which is where the corner grips stopped responding.
pub fn sticker_corners(
    place: &caprust_core::OverlayPlacement,
    frame_rect: egui::Rect,
    extra_degrees: f32,
) -> [egui::Pos2; 4] {
    let fa = frame_rect.width() / frame_rect.height().max(1.0);
    // `frame_box_at` rather than `frame_box`: the caller passes a live
    // angle for a sticker whose rotation handle is down, and the box has
    // to describe the angle on screen.
    let (bx, by, bw, bh) = place.frame_box_at(fa, extra_degrees);
    let (cx, cy) = (bx + bw / 2.0, by + bh / 2.0);
    let (sin, cos) = (place.rotation + extra_degrees).to_radians().sin_cos();
    let w = frame_rect.width() * bw;
    let h = frame_rect.height() * bh;
    let at = |dx: f32, dy: f32| {
        egui::pos2(
            frame_rect.left() + frame_rect.width() * cx + dx * cos - dy * sin,
            frame_rect.top() + frame_rect.height() * cy + dx * sin + dy * cos,
        )
    };
    [
        at(-w / 2.0, -h / 2.0),
        at(w / 2.0, -h / 2.0),
        at(w / 2.0, h / 2.0),
        at(-w / 2.0, h / 2.0),
    ]
}

/// The bounding box of four corners, for the rotation handle to sit above.
fn corners_bounds(corners: &[egui::Pos2; 4]) -> egui::Rect {
    let mut r = egui::Rect::from_min_max(corners[0], corners[0]);
    for c in corners {
        r = r.union(egui::Rect::from_min_max(*c, *c));
    }
    r
}

/// How far past the edge of the frame a sticker may be dragged, as a
/// fraction of the frame. Enough to hang a mark off the edge on purpose,
/// not enough to lose it: anything beyond this puts the centre off the
/// picture, and a mark whose centre is off the picture cannot be grabbed
/// to come back.
pub const OVERDRAG: f32 = 0.08;

/// Where an animation has put the sticker, relative to its resting
/// placement.
///
/// The export applies these as ffmpeg filters over the same `t`; the
/// preview has to apply something equivalent or the two disagree about
/// what a name means. The magnitudes are the ones in
/// `sticker_animation_fragment`, in the sticker's own pixels, scaled to
/// the on-screen size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StickerMotion {
    /// Extra rotation in radians, about the sticker's own centre.
    pub angle: f32,
    /// Offset in screen pixels.
    pub dx: f32,
    pub dy: f32,
    /// Multiplier on the drawn size.
    pub scale: f32,
    /// Multiplier on the drawn opacity.
    pub alpha: f32,
}

impl Default for StickerMotion {
    /// "No motion", which is not the same as all-zeros: a size of zero
    /// would erase the sticker and an alpha of zero would hide it.
    fn default() -> Self {
        Self {
            angle: 0.0,
            dx: 0.0,
            dy: 0.0,
            scale: 1.0,
            alpha: 1.0,
        }
    }
}

/// The motion for an animation id at `t` seconds, as the export computes
/// it. Unknown ids are static, which is also what the render does.
pub fn sticker_motion(id: Option<&str>, t: f32, source_px: f32, screen_px: f32) -> StickerMotion {
    let mut m = StickerMotion {
        scale: 1.0,
        alpha: 1.0,
        ..StickerMotion::default()
    };
    let Some(id) = id else {
        return m;
    };
    // Everything in the filtergraph is in the sticker's own pixels
    // (256x256 for the bundled ones), so the offsets are scaled to how
    // big the mark actually is on screen.
    let k = screen_px / source_px.max(1.0);
    match id {
        // Fade up in place over 0.2 s.
        "pop" => {
            m.alpha = (t / 0.20).clamp(0.0, 1.0);
        }
        // Two decaying hops, then a settle. The filtergraph crops to
        // iw/1.25 and scales back by 1.25, which is a net 1.25 with a
        // vertical offset.
        "bounce" => {
            m.scale = 1.25;
            let hop = (2.0 * std::f32::consts::PI * 1.6 * t).sin().abs() * (-3.2 * t).exp();
            m.dy = -18.0 * hop * k;
        }
        // Continuous rotation about its own centre.
        "spin" => {
            m.angle = 1.6 * t;
        }
        // Slow figure of eight, with the same 1.15 crop-and-scale the
        // filtergraph uses around it.
        "drift" => {
            m.scale = 1.15;
            m.dx = 26.0 * (2.0 * std::f32::consts::PI * t / 6.0).sin() * k;
            m.dy = 16.0 * (2.0 * std::f32::consts::PI * t / 9.0).cos() * k;
        }
        _ => {}
    }
    m
}

/// The factor a track's stored height is multiplied by to get the row
/// height it is drawn at.
///
/// Two things decide it. The timeline size gives a nominal factor -- 1.0,
/// 1.3, 1.7 for S, M and L -- and the tracks' own heights give the
/// proportions between them.
///
/// When the nominal size does not fit, everything is scaled down
/// together so that all the tracks stay on screen. In L the rows are
/// 1.7 times taller, five of them do not fit, and the last one was cut
/// off with no way to scroll to it. The proportions survive: a track
/// resized to twice another's is still twice as tall, and the whole set
/// is scaled to fit the space there is.
///
/// The result is that the sum of the row heights is at most
/// `available_h`, and exactly that when they do not fit.
pub fn lane_factor(size: caprust_core::TimelineSize, heights: &[f32], available_h: f32) -> f32 {
    lane_layout(size, heights, available_h).factor
}

/// How the rows are laid out: the factor the stored heights are
/// multiplied by, and what it takes to move a divider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LaneLayout {
    /// The factor the stored heights are multiplied by.
    pub factor: f32,
    /// The sum of the stored heights.
    pub total: f32,
    /// Whether the set was scaled down to fit the space.
    pub fitted: bool,
}

impl LaneLayout {
    /// How much the dragged track's row height changes per unit of its
    /// stored height, at this layout.
    ///
    /// When the set is scaled to fit, the rows are
    /// `height * available / total`, and the total moves as the dragged
    /// height moves. So the marginal rate is not the factor but the
    /// factor times the share of the other tracks. Dividing the drag
    /// delta by the plain factor left the divider trailing the pointer
    /// by the dragged track's share of the height.
    pub fn row_height_per_stored(&self, dragged_height: f32) -> f32 {
        if !self.fitted || self.total <= 0.0 {
            return self.factor;
        }
        self.factor * (self.total - dragged_height).max(0.0) / self.total
    }
}

/// One frame of the track-height drag.
///
/// `dy` is the pointer's movement in screen pixels. The stored height
/// moves by that divided by how much a unit of stored height changes the
/// row -- which is not the factor when the rows are scaled to fit, and
/// dividing by the plain factor left the divider trailing the pointer.
///
/// A function so the drag and its test are the same code, rather than
/// the test driving a copy of it.
pub fn stored_height_after_drag(stored: f32, dy: f32, layout: &LaneLayout) -> f32 {
    let per_stored = layout.row_height_per_stored(stored);
    (stored + dy / per_stored.max(1e-3)).clamp(40.0, 160.0)
}

/// The layout of the track rows.
pub fn lane_layout(
    size: caprust_core::TimelineSize,
    heights: &[f32],
    available_h: f32,
) -> LaneLayout {
    use caprust_core::TimelineSize;
    let nominal = match size {
        TimelineSize::S => 1.0,
        TimelineSize::M => 1.3,
        TimelineSize::L => 1.7,
    };
    let total: f32 = heights.iter().sum();
    if total <= 0.0 || available_h <= 0.0 {
        return LaneLayout {
            factor: nominal,
            total,
            fitted: false,
        };
    }
    // Fits: the nominal size, untouched.
    if total * nominal <= available_h {
        return LaneLayout {
            factor: nominal,
            total,
            fitted: false,
        };
    }
    // Does not: scale the whole set so the sum lands exactly on the
    // space available.
    LaneLayout {
        factor: (available_h / total).min(nominal),
        total,
        fitted: true,
    }
}

/// A point in screen pixels, as a fraction of the frame.
///
/// Two coordinate systems run through the overlay and they are not
/// interchangeable: the painter, the handles and `drag_delta` are all in
/// screen pixels, while a placement's `x` and `y` are fractions of the
/// frame. The drag used to keep its frozen centre in pixels and hand it
/// to `at_centre` as though it were a fraction -- so a scale or a rotate
/// solved `x = (974 - 0.07) / 0.85`, clamped to the maximum, and the
/// sticker jumped to the corner of the frame every time it was touched.
pub fn to_unit(p: egui::Pos2, frame_rect: egui::Rect) -> (f32, f32) {
    (
        (p.x - frame_rect.left()) / frame_rect.width().max(1.0),
        (p.y - frame_rect.top()) / frame_rect.height().max(1.0),
    )
}

/// Where a drag is allowed to put a sticker's centre.
///
/// Its own function so it can be checked. The clamp used to be inline in
/// the move branch, where a test could only read the constant and not
/// what the drag did with it -- so the drag could have ignored the
/// constant entirely and every assertion would still have held.
pub fn clamp_centre(cx: f32, cy: f32) -> egui::Pos2 {
    egui::pos2(
        cx.clamp(-OVERDRAG, 1.0 + OVERDRAG),
        cy.clamp(-OVERDRAG, 1.0 + OVERDRAG),
    )
}

/// Which part of a sticker's overlay the pointer is over.
///
/// Decided from the pointer rather than from which of several
/// overlapping egui interact rects fired. Four corners, a rotation
/// handle and a body all overlap, and egui hands the pointer to whichever
/// was registered last, so half of each corner is unreachable and a
/// gesture fails silently on whichever piece lost. That produced a
/// control where resize worked, move did not and rotate was a 16px
/// target you had to hit exactly.
///
/// The order matters: rotation wins over the corners, because the corner
/// test is a radius around each corner and the handle sits above them.
pub fn sticker_handle_at(
    pointer: egui::Pos2,
    corners: [egui::Pos2; 4],
    grip: f32,
    rot_rect: egui::Rect,
) -> StickerMode {
    if rot_rect.contains(pointer) {
        return StickerMode::Rotate;
    }
    // A square a little larger than the mark that is drawn, rather than
    // a circle inscribed in it. The circle is what made the corners of
    // the mark unreachable -- you could see a handle and miss it by
    // aiming at its corner.
    for c in corners {
        let r = egui::Rect::from_center_size(c, egui::vec2(grip, grip) * 1.5);
        if r.contains(pointer) {
            return StickerMode::Scale;
        }
    }
    StickerMode::Move
}

/// One sticker resolved to screen space, ready to paint.
#[derive(Debug, Clone, PartialEq)]
pub struct StickerQuad {
    /// The PNG to draw. Bundled stickers are addressed by id, anything
    /// else by path; `paint_sticker_quad` works out which.
    pub path: String,
    /// Corners in frame order: top-left, top-right, bottom-right,
    /// bottom-left. Already turned.
    pub corners: [egui::Pos2; 4],
    /// The animation id, if the clip has one. The same four the export
    /// graph knows -- `pop`, `bounce`, `spin`, `drift` -- so a name means
    /// the same thing in both.
    pub animation: Option<String>,
    /// Seconds since the sticker came on screen. Every animation is a
    /// function of this, as it is in the filtergraph.
    pub t_sec: f32,
}

/// Every sticker that belongs on screen over `frame_rect`, in timeline
/// order.
///
/// Free-standing, taking the project and the playhead and nothing else,
/// so what gets drawn cannot quietly grow a condition. The version of
/// this that lived in `App` gated the painting on the selection --
/// correct while ffmpeg was compositing stickers as well, and the reason
/// every sticker in an edit went invisible the moment it stopped being
/// selected, which is nearly always: ffmpeg was not drawing it either by
/// then.
pub fn sticker_quads(
    project: &caprust_core::ProjectState,
    playhead_ms: u64,
    frame_rect: egui::Rect,
) -> Vec<StickerQuad> {
    let mut out = Vec::new();
    for c in project.stickers_under_playhead(playhead_ms) {
        let Some(place) = c.overlay.as_ref() else {
            continue;
        };
        let path = match &c.clip_type {
            caprust_core::ClipType::Image { path, .. } => path.clone(),
            _ => continue,
        };
        // Seconds since the sticker came up. Every animation is a
        // function of this, as it is in the filtergraph.
        let t_sec = playhead_ms.saturating_sub(c.start_time_ms) as f32 / 1000.0;
        out.push(StickerQuad {
            path,
            corners: sticker_corners(place, frame_rect, 0.0),
            animation: place.animation.clone(),
            t_sec,
        });
    }
    out
}

/// Which gesture a sticker drag is currently doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StickerMode {
    Move,
    Scale,
    Rotate,
}

/// Where a drag wants the sticker to be, all in one bundle.
///
/// Six scalars arrived as eight arguments across three call sites, and
/// `aspect` sitting next to `scale` invited swapping them at a call site
/// where the difference would be invisible.
#[derive(Debug, Clone, Copy)]
pub struct StickerGeom {
    pub scale: f32,
    /// Where the gesture wants the box's centre, in **frame fractions**.
    /// Not pixels: this is handed to `at_centre`, which is in fractions.
    pub centre: (f32, f32),
    pub aspect: f32,
    pub frame_aspect: f32,
    pub rotation: f32,
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

/// The app's side of [`caprust_mcp_core::server::SessionControl`], lent
/// to the MCP batch for the length of one frame.
///
/// Holds borrows of two separate `CapRustApp` fields rather than the app
/// itself, so it can coexist with the live borrow of `project` and
/// `undo_stack` that the tools need. Loading the project is not done
/// here -- see `pending_remote_open` for why.
struct RemoteSession<'a> {
    recent: &'a mut RecentList,
    pending: &'a mut Option<String>,
    /// Raw last preview frame, and the playhead it belongs to.
    /// Borrowed so the frame is only copied when a phone actually asks.
    /// Cloning it every UI frame costs about a megabyte of allocation
    /// sixty times a second for a request that arrives once a second at
    /// best.
    player: &'a crate::preview_player::PreviewPlayer,
    /// Export request recorded during the batch, acted on afterwards.
    export_dest: &'a mut Option<String>,
    export_progress: f32,
    /// Mirror of the export settings, written back after the batch.
    export: &'a mut ExportSettingsMirror,
    /// Playback state, recorded at the start of the batch and written
    /// back after. The playhead lives in `self` next to everything else
    /// the batch borrows, so it cannot be handed out as a field
    /// reference alongside `recent` and `player`.
    transport: &'a mut PreviewTransport,
}

/// Playback state a remote transport command acts on.
#[derive(Default)]
pub struct PreviewTransport {
    playing: bool,
    at_ms: u64,
    /// Set by a transport command, acted on once the batch has ended.
    action: Option<(String, Option<u64>)>,
}

/// Mirror of the export settings, for the same reason: the export
/// window's own state is on `self` and cannot be borrowed alongside the
/// rest of the batch.
pub struct ExportSettingsMirror {
    resolution: String,
    frame_rate: String,
    quality: String,
    rate_mode: String,
    color_range: String,
    subtitles_srt: bool,
    advanced: bool,
    /// Set when a phone actually changed something.
    ///
    /// Without this the mirror is written back every frame, so the
    /// export window on the computer is reset to whatever the phone
    /// last saw -- and to the defaults, since that is what the mirror
    /// starts as. Changing a setting in the export window would not
    /// stick, which is worse than the feature being absent.
    dirty: bool,
}

impl ExportSettingsMirror {
    fn of(state: &crate::panels::export_window::ExportState) -> Self {
        Self {
            resolution: state.resolution.id().into(),
            frame_rate: state.frame_rate.id().into(),
            quality: state.quality.id().into(),
            rate_mode: state.rate_mode.id().into(),
            color_range: state.color_range.id().into(),
            subtitles_srt: state.subtitles_srt,
            advanced: state.advanced,
            dirty: false,
        }
    }

    /// What a setting holds now.
    fn get(&self, key: &str) -> Option<String> {
        Some(match key {
            "resolution" => self.resolution.clone(),
            "frame_rate" => self.frame_rate.clone(),
            "quality" => self.quality.clone(),
            "rate_mode" => self.rate_mode.clone(),
            "color_range" => self.color_range.clone(),
            "subtitles_srt" => self.subtitles_srt.to_string(),
            "advanced" => self.advanced.to_string(),
            _ => return None,
        })
    }

    /// Apply a setting. Unknown keys are rejected here so a client
    /// with a stale vocabulary is told rather than silently ignored.
    fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        self.dirty = true;
        match key {
            "resolution" => self.resolution = value.into(),
            "frame_rate" => self.frame_rate = value.into(),
            "quality" => self.quality = value.into(),
            "rate_mode" => self.rate_mode = value.into(),
            "color_range" => self.color_range = value.into(),
            "subtitles_srt" => self.subtitles_srt = parse_bool(value)?,
            "advanced" => self.advanced = parse_bool(value)?,
            other => {
                return Err(format!(
                    "no export setting called `{other}`; valid keys are resolution, \
                     frame_rate, quality, rate_mode, color_range, subtitles_srt, advanced"
                ))
            }
        }
        Ok(())
    }
}

impl caprust_mcp_core::server::SessionControl for RemoteSession<'_> {
    fn recent(&self) -> Vec<RecentProject> {
        self.recent.items.clone()
    }

    fn request_open(&mut self, path: &std::path::Path) -> Result<(), String> {
        if let Some(p) = self.recent.find(&path.to_string_lossy()) {
            tracing::debug!("remote: requested open of {}", p.name);
        }
        *self.pending = Some(path.to_string_lossy().to_string());
        Ok(())
    }

    fn preview_frame(&self, max_w: u32) -> Result<Option<String>, String> {
        // Read here rather than copied into the struct up front. This
        // runs on every UI frame whether or not anyone is watching.
        let Some(f) = self.player.raw_frame() else {
            return Ok(None);
        };
        encode_jpeg_base64(f, max_w as usize).map(Some)
    }

    fn preview_transport(&mut self, action: &str, at_ms: Option<u64>) -> Result<bool, String> {
        // Recorded, not performed. The playhead is borrowed by the
        // batch closure, so it is applied once the batch has returned.
        self.transport.action = Some((action.to_string(), at_ms));
        Ok(true)
    }

    fn preview_status(&self) -> Option<(bool, u64)> {
        Some((self.transport.playing, self.transport.at_ms))
    }

    fn request_export(&mut self, destination: &str) -> Result<(), String> {
        *self.export_dest = Some(destination.to_string());
        Ok(())
    }

    fn export_progress(&self) -> Option<f32> {
        if self.export_progress <= 0.0 {
            None
        } else {
            Some(self.export_progress)
        }
    }

    fn export_settings(&self) -> Option<serde_json::Value> {
        Some(export_settings_value(self.export))
    }

    fn set_export_setting(&mut self, key: &str, value: &str) -> Result<(), String> {
        self.export.set(key, value)
    }
}

/// Describe the export settings with the accepted values alongside.
///
/// The accepted values come from the enums themselves, not a
/// hand-written list. A client with its own list breaks the day the
/// desktop adds a resolution, and will happily send a value the
/// encoder has never heard of.
fn export_settings_value(state: &ExportSettingsMirror) -> serde_json::Value {
    use crate::panels::export_window::{ColorRange, QualityTier};
    use caprust_media_io::export::{ExportFrameRate, ExportResolution, RateMode};
    use serde_json::json;

    let opts = |items: Vec<(&str, &str)>| -> serde_json::Value {
        json!(items
            .into_iter()
            .map(|(id, label)| json!({ "id": id, "label": label }))
            .collect::<Vec<_>>())
    };

    json!({
        "resolution": state.get("resolution").unwrap_or_default(),
        "frame_rate": state.get("frame_rate").unwrap_or_default(),
        "quality": state.get("quality").unwrap_or_default(),
        "rate_mode": state.get("rate_mode").unwrap_or_default(),
        "color_range": state.get("color_range").unwrap_or_default(),
        "subtitles_srt": state.subtitles_srt,
        "advanced": state.advanced,
        "available": {
            "resolution": opts(ExportResolution::all().iter().map(|r| (r.id(), r.label())).collect()),
            "frame_rate": opts(ExportFrameRate::all().iter().map(|r| (r.id(), r.label())).collect()),
            "quality": opts(
                [QualityTier::Small, QualityTier::Regular, QualityTier::Large]
                    .iter().map(|q| (q.id(), q.label())).collect()
            ),
            "rate_mode": opts(RateMode::all().iter().map(|r| (r.id(), r.label())).collect()),
            "color_range": opts(
                [ColorRange::Limited, ColorRange::Full]
                    .iter().map(|c| (c.id(), c.label())).collect()
            ),
        }
    })
}

/// Apply the mirrored settings back onto the real export state.
///
/// The one place a wire id becomes an enum, validated here rather than
/// on the way in. Validating early would mean a second copy of every
/// accepted-value list; validating here means a bad id never reaches
/// the mirror looking accepted, and the error names the fix.
fn apply_export_mirror(
    state: &mut crate::panels::export_window::ExportState,
    mirror: &ExportSettingsMirror,
) -> Result<(), String> {
    use crate::panels::export_window::{ColorRange, QualityTier};
    use caprust_media_io::export::{ExportFrameRate, ExportResolution, RateMode};

    state.resolution = ExportResolution::all()
        .into_iter()
        .find(|r| r.id() == mirror.resolution)
        .ok_or_else(|| bad("resolution", &mirror.resolution))?;
    state.frame_rate = ExportFrameRate::all()
        .into_iter()
        .find(|r| r.id() == mirror.frame_rate)
        .ok_or_else(|| bad("frame_rate", &mirror.frame_rate))?;
    state.quality = [QualityTier::Small, QualityTier::Regular, QualityTier::Large]
        .into_iter()
        .find(|q| q.id() == mirror.quality)
        .ok_or_else(|| bad("quality", &mirror.quality))?;
    state.rate_mode = RateMode::all()
        .into_iter()
        .find(|r| r.id() == mirror.rate_mode)
        .ok_or_else(|| bad("rate_mode", &mirror.rate_mode))?;
    state.color_range = [ColorRange::Limited, ColorRange::Full]
        .into_iter()
        .find(|c| c.id() == mirror.color_range)
        .ok_or_else(|| bad("color_range", &mirror.color_range))?;
    state.subtitles_srt = mirror.subtitles_srt;
    state.advanced = mirror.advanced;
    Ok(())
}

fn bad(key: &str, value: &str) -> String {
    format!("`{value}` is not a valid `{key}`; call export_settings for the accepted values")
}

fn parse_bool(v: &str) -> Result<bool, String> {
    match v {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        other => Err(format!("`{other}` is not true or false")),
    }
}

/// Encode a preview frame as a base64 JPEG, scaled so the longest side
/// is at most `max_w`.
///
/// JPEG rather than PNG because this crosses a socket to a phone: a
/// 320x180 frame is roughly 8 KB as JPEG against 230 KB of raw RGBA,
/// and preview is polled. Scaling here rather than in the decoder
/// because the decoder's size is chosen for the desktop window, which
/// is not the size the phone wants.
///
/// Nearest-neighbour, not a resampling filter. The preview is a
/// thumbnail on a small screen; a filter would cost more than it shows,
/// and nearest keeps this to a handful of lines.
/// RGBA to RGB, dropping the alpha byte from every pixel.
fn drop_alpha(rgba: &[u8]) -> Vec<u8> {
    let px = rgba.len() / 4;
    let mut rgb = Vec::with_capacity(px * 3);
    for i in 0..px {
        rgb.extend_from_slice(&rgba[i * 4..i * 4 + 3]);
    }
    rgb
}

/// Returns the encoded string, or the reason it could not be encoded.
/// `String` rather than `anyhow::Result` because this crate does not
/// depend on anyhow and the caller turns it into a tool error anyway.
fn encode_jpeg_base64(
    frame: &crate::preview_player::RawFrame,
    max_w: usize,
) -> Result<String, String> {
    use base64::Engine;

    let (w, h) = (frame.width, frame.height);
    if w == 0 || h == 0 {
        return Err("preview frame has no pixels".into());
    }
    let scale = if max_w > 0 && w > max_w {
        max_w as f32 / w as f32
    } else {
        1.0
    };
    let (dw, dh) = (
        ((w as f32 * scale) as usize).max(1),
        ((h as f32 * scale) as usize).max(1),
    );

    let mut small = vec![0u8; dw * dh * 4];
    for y in 0..dh {
        let sy = (y as f32 / scale) as usize;
        let sy = sy.min(h - 1);
        for x in 0..dw {
            let sx = ((x as f32 / scale) as usize).min(w - 1);
            let s = (sy * w + sx) * 4;
            let d = (y * dw + x) * 4;
            small[d..d + 4].copy_from_slice(&frame.rgba[s..s + 4]);
        }
    }

    // JPEG has no alpha channel, and the encoder rejects Rgba8 outright
    // rather than quietly dropping the fourth byte. So it has to be
    // dropped here. The source alpha is always 255 by the time a frame
    // reaches a preview surface.
    let rgb = drop_alpha(&small);

    let mut out = Vec::new();
    // 70 is a deliberate compromise: visibly softer than a PNG at
    // this size, but the banding a lower quality would add is not
    // worth the bytes on a preview that refreshes several times a
    // second.
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 70);
    enc.encode(&rgb, dw as u32, dh as u32, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("preview encode failed: {e}"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(out))
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
        // Media-bin layout (tab mode + sidebar width) is persisted
        // across sessions; without this the View → Tab layout toggle
        // always reverted to "No filter" on the next launch.
        let media_bin_layout: MediaBinLayout = cc
            .storage
            .and_then(|s| s.get_string("media_bin_layout"))
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let media_bin = MediaBinState {
            tab_layout: media_bin_layout.tab_layout,
            sidebar_width: media_bin_layout.sidebar_width,
            ..MediaBinState::default()
        };
        let update_rx = spawn_update_check(&settings);
        let mut app = Self {
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
            encoder_probe_rx: None,
            media_bin,
            preview_size: PreviewSize::Medium,
            tab_layout: media_bin_layout.tab_layout,
            timeline_tools: TimelineToolState::default(),
            playhead_ms: 0,
            timeline_zoom: 1.0,
            settings_tab: Default::default(),
            ffmpeg_prompt: Default::default(),
            ffmpeg_prompt_open: false,
            ffmpeg_prompt_checked: false,
            settings_sync_prompt: None,
            settings_sync_prompt_checked: false,
            settings_status: Default::default(),
            remote_bind: None,
            last_sync_hash: None,
            last_applied_font_family: None,
            relink_dialog: Default::default(),
            relink_dialog_open: false,
            missing_media_dismissed: false,
            last_autosave_at: None,
            last_autosave_hash: None,
            last_saved_hash: None,
            recovery_prompt: None,
            recovery_slot_path: None,
            pending_missing_scan: false,
            timeline_collapsed: false,
            timeline_drag: None,
            left_split: None,
            right_split: None,
            modern_left_on: true,
            modern_right_on: true,
            mcp_server: Default::default(),
            remote_bridge: None,
            remote_last_batch: 0,
            #[cfg(windows)]
            screen_record: crate::panels::screen_record::ScreenRecordState::new_defaults(),
            #[cfg(windows)]
            screen_record_open: false,
            dock_state: starting_dock_state(&settings),
            modern: ModernState::default(),
            preview: PreviewState::default(),
            selected_clips: Vec::new(),
            clip_drag: None,
            marquee: None,
            fade_drag: None,
            sticker_drag: None,
            sticker_tex: Default::default(),
            settings,
            ffmpeg_status: ffmpeg_status.clone(),
            last_dnd_payload: None,
            recent,
            pending_remote_open: None,
            pending_remote_export: None,
            remote_transport: PreviewTransport::default(),
            remote_export: ExportSettingsMirror::of(&ExportState::default()),
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
            anon_rx: None,
            anon_job_id: None,
            anon_rel_path: None,
            stab_rx: None,
            stab_job_id: None,
            stab_rel_path: None,
            save_frame_rx: None,
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
            clip_rename: None,
            template_confirm: None,
            template_save: None,
            sfx_download: None,
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
            importer_cache: None,
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
        };
        // Scan once at startup so the start-screen Settings shows the
        // true on-disk state. Without this the registry is all
        // NotDownloaded until some job path happens to scan, which is
        // exactly the "models need downloading although they are on
        // disk" report after a version update.
        app.project.models.merge_missing_defaults();
        app.project
            .models
            .scan_local(&app.settings.effective_models_dir());
        // Extract embedded LUTs so built-in grades work on installed
        // builds (installer ships only the exe). Best-effort: a
        // failure only logs, custom .cube paths are unaffected.
        if let Err(e) = caprust_media_io::export_graph::ensure_builtin_luts() {
            tracing::warn!("builtin LUT extract failed: {e:#}");
        }
        app
    }

    fn create_project(&mut self) {
        // The selected location is the parent folder; the project gets its
        // own subdirectory named after the project. The file will be
        // <location>/<name>/<name>.caprust.
        let mut project_folder = std::path::PathBuf::from(&self.draft.location);
        let safe_name: String = self
            .draft
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        project_folder.push(&safe_name);
        std::fs::create_dir_all(&project_folder).ok();

        self.project = ProjectState {
            name: self.draft.name.clone(),
            aspect_ratio: self.draft.aspect_ratio.clone(),
            base_resolution: self.draft.base_resolution,
            frame_rate: self.draft.frame_rate,
            project_path: Some(project_folder.to_string_lossy().to_string()),
            ..ProjectState::default()
        };
        // Fresh registry from Default is all NotDownloaded; rescan so
        // Settings shows the on-disk truth (same reason as new()).
        self.project.models.merge_missing_defaults();
        let models_dir = self.settings.effective_models_dir();
        self.project.models.scan_local(&models_dir);
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
                // The on-disk file now matches the in-memory project,
                // so nothing is pending and any recovery snapshot is
                // stale. Clearing both keeps the next autosave from
                // re-writing what we just saved, and stops a restore
                // prompt from offering to undo a manual Save.
                self.last_saved_hash = Some(self.project.render_hash());
                self.recovery_slot_path = Some(path.clone());
                caprust_core::recovery::clear_snapshots(&path);
                tracing::info!("Saved project to {}", path.display());
            }
            Err(e) => {
                tracing::error!("Save failed: {e}");
                self.toast_error(format!("{}: {e}", tr("toast-save-failed")));
            }
        }
    }

    /// Periodic crash-recovery snapshot. Called once per frame from
    /// `update`; does real work only on the configured interval.
    ///
    /// Guards, in order:
    ///   * autosave disabled in Settings, or the project has never been
    ///     saved to disk (no path, so there is nothing to recover
    ///     * the project has no unsaved changes since the last Save
    ///   * the interval has not elapsed
    ///   * nothing changed since the previous autosave (idle session)
    ///
    /// The interval check comes before the change check so an idle
    /// session does not pay for a hash comparison every frame.
    fn tick_autosave(&mut self) {
        let Some(interval_min) = self.settings.autosave_minutes.filter(|m| *m > 0) else {
            return;
        };
        // Same slot the offer will be read from. Falling back to
        // `project_file_path` would reintroduce a write/read mismatch.
        let Some(path) = self.recovery_slot_path.clone() else {
            return;
        };
        let now = std::time::Instant::now();
        if let Some(last) = self.last_autosave_at {
            if now.duration_since(last) < std::time::Duration::from_secs(interval_min * 60) {
                return;
            }
        }
        self.last_autosave_at = Some(now);

        // Dirty = current state differs from what is on disk.
        let hash = self.project.render_hash();
        if Some(hash) == self.last_saved_hash {
            return;
        }
        // Skip when the previous autosave captured the same state, so
        // an open-but-idle project does not rewrite an identical file.
        if Some(hash) == self.last_autosave_hash {
            return;
        }

        let saved_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        match caprust_core::recovery::write_snapshot(&path, &self.project, saved_at) {
            Ok(written) => {
                self.last_autosave_hash = Some(hash);
                tracing::info!("recovery autosave: {}", written.display());
            }
            Err(e) => {
                // Autosave failing must not interrupt editing; log it
                // and wait for the next tick.
                tracing::error!("recovery autosave failed: {e:#}");
            }
        }
    }

    /// Set `recovery_prompt` when `project_file` has a crash-recovery
    /// snapshot worth offering. Silent when autosave recovery is off
    /// in Settings, or when there is nothing to restore.
    /// Handle whatever arrived on the remote-control socket this frame.
    ///
    /// Runs on the UI thread on purpose: the tools mutate the open project
    /// and push onto the same undo stack the window uses, so a split made
    /// from the phone lands in the timeline in front of the user and Ctrl+Z
    /// takes it back. A separate process could not do that without both
    /// copies overwriting each other on save.
    /// Carry out one transport command from a remote client.
    ///
    /// A seek is clamped to the project duration. Parking the playhead
    /// past the end decodes nothing, so the phone would show a blank frame
    /// with no way to tell a seek worked from one that did nothing.
    fn apply_remote_transport(&mut self, action: &str, at_ms: Option<u64>) {
        let total = self.total_duration_ms();
        match action {
            "play" => self.preview.playing = true,
            "pause" => self.preview.playing = false,
            "toggle" => self.preview.playing = !self.preview.playing,
            "seek" | "seek_back" | "seek_fwd" => {
                // Relative seeks arrive already resolved to an absolute
                // position by the tool layer, which is the only place that
                // can read the playhead without borrowing it here.
                if let Some(at) = at_ms {
                    self.playhead_ms = at.min(total);
                    // The cached frame belongs to the old position. Keeping
                    // it shows the previous moment until the next decode
                    // lands, which reads as the seek not working.
                    self.preview_player.cancel_pending();
                }
            }
            other => tracing::warn!("remote: unknown transport action {other:?}"),
        }
        tracing::info!(
            "remote: transport {action} -> playing={} playhead={}",
            self.preview.playing,
            self.playhead_ms
        );
    }

    fn tick_remote_control(&mut self) {
        self.pending_remote_open = None;
        self.pending_remote_export = None;
        // Reconcile the listener with the setting first. Done here rather
        // than on the toggle click so a settings change that arrives any
        // other way (settings file, sync, undo) still takes effect.
        // Loopback is only reachable from this machine. A phone on the same
        // network needs every interface, which is what puts the port on the
        // LAN -- which is exactly what the pairing code is for. Restart the
        // listener when this flips; a bound socket cannot change address.
        let want_bind: &str = if self.settings.remote_control_lan {
            "0.0.0.0"
        } else {
            "127.0.0.1"
        };
        let want = self.settings.remote_control_enabled;

        // Adopt the persisted token so a restart does not force the
        // user to pair again. `set_token` refuses to clobber a token
        // that is already live, so this is safe every frame.
        self.mcp_server
            .load_token(self.settings.remote_control_token.clone());

        match (&self.remote_bridge, want, want_bind) {
            (Some(_), true, bound) if Some(bound) != self.remote_bind.as_deref() => {
                // The address changed under a live listener. Drop it so the
                // next match arm re-binds.
                tracing::info!("remote control: bind changed to {bound}, restarting");
                self.remote_bridge = None;
                self.remote_bind = Some(bound.to_string());
            }
            (None, true, _) => match caprust_mcp_core::remote::RemoteBridge::spawn(
                want_bind,
                self.settings.remote_control_port,
            ) {
                Ok(bridge) => {
                    let port = bridge.port();
                    self.settings.remote_control_port = port;
                    self.remote_bind = Some(want_bind.to_string());
                    tracing::info!("remote control: listening on {want_bind}:{port}");
                    self.toast(tr("toast-remote-on"));
                    self.remote_bridge = Some(bridge);
                }
                Err(e) => {
                    // Port taken is the common case. Say so instead of
                    // failing silently every frame.
                    tracing::error!("remote control: could not start: {e:#}");
                    self.toast_error(format!("{}: {e}", tr("toast-remote-failed")));
                    self.settings.remote_control_enabled = false;
                }
            },
            (Some(_), false, _) => {
                // Dropping closes the listener and the port.
                self.remote_bridge = None;
                self.remote_bind = None;
                tracing::info!("remote control: stopped");
            }
            _ => {}
        }

        let Some(bridge) = self.remote_bridge.as_ref() else {
            self.remote_last_batch = 0;
            return;
        };
        if !bridge.status().connected() {
            self.remote_last_batch = 0;
            return;
        }

        // Clones taken once, outside the closure, so the borrows the
        // closure needs do not conflict with `self.mcp_server`.
        let project_path = self.project.project_path.clone().unwrap_or_default();
        let path = std::path::PathBuf::from(&project_path);

        let Self {
            mcp_server,
            project,
            undo_stack,
            recent,
            pending_remote_open,
            pending_remote_export,
            remote_transport,
            remote_export,
            preview_player,
            export_progress,
            ..
        } = self;
        let _ = project_path;

        // The trait object borrows several separate fields while
        // `project` and `undo_stack` are also borrowed. Different
        // fields, so the borrows do not conflict.
        let mut session = RemoteSession {
            recent,
            pending: pending_remote_open,
            player: preview_player,
            export_dest: pending_remote_export,
            export_progress: *export_progress,
            transport: remote_transport,
            export: remote_export,
        };

        let handled = bridge.drain(
            crate::panels::settings_dialog::REMOTE_MAX_PER_FRAME,
            |line| {
                let mut ctx = caprust_mcp_core::server::Ctx {
                    project,
                    undo: undo_stack,
                    project_path: &path,
                    session: Some(&mut session),
                };
                mcp_server.handle_line(line, &mut ctx)
            },
        );
        self.remote_last_batch = handled;

        // Deferred for the same reason as the project switch: both
        // replace state the closure was holding a `&mut` to.
        if let Some(target) = self.pending_remote_open.take() {
            tracing::info!("remote control: opening project {target}");
            self.load_project_from(&target);
        }
        // Export settings changed remotely, validated against the real enums
        // before they reach the export window. Only when something was
        // actually changed -- see ExportSettingsMirror::dirty.
        if self.remote_export.dirty {
            if let Err(e) = apply_export_mirror(&mut self.export_state, &self.remote_export) {
                tracing::warn!("remote control: bad export setting: {e}");
            }
            self.remote_export.dirty = false;
        }

        // A transport command, applied now the borrow has ended.
        if let Some((action, at_ms)) = self.remote_transport.action.take() {
            self.apply_remote_transport(&action, at_ms);
        }
        // The playhead moves during playback, so the cached value has
        // to follow it or the phone's readout drifts within a second
        // of starting.
        self.remote_transport.playing = self.preview.playing;
        self.remote_transport.at_ms = self.playhead_ms;

        if let Some(dest) = self.pending_remote_export.take() {
            tracing::info!("remote control: starting export to {dest}");
            self.export_state.destination = dest.clone();
            self.start_export();
            // The window is how progress is reported; a remote start
            // with nothing on screen would leave the phone polling a
            // number that never moves without explanation.
            self.export_open = true;
            self.toast(tr("toast-export-started"));
        }

        // A successful `auth` mints a new token inside the server.
        // Persist it here, otherwise the phone would be paired only
        // until the next restart.
        let live_token = self.mcp_server.token().map(str::to_string);
        if live_token.is_some() && live_token != self.settings.remote_control_token {
            tracing::info!("remote control: device paired, token stored");
            self.settings.remote_control_token = live_token;
            self.settings_status.remote_pair_code = None;
            self.toast(tr("toast-remote-paired"));
        }

        // Stop showing a code that has expired, rather than leaving a
        // dead one on screen.
        if self.settings_status.remote_pair_code.is_some()
            && self.mcp_server.auth_mut().active_code().is_none()
        {
            self.settings_status.remote_pair_code = None;
        }

        if handled == 0 {
            return;
        }

        // A remote tool can delete a clip. The UI prunes its selection at
        // every delete site it owns; a remote delete bypasses all of them,
        // which would leave Clip Properties indexing a clip that is gone.
        let live: std::collections::HashSet<uuid::Uuid> =
            self.project.clips.iter().map(|c| c.id).collect();
        let before = self.selected_clips.len();
        self.selected_clips.retain(|id| live.contains(id));
        if self.selected_clips.len() != before {
            tracing::info!(
                "remote: pruned {} selected clip id(s) that no longer exist",
                before - self.selected_clips.len()
            );
        }
    }

    /// Mint a pairing code for the remote-control token. The code is shown
    /// in Settings and expires on its own; the token it turns into is what
    /// gets stored.
    fn start_remote_pairing(&mut self) {
        let code = self.mcp_server.auth_mut().begin_pairing();
        self.settings_status.remote_pair_code = Some(code);
        tracing::info!("remote control: pairing code issued");
    }

    /// Forget the token and end every live remote session. This is the
    /// "something is wrong" button, so it has to be immediate and it must
    /// not wait for the socket to drain.
    fn revoke_remote_access(&mut self) {
        self.mcp_server.auth_mut().revoke_all();
        self.settings.remote_control_token = None;
        self.settings_status.remote_pair_code = None;
        // Dropping the bridge closes the port too, so a paired device
        // cannot reconnect with a token it remembers.
        self.remote_bridge = None;
        self.settings.remote_control_enabled = false;
        self.toast(tr("toast-remote-revoked"));
        tracing::info!("remote control: access revoked, listener closed");
    }

    fn offer_recovery(&mut self, project_file: &std::path::Path, loaded_hash: u64) {
        if !self.settings.autosave_recovery {
            return;
        }
        match caprust_core::recovery::offer_for(project_file, loaded_hash) {
            Some(offer) => {
                tracing::info!(
                    "recovery: snapshot from {} is newer than {}",
                    offer.saved_at,
                    project_file.display()
                );
                self.recovery_prompt = Some((project_file.to_path_buf(), offer));
            }
            None => self.recovery_prompt = None,
        }
    }

    /// Restore / Discard / Later dialog for a crash-recovery snapshot.
    /// Mirrors the settings-sync prompt: a single modal driven by
    /// `recovery_prompt`, closed by whichever button is clicked.
    fn show_recovery_prompt_window(&mut self, ctx: &egui::Context) {
        let Some((path, offer)) = self.recovery_prompt.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new(tr("recovery-prompt-title"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(tr("recovery-prompt-body"));
                ui.add_space(space::S);
                let when = format_clock(offer.saved_at);
                ui.label(
                    egui::RichText::new(format!("{}: {when}", tr("recovery-prompt-saved")))
                        .small()
                        .monospace()
                        .color(egui::Color32::from_gray(150)),
                );
                ui.add_space(space::M_PLUS);
                ui.horizontal(|ui| {
                    if ui.button(tr("recovery-prompt-restore")).clicked() {
                        match caprust_core::recovery::restore(&path) {
                            Some(mut state) => {
                                // Keep project_path pointing at the file
                                // we were loaded from so the next autosave
                                // and Save go to the same place.
                                state.project_path =
                                    Some(self.project.project_path.clone().unwrap_or_default());
                                state.models.merge_missing_defaults();
                                state
                                    .models
                                    .scan_local(&self.settings.effective_models_dir());
                                self.project = state;
                                self.undo_stack = UndoStack::new();
                                // Restored state is on disk only in the
                                // snapshot, so the project is dirty and a
                                // fresh snapshot should be allowed to
                                // overwrite the one we just read.
                                let hash = self.project.render_hash();
                                self.last_saved_hash = None;
                                self.last_autosave_hash = None;
                                self.last_autosave_at = None;
                                // Clear the prompt before rescanning, or
                                // the scan defers itself behind a prompt
                                // that no longer exists.
                                self.recovery_prompt = None;
                                // The restored session is a different set
                                // of clips pointing at a different set of
                                // media, so the missing-file scan from
                                // load time describes a project that is no
                                // longer on screen.
                                self.check_missing_media_on_load();
                                tracing::info!("recovery: restored session, hash {hash}");
                                self.toast(tr("toast-recovery-restored"));
                            }
                            None => self.toast_error(tr("toast-recovery-failed")),
                        }
                        self.recovery_prompt = None;
                    }
                    if ui.button(tr("recovery-prompt-later")).clicked() {
                        // Keep the snapshot; it will be offered again
                        // on the next open of this project.
                        self.recovery_prompt = None;
                    }
                    if ui.button(tr("recovery-prompt-discard")).clicked() {
                        caprust_core::recovery::clear_snapshots(&path);
                        self.recovery_prompt = None;
                        tracing::info!("recovery: snapshots discarded for {}", path.display());
                    }
                });
            });

        // Closing the window with X means "Later", not "Discard": the
        // snapshot stays so no unsaved work is thrown away by a stray
        // click.
        if !open {
            self.recovery_prompt = None;
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
                // Rescan: a project saved by an older build carries
                // stale statuses, and merged-in entries start as
                // NotDownloaded even when the file is on disk.
                let models_dir = self.settings.effective_models_dir();
                state.models.scan_local(&models_dir);
                self.project = state;
                self.undo_stack = UndoStack::new();
                self.mode = AppMode::Editor;
                let entry = caprust_core::recent::entry_from(&self.project, path);
                self.recent.push(entry);
                tracing::info!("Loaded project {path}");
                // The file on disk is now the baseline: no unsaved
                // changes, and the autosave timer restarts for this
                // project.
                let loaded_hash = self.project.render_hash();
                self.last_saved_hash = Some(loaded_hash);
                self.last_autosave_hash = Some(loaded_hash);
                self.last_autosave_at = None;
                // Snapshots for this file are read from and written
                // to this exact path, so the autosave slot and the
                // offer slot cannot drift apart.
                self.recovery_slot_path = Some(std::path::PathBuf::from(path));
                // Offer to restore a newer crash-recovery snapshot, if
                // one exists and actually differs from this file.
                self.offer_recovery(std::path::Path::new(path), loaded_hash);
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
    /// Persistent highlight for an open top-level menu (File/Edit/View).
    /// egui's own open-state fill never fires: `is_menu_open` compares the
    /// bar id against a different id scheme than the button response, so
    /// we paint it ourselves on the background layer (below the label).
    /// Gray fill + accent underline, mirroring toolbar toggles.
    fn paint_menu_open(&self, ui: &egui::Ui, rect: egui::Rect) {
        let bg = ui.ctx().layer_painter(egui::LayerId::background());
        bg.rect_filled(
            rect.expand2(egui::vec2(4.0, 2.0)),
            6.0,
            egui::Color32::from_gray(45),
        );
        bg.line_segment(
            [
                egui::pos2(rect.left() + 4.0, rect.bottom() - 1.0),
                egui::pos2(rect.right() - 4.0, rect.bottom() - 1.0),
            ],
            egui::Stroke::new(2.0, self.theme.accent_color()),
        );
    }

    /// Format presets as cards, each showing the real proportions.
    ///
    /// Was a dropdown whose only clue was a tooltip on hover. Choosing
    /// 9:16 over 16:9 is the single most consequential decision in
    /// this dialog and the least visible one: the names are nearly
    /// interchangeable and nothing on screen shows the difference until
    /// the timeline is built. A little rectangle in the right
    /// proportions answers it before the question is asked.
    fn format_cards(&mut self, ui: &mut egui::Ui) {
        let presets = AspectRatio::presets();
        let per_row = if ui.available_width() > 520.0 { 3 } else { 2 };

        egui::Grid::new("new_format_cards")
            .num_columns(per_row)
            .spacing([space::M, space::M])
            .show(ui, |ui| {
                for (i, preset) in presets.iter().enumerate() {
                    let selected = self.draft.aspect_ratio == *preset;
                    let clicked = format_card(ui, preset, selected);
                    if clicked {
                        self.draft.aspect_ratio = preset.clone();
                    }
                    if (i + 1) % per_row == 0 {
                        ui.end_row();
                    }
                }
            });
    }

    fn show_new_project_modal(&mut self, ctx: &egui::Context) {
        if !self.new_project_modal_open {
            return;
        }
        let mut open = self.new_project_modal_open;
        let mut created = false;

        egui::Window::new(tr("new-title"))
            .id(egui::Id::new("new_project_modal"))
            .collapsible(false)
            .resizable(true)
            .default_width(1120.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_min_width(880.0);
                // Two columns laid out by hand.
                //
                // A SidePanel here was the obvious way to do it and was
                // the wrong one: it sized itself against the parent and
                // painted over the third column of format cards,
                // swallowing the clicks. Only one of the six was
                // reachable, and it happened to be the default. A plain
                // horizontal split cannot overlap anything.
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.add_space(space::M);
                        ui.label(tr("new-pick-format"));
                        ui.label(
                            egui::RichText::new(tr("new-pick-format-hint"))
                                .small()
                                .color(egui::Color32::from_gray(150)),
                        );
                        ui.add_space(space::S);
                        self.format_cards(ui);
                    });
                    ui.add_space(space::L);
                    ui.vertical(|ui| {
                        // Wide enough for a 21:9 frame to show at a
                        // size where the safe-area guides are actually
                        // readable. The preview is the reason to look at
                        // this panel at all; squeezing it to a column
                        // made the guides decorative.
                        ui.set_width(420.0);
                        new_project_preview(ui, &self.draft);
                    });
                });

                ui.add_space(space::XL);
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
                // Both buttons sized by hand. Create is the action the
                // whole dialog exists for, and it was the same height as
                // Cancel -- which gave the one thing worth pressing no
                // more presence than the way out.
                ui.horizontal(|ui| {
                    let can_create = !self.draft.name.trim().is_empty()
                        && !self.draft.location.trim().is_empty();
                    let mut create = egui::Button::new(tr("new-button-create"))
                        .min_size(egui::vec2(180.0, 44.0));
                    if can_create {
                        create = create.fill(ui.visuals().selection.bg_fill);
                    }
                    let resp = if can_create {
                        ui.add(create)
                    } else {
                        // The hint belongs on the response, not on the
                        // button: a disabled widget never emits hover
                        // events of its own.
                        ui.add_enabled(false, create)
                            .on_disabled_hover_text(tr("new-button-create-hint"))
                    };
                    if resp.clicked() {
                        self.create_project();
                        created = true;
                    }
                    if ui
                        .add_sized([120.0, 44.0], egui::Button::new(tr("menu-file-quit")))
                        .clicked()
                    {
                        created = true;
                    }
                });
            });

        // Apply open/close after the borrow of `self` inside `.show()`
        // has ended (DIRECTIVES section 5, egui::Window::open trap).
        self.new_project_modal_open = open && !created;
    }

    /// Importers installed under `%APPDATA%/CapRust/importers`.
    /// Cached after the first scan; the menu is rebuilt every frame, so
    /// rescanning here would stat the directory 60 times a second.
    fn available_importers(&mut self) -> Vec<caprust_core::importer::ImporterPlugin> {
        if self.importer_cache.is_none() {
            let dir = self.importer_dir();
            let found = caprust_core::importer::discover(&dir);
            tracing::info!("importers: found {} in {}", found.len(), dir.display());
            self.importer_cache = Some(found);
        }
        self.importer_cache.clone().unwrap_or_default()
    }

    fn importer_dir(&self) -> std::path::PathBuf {
        std::env::var_os("APPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_default()
            .join("CapRust")
            .join("importers")
    }

    /// Ask the user for the thing this plugin reads, then run it.
    ///
    /// The plugin says whether that is a file or a folder. Matching on the
    /// kind instead meant OTIO -- a single JSON document -- opened a folder
    /// picker, and any new plugin had to be added to a hardcoded list in
    /// the host to get the right dialog.
    fn pick_and_run_importer(&mut self, plugin: &caprust_core::importer::ImporterPlugin) {
        let picked = match plugin.manifest.target {
            caprust_core::importer::ImportTarget::Folder => rfd::FileDialog::new()
                .pick_folder()
                .map(|p| p.to_string_lossy().to_string()),
            caprust_core::importer::ImportTarget::File => rfd::FileDialog::new()
                .pick_file()
                .map(|p| p.to_string_lossy().to_string()),
        };
        if let Some(path) = picked {
            self.run_importer(plugin, &path);
        }
    }

    /// Load a plugin from wherever the user points, and import through it.
    ///
    /// This is the escape hatch for a plugin that is not installed in
    /// `%APPDATA%/CapRust/importers`: one just built, one being tested, one
    /// living somewhere else on disk. Either the manifest or the executable
    /// can be picked -- `load` takes both, since the two name each other and
    /// the manifest is where `target` is written down.
    ///
    /// Nothing is copied into the importers directory and nothing is cached.
    /// The plugins listed in the menu are the installed ones; this one runs
    /// once and is forgotten, which is what makes it safe to point at a
    /// build that is mid-edit.
    fn open_plugin_and_import(&mut self) {
        let Some(picked) = rfd::FileDialog::new()
            .add_filter("Importer plugin", &["json", "exe"])
            .pick_file()
        else {
            return;
        };
        match caprust_core::importer::load(&picked) {
            Ok(plugin) => self.pick_and_run_importer(&plugin),
            Err(e) => self.toast_error(format!("{}: {e}", tr("toast-import-failed"))),
        }
    }

    /// Run one importer and replace the current project with what it
    /// returns. Mirrors the load path: models backfilled, undo reset,
    /// thumbnails regenerated.
    fn run_importer(&mut self, plugin: &caprust_core::importer::ImporterPlugin, path: &str) {
        let src = std::path::Path::new(path);
        match caprust_core::importer::run_import(plugin, src) {
            Ok(mut state) => {
                // A plugin that imports a *folder* (a CapCut project, a
                // Premiere sequence bundle) knows the project belongs in
                // that folder, not above it. Taking `src.parent()`
                // unconditionally put the cache one level too high and
                // thumbnails resolved against the wrong root. Only fill
                // it in when the plugin had nothing to say.
                if state.project_path.is_none() {
                    state.project_path = src.parent().map(|p| p.to_string_lossy().to_string());
                }
                state.models.merge_missing_defaults();
                let models_dir = self.settings.effective_models_dir();
                state.models.scan_local(&models_dir);
                self.undo_stack = UndoStack::new();
                self.project = state;
                self.selected_clips.clear();
                self.mode = AppMode::Editor;
                self.backfill_missing_probes();
                self.regen_missing_thumbnails();
                self.audio_cache = AudioCacheState::default();
                let h = self.project.render_hash();
                self.last_saved_hash = Some(h);
                self.last_autosave_hash = Some(h);
                self.check_missing_media_on_load();
                self.backfill_waveforms();
                self.toast(tr("menu-file-import-done"));
            }
            Err(e) => self.toast_error(format!("{}: {e}", tr("toast-import-failed"))),
        }
    }

    fn show_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                let file_menu = ui.menu_button(tr("menu-file"), |ui| {
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
                    // Process-based importers, under one entry rather
                    // than one per plugin. They are discovered from
                    // %APPDATA%/CapRust/importers, so what appears here
                    // depends on what the user installed -- a build of
                    // CapRust should not advertise importers it does not
                    // ship.
                    //
                    // The submenu is shown even when nothing is installed,
                    // because it also carries *Open plugin*. That entry used
                    // to be missing, and with an empty importers directory
                    // the whole thing disappeared -- the one case where a
                    // user without any plugin installed could not get to
                    // the feature that lets them load one.
                    let importers = self.available_importers();
                    ui.menu_button(tr("menu-file-import-other"), |ui| {
                        let any_installed = !importers.is_empty();
                        for plugin in importers {
                            let label = plugin.manifest.name.clone();
                            if ui.button(label).clicked() {
                                self.pick_and_run_importer(&plugin);
                                ui.close_menu();
                            }
                        }
                        if any_installed {
                            ui.separator();
                        }
                        if ui.button(tr("menu-file-import-plugin")).clicked() {
                            self.open_plugin_and_import();
                            ui.close_menu();
                        }
                    });
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

                let edit_menu = ui.menu_button(tr("menu-edit"), |ui| {
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

                let view_menu = ui.menu_button(tr("menu-view"), |ui| {
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
                    ui.menu_button(tr("menu-view-tab-layout"), |ui| {
                        for l in crate::panels::media_bin::TabLayout::all() {
                            let sel = self.media_bin.tab_layout == l;
                            if ui.selectable_label(sel, tr(l.ftl_key())).clicked() {
                                self.media_bin.tab_layout = l;
                                self.tab_layout = l; // Keep both in sync
                                ui.close_menu();
                            }
                        }
                    });
                    // The editor itself. Two of them, chosen here and
                    // in Settings -> Appearance -> Layout.
                    ui.menu_button(tr("menu-view-layout"), |ui| {
                        for m in [
                            caprust_core::LayoutMode::Default,
                            caprust_core::LayoutMode::Modern,
                        ] {
                            let sel = self.settings.layout == m;
                            if ui
                                .selectable_label(sel, tr(layout_mode_key(m)))
                                .on_hover_text(tr(layout_mode_hint_key(m)))
                                .clicked()
                            {
                                self.set_layout_mode(m);
                                ui.close_menu();
                            }
                        }
                    });

                    // Modern editor zone management submenu
                    if self.settings.layout == caprust_core::LayoutMode::Modern {
                        ui.menu_button(tr("menu-view-modern-zones"), |ui| {
                            // Visibility first. The timeline size preset
                            // hides a sidebar at M and L, and without this
                            // the left column was simply gone with nothing
                            // to bring it back.
                            // The panel id carries these flags, so a sidebar
                            // coming back starts at the preset width rather
                            // than its last dragged one.
                            ui.checkbox(&mut self.modern_left_on, tr("menu-zone-side-left"));
                            ui.checkbox(&mut self.modern_right_on, tr("menu-zone-side-right"));
                            ui.separator();
                            // Add container, then which side. Both menus go
                            // through `Zone::set`, which inserts at the front
                            // and caps the stack at three -- pushing onto the
                            // `Vec` directly would leave the new card
                            // invisible behind the existing ones, which is why
                            // this menu appeared to do nothing.
                            ui.menu_button(tr("menu-view-modern-add"), |ui| {
                                ui.menu_button(tr("menu-zone-side-left"), |ui| {
                                    for (zone, label) in [
                                        (Zone::LeftTop, "menu-zone-add-left-top"),
                                        (Zone::LeftBottom, "menu-zone-add-left-bottom"),
                                    ] {
                                        if ui.button(tr(label)).clicked() {
                                            zone.add_next(&mut self.modern);
                                            ui.close_menu();
                                        }
                                    }
                                });
                                ui.menu_button(tr("menu-zone-side-right"), |ui| {
                                    for (zone, label) in [
                                        (Zone::RightTop, "menu-zone-add-right-top"),
                                        (Zone::RightBottom, "menu-zone-add-right-bottom"),
                                    ] {
                                        if ui.button(tr(label)).clicked() {
                                            zone.add_next(&mut self.modern);
                                            ui.close_menu();
                                        }
                                    }
                                });
                            });
                            ui.separator();
                            ui.menu_button(tr("menu-view-modern-remove"), |ui| {
                                for (zone, label) in [
                                    (Zone::LeftTop, "menu-zone-remove-left-top"),
                                    (Zone::LeftBottom, "menu-zone-remove-left-bottom"),
                                    (Zone::RightTop, "menu-zone-remove-right-top"),
                                    (Zone::RightBottom, "menu-zone-remove-right-bottom"),
                                ] {
                                    if ui.button(tr(label)).clicked() {
                                        zone.clear_zone(&mut self.modern);
                                        ui.close_menu();
                                    }
                                }
                            });
                        });
                    }

                    // Arrangements *within* the dock. Only meaningful in
                    // Default; Modern has fixed panels, and a menu full
                    // of choices that do nothing reads as broken rather
                    // than as unavailable, so it is hidden instead.
                    if self.settings.layout == caprust_core::LayoutMode::Default {
                        ui.menu_button(tr("menu-view-dock-presets"), |ui| {
                            for p in crate::dock::LayoutPreset::all() {
                                let sel = self.settings.layout_preset.as_deref() == Some(p.id());
                                if ui.selectable_label(sel, tr(p.label_key())).clicked() {
                                    self.apply_layout_preset(p);
                                    ui.close_menu();
                                }
                            }
                            ui.separator();
                            let following_saved = self.settings.layout_preset.is_none();
                            if ui
                                .selectable_label(
                                    following_saved,
                                    tr("menu-view-layout-my-arrangement"),
                                )
                                .clicked()
                            {
                                self.reset_layout();
                                ui.close_menu();
                            }
                        });
                    }
                });

                if file_menu.inner.is_some() {
                    self.paint_menu_open(ui, file_menu.response.rect);
                }
                if edit_menu.inner.is_some() {
                    self.paint_menu_open(ui, edit_menu.response.rect);
                }
                if view_menu.inner.is_some() {
                    self.paint_menu_open(ui, view_menu.response.rect);
                }
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

        // Beat markers from analysed audio clips. Same timeline mapping
        // as the ruler markers; speed-ramped clips contribute nothing.
        for clip in &self.project.clips {
            let mid = match clip.media_id {
                Some(id) => id,
                None => continue,
            };
            let entry = match self.project.beat_grids.get(&mid) {
                Some(e) => e,
                None => continue,
            };
            for t in caprust_core::beat::timeline_beats(
                clip.start_time_ms,
                clip.duration_ms,
                clip.source_offset_ms,
                clip.speed,
                &entry.beats_ms,
            ) {
                let target = t as i64;
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

    /// Split every video clip at this audio clip's beat markers.
    /// One MacroCommand so Ctrl+Z is a single step. Beats within
    /// 150 ms of a clip edge are skipped (they would leave slivers).
    /// Toasts and no-ops when the clip has no analysed grid yet.
    fn auto_cut_to_beats(&mut self, clip_id: uuid::Uuid) {
        let Some(clip) = self.project.clips.iter().find(|c| c.id == clip_id) else {
            return;
        };
        let mid = match clip.media_id {
            Some(id) => id,
            None => {
                self.toast(tr("toast-autocut-no-beats"));
                return;
            }
        };
        let beats: Vec<u64> = match self.project.beat_grids.get(&mid) {
            Some(e) => caprust_core::beat::timeline_beats(
                clip.start_time_ms,
                clip.duration_ms,
                clip.source_offset_ms,
                clip.speed,
                &e.beats_ms,
            ),
            None => {
                self.toast(tr("toast-autocut-no-beats"));
                return;
            }
        };
        if beats.is_empty() {
            self.toast(tr("toast-autocut-no-beats"));
            return;
        }
        const EDGE_MARGIN_MS: u64 = 150;
        let mut cmds: Vec<Box<dyn caprust_core::commands::Command>> = Vec::new();
        // Ascending beats: earlier splits only subdivide, so later beat
        // times stay valid against the evolving clip list (each split
        // is looked up by clip id at execute time).
        let mut sorted = beats;
        sorted.sort_unstable();
        for t in sorted {
            // Snapshot the current video clips spanning t. Re-query per
            // beat because previous splits changed the list.
            let targets: Vec<uuid::Uuid> = self
                .project
                .clips
                .iter()
                .filter(|c| {
                    matches!(
                        c.clip_type,
                        caprust_core::clip::ClipType::Video { .. }
                            | caprust_core::clip::ClipType::Image { .. }
                    )
                })
                .filter(|c| {
                    let end = c.start_time_ms.saturating_add(c.duration_ms);
                    t > c.start_time_ms.saturating_add(EDGE_MARGIN_MS) && t + EDGE_MARGIN_MS < end
                })
                .map(|c| c.id)
                .collect();
            for id in targets {
                cmds.push(Box::new(SplitClipCommand::new(id, t)));
            }
        }
        if cmds.is_empty() {
            self.toast(tr("toast-autocut-nothing"));
            return;
        }
        let n = cmds.len();
        let cmd = caprust_core::commands::macro_command::MacroCommand::new(
            format!("auto-cut {n} splits"),
            cmds,
        );
        let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
        self.toast(format!("{}: {n}", tr("toast-autocut-done")));
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

    fn drain_anon_job(&mut self) {
        let Some(rx) = self.anon_rx.take() else {
            return;
        };
        let mut still_live = true;
        loop {
            match rx.try_recv() {
                Ok(crate::media_jobs::AnonEvent::Started { total_frames }) => {
                    tracing::info!("anon: started, {total_frames} frames to process");
                    if let Some(id) = self.anon_job_id {
                        self.update_job_progress(id, BackgroundJob::INDETERMINATE);
                    }
                }
                Ok(crate::media_jobs::AnonEvent::Progress { done, total }) => {
                    let p = if total == 0 {
                        0.0
                    } else {
                        done as f32 / total as f32
                    };
                    if let Some(id) = self.anon_job_id {
                        self.update_job_progress(id, p);
                    }
                }
                Ok(crate::media_jobs::AnonEvent::Finished {
                    clip_id,
                    mask_path,
                    frame_count,
                }) => {
                    let rel = self.anon_rel_path.take();
                    let stored = match rel {
                        Some(r) => Some(r),
                        None => {
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
                            .anon_mask(Some(rel));
                        if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                            tracing::error!("anon: apply failed: {e}");
                            self.toast_error(format!("{}: {e}", tr("toast-anon-failed")));
                        } else {
                            tracing::info!(
                                "anon: clip {} mask applied ({frame_count} frames)",
                                clip_id
                            );
                            self.toast(tr("toast-anon-done"));
                        }
                    } else {
                        tracing::warn!("anon: finished but no relative path available");
                        self.toast_error(tr("toast-anon-failed"));
                    }
                    if let Some(id) = self.anon_job_id.take() {
                        self.finish_job(id);
                    }
                    still_live = false;
                }
                Ok(crate::media_jobs::AnonEvent::Failed(msg)) => {
                    tracing::error!("anon: job failed: {msg}");
                    self.toast_error(format!("{}: {msg}", tr("toast-anon-failed")));
                    if let Some(id) = self.anon_job_id.take() {
                        self.finish_job(id);
                    }
                    self.anon_rel_path = None;
                    still_live = false;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    tracing::warn!("anon: receiver disconnected unexpectedly");
                    if let Some(id) = self.anon_job_id.take() {
                        self.finish_job(id);
                    }
                    self.anon_rel_path = None;
                    still_live = false;
                    break;
                }
            }
        }
        if still_live {
            self.anon_rx = Some(rx);
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

    /// Kick off a face-anonymize job for the given clip. Requires a
    /// saved project (mask cache), the YuNet model on disk, and
    /// ffmpeg + ffprobe. Video clips only, 1.0x non-reversed (the
    /// mask is sampled at the clip's own frame rate and must align
    /// 1:1). Reuses a cached mask when one exists.
    fn start_anon_job(&mut self, clip_id: uuid::Uuid) {
        if self.anon_rx.is_some() {
            self.toast(tr("toast-anon-busy"));
            return;
        }
        let Some(project_path) = self.project.project_path.clone() else {
            self.toast(tr("toast-anon-needs-project"));
            return;
        };
        let Some(clip) = self.project.clips.iter().find(|c| c.id == clip_id) else {
            return;
        };
        let source_path = match &clip.clip_type {
            caprust_core::ClipType::Video { path, .. } => std::path::PathBuf::from(path),
            _ => {
                self.toast(tr("toast-anon-needs-video"));
                return;
            }
        };
        if clip.reversed || (clip.speed - 1.0).abs() > 0.001 || clip.speed_end.is_some() {
            self.toast(tr("toast-anon-unsupported"));
            return;
        }
        let duration_ms = clip.duration_ms;
        if duration_ms == 0 {
            self.toast(tr("toast-anon-needs-video"));
            return;
        }
        let t_start_ms = clip.source_offset_ms;

        // Model: prefer SCRFD (working decoder, same preference as
        // auto-reframe), fall back to YuNet (its 2022mar decode is a
        // stub, so a YuNet-only setup yields empty masks).
        self.project
            .models
            .scan_local(&self.settings.effective_models_dir());
        let models_dir = self.settings.effective_models_dir();
        let find_model = |kind: caprust_core::ModelKind| {
            self.project
                .models
                .models
                .iter()
                .find(|m| m.kind == kind)
                .map(|m| {
                    caprust_core::models::ModelRegistry::local_path_static(
                        &self.project.models.models,
                        &models_dir,
                        &m.id,
                    )
                })
                .filter(|p| p.is_file())
        };
        let (model_path, use_scrfd) = match (
            find_model(caprust_core::ModelKind::ScrfdDetector),
            find_model(caprust_core::ModelKind::FaceDetector),
        ) {
            (Some(p), _) => (p, true),
            (None, Some(p)) => (p, false),
            (None, None) => {
                self.toast(tr("toast-anon-needs-model"));
                return;
            }
        };

        let Some(ffmpeg) = self.ffmpeg_status.ffmpeg.clone() else {
            self.toast(tr("toast-anon-needs-ffmpeg"));
            return;
        };
        let Some(ffprobe) = self.ffmpeg_status.ffprobe.clone() else {
            self.toast(tr("toast-anon-needs-ffmpeg"));
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

        let rel = format!("cache/anon/{clip_id}.mkv");
        if caprust_core::cache::anon_exists(std::path::Path::new(&project_path), clip_id) {
            let cmd =
                caprust_core::commands::set_clip::SetClipCommand::new(clip_id).anon_mask(Some(rel));
            if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                tracing::error!("anon: apply cached failed: {e}");
                self.toast_error(format!("{}: {e}", tr("toast-anon-failed")));
            } else {
                self.toast(tr("toast-anon-done"));
            }
            return;
        }
        let abs = caprust_core::cache::anon_path(std::path::Path::new(&project_path), clip_id);

        let req = crate::media_jobs::AnonRequest {
            clip_id,
            source_path,
            t_start_ms,
            duration_ms,
            fps,
            model_path,
            use_scrfd,
            mask_output: abs,
            max_side: 480,
        };
        let rx = crate::media_jobs::spawn_anon_job(
            std::path::PathBuf::from(ffmpeg),
            std::path::PathBuf::from(ffprobe),
            req,
        );
        let job_id = self.begin_job(JobKind::Anon, tr("job-anon"));
        self.anon_job_id = Some(job_id);
        self.anon_rx = Some(rx);
        self.anon_rel_path = Some(rel);
        self.toast(tr("toast-anon-started"));
    }

    /// Kick off a stabilization detect job (vidstab pass 1) for the
    /// given clip. Requires ffmpeg with the vidstab filters and a
    /// saved project (so the transforms cache has somewhere to live).
    /// When a transforms file is already cached, it is applied
    /// directly with no job. Only plain Video clips at 1.0x speed:
    /// reversed or re-timed clips would measure the wrong motion.
    fn start_stab_job(&mut self, clip_id: uuid::Uuid) {
        if self.stab_rx.is_some() {
            self.toast(tr("toast-stab-busy"));
            return;
        }
        let Some(project_path) = self.project.project_path.clone() else {
            self.toast(tr("toast-stab-needs-project"));
            return;
        };
        let Some(clip) = self.project.clips.iter().find(|c| c.id == clip_id) else {
            return;
        };
        let source_path = match &clip.clip_type {
            caprust_core::ClipType::Video { path, .. } => std::path::PathBuf::from(path),
            _ => {
                self.toast(tr("toast-stab-needs-video"));
                return;
            }
        };
        if clip.reversed || (clip.speed - 1.0).abs() > 0.001 || clip.speed_end.is_some() {
            self.toast(tr("toast-stab-unsupported"));
            return;
        }
        let duration_ms = clip.duration_ms;
        if duration_ms == 0 {
            self.toast(tr("toast-stab-needs-video"));
            return;
        }
        // Speed is 1.0 here (checked above), so the timeline duration
        // equals the source window length.
        let t_start_ms = clip.source_offset_ms;
        let Some(ffmpeg) = self.ffmpeg_status.ffmpeg.clone() else {
            self.toast(tr("toast-stab-needs-ffmpeg"));
            return;
        };

        let rel = format!("cache/stab/{clip_id}.trf");
        let abs = caprust_core::cache::stab_path(std::path::Path::new(&project_path), clip_id);
        // Cached transforms from an earlier run apply instantly.
        if caprust_core::cache::stab_exists(std::path::Path::new(&project_path), clip_id) {
            let cmd =
                caprust_core::commands::set_clip::SetClipCommand::new(clip_id).stab_trf(Some(rel));
            if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                tracing::error!("stab: apply cached failed: {e}");
                self.toast_error(format!("{}: {e}", tr("toast-stab-failed")));
            } else {
                tracing::info!("stab: clip {clip_id} transforms applied from cache");
                self.toast(tr("toast-stab-done"));
            }
            return;
        }

        let fps = {
            let fr = &self.project.frame_rate;
            if fr.den == 0 {
                30.0
            } else {
                fr.num as f64 / fr.den as f64
            }
        };
        let req = crate::media_jobs::StabRequest {
            clip_id,
            source_path,
            t_start_ms,
            duration_ms,
            fps,
            trf_output: abs,
        };
        let rx = crate::media_jobs::spawn_stab_job(std::path::PathBuf::from(ffmpeg), req);
        let job_id = self.begin_job(JobKind::Stab, tr("job-stab"));
        self.stab_job_id = Some(job_id);
        self.stab_rx = Some(rx);
        // Remember the relative path so drain can store it in the clip
        // without recomputing. Keyed on the job id because at most one
        // job runs at a time and clip_id is recoverable from Finished.
        self.stab_rel_path = Some(rel);
        self.toast(tr("toast-stab-started"));
        tracing::info!("stab: job spawned for clip {clip_id}");
    }

    /// Poll the in-flight stab detect job. On success, write the
    /// transforms path into the clip through SetClipCommand (undoable,
    /// and it flips render_hash so the preview respawns stabilized).
    /// On failure, toast and clear state.
    fn drain_stab_job(&mut self) {
        // Take the receiver so we can process every queued event and
        // put it back only while the stream is still live. Same shape
        // as drain_bg_removal_job.
        let Some(rx) = self.stab_rx.take() else {
            return;
        };
        let mut still_live = true;
        loop {
            match rx.try_recv() {
                Ok(crate::media_jobs::StabEvent::Started { total_frames }) => {
                    tracing::info!("stab: started, {total_frames} frames to analyze");
                    if let Some(id) = self.stab_job_id {
                        // Stay indeterminate until the first Progress
                        // event: a 0 percent bar reads as stuck.
                        self.update_job_progress(id, BackgroundJob::INDETERMINATE);
                    }
                }
                Ok(crate::media_jobs::StabEvent::Progress { done, total }) => {
                    let p = if total == 0 {
                        0.0
                    } else {
                        done as f32 / total as f32
                    };
                    if let Some(id) = self.stab_job_id {
                        self.update_job_progress(id, p);
                    }
                }
                Ok(crate::media_jobs::StabEvent::Finished {
                    clip_id,
                    trf_path,
                    frame_count,
                }) => {
                    let rel = self.stab_rel_path.take();
                    let stored = match rel {
                        Some(r) => Some(r),
                        None => {
                            // Fallback: derive from the absolute path
                            // relative to the project dir. Should not
                            // happen, but a missing key would silently
                            // drop the transforms and waste the job.
                            let proj = self.project.project_path.clone();
                            proj.and_then(|p| {
                                trf_path
                                    .strip_prefix(&p)
                                    .ok()
                                    .map(|r| r.to_string_lossy().into_owned())
                            })
                        }
                    };
                    if let Some(rel) = stored {
                        let cmd = caprust_core::commands::set_clip::SetClipCommand::new(clip_id)
                            .stab_trf(Some(rel));
                        if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                            tracing::error!("stab: apply failed: {e}");
                            self.toast_error(format!("{}: {e}", tr("toast-stab-failed")));
                        } else {
                            tracing::info!(
                                "stab: clip {} transforms applied ({frame_count} frames)",
                                clip_id
                            );
                            self.toast(tr("toast-stab-done"));
                        }
                    } else {
                        tracing::warn!("stab: finished but no relative path available");
                        self.toast_error(tr("toast-stab-failed"));
                    }
                    if let Some(id) = self.stab_job_id.take() {
                        self.finish_job(id);
                    }
                    still_live = false;
                }
                Ok(crate::media_jobs::StabEvent::Failed(msg)) => {
                    tracing::error!("stab: job failed: {msg}");
                    self.toast_error(format!("{}: {msg}", tr("toast-stab-failed")));
                    if let Some(id) = self.stab_job_id.take() {
                        self.finish_job(id);
                    }
                    self.stab_rel_path = None;
                    still_live = false;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    tracing::warn!("stab: receiver disconnected unexpectedly");
                    if let Some(id) = self.stab_job_id.take() {
                        self.finish_job(id);
                    }
                    self.stab_rel_path = None;
                    still_live = false;
                    break;
                }
            }
        }
        if still_live {
            self.stab_rx = Some(rx);
        }
    }

    /// Save the paused preview frame as PNG, rendered at full project
    /// resolution through the same filtergraph the preview shows.
    /// Runs ffmpeg on a worker thread; the result is toasted by
    /// drain_save_frame. No-op while playing or without a frame.
    fn save_frame_png(&mut self) {
        if self.preview.playing || !self.preview_player.has_frame {
            self.toast(tr("toast-frame-no-frame"));
            return;
        }
        let Some(ffmpeg) = self.ffmpeg_status.ffmpeg.clone() else {
            self.toast_error(tr("toast-frame-failed"));
            return;
        };
        // Verify ffmpeg exists and is executable before spawning worker.
        if !std::path::Path::new(&ffmpeg).is_file() {
            tracing::error!("save_frame: ffmpeg path is not a file: {}", ffmpeg);
            self.toast_error(format!(
                "{}: ffmpeg not found at path",
                tr("toast-frame-failed")
            ));
            return;
        }
        let playhead = self.playhead_ms;
        let stamp = crate::panels::preview_window::format_ms(playhead).replace([':', '.'], "-");
        let default_name = format!("caprust-frame-{stamp}.png");
        let Some(mut path) = rfd::FileDialog::new()
            .set_title(tr("preview-save-frame"))
            .add_filter("PNG", &["png"])
            .set_directory(self.settings.effective_screenshots_dir())
            .set_file_name(&default_name)
            .save_file()
        else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("png");
        }

        let (pw, ph) = self.project.project_dimensions();
        let (fps_num, fps_den) = self.export_state.frame_rate.fraction(
            self.project.frame_rate.num as i64,
            self.project.frame_rate.den as i64,
        );
        let fps_f = fps_num as f64 / fps_den.max(1) as f64;
        let models_dir = self.settings.effective_models_dir();
        let project = self.project.clone();
        let ffmpeg_path = ffmpeg; // move into closure
        let (tx, rx) = std::sync::mpsc::channel();
        self.save_frame_rx = Some(rx);
        std::thread::Builder::new()
            .name("caprust-save-frame".into())
            .spawn(move || {
                let out: Result<std::path::PathBuf, String> = (|| {
                    let plan = caprust_media_io::export_graph::plan_from_project(
                        &project,
                        pw,
                        ph,
                        fps_num,
                        fps_den,
                        23,
                        caprust_media_io::export::RateMode::Vbr,
                        8000,
                        "veryfast",
                        &models_dir,
                        playhead,
                        caprust_core::project::VideoEncoder::H264Cpu,
                    )
                    .map_err(|e| format!("plan: {e:#}"))?;
                    caprust_media_io::preview_render::render_single_frame(
                        std::path::Path::new(&ffmpeg_path),
                        &plan,
                        playhead,
                        &path,
                        fps_f,
                    )
                    .map_err(|e| format!("render: {e:#}"))?;
                    Ok(path)
                })();
                let _ = tx.send(out);
            })
            .expect("spawn save-frame thread");
    }

    /// Poll the in-flight save-frame worker. Toasts success/failure.
    fn drain_save_frame(&mut self) {
        let Some(rx) = self.save_frame_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(path)) => {
                tracing::info!("frame saved to {}", path.display());
                self.toast(format!("{} {}", tr("toast-frame-saved"), path.display()));
            }
            Ok(Err(e)) => {
                tracing::error!("save frame failed: {e}");
                self.toast_error(format!("{}: {e}", tr("toast-frame-failed")));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                self.save_frame_rx = Some(rx);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                tracing::warn!("save frame: worker disconnected");
                self.save_frame_rx = None;
            }
        }
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

    /// Start a background download for one SFX catalog entry. No-op
    /// while another SFX download is in flight or the file is already
    /// cached. Uses the shared streaming downloader (`.part` + rename,
    /// no SHA pinning — Mixkit serves no published hashes).
    fn start_sfx_download(&mut self, entry_id: &'static str) {
        if self.sfx_download.is_some() {
            return;
        }
        let Some(entry) = caprust_core::sfx::CATALOG.iter().find(|e| e.id == entry_id) else {
            return;
        };
        if caprust_core::sfx::is_downloaded(entry) {
            return;
        }
        let url = entry.url.to_string();
        let dest = caprust_core::sfx::local_path(entry);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name(format!("caprust-sfx-{entry_id}"))
            .spawn(move || {
                let r = caprust_core::models::download::download_file(&url, &dest, "", None)
                    .map(|()| dest)
                    .map_err(|e| e.to_string());
                let _ = tx.send(r);
            })
            .ok();
        self.sfx_download = Some((entry_id.to_string(), rx));
        self.asset_browser.sfx_downloading = Some(entry_id.to_string());
    }

    /// Poll the in-flight SFX download. On success the file is
    /// imported into the media bin (deduped by path) and probe +
    /// thumbnail jobs are enqueued through the standard path, so
    /// drag-to-timeline works immediately.
    fn drain_sfx_download(&mut self) {
        let Some((entry_id, rx)) = self.sfx_download.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(path)) => {
                let path_str = path.to_string_lossy().to_string();
                if self.project.media.find_by_path(&path_str).is_none() {
                    let id = self
                        .project
                        .media
                        .add(&path_str, caprust_core::MediaKind::Audio);
                    if let Some(item) = self.project.media.items.iter().find(|m| m.id == id) {
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
                tracing::info!("sfx downloaded: {entry_id}");
                self.toast(format!("{}: {entry_id}", tr("toast-sfx-ready")));
            }
            Ok(Err(msg)) => {
                tracing::error!("sfx download {entry_id} failed: {msg}");
                self.toast_error(format!("{}: {msg}", tr("toast-sfx-failed")));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                self.sfx_download = Some((entry_id, rx));
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                tracing::warn!("sfx download thread vanished");
            }
        }
        self.asset_browser.sfx_downloading = None;
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
        if ev.save_frame {
            self.save_frame_png();
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

    /// How much vertical room the timeline gets.
    fn timeline_height_default(&self) -> f32 {
        match self.settings.timeline_size {
            caprust_core::TimelineSize::S => 180.0,
            caprust_core::TimelineSize::M => 280.0,
            caprust_core::TimelineSize::L => 420.0,
        }
    }

    /// Height of one track row at each timeline size. Used to scale the
    /// lanes so L really is taller than S, not just a taller box.
    fn timeline_height_min(&self) -> f32 {
        match self.settings.timeline_size {
            caprust_core::TimelineSize::S => 120.0,
            caprust_core::TimelineSize::M => 180.0,
            caprust_core::TimelineSize::L => 240.0,
        }
    }

    /// The timeline's own height, in logical pixels.
    ///
    /// `timeline_drag` is the height the user pulled to, and it wins while
    /// it is set -- dragging is still allowed, because that is how the
    /// timeline has always been sized and taking it away made the layout
    /// feel fixed. What changed is who remembers it: a resizable
    /// `TopBottomPanel` stores the dragged height in egui's context state,
    /// where `default_height` can no longer reach it, and picking a size
    /// afterwards did nothing. Here the drag lives in a field the selector
    /// clears, so S/M/L always wins over the drag and the timeline always
    /// comes back to where the size says it should be.
    fn timeline_height(&self, area_h: f32) -> f32 {
        if self.timeline_collapsed {
            return TIMELINE_HANDLE_HEIGHT;
        }
        let base = self
            .timeline_drag
            .unwrap_or_else(|| self.timeline_height_default())
            .max(self.timeline_height_min());
        base.clamp(80.0, (area_h * 0.7).max(80.0))
    }

    /// What the timeline needs in order to show every track.
    ///
    /// The band has to be at least this tall or the lanes run off the bottom
    /// of the window, which is what happened at M: the setting asked for
    /// 280px and the five lanes with the ruler and the toolbar needed more,
    /// so the bottom tracks were simply cut away.
    fn timeline_content_height(&self) -> f32 {
        let factor = match self.settings.timeline_size {
            caprust_core::TimelineSize::S => 1.0,
            caprust_core::TimelineSize::M => 1.3,
            caprust_core::TimelineSize::L => 1.7,
        };
        let lanes: f32 = self.project.tracks.iter().map(|t| t.height * factor).sum();
        TIMELINE_CHROME_H + crate::timeline::ruler::RULER_HEIGHT + lanes
    }

    /// Drag handle between the preview and the timeline.
    ///
    /// Returns the new height when the user pulled it this frame. The handle
    /// is drawn in its own layer so it sits on the boundary regardless of
    /// what the two regions painted.
    fn timeline_drag_separator(&mut self, ui: &mut egui::Ui, y: f32, max_h: f32) {
        const HANDLE: f32 = 7.0;
        let rect = egui::Rect::from_center_size(
            egui::pos2(ui.available_width() * 0.5, y),
            egui::vec2(ui.available_width(), HANDLE),
        );
        let painter = ui.painter_at(rect);
        let resp = ui.interact(rect, ui.id().with("timeline_drag"), egui::Sense::drag());
        let accent = self.theme.accent_color();
        let hot = resp.hovered() || resp.dragged();
        painter.rect_filled(
            rect,
            egui::CornerRadius::same(3),
            if hot {
                accent.gamma_multiply(0.55)
            } else {
                ui.visuals().widgets.noninteractive.bg_stroke.color
            },
        );

        if resp.dragged() {
            // `dy` is negative when the pointer moves up, which is a taller
            // timeline, so the sign is flipped.
            let dy = ui.input(|i| i.pointer.delta().y);
            let next = (self.timeline_drag.unwrap_or(self.timeline_height_default()) - dy)
                .clamp(self.timeline_height_min(), max_h);
            self.timeline_drag = Some(next);
        }
    }

    /// The strip left along the bottom when the timeline is collapsed.
    fn show_timeline_handle(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            if ui
                .add(egui::Button::new(
                    egui::RichText::new(format!(
                        "{}  {}",
                        crate::dock::Tab::Timeline.icon(),
                        tr("menu-timeline-show")
                    ))
                    .size(12.0),
                ))
                .on_hover_text(tr("menu-timeline-collapse"))
                .clicked()
            {
                self.timeline_collapsed = false;
                self.timeline_drag = None;
            }
        });
    }

    /// The timeline's title strip: the picker lives in the dropdown on the
    /// left, the title sits in the middle, like every other container in
    /// this editor.
    fn show_timeline_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let sizes = [
                caprust_core::TimelineSize::S,
                caprust_core::TimelineSize::M,
                caprust_core::TimelineSize::L,
            ];
            let selected = sizes
                .iter()
                .position(|s| *s == self.settings.timeline_size)
                .unwrap_or(0);

            ui.menu_button(
                egui::RichText::new(format!(
                    "{}  {}  {}",
                    crate::dock::Tab::Timeline.icon(),
                    sizes[selected].as_str().to_uppercase(),
                    tr(timeline_size_key(sizes[selected])),
                ))
                .size(14.0),
                |ui| {
                    ui.label(
                        egui::RichText::new(tr("menu-timeline-size"))
                            .strong()
                            .size(12.0),
                    );
                    ui.separator();
                    for (i, size) in sizes.iter().enumerate() {
                        let label = format!(
                            "{}   {}",
                            if i == selected { "●" } else { "○" },
                            tr(timeline_size_key(*size))
                        );
                        if ui.selectable_label(i == selected, label).clicked() {
                            self.settings.timeline_size = *size;
                            // Picking a size is a request to see the
                            // timeline at that size, so the drag that set
                            // some other height is dropped.
                            self.timeline_drag = None;
                            self.timeline_collapsed = false;
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    if ui.button(tr("menu-timeline-collapse")).clicked() {
                        self.timeline_collapsed = true;
                        self.timeline_drag = None;
                        ui.close_menu();
                    }
                    if ui.button(tr("menu-timeline-reset-size")).clicked() {
                        self.timeline_drag = None;
                        ui.close_menu();
                    }
                },
            );

            // Collapse stays out here as well as inside the dropdown: buried
            // in a menu it read as missing.
            if ui
                .add(egui::Button::new(
                    egui::RichText::new(tr("menu-timeline-hide")).size(12.0),
                ))
                .on_hover_text(tr("menu-timeline-collapse"))
                .clicked()
            {
                self.timeline_collapsed = true;
                self.timeline_drag = None;
            }

            let spacer = ((ui.available_width() - 190.0) * 0.5).max(0.0);
            ui.add_space(spacer);
            ui.label(
                egui::RichText::new(tr(crate::dock::Tab::Timeline.title_key()))
                    .strong()
                    .size(15.0),
            );

            if self.timeline_drag.is_some()
                && ui
                    .add(egui::Button::new(
                        egui::RichText::new(tr("menu-timeline-reset-size")).size(10.0),
                    ))
                    .clicked()
            {
                self.timeline_drag = None;
            }
        });
    }

    pub(crate) fn render_timeline_panel(&mut self, ui: &mut egui::Ui) {
        let ctx_owned = ui.ctx().clone();
        let ctx: &egui::Context = &ctx_owned;
        // The body must never ask for more room than the panel was given,
        // or `set_min_height` fights `exact_height` and the panel grows
        // back. Collapsed means the tracks keep their pixels but nothing
        // else claims space.
        let min_h = if self.timeline_collapsed {
            0.0
        } else {
            match self.settings.timeline_size {
                caprust_core::TimelineSize::S => 140.0,
                caprust_core::TimelineSize::M => 240.0,
                caprust_core::TimelineSize::L => 360.0,
            }
        };
        ui.set_min_height(min_h);
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

        // The rows have to be the same height in both columns or the
        // headers drift away from the lanes, and the factor is what
        // decides that. Computed once, here, and used by both.
        let lane_layout = lane_layout(
            self.settings.timeline_size,
            &self
                .project
                .tracks
                .iter()
                .map(|t| t.height)
                .collect::<Vec<_>>(),
            full_h - crate::timeline::ruler::RULER_HEIGHT,
        );
        let lane_factor = lane_layout.factor;

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
                        // How much bigger the lane is drawn at each
                        // timeline size, scaled down if the set does not
                        // fit. Named, because the drag handle below has
                        // to divide by it to turn screen pixels back into
                        // a stored height.
                        let lane_scale = lane_factor;
                        let row_h = track.height * lane_scale;
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
                                // The row on screen is
                                // `track.height * timeline_size_factor`
                                // tall, and the drag delta is in screen
                                // pixels. Writing the row height straight
                                // back into `track.height` fed the factor
                                // into its own value on every frame of the
                                // drag: at M it grew by 1.3x per frame, at L
                                // by 1.7x. It ran away upward, the clamp
                                // pinned it at the maximum, and from there
                                // dragging up could not lower it -- because
                                // the row it was comparing against was the
                                // clamped value times the factor again. The
                                // reported symptom was "I can make it
                                // bigger but never smaller".
                                //
                                // The drag moves the divider by `dy` pixels,
                                // so the stored height moves by `dy`
                                // divided by the factor. Bounds are in
                                // stored units, as they always were.
                                // The divider has to stay under the
                                // pointer. When the rows are scaled to
                                // fit, the marginal rate of the row
                                // height is not the factor but the
                                // factor times the dragged track's
                                // share of the rest, so dividing by the
                                // plain factor left the divider
                                // trailing behind.
                                track.height = stored_height_after_drag(
                                    track.height,
                                    dy,
                                    &lane_layout,
                                );
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
                        let row_h = track.height * lane_factor;
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
                                egui::Stroke::new(
                                    theme_snapshot.clip_border_width(),
                                    border_color,
                                ),
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

                            // Split clip display: thumbnail strip on top,
                            // waveform band at the bottom. Per-clip
                            // override wins; otherwise the global
                            // Settings → Appearance → Timeline toggle
                            // decides. Video clips only; audio clips
                            // keep the full-height waveform.
                            let split_layout: Option<(egui::Rect, egui::Rect)> = {
                                let is_video = matches!(
                                    ctype,
                                    caprust_core::ClipType::Video { .. }
                                );
                                let enabled = self
                                    .project
                                    .clips
                                    .iter()
                                    .find(|cc| cc.id == clip_id)
                                    .and_then(|cc| cc.split_display)
                                    .unwrap_or(self.settings.waveform_bottom);
                                if is_video && enabled {
                                    crate::timeline::split_clip_rects(clip_rect)
                                } else {
                                    None
                                }
                            };
                            let thumb_rect = split_layout.map(|(s, _)| s).unwrap_or(clip_rect);

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
                                let tile_h = thumb_rect.height();
                                let tile_w = (tile_h * aspect).max(8.0);
                                let mut x = thumb_rect.left();
                                let right = thumb_rect.right();
                                let mut guard = 0;
                                while x < right - 1.0 && guard < 200 {
                                    let w = (right - x).min(tile_w);
                                    let tile_rect = egui::Rect::from_min_size(
                                        egui::pos2(x, thumb_rect.top()),
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
                                p.rect_filled(thumb_rect, 4.0, c.gamma_multiply(0.35));
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
                                //
                                // Split display: the waveform lives in
                                // the bottom band, not centered over
                                // the thumbnails. Video without audio
                                // (or detached) shows a darker empty
                                // band instead.
                                let wave_rect =
                                    split_layout.map(|(_, w)| w).unwrap_or(clip_rect);
                                if split_layout.is_some()
                                    && matches!(ctype, caprust_core::ClipType::Video { .. })
                                {
                                    let has_audio_wave = self
                                        .project
                                        .clips
                                        .iter()
                                        .find(|cc| cc.id == clip_id)
                                        .and_then(|cc| cc.media_id)
                                        .and_then(|mid| self.waveform_peaks_for(mid))
                                        .is_some();
                                    if !has_audio_wave {
                                        p.rect_filled(
                                            wave_rect,
                                            0.0,
                                            egui::Color32::BLACK.gamma_multiply(0.45),
                                        );
                                    }
                                }
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
                                        let center_y = wave_rect.center().y;
                                        let amp = wave_rect.height() * 0.42;
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
                                        let clip_paint_rect = split_layout
                                            .map(|(_, w)| w)
                                            .unwrap_or(clip_rect);
                                        let clip = ui.painter().with_clip_rect(clip_paint_rect);
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
                                    let pos_color = theme_snapshot.waveform_positive_color();
                                    let neg_color = theme_snapshot.waveform_negative_color();
                                    let center_color = theme_snapshot.waveform_center_color();
                                    let duck_color = theme_snapshot.waveform_color();
                                    crate::timeline::draw_audio_envelope(
                                        &p,
                                        full_rect,
                                        clip_rect,
                                        &samples,
                                        &zones,
                                        pos_color,
                                        neg_color,
                                        center_color,
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
                            // Properties panel. Non-text clips open
                            // the rename modal instead.
                            if resp.double_clicked() {
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
                                    pending_actions.push(ClipAction::FocusTextContent(clip_id));
                                } else {
                                    pending_actions.push(ClipAction::RenameClip(clip_id));
                                }
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
                                // --- Rename clip ---
                                if ui.button(tr("clip-ctx-rename")).clicked() {
                                    pending_actions.push(ClipAction::RenameClip(clip_id));
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
                                // --- Clip display: standard vs split ---
                                // Per-clip waveform layout override. Only
                                // offered on video clips (the split puts
                                // the thumbnail strip on top and the
                                // waveform band at the bottom).
                                {
                                    let is_video = self
                                        .project
                                        .clips
                                        .iter()
                                        .find(|c| c.id == clip_id)
                                        .map(|c| {
                                            matches!(
                                                c.clip_type,
                                                caprust_core::ClipType::Video { .. }
                                            )
                                        })
                                        .unwrap_or(false);
                                    if is_video {
                                        let effective_split = self
                                            .project
                                            .clips
                                            .iter()
                                            .find(|c| c.id == clip_id)
                                            .and_then(|c| c.split_display)
                                            .unwrap_or(self.settings.waveform_bottom);
                                        ui.menu_button(tr("clip-ctx-display"), |ui| {
                                            if ui
                                                .selectable_label(
                                                    !effective_split,
                                                    tr("clip-ctx-display-standard"),
                                                )
                                                .clicked()
                                            {
                                                pending_actions.push(
                                                    ClipAction::SetSplitDisplay(
                                                        clip_id,
                                                        Some(false),
                                                    ),
                                                );
                                                ui.close_menu();
                                            }
                                            if ui
                                                .selectable_label(
                                                    effective_split,
                                                    tr("clip-ctx-display-split"),
                                                )
                                                .clicked()
                                            {
                                                pending_actions.push(
                                                    ClipAction::SetSplitDisplay(
                                                        clip_id,
                                                        Some(true),
                                                    ),
                                                );
                                                ui.close_menu();
                                            }
                                        });
                                    }
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
                ClipAction::RenameClip(id) => {
                    self.selected_clips = vec![id];
                    let buf = self
                        .project
                        .clips
                        .iter()
                        .find(|c| c.id == id)
                        .and_then(|c| c.name.clone())
                        .unwrap_or_default();
                    self.clip_rename = Some((id, buf));
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
                ClipAction::SetSplitDisplay(id, v) => {
                    let cmd =
                        caprust_core::commands::set_clip::SetClipCommand::new(id).split_display(v);
                    if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                        tracing::error!("set split display failed: {e}");
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
                // Photos are stills. ffprobe reports a sub-second nil
                // duration (or 0) for them, which used to create
                // 0.04-second clips that could not be grabbed or dragged.
                // Seed a 3 s box and let stills stretch up to 10 s.
                const IMAGE_DEFAULT_MS: u64 = 3_000;
                const IMAGE_MAX_MS: u64 = 10_000;
                let dur = match item.kind {
                    MediaKind::Image => item.duration_ms.clamp(IMAGE_DEFAULT_MS, IMAGE_MAX_MS),
                    _ => {
                        if item.duration_ms > 0 {
                            item.duration_ms
                        } else {
                            3000
                        }
                    }
                };
                let mut clip = match item.kind {
                    MediaKind::Video => Clip::new_video(&item.path, ti, cursor_ms, dur),
                    MediaKind::Audio => Clip::new_audio(&item.path, ti, cursor_ms, dur),
                    MediaKind::Image => Clip::new_image(&item.path, ti, cursor_ms, dur),
                };
                clip.media_id = Some(item.id);
                // Cap growth at a useful maximum for stills. Media probe
                // duration is irrelevant for JPG/HEIC and must not cap a
                // photo at 0.04 s.
                clip.source_duration_ms = match item.kind {
                    MediaKind::Image => IMAGE_MAX_MS,
                    _ => item.duration_ms,
                };
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

    /// Switch between the two editors.
    ///
    /// Leaving `Modern` for `Default` restores the dock exactly as the
    /// user left it, because the dock tree is kept either way -- going
    /// to Modern and back does not cost you the arrangement you spent
    /// ten minutes on.
    fn set_layout_mode(&mut self, mode: caprust_core::LayoutMode) {
        if self.settings.layout == mode {
            return;
        }
        self.settings.layout = mode;
        tracing::info!("layout mode: {mode:?}");
    }

    /// Switch the editor to a named dock preset and remember the choice.
    ///
    /// Selecting a preset also puts the app in `Dockable`, because "I
    /// want this layout every time" and "I want my dragged layout" are
    /// different requests and the menu cannot offer both at once. The
    /// preset is written to settings and re-applied on the next launch,
    /// which is what the old View-menu buttons failed to do: they
    /// overwrote the dock tree and then let the save tick store that
    /// tree as if the user had dragged it there, so the choice never
    /// survived a restart and there was no way back to the default
    /// arrangement without hand-editing the settings file.
    fn apply_layout_preset(&mut self, preset: crate::dock::LayoutPreset) {
        self.dock_state = crate::dock::preset_dock_state(preset);
        self.settings.layout_preset = Some(preset.id().to_string());
        tracing::info!("dock layout preset applied: {:?} ({})", preset, preset.id());
    }

    /// Go back to following the arrangement saved on this machine and
    /// drop the preset. The arrangement itself is reset to the built-in
    /// one, because by this point "saved arrangement" and "whatever the
    /// preset left behind" are the same tree.
    fn reset_layout(&mut self) {
        self.dock_state = crate::dock::default_dock_state();
        self.settings.layout = caprust_core::LayoutMode::Default;
        self.settings.layout_preset = None;
        tracing::info!("dock layout reset to default");
    }

    /// Which editor this session shows.
    ///
    /// Two real renderers, not one dock with a different arrangement.
    /// They differ in what a user can do, not in how it is spaced: the
    /// dock has tabs you drag between and resave, `Modern` has fixed
    /// panels and nothing to move. Offering only presets over the dock
    /// would have made the choice a matter of taste about ratios, which
    /// is not what someone picking an editor is deciding.
    fn show_editor(&mut self, ctx: &egui::Context) {
        match self.settings.layout {
            caprust_core::LayoutMode::Default => self.show_editor_dock(ctx),
            caprust_core::LayoutMode::Modern => self.show_editor_modern(ctx),
        }
    }

    fn show_editor_dock(&mut self, ctx: &egui::Context) {
        self.show_menu_bar(ctx);
        self.show_toolbar(ctx);
        // Jobs bar sits above the timeline so it is visible from any
        // panel. In the modern layout show_editor_modern calls this; in
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

    /// Fixed panels: media on the left, properties on the right, the
    /// preview in the middle, the timeline across the bottom.
    ///
    /// The shape an editor is expected to have before you have arranged
    /// anything, and the one that stays put when the window changes. No
    /// tabs to click through, no drag handles, nothing to re-do on a
    /// second monitor.
    ///
    /// The vertical strip of category icons that a modern editor puts
    /// down the far left is deliberately not copied: it would be a
    /// second way into the same panels for no gain here, and the asset
    /// Fixed zones, no dragging.
    ///
    ///   ┌──────────┬───────────────┬─────────────┐
    ///   │  browser │               │  properties │
    ///   │ (top)    │    preview    │             │
    ///   ├──────────┤               ├─────────────┤
    ///   │ effects  │               │  plugins    │
    ///   │ trans.   │               │ (swappable) │
    ///   │ filters  │               │             │
    ///   │ text     │               │             │
    ///   └──────────┴───────────────┴─────────────┘
    ///   │            timeline                    │
    ///   └────────────────────────────────────────┘
    ///
    /// The zones are fixed; their contents are not. That is the whole
    /// reason this is a second editor rather than an arrangement of the
    /// dock: you stop spending time putting panels back, and stop
    /// wondering where one went, while still being able to put the one
    /// you actually need on screen.
    ///
    /// The zone headers are tab strips running down the side rather than
    /// across the top. Horizontal tabs inside a narrow column leave
    /// room for two labels, and they read as an older interface; a
    /// Four dock zones in a fixed frame.
    ///
    ///   ┌──────────┬───────────────┬─────────────┐
    ///   │  browser │               │  properties │
    ///   │ (dock)   │    preview    │   (dock)    │
    ///   ├──────────┤               ├─────────────┤
    ///   │ effects  │               │  plugins    │
    ///   │ (dock)   │               │   (dock)    │
    ///   └──────────┴───────────────┴─────────────┘
    ///   │              timeline                    │
    ///   └──────────────────────────────────────────┘
    ///
    /// The frame is fixed; the docks inside it are not. Each zone is a
    /// real egui_dock area, so dragging a tab offers the same positions
    /// it offers in the dock editor -- tab, below, side, above -- and a
    /// zone can be split, collapsed and put back. The user is not locked
    /// into one arrangement, they are spared having to build it from
    /// scratch every time the window moves.
    ///
    /// The frame is what separates this from the dock editor. There, one
    /// tree fills the window and everything is negotiable, including
    /// which quarter of the screen the timeline gets. Here the four
    /// zones are where a clip editor puts them, and the docks handle the
    /// Four dock zones in a fixed frame.
    ///
    ///   ┌──────────┬───────────────┬─────────────┐
    ///   │  browser │               │  properties │
    ///   │  (dock)  │    preview    │   (dock)    │
    ///   ├──────────┤               ├─────────────┤
    ///   │ effects  │               │  plugins    │
    ///   │  (dock)  │               │   (dock)    │
    ///   └──────────┴───────────────┴─────────────┘
    ///   │              timeline                    │
    ///   └──────────────────────────────────────────┘
    ///
    /// The frame is fixed; the docks inside it are not. Each zone is a
    /// real egui_dock area, so dragging a tab offers the same positions
    /// it offers in the dock editor -- stacked tab, below, side, above --
    /// and a zone can be split, collapsed and put back. Nothing is locked
    /// to a cell; the user is spared having to rebuild the arrangement
    /// from scratch every time the window moves.
    ///
    /// The frame is what separates this from the dock editor. There, one
    /// tree fills the window and everything is negotiable, including
    /// which quarter of the screen the timeline gets. Here the four
    /// zones sit where a clip editor puts them, and the docks handle the
    /// rest.
    ///
    /// `modern` is moved out of `self` first because a dock borrows its
    /// tree for the duration of the frame while the viewer needs `self`
    /// Four dock zones in a fixed frame.
    ///
    ///   ┌──────────┬───────────────┬─────────────┐
    ///   │  browser │               │  properties │
    ///   │  (dock)  │    preview    │   (dock)    │
    ///   ├──────────┤               ├─────────────┤
    ///   │ effects  │               │  plugins    │
    ///   │  (dock)  │               │   (dock)    │
    ///   └──────────┴───────────────┴─────────────┘
    ///   │              timeline                    │
    ///   └──────────────────────────────────────────┘
    ///
    /// The frame is fixed; the docks inside it are not. Each zone is a
    /// real egui_dock area, so dragging a tab offers the same positions
    /// it offers in the dock editor -- stacked tab, below, side, above --
    /// and a zone can be split, collapsed and put back. Nothing is locked
    /// to a cell; the user is spared having to rebuild the arrangement
    /// from scratch every time the window moves.
    ///
    /// The frame is what separates this from the dock editor. There, one
    /// tree fills the window and everything is negotiable, including
    /// which quarter of the screen the timeline gets. Here the four
    /// zones sit where a clip editor puts them, and the docks handle the
    /// rest.
    ///
    /// `modern` is moved out of `self` first: a dock borrows its tree for
    /// the frame while the viewer borrows the app, and holding the two
    /// apart is what lets both exist. The viewer reborrows rather than
    /// Four dock zones in a fixed frame.
    ///
    ///   ┌──────────┬───────────────┬─────────────┐
    ///   │  browser │               │  properties │
    ///   │  (dock)  │    preview    │   (dock)    │
    ///   ├──────────┤               ├─────────────┤
    ///   │ effects  │               │  plugins    │
    ///   │  (dock)  │               │   (dock)    │
    ///   └──────────┴───────────────┴─────────────┘
    ///   │              timeline                    │
    ///   └──────────────────────────────────────────┘
    ///
    /// The frame is fixed; the docks inside it are not. Each zone is a
    /// real egui_dock area, so dragging a tab offers the same positions
    /// it offers in the dock editor -- stacked tab, below, side, above --
    /// and a zone can be split, collapsed and put back. Nothing is locked
    /// to a cell; the user is spared having to rebuild the arrangement
    /// from scratch every time the window moves.
    ///
    /// The frame is what separates this from the dock editor. There, one
    /// tree fills the window and everything is negotiable, including
    /// which quarter of the screen the timeline gets. Here the four
    /// zones sit where a clip editor puts them, and the docks handle the
    /// rest.
    ///
    /// `modern` is moved out of `self` first: a dock borrows its tree for
    /// the frame while the viewer borrows the app, and holding the two
    /// apart is what lets both exist. The viewer reborrows rather than
    /// Four fixed zones.
    ///
    ///   ┌──────────┬───────────────┬─────────────┐
    ///   │  browser │               │  properties │
    ///   │          │    preview    │             │
    ///   ├──────────┤               ├─────────────┤
    ///   │ effects  │               │  plugins    │
    ///   │          │               │             │
    ///   └──────────┴───────────────┴─────────────┘
    ///   │              timeline                    │
    ///   └──────────────────────────────────────────┘
    ///
    /// The split is fixed and there is no handle on it: these zones are
    /// not meant to be resized, which is what separates this editor from
    /// the dock. Each zone is a real egui_dock area, so a tab can still be
    /// dragged to stacked, below, side or above within it.
    ///
    /// The zones are sized with `allocate_ui` rather than
    /// `TopBottomPanel`. A `TopBottomPanel` nested in a `SidePanel` keeps
    /// its own height in the context and fights the side panel for the
    /// same space; on the first frames after the window gains focus the
    /// two halves slide apart, the top one off the top and the bottom
    /// one off the bottom, until only a sliver of each is visible.
    /// `allocate_ui` takes exactly the rectangle asked for and holds no
    /// state, so the split is the same on every frame.
    ///
    /// The timeline is drawn before the side panels so it takes its space
    /// out of the window first; otherwise the side panels claim the full
    /// Three columns, two zones each, six panels that stay put.
    ///
    ///   ┌────────┬──────────────────┬────────┐
    ///   │        │                  │        │
    ///   │ L top  │    preview       │ R top  │
    ///   │        │                  │        │
    ///   ├────────┤                  ├────────┤
    ///   │        │                  │        │
    ///   │ L bot  │                  │ R bot  │
    ///   │        │                  │        │
    ///   └────────┴──────────────────┴────────┘
    ///   │              timeline              │
    ///   └────────────────────────────────────┘
    ///
    /// Columns start at 20 / 60 / 20 and each column splits down the
    /// middle. Both splits drag: the column divider moves left and right,
    /// the zone divider moves up and down, exactly as they do in the dock
    /// editor. What this editor fixes is that there are four side panels
    /// and one preview, and the user never has to put them there.
    ///
    /// The contents are picked, not dragged. Each zone has a dropdown of
    /// every panel the editor has; choosing one puts it in that zone and
    /// leaves it there. That is the trade: no moving between zones, in
    /// exchange for the arrangement never drifting.
    fn show_editor_modern(&mut self, ctx: &egui::Context) {
        self.show_menu_bar(ctx);
        self.show_toolbar(ctx);

        self.sync_plugin_scan_dirs();

        // Bottom first: it reserves its space, so the columns below see
        // only what is left.
        self.show_jobs_bar(ctx);

        // Sidebars and the middle, as fractions of the window's width.
        //
        // S keeps both sidebars at 20% and the timeline takes the middle
        // 60%; M drops the left one so the timeline takes that side and
        // spans 80%; L drops both and it spans the window.
        //
        // The preset is the *starting* width, not a constant one. Forcing it
        // every frame with `exact_width` made the sidebars impossible to
        // resize -- the handle was there, the drag moved, and the next frame
        // put it back, so the only direction that felt alive was the one
        // that shrank the gap. `default_width` seeds the panel, the user
        // drags it wherever they like afterwards, and the panel id carries
        // the preset so choosing a new size starts a clean panel instead of
        // resurrecting a width set under the old one.
        let area = ctx.available_rect();
        //
        // The sidebars keep their width at every size. What changes is how
        // much of the width the timeline takes: at S it sits between the two
        // sidebars, at M it takes the space the right sidebar gave up, and at
        // L it crosses both of them. Hiding a sidebar outright was the wrong
        // reading -- at L the columns stay, and only the timeline's own band
        // of height runs past them.
        let (left_frac, right_frac) = match self.settings.timeline_size {
            caprust_core::TimelineSize::S => (0.20, 0.20),
            caprust_core::TimelineSize::M => (0.20, 0.00),
            caprust_core::TimelineSize::L => (0.20, 0.20),
        };
        let left_on = self.modern_left_on;
        let right_on = self.modern_right_on && right_frac > 0.0;
        let left_w = if left_on {
            (area.width() * left_frac).min(area.width() * 0.5)
        } else {
            0.0
        };
        let right_w = if right_on {
            (area.width() * right_frac).min(area.width() * 0.5)
        } else {
            0.0
        };

        // Sidebars run the full height of the frame, past the timeline.
        //
        // A hidden sidebar is one that is never shown. Asking a panel for a
        // width of zero did not hide it -- it still drew, at its previous
        // width or at the preset -- which is why L kept a 20% column and M
        // kept two. A panel that is not shown claims no space at all, so the
        // middle gets exactly the width the size asked for.
        let split = if self.timeline_collapsed { 0.5 } else { 0.55 };

        // L is the one size where the timeline leaves the middle column.
        //
        // At S and M it is drawn inside the `CentralPanel`, so it is exactly
        // as wide as the space between the sidebars. At L it has to cross
        // both of them, which a panel inside the centre cannot do -- so it
        // becomes a real bottom panel first, taking the band across the whole
        // window, and the sidebars are then drawn into what is left above
        // it. That is the whole difference from S: the columns keep running
        // beside the preview and only stop at the timeline's own height.
        if self.settings.timeline_size == caprust_core::TimelineSize::L {
            // The timeline comes first, then the sidebars.
            //
            // A panel is carved out of whatever space is left when it is
            // shown, so the order decides the shape. Showing the sidebars
            // first and the bottom panel after left the timeline only as
            // wide as the gap between them -- it sat in the middle, and the
            // columns ran all the way to the bottom of the window on either
            // side of it. Showing the band first takes the full width of the
            // window, and the sidebars are then given what is left above it.
            //
            // The columns are ordinary `SidePanel`s, so they claim space and
            // keep egui's own resize handle. Drawing them as `Area`s to let
            // the band cross them was the earlier attempt: an `Area` claims
            // nothing, so widening a sidebar left the preview where it was
            // and the two overlapped.
            // Captured before the panel is shown: once it exists, the
            // available rect is already cut down to what is above the band.
            let avail = ctx.available_rect().height();
            let band_h = if self.timeline_collapsed {
                TIMELINE_HANDLE_HEIGHT
            } else {
                self.timeline_height(avail)
                    .max(self.timeline_content_height())
                    .min(avail * 0.8)
            };
            egui::TopBottomPanel::bottom("modern_timeline")
                .resizable(false)
                .exact_height(band_h)
                .show(ctx, |ui| {
                    if self.timeline_collapsed {
                        self.show_timeline_handle(ui);
                    } else {
                        // Same seam S and M put between the preview and
                        // the timeline, for the same reason: the band is
                        // the one piece of this layout with nothing you
                        // can pull on. It is drawn here rather than by
                        // `resizable(true)` because a resizable panel
                        // keeps the dragged height in egui's own state,
                        // where picking S, M or L can no longer reach it.
                        let seam_h = 9.0;
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), seam_h),
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                self.timeline_drag_separator(ui, seam_h * 0.5, avail * 0.7);
                            },
                        );
                        let body_h = (band_h - seam_h).max(60.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), body_h),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_min_height(body_h);
                                self.show_timeline_header(ui);
                                ui.separator();
                                self.render_timeline_panel(ui);
                            },
                        );
                    }
                });

            if left_on {
                egui::SidePanel::left(egui::Id::new(("modern_left", self.settings.timeline_size)))
                    .resizable(true)
                    .default_width(left_w)
                    .min_width(200.0)
                    .max_width(area.width() * 0.5)
                    .show(ctx, |ui| {
                        self.show_zone_full(ui, Zone::LeftTop);
                    });
            }
            if right_on {
                egui::SidePanel::right(egui::Id::new((
                    "modern_right",
                    self.settings.timeline_size,
                )))
                .resizable(true)
                .default_width(right_w)
                .min_width(220.0)
                .max_width(area.width() * 0.5)
                .show(ctx, |ui| {
                    self.show_zone_full(ui, Zone::RightTop);
                });
            }

            egui::CentralPanel::default().show(ctx, |ui| {
                self.render_preview_panel(ui);
            });
            return;
        }

        if left_on {
            egui::SidePanel::left(egui::Id::new(("modern_left", self.settings.timeline_size)))
                .resizable(true)
                .default_width(left_w)
                .min_width(200.0)
                .max_width(area.width() * 0.5)
                .show(ctx, |ui| {
                    self.show_zone_column(ui, split, true);
                });
        }
        if right_on {
            egui::SidePanel::right(egui::Id::new(("modern_right", self.settings.timeline_size)))
                .resizable(true)
                .default_width(right_w)
                .min_width(220.0)
                .max_width(area.width() * 0.5)
                .show(ctx, |ui| {
                    self.show_zone_column(ui, split, false);
                });
        }

        // The middle column is whatever the sidebars left. The timeline is
        // drawn inside it rather than as a bottom panel, which is the whole
        // reason the widths above can change: a bottom panel always spans
        // the window, and the sidebars would have to stop above it.
        egui::CentralPanel::default().show(ctx, |ui| {
            let avail_h = ui.available_height();
            let width = ui.available_width();
            let band_h = if self.timeline_collapsed {
                TIMELINE_HANDLE_HEIGHT
            } else {
                // Never smaller than the tracks need, so nothing is cut off.
                self.timeline_height(avail_h)
                    .max(self.timeline_content_height())
                    .min(avail_h * 0.8)
            };
            let preview_h = (avail_h - band_h).max(80.0);

            ui.allocate_ui_with_layout(
                egui::vec2(width, preview_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_height(preview_h);
                    self.render_preview_panel(ui);
                },
            );

            if self.timeline_collapsed {
                ui.allocate_ui_with_layout(
                    egui::vec2(width, band_h),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        self.show_timeline_handle(ui);
                    },
                );
            } else {
                // The seam sits on the boundary between the two bands and
                // takes a few pixels from each, so it can be grabbed either
                // side of the line.
                let seam_h = 9.0;
                ui.allocate_ui_with_layout(
                    egui::vec2(width, seam_h),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        self.timeline_drag_separator(ui, seam_h * 0.5, avail_h * 0.7);
                    },
                );
                let body_h = (band_h - seam_h).max(60.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(width, body_h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_height(body_h);
                        self.show_timeline_header(ui);
                        ui.separator();
                        self.render_timeline_panel(ui);
                    },
                );
            }
        });
    }

    /// One side column of the Modern editor: two zones filling the whole
    /// column height, with a small gap between them.
    ///
    /// `split` is the fraction of the column the top zone takes. The gap is
    /// subtracted before dividing, so the two zones plus the gap always
    /// equal the height the column was handed -- no dead band, whatever the
    /// timeline is doing.
    fn show_zone_column(&mut self, ui: &mut egui::Ui, split: f32, left: bool) {
        // The seam, not the old 6px gap. It has to be wide enough to grab,
        // and it takes its pixels out of the two zones rather than adding a
        // band of dead space between them.
        const SEAM: f32 = 9.0;
        let (top, bottom) = if left {
            (Zone::LeftTop, Zone::LeftBottom)
        } else {
            (Zone::RightTop, Zone::RightBottom)
        };

        let avail_h = ui.available_height();
        let usable = (avail_h - SEAM).max(0.0);
        let width = ui.available_width();

        // The drag wins over the preset while it is set, the same rule the
        // timeline follows. The clamps stop either container collapsing to
        // nothing, which is the only way this can be gotten wrong.
        let split = if left {
            self.left_split
        } else {
            self.right_split
        }
        .unwrap_or(split)
        .clamp(0.15, 0.85);
        let top_h = usable * split;
        let bot_h = usable - top_h;

        ui.allocate_ui_with_layout(
            egui::vec2(width, top_h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_height(top_h);
                self.render_zone(ui, top);
            },
        );
        ui.allocate_ui_with_layout(
            egui::vec2(width, SEAM),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                self.column_drag_separator(ui, SEAM * 0.5, avail_h, split, left);
            },
        );
        ui.allocate_ui_with_layout(
            egui::vec2(width, bot_h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_height(bot_h);
                self.render_zone(ui, bottom);
            },
        );
    }

    /// Drag handle across the seam of a side column.
    ///
    /// Drawn as a hairline with a small grip in the middle rather than the
    /// solid bar the timeline's seam uses: a sidebar is a fraction of the
    /// window wide, and a full-width block at that size reads as a panel
    /// divider, not as a handle.
    fn column_drag_separator(
        &mut self,
        ui: &mut egui::Ui,
        y: f32,
        column_h: f32,
        split: f32,
        left: bool,
    ) {
        const HANDLE: f32 = 7.0;
        let width = ui.available_width();
        let rect =
            egui::Rect::from_center_size(egui::pos2(width * 0.5, y), egui::vec2(width, HANDLE));
        let painter = ui.painter_at(rect);
        let resp = ui.interact(
            rect,
            ui.id()
                .with(if left { "left_split" } else { "right_split" }),
            egui::Sense::drag(),
        );
        let accent = self.theme.accent_color();
        let base = ui.visuals().widgets.noninteractive.bg_stroke.color;
        let hot = resp.hovered() || resp.dragged();

        painter.line_segment(
            [egui::pos2(0.0, y), egui::pos2(width, y)],
            egui::Stroke::new(1.0, if hot { accent } else { base }),
        );
        let grip = egui::Rect::from_center_size(egui::pos2(width * 0.5, y), egui::vec2(22.0, 5.0));
        painter.rect_filled(
            grip,
            egui::CornerRadius::same(2),
            if hot {
                accent
            } else {
                base.gamma_multiply(1.3)
            },
        );

        if resp.dragged() {
            // Pointer down moves the seam down, so the top container grows.
            let dy = ui.input(|i| i.pointer.delta().y);
            let next = (split + dy / column_h.max(1.0)).clamp(0.15, 0.85);
            if left {
                self.left_split = Some(next);
            } else {
                self.right_split = Some(next);
            }
        }
    }

    /// One zone filling whatever space it is given.
    ///
    /// `render_zone` on its own sizes the card to its contents. In a column
    /// that is wrong -- the zone is meant to be a panel that reaches the
    /// edges of its cell -- so the cell is allocated at its full height and
    /// set as a minimum, the same way `show_zone_column` does for two zones.
    fn show_zone_full(&mut self, ui: &mut egui::Ui, zone: Zone) {
        let w = ui.available_width();
        let h = ui.available_height();
        ui.allocate_ui_with_layout(
            egui::vec2(w, h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_height(h);
                self.render_zone(ui, zone);
            },
        );
    }

    /// One zone: a stack of up to three cards, like a hand held face-down.
    ///
    /// The front card carries the panel and the whole zone's title. The ones
    /// behind it show only their icon and name on a thin strip, each one
    /// offset a little further down-right and dimmer, so the stack reads as
    /// depth rather than as a list. Clicking a strip brings that card to the
    /// front.
    fn render_zone(&mut self, ui: &mut egui::Ui, zone: Zone) {
        let stack = zone.stacked_tabs(&self.modern);
        let accent = self.theme.accent_color();

        // The card is raised off the column, and a raised thing needs three
        // things to read as raised: a lit edge on the side facing the light,
        // a dark edge opposite it, and a cast shadow below. One uniform
        // stroke -- all `Frame` gives -- is a flat outline, which is what
        // "the border is not 3D" was about.
        //
        // The lit edge is the card's own fill pushed towards white rather
        // than a theme colour, so it stays visible in both light and dark.
        let card_fill = ui.style().visuals.extreme_bg_color.gamma_multiply(1.06);
        let lit = card_fill.gamma_multiply(1.9);
        let shade = card_fill.gamma_multiply(0.35);
        let accent_edge = accent.gamma_multiply(0.85);
        let rounding = egui::CornerRadius::same(10);
        const RAISED: f32 = 2.0;

        let outer = egui::Frame::new()
            .fill(card_fill)
            .corner_radius(rounding)
            .inner_margin(egui::Margin::same(8))
            .outer_margin(egui::Margin::same(4))
            .show(ui, |ui| {
                // The title strip is one row: the picker on the left, the
                // front card's name in the middle, and the cards behind it
                // as narrow tabs on the right -- the shape in the sketch.
                let header_h = 24.0;
                let w = ui.available_width();
                let (header, _) =
                    ui.allocate_exact_size(egui::vec2(w, header_h), egui::Sense::hover());
                self.render_zone_header(ui, zone, &stack, header);

                ui.separator();

                // The body takes everything that is left and scrolls.
                //
                // `auto_shrink([false, false])` is the part that matters: by
                // default a `ScrollArea` is only as tall as its contents, so
                // the card ended at the bottom of the panel and the rest of
                // the zone was bare background -- the "the container does not
                // fill the space" and "it looks like a hover highlight"
                // reports. With it, the scroll area claims the full height
                // the zone was given, the card reaches the edges of the cell,
                // and a panel taller than the zone scrolls instead of
                // spilling out of it.
                egui::ScrollArea::vertical()
                    .id_salt(zone.slot())
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if let Some(front) = stack.first().copied() {
                            self.render_tab_body(ui, front);
                        } else {
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(tr("menu-zone-empty"))
                                    .color(ui.visuals().weak_text_color()),
                            );
                            ui.add_space(4.0);
                        }
                    });
            });

        // The bevel, drawn on the frame's own rect.
        //
        // Every stroke here is `Inside`. An `Outside` stroke is clipped away
        // wherever the card sits in a tight clip -- which is how the Modern
        // editor's L layout draws its sidebars -- and that is why the border
        // disappeared there and the card read as a hover highlight rather
        // than a panel. Inside strokes cannot be clipped away, and the pair
        // of them still gives the raised edge.
        let rect = outer.response.rect;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect.expand(0.5), rounding, shade.gamma_multiply(0.45));
        painter.rect_stroke(
            rect,
            rounding,
            egui::Stroke::new(1.5, accent_edge),
            egui::StrokeKind::Middle,
        );
        painter.rect_stroke(
            rect.translate(egui::vec2(-RAISED, -RAISED)),
            rounding,
            egui::Stroke::new(1.5, lit),
            egui::StrokeKind::Inside,
        );
        painter.rect_stroke(
            rect.translate(egui::vec2(RAISED, RAISED)),
            rounding,
            egui::Stroke::new(1.5, shade),
            egui::StrokeKind::Inside,
        );
    }

    /// The zone's title strip.
    ///
    /// The strip is divided into three: the picker sits on the left, the
    /// front card's name takes the wide middle, and each card behind it gets
    /// an equal slice on the right, drawn as a tab standing a few pixels
    /// proud of the card's top edge with only its index showing. That is the
    /// stack shape from the sketch -- one card in front, one third of the
    /// title visible per card behind -- rather than a vertical list.
    fn render_zone_header(
        &mut self,
        ui: &mut egui::Ui,
        zone: Zone,
        stack: &[crate::dock::Tab],
        rect: egui::Rect,
    ) {
        let accent = self.theme.accent_color();
        let front = stack.first().copied();
        let behind: Vec<crate::dock::Tab> = stack.iter().skip(1).copied().collect();
        let behind_n = behind.len().max(1) as f32;

        // The slice each background tab gets: a third of what the strip has
        // left after the picker and the name, floored so a narrow sidebar
        // still shows something clickable.
        let picker_w = 132.0_f32.min(rect.width() * 0.45);
        let tab_zone_w = if behind.is_empty() {
            0.0
        } else {
            (rect.width() - picker_w) * 0.55 / behind_n
        };

        // Picker, on its own child `Ui` at the left of the strip.
        let picker_rect = egui::Rect::from_min_size(rect.min, egui::vec2(picker_w, rect.height()));
        let mut picker_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(picker_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let label = match front {
            Some(tab) => format!("{}  {}", tab.icon(), tr(tab.title_key())),
            None => tr("menu-zone-none"),
        };
        picker_ui.menu_button(egui::RichText::new(label).size(15.0), |ui| {
            for tab in crate::dock::Tab::all() {
                let sel = front == Some(tab);
                // Where the card sits in the stack, as a number. This used
                // to be a caret glyph the UI font does not carry, so it
                // rendered as an empty box.
                let place = stack.iter().position(|t| *t == tab).map(|i| i + 1);
                let marker = match place {
                    Some(n) => format!("{n}) "),
                    None => "  ".to_string(),
                };
                if ui
                    .selectable_label(sel, format!("{marker}{}", tr(tab.title_key())))
                    .clicked()
                {
                    zone.set(&mut self.modern, tab);
                    ui.close_menu();
                }
            }
            ui.separator();
            if ui
                .selectable_label(stack.is_empty(), tr("menu-zone-none"))
                .clicked()
            {
                zone.clear_zone(&mut self.modern);
                ui.close_menu();
            }
        });

        // The name, centred in what the tabs do not cover.
        if let Some(tab) = front {
            let name_w = (rect.width() - picker_w - tab_zone_w * behind_n).max(40.0);
            let name_rect = egui::Rect::from_min_size(
                egui::pos2(picker_rect.max.x, rect.min.y),
                egui::vec2(name_w, rect.height()),
            );
            let mut name_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(name_rect)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            name_ui.label(egui::RichText::new(tr(tab.title_key())).strong().size(15.0));
        }

        // Background tabs, standing proud of the strip's top edge, laid out
        // from the right edge leftwards so the add/remove button always sits
        // past the last of them.
        let painter = ui.painter_at(rect);
        let tab_h = rect.height() + 6.0;
        const CTRL_W: f32 = 22.0;
        for (i, tab) in behind.iter().enumerate() {
            let x0 = rect.max.x - CTRL_W - (behind_n - i as f32) * tab_zone_w;
            let tab_rect = egui::Rect::from_min_size(
                egui::pos2(x0, rect.min.y - 6.0),
                egui::vec2(tab_zone_w, tab_h),
            );
            let dims = 0.9 - i as f32 * 0.12;
            let tab_rounding = egui::CornerRadius {
                nw: 6,
                ne: 6,
                sw: 0,
                se: 0,
            };
            painter.rect_filled(tab_rect, tab_rounding, accent.gamma_multiply(0.30 * dims));
            painter.rect_stroke(
                tab_rect,
                tab_rounding,
                egui::Stroke::new(1.0, accent.gamma_multiply(0.8 * dims)),
                egui::StrokeKind::Inside,
            );
            painter.text(
                tab_rect.center(),
                egui::Align2::CENTER_CENTER,
                format!("{}", i + 2),
                egui::FontId::proportional(12.0),
                ui.visuals().text_color().gamma_multiply(dims),
            );
            if ui
                .interact(
                    tab_rect,
                    ui.id().with(("zone_tab", tab.title_key())),
                    egui::Sense::click(),
                )
                .on_hover_text(tr(tab.title_key()))
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                zone.promote(&mut self.modern, *tab);
            }
        }

        // Add / remove, past every stacked tab on the right.
        //
        // Two controls, not one that swaps. `+` stays rightmost as asked;
        // `−` sits just inside it and is live at every depth above one.
        // A single swapping button could only remove at three, so a stack
        // that reached three could come down to two and then no further --
        // there was no way back to a single card.
        let mut ctrl_x = rect.max.x;
        if stack.len() < 3 {
            let plus_rect = egui::Rect::from_min_size(
                egui::pos2(ctrl_x - CTRL_W, rect.min.y),
                egui::vec2(CTRL_W, rect.height()),
            );
            let mut plus_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(plus_rect)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            if plus_ui
                .add(egui::Button::new(egui::RichText::new("+").size(13.0)))
                .on_hover_text(tr("menu-zone-add-stack"))
                .clicked()
            {
                if let Some(next) = crate::dock::Tab::all()
                    .into_iter()
                    .find(|t| !stack.contains(t))
                {
                    zone.set(&mut self.modern, next);
                }
            }
            ctrl_x -= CTRL_W;
        }
        if stack.len() > 1 {
            let minus_rect = egui::Rect::from_min_size(
                egui::pos2(ctrl_x - CTRL_W, rect.min.y),
                egui::vec2(CTRL_W, rect.height()),
            );
            let mut minus_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(minus_rect)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            if minus_ui
                .add(egui::Button::new(egui::RichText::new("-").size(13.0)))
                .on_hover_text(tr("menu-zone-remove-stack"))
                .clicked()
            {
                zone.drop_back(&mut self.modern);
            }
            ctrl_x -= CTRL_W;
        }
        let _ = ctrl_x;
    }

    /// Body of one panel, whichever zone it is sitting in.
    ///
    /// The asset tabs go through the shared renderer so the filter
    /// sidebar, the active category and the card size are one set of
    /// state across both editors rather than two that drift.
    fn render_tab_body(&mut self, ui: &mut egui::Ui, tab: crate::dock::Tab) {
        use crate::dock::Tab;
        use crate::panels::asset_browser::AssetTab;
        match tab {
            Tab::AssetMedia => self.render_assets_panel(ui, AssetTab::Media),
            Tab::AssetTransitions => self.render_assets_panel(ui, AssetTab::Transitions),
            Tab::AssetEffects => self.render_assets_panel(ui, AssetTab::Effects),
            Tab::AssetFilters => self.render_assets_panel(ui, AssetTab::Filters),
            Tab::AssetText => self.render_assets_panel(ui, AssetTab::Text),
            Tab::AssetTemplates => self.render_assets_panel(ui, AssetTab::Templates),
            Tab::AssetPlugins => self.render_assets_panel(ui, AssetTab::Plugins),
            Tab::AssetSfx => self.render_assets_panel(ui, AssetTab::Sfx),
            Tab::AssetStickers => self.render_assets_panel(ui, AssetTab::Stickers),
            Tab::Preview => self.render_preview_panel(ui),
            Tab::Properties => self.render_properties_panel(ui),
            Tab::Timeline => self.render_timeline_panel(ui),
            Tab::MasterChain => self.render_master_chain_panel(ui),
            Tab::MultiCam => self.render_multicam_panel(ui),
        }
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
                        PendingEdit::CaptionSegmentAdd {
                            idx,
                            start_ms,
                            end_ms,
                            text,
                        } => {
                            let c = caprust_core::commands::caption_segments::AddCaptionSegmentCommand::new(
                                id, idx, start_ms, end_ms, text,
                            );
                            let _ = self.undo_stack.execute(Box::new(c), &mut self.project);
                        }
                        PendingEdit::CaptionSegmentRemove { idx } => {
                            let c = caprust_core::commands::caption_segments::RemoveCaptionSegmentCommand::new(
                                id, idx,
                            );
                            let _ = self.undo_stack.execute(Box::new(c), &mut self.project);
                        }
                        PendingEdit::CaptionSegmentSplit { idx, at_ms } => {
                            let c = caprust_core::commands::caption_segments::SplitCaptionSegmentCommand::new(
                                id, idx, at_ms,
                            );
                            let _ = self.undo_stack.execute(Box::new(c), &mut self.project);
                        }
                        PendingEdit::CaptionSegmentMerge { idx } => {
                            let c = caprust_core::commands::caption_segments::MergeCaptionSegmentsCommand::new(
                                id, idx,
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
                        PendingEdit::StartAnon => {
                            self.start_anon_job(id);
                        }
                        PendingEdit::ClearAnon => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .anon_mask(None);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::StartBeatAutocut => {
                            self.auto_cut_to_beats(id);
                        }
                        PendingEdit::StartBgRemoval => {
                            self.start_bg_removal_job(id);
                        }
                        PendingEdit::StartStab => {
                            self.start_stab_job(id);
                        }
                        PendingEdit::ClearStab => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .stab_trf(None);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
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
                        PendingEdit::SetLut(lut) => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .lut(Some(lut));
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::ClearLut => {
                            let cmd =
                                caprust_core::commands::set_clip::SetClipCommand::new(id).lut(None);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::BlendMode(m) => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .blend_mode(m);
                            let _ = self.undo_stack.execute(Box::new(cmd), &mut self.project);
                        }
                        PendingEdit::ColorGrade(g) => {
                            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id)
                                .color_grade(Some(g));
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
                                PendingEdit::Overlay(v) => cmd.overlay(v),
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
    /// Track index stickers land on: the overlay lane, created on first use.
    ///
    /// `TrackKind::Overlay` already existed for watermarks and logos and
    /// is the only lane meant to sit above the picture rather than be it.
    /// A sticker is exactly that, so it goes there instead of onto a
    /// video lane where it would push the footage around.
    fn sticker_track_index(&mut self) -> usize {
        // It used to be the pinned Overlay lane, on the theory that a
        // sticker belongs above the picture. Two things came out of that:
        // the singleton Overlay lane is not where anyone looks for a clip
        // they just added, and RippleInsertCommand treats Overlay as a
        // video lane, so inserting there moved the whole edit sideways. A
        // video lane is where clips live, and `Clip.overlay` is what keeps
        // a sticker small and positioned whatever lane it sits on.
        if let Some(i) = self
            .project
            .tracks
            .iter()
            .rposition(|t| t.kind == caprust_core::TrackKind::Video)
        {
            return i;
        }
        let name = format!("V{}", self.project.tracks.len() + 1);
        self.project.tracks.push(caprust_core::Track::new(
            &name,
            caprust_core::TrackKind::Video,
        ));
        self.project.tracks.len() - 1
    }

    fn handle_asset_browser_output(
        &mut self,
        out: crate::panels::asset_browser::AssetBrowserOutput,
    ) {
        // Template actions from the Templates tab.
        if let Some(path) = out.template_delete_requested {
            match caprust_core::template::delete_template(&path) {
                Ok(()) => self.toast(tr("toast-template-deleted")),
                Err(e) => self.toast_error(format!("{}: {e:#}", tr("toast-template-failed"))),
            }
        }
        if out.template_import_requested {
            if let Some(src) = rfd::FileDialog::new()
                .add_filter("Template", &[caprust_core::template::TEMPLATE_EXT])
                .pick_file()
            {
                match caprust_core::template::import_template_file(&src) {
                    Ok(dst) => self.toast(format!(
                        "{}: {}",
                        tr("toast-template-imported"),
                        dst.display()
                    )),
                    Err(e) => self.toast_error(format!("{}: {e:#}", tr("toast-template-failed"))),
                }
            }
        }
        if out.template_save_requested {
            let suggestion = format!("{} template", self.project.name);
            self.template_save = Some(suggestion);
        }
        if let Some(req) = out.template_apply_requested {
            self.template_confirm = Some(req);
        }
        if let Some(id) = out.sfx_download_requested {
            self.start_sfx_download(id);
        }

        // Master-chain plugin add request from the browser.
        if let Some(info) = out.plugin_add_requested {
            // The browser only lists scanned dirs, but the path still
            // comes from the filesystem — refuse anything outside the
            // effective scan set (defaults + configured custom dir).
            let extra = self
                .settings
                .clap_dir
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .map(std::path::Path::new);
            let dirs = caprust_media_io::clap_host::effective_scan_dirs(extra);
            if !caprust_media_io::clap_host::is_trusted_plugin_path_in(&info.path, &dirs) {
                tracing::warn!("refusing untrusted plugin path {}", info.path.display());
                self.toast_error(format!("{}: {}", tr("toast-plugin-untrusted"), info.name));
            } else {
                let inst = caprust_core::plugin::PluginInstance::new(
                    info.id.clone(),
                    info.path.clone(),
                    info.name.clone(),
                );
                let cmd =
                    caprust_core::commands::add_master_plugin::AddMasterPluginCommand::new(inst);
                if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                    tracing::warn!("add master plugin failed: {e:#}");
                } else {
                    tracing::info!("added master plugin: {}", info.name);
                }
            }
        }

        if out.media.skipped_duplicates > 0 {
            self.toast(tr("media-import-duplicates-skipped"));
        }
        if let Some((found, missing)) = out.media.refreshed {
            if missing == 0 {
                self.toast(format!("{}: {found}", tr("media-refresh-ok")));
            } else {
                self.toast_error(format!(
                    "{}: {missing}, {}: {found}",
                    tr("media-refresh-missing"),
                    tr("media-refresh-found")
                ));
            }
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

                    // Stickers carry their own click path: a sticker is not a preset
                    // applied to the selection, it is a clip placed on the timeline.
                    AssetTab::Media
                    | AssetTab::Templates
                    | AssetTab::Plugins
                    | AssetTab::Sfx
                    | AssetTab::Stickers => {}
                }
            } else {
                tracing::warn!("preset '{preset_id}' clicked but no clip selected");
            }
        }

        // ---- Stickers ----------------------------------------------------
        // A sticker is not a preset applied to whatever happens to be
        // selected. It is a mark put at a place in the frame, so clicking
        // one puts a clip on the timeline at the playhead, on the overlay
        // lane, with a placement already set.
        if let Some(id) = out.sticker_clicked {
            match caprust_core::stickers::materialize(&id) {
                Ok(path) => {
                    let lane = self.sticker_track_index();

                    // A sticker is a still, so its "duration" is how long
                    // it stays on screen. Three seconds is the length a
                    // badge wants by default; the user drags the edge like
                    // any other clip.
                    let mut clip = caprust_core::Clip::new_image(
                        &path.to_string_lossy(),
                        lane,
                        self.playhead_ms,
                        3000,
                    );
                    let mut place = caprust_core::OverlayPlacement::default_sticker();
                    place.aspect = caprust_core::stickers::aspect(&id);
                    clip.overlay = Some(place);
                    clip.source_duration_ms = 0;

                    // Not RippleInsertCommand: it shifts every audio,
                    // caption and text clip that overlapped, so adding a
                    // sticker moved the whole edit sideways.
                    let cmd = caprust_core::commands::add_clip::AddClipCommand::new(clip);
                    // Not `let _ =`: swallowing this is what made a failed
                    // insert look exactly like a click that did nothing,
                    // which is the report this was chased from.
                    match self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                        Ok(()) => {
                            self.selected_clips.clear();
                            tracing::info!("placed sticker '{id}' on lane {lane}");
                            self.toast(format!("{}: {}", tr("toast-sticker-placed"), id));
                        }
                        Err(e) => {
                            tracing::error!("sticker '{id}' insert failed: {e:#}");
                            self.toast_error(format!("{}: {e:#}", tr("toast-sticker-failed")));
                        }
                    }
                }
                Err(e) => self.toast_error(format!("{}: {e}", tr("toast-sticker-failed"))),
            }
        }
    }

    /// Recompute the CLAP scan set from Settings and invalidate the
    /// plugin browser when it changed (custom dir added/removed).
    /// Called before every asset-browser render so both layouts stay
    /// in sync without signature churn in the panel functions.
    fn sync_plugin_scan_dirs(&mut self) {
        let extra = self
            .settings
            .clap_dir
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(std::path::Path::new);
        let dirs = caprust_media_io::clap_host::effective_scan_dirs(extra);
        let pb = &mut self.asset_browser.plugin_browser;
        if pb.scan_dirs != dirs {
            pb.scan_dirs = dirs;
            pb.scanned = false;
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
        self.sync_plugin_scan_dirs();
        let tab_layout = self.media_bin.tab_layout;
        let out = crate::panels::asset_browser::render_tab_content(
            ui,
            tab,
            &mut self.project,
            &mut self.asset_browser,
            &mut self.media_bin,
            tab_layout,
        );
        self.handle_asset_browser_output(out);
    }

    /// Draw and manipulate the selected sticker over the preview frame.
    ///
    /// Paint every sticker the playhead is over, then put handles on the
    /// selected one.
    ///
    /// Stickers are painted here, by egui, rather than left to ffmpeg.
    /// That is what makes a drag feel immediate: compositing a PNG over a
    /// frame is work the renderer does every frame anyway, while the same
    /// change in the filtergraph means tearing down and rebuilding an
    /// encoder process. The only way to make that cheaper was to lower
    /// the preview quality, which costs the rest of the picture and is a
    /// global setting for what is a one-mark problem.
    ///
    /// ffmpeg still composites stickers for the export, from the same
    /// `OverlayPlacement`. The preview plan leaves sticker clips out for
    /// exactly this reason, and `preview_hash` is the matching half: two
    /// copies of one mark in a frame would show a stale one whenever the
    /// encoder had not caught up yet.
    ///
    /// Which is why the painting may not be conditional on a selection.
    /// Painting only the selected sticker, on top of dropping it from
    /// ffmpeg, made every sticker in the edit invisible: the preview went
    /// blank as soon as a sticker stopped being selected, which is most of
    /// the time. The handles follow the selection. The picture does not.
    fn preview_sticker_drag(&mut self, ui: &mut egui::Ui, frame_rect: egui::Rect) {
        let fa = frame_rect.width() / frame_rect.height().max(1.0);
        self.preview_stickers_paint(ui, frame_rect);
        self.preview_sticker_handles(ui, frame_rect, fa);
    }

    /// Draw every sticker on a visible track that the playhead is over.
    /// Draw every sticker that belongs on screen over the frame.
    ///
    /// Which stickers those are, and where each one goes, comes from
    /// `sticker_quads`, which takes the project and the playhead and
    /// nothing else. All that is left here is loading the pixels, so
    /// there is nowhere for a "but only if it is selected" to hide.
    fn preview_stickers_paint(&mut self, ui: &mut egui::Ui, frame_rect: egui::Rect) {
        let quads = sticker_quads(&self.project, self.playhead_ms, frame_rect);
        // What happened to each one, not how many were queued: a quad
        // whose PNG would not decode is not a sticker on screen and
        // counting it as one is exactly the blind spot that let a
        // failing texture read as a correctly placed one.
        let outcomes: Vec<String> = quads
            .iter()
            .map(|q| self.paint_sticker_quad(ui, q))
            .collect();
        self.log_sticker_overlay(quads.len(), frame_rect, &outcomes);
    }

    /// Why the overlay drew what it drew, once every few seconds.
    ///
    /// A sticker that is not on screen looks identical whether the playhead
    /// missed it, its track went invisible, the PNG would not decode or
    /// the drawing happened behind the picture, and guessing between them
    /// from the outside has been wrong every time.
    fn log_sticker_overlay(&mut self, queued: usize, frame_rect: egui::Rect, outcomes: &[String]) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static LAST: AtomicU64 = AtomicU64::new(0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if now < LAST.load(Ordering::Relaxed) + 5 {
            return;
        }
        LAST.store(now, Ordering::Relaxed);

        let ph = self.playhead_ms;
        let mut why: Vec<String> = Vec::new();
        for c in self.project.clips.iter().filter(|c| c.overlay.is_some()) {
            let on = ph >= c.start_time_ms && ph < c.start_time_ms + c.duration_ms;
            let vis = self
                .project
                .tracks
                .get(c.track_index)
                .map(|t| t.visible)
                .unwrap_or(true);
            let path = match &c.clip_type {
                caprust_core::ClipType::Image { path, .. } => path.clone(),
                _ => String::from("<not an image>"),
            };
            why.push(format!(
                "{} @{}ms..{}ms track{} vis={} -> {}",
                &path.rsplit(['\\', '/']).next().unwrap_or(&path),
                c.start_time_ms,
                c.start_time_ms + c.duration_ms,
                c.track_index,
                vis,
                if on && vis {
                    "DRAWN"
                } else if !on {
                    "off playhead"
                } else {
                    "track hidden"
                },
            ));
        }
        tracing::info!(
            "sticker overlay: queued={queued} of {} at playhead={}ms frame={:.0}x{:.0} texture={} | {} | {}",
            why.len(),
            ph,
            frame_rect.width(),
            frame_rect.height(),
            self.preview_player.texture.is_some(),
            why.join(" ; "),
            if outcomes.is_empty() {
                String::from("nothing queued")
            } else {
                outcomes.join(" ; ")
            },
        );
    }

    /// Handles and manipulation, for the selected sticker only.
    fn preview_sticker_handles(&mut self, ui: &mut egui::Ui, frame_rect: egui::Rect, fa: f32) {
        let Some(clip_id) = self.selected_clips.first().copied() else {
            self.sticker_drag = None;
            return;
        };
        let Some(clip) = self.project.clips.iter().find(|c| c.id == clip_id) else {
            self.sticker_drag = None;
            return;
        };
        let Some(place) = clip.overlay.clone() else {
            self.sticker_drag = None;
            return;
        };
        // Only while the playhead is over it. The sticker is not on
        // screen the rest of the time, and drawing it anyway would put a
        // mark in a frame that has none.
        let ph = self.playhead_ms;
        if !(ph >= clip.start_time_ms && ph < clip.start_time_ms + clip.duration_ms) {
            self.sticker_drag = None;
            return;
        }

        // The angle on screen is not always the angle in the model: while
        // the rotation handle is down, the pointer's angle is ahead of
        // the model until the next line runs.
        let live = self.sticker_live_rotation(clip_id, place.rotation);
        // The corners of the sticker itself, not of the box around it.
        // On a turned sticker the box's corners are empty space, up to
        // half a sticker-width away from the mark, which is why the
        // grips stopped responding after a rotation.
        let corners = sticker_corners(&place, frame_rect, live - place.rotation);
        let bounds = corners_bounds(&corners);
        // The centre the gestures work in, in frame fractions. The
        // pointer stays in pixels; these two meet only through `to_unit`.
        let centre = to_unit(bounds.center(), frame_rect);

        // --- Handles ---
        // Drawn whenever the sticker is selected, not on hover. A handle
        // that appears only once the pointer is already on it is a handle
        // nobody finds.
        const GRIP: f32 = 12.0;
        let accent = self.theme.accent_color();
        let stroke = egui::Stroke::new(1.5, accent);
        let rot_rect = rotation_grip_rect(bounds);

        // ONE interact rect for the whole overlay, and the zone worked
        // out from the pointer.
        //
        // Four corner rects, a rotation rect and a body rect all overlap
        // here, and egui settles that by registration order: the piece
        // added last takes the pointer. Whichever way that falls, half of
        // each corner is unreachable and a gesture silently fails on the
        // other pieces. That is how this read -- resize worked because
        // its grips stuck out past the body, move never started because
        // the body was `Sense::click()`, which is not draggable, and
        // rotate had a 16px target the body sat next to. Deciding the
        // zone ourselves removes the question.
        let resp = ui.interact(
            bounds.union(rot_rect),
            egui::Id::new(("sticker_overlay", clip_id)),
            egui::Sense::click_and_drag(),
        );

        // The outline is the sticker's own turned rectangle, closed, and
        // the grips sit on its corners -- so they are on the thing the
        // user is looking at at any angle.
        let outline = [corners[0], corners[1], corners[2], corners[3], corners[0]];
        for pair in outline.windows(2) {
            ui.painter().line_segment([pair[0], pair[1]], stroke);
        }
        for corner in corners {
            ui.painter().rect_stroke(
                egui::Rect::from_center_size(corner, egui::vec2(GRIP, GRIP)),
                0.0,
                stroke,
                egui::StrokeKind::Middle,
            );
        }
        // The stem reaches the topmost corner, which after a rotation is
        // no longer directly above the middle.
        let top = corners
            .iter()
            .copied()
            .fold(corners[0], |a, b| if b.y < a.y { b } else { a });
        ui.painter().line_segment([top, rot_rect.center()], stroke);
        ui.painter()
            .circle_filled(rot_rect.center(), GRIP * 0.42, accent);

        let pointer = resp.interact_pointer_pos();
        let under = pointer.map(|p| sticker_handle_at(p, corners, GRIP, rot_rect));

        // Latched at the start of a drag, so a gesture does not change
        // meaning halfway through because the pointer crossed a corner.
        let active = self
            .sticker_drag
            .as_ref()
            .filter(|d| d.clip_id == clip_id)
            .map(|d| d.mode);
        let mode = match (resp.dragged(), active) {
            (true, Some(m)) => m,
            (true, None) => under.unwrap_or(StickerMode::Move),
            (false, _) => StickerMode::Move,
        };

        if resp.hovered() || mode != StickerMode::Move {
            let icon = match mode {
                StickerMode::Rotate => egui::CursorIcon::Grab,
                StickerMode::Scale => egui::CursorIcon::Crosshair,
                StickerMode::Move => egui::CursorIcon::Move,
            };
            ui.ctx().set_cursor_icon(icon);
        }

        // --- Gestures ---
        if resp.dragged() {
            match mode {
                StickerMode::Rotate => {
                    let p = pointer.unwrap_or(bounds.center());
                    let ang = (p.y - bounds.center().y)
                        .atan2(p.x - bounds.center().x)
                        .to_degrees();
                    // The handle is drawn straight up, so straight up is
                    // zero rather than atan2's rightward zero. Egui's y
                    // points down, so this is clockwise on screen.
                    let mut deg = (ang + 90.0).rem_euclid(360.0);
                    // Shift snaps to 15 degrees, which is what every
                    // editor does and the only way to get anything
                    // straight.
                    if ui.input(|i| i.modifiers.shift) {
                        deg = (deg / 15.0).round() * 15.0;
                    }
                    self.drag_sticker(
                        clip_id,
                        StickerGeom {
                            scale: place.scale,
                            centre,
                            aspect: place.aspect,
                            frame_aspect: fa,
                            rotation: deg,
                        },
                        StickerMode::Rotate,
                    );
                }
                StickerMode::Scale => {
                    // Size is the pointer's distance from where the box
                    // was centred when the gesture began, so the centre
                    // holds still and the sticker grows in place.
                    let p = pointer.unwrap_or(bounds.center());
                    let d = p - bounds.center();
                    // The placement's own limit, not a constant. A sticker
                    // is not allowed to grow past what fits in the frame,
                    // because past that its position has no solution and
                    // the next thing to touch it moves it somewhere
                    // unrelated -- which is what a resize used to do.
                    let max = place.max_scale(fa);
                    let scale = (d.length() / frame_rect.height().max(1.0)).clamp(0.02, max);
                    self.drag_sticker(
                        clip_id,
                        StickerGeom {
                            scale,
                            centre,
                            aspect: place.aspect,
                            frame_aspect: fa,
                            rotation: live,
                        },
                        StickerMode::Scale,
                    );
                }
                StickerMode::Move => {
                    let d = resp.drag_delta();
                    // Held just inside the frame, with a little overhang
                    // so a mark can be pushed to the edge on purpose.
                    //
                    // It used to be allowed to go to +/-1 frame away, on
                    // the reasoning that a clamp of 2.0 was generous. It
                    // is not: the sticker ended up at x = 2.33, which is
                    // past what the overlay filter accepts, so the export
                    // and the preview disagreed and the mark was parked
                    // off-screen with no way to drag it back.
                    let clamped = clamp_centre(
                        centre.0 + d.x / frame_rect.width().max(1.0),
                        centre.1 + d.y / frame_rect.height().max(1.0),
                    );
                    let (cx, cy) = (clamped.x, clamped.y);
                    self.drag_sticker(
                        clip_id,
                        StickerGeom {
                            scale: place.scale,
                            centre: (cx, cy),
                            aspect: place.aspect,
                            frame_aspect: fa,
                            rotation: live,
                        },
                        StickerMode::Move,
                    );
                }
            }
        }

        // Release: one command for the whole gesture.
        if self.sticker_drag.is_some() && !resp.dragged() {
            if let Some(d) = self.sticker_drag.take() {
                let moved = d.from.scale != d.to.scale
                    || d.from.x != d.to.x
                    || d.from.y != d.to.y
                    || d.from.rotation != d.to.rotation;
                if moved {
                    let cmd = caprust_core::commands::move_overlay::MoveOverlayCommand::new(
                        d.clip_id, d.from, d.to,
                    );
                    if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                        tracing::error!("sticker drag failed: {e}");
                    }
                }
            }
        }
    }

    /// The angle to draw this frame, which during a rotation drag is the
    /// model's plus whatever the pointer is asking for on top.
    fn sticker_live_rotation(&self, clip_id: uuid::Uuid, model: f32) -> f32 {
        match &self.sticker_drag {
            Some(d) if d.clip_id == clip_id && d.mode == StickerMode::Rotate => {
                model + d.extra_degrees
            }
            _ => model,
        }
    }

    /// Paint one resolved sticker quad.
    ///
    /// All the geometry is already done by `sticker_quads`; this is just
    /// pixels into a texture and two triangles.
    fn paint_sticker_quad(&mut self, ui: &mut egui::Ui, q: &StickerQuad) -> String {
        // An animated sticker has to keep repainting for as long as it is
        // on screen, or the motion happens once and then stops.
        if q.animation.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        let name = q
            .path
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&q.path)
            .to_string();
        let tex = match caprust_core::stickers::find(&q.path) {
            Some(_) => self.sticker_tex.load_embedded(ui.ctx(), &q.path),
            None => self
                .sticker_tex
                .load_path(ui.ctx(), &q.path, std::path::Path::new(&q.path)),
        };
        // The source size, for scaling the animation's offsets, which the
        // filtergraph expresses in the sticker's own pixels.
        let src_px = caprust_core::stickers::find(&q.path)
            .and_then(|s| caprust_core::stickers::size(s.id).map(|(w, _)| w as f32))
            .unwrap_or(256.0);
        let screen_px = (q.corners[2].x - q.corners[0].x)
            .abs()
            .max((q.corners[2].y - q.corners[0].y).abs());
        let motion = sticker_motion(q.animation.as_deref(), q.t_sec, src_px, screen_px);

        let Some(tex) = tex else {
            let embedded = caprust_core::stickers::find(&q.path).is_some();
            return format!(
                "{name}: NOT PAINTED, {}",
                if embedded {
                    "no bundled sticker with that id"
                } else {
                    "PNG would not decode or is missing"
                }
            );
        };
        // The resting corners, then the motion on top: the same
        // translate, scale and turn the filtergraph applies, in the same
        // order.
        let (tl, tr, br, bl) = (q.corners[0], q.corners[1], q.corners[2], q.corners[3]);
        let centre = egui::Rect::from_min_max(tl, br).center();
        let (sin, cos) = motion.angle.sin_cos();
        let place = |p: egui::Pos2| {
            let v = p - centre;
            egui::pos2(
                centre.x + motion.dx + v.x * cos * motion.scale - v.y * sin * motion.scale,
                centre.y + motion.dy + v.x * sin * motion.scale + v.y * cos * motion.scale,
            )
        };
        let (tl, tr, br, bl) = (place(tl), place(tr), place(br), place(bl));

        let white =
            egui::Color32::from_white_alpha((motion.alpha.clamp(0.0, 1.0) * 255.0).round() as u8);
        // A mesh vertex's uv is a Pos2 -- it is the texel coordinate, not
        // an offset. egui's image widgets only do axis-aligned rects, and
        // the turn is the entire point.
        let v = |p: egui::Pos2, u: f32, w: f32| egui::epaint::Vertex {
            pos: p,
            uv: egui::epaint::Pos2::new(u, w),
            color: white,
        };
        let mesh = egui::epaint::Mesh {
            texture_id: tex.id(),
            vertices: vec![
                v(tl, 0.0, 0.0),
                v(bl, 0.0, 1.0),
                v(tr, 1.0, 0.0),
                v(tr, 1.0, 0.0),
                v(bl, 0.0, 1.0),
                v(br, 1.0, 1.0),
            ],
            indices: vec![0, 1, 2, 3, 4, 5],
        };
        ui.painter().add(egui::Shape::mesh(mesh));
        format!(
            "{name}: painted at {:.0},{:.0} {:.0}x{:.0} anim={:?}",
            q.corners[0].x,
            q.corners[0].y,
            (q.corners[2].x - q.corners[0].x).abs(),
            (q.corners[2].y - q.corners[0].y).abs(),
            q.animation,
        )
    }

    /// Where the box was centred when this gesture started, in
    /// unit-frame coordinates. Frozen: measuring size against a live
    /// centre feeds the sticker's own growth back into its size and the
    /// grip runs away from the pointer.
    fn sticker_drag_origin(&self, clip_id: uuid::Uuid, current: (f32, f32)) -> (f32, f32) {
        match &self.sticker_drag {
            Some(d) if d.clip_id == clip_id => d.origin_centre,
            _ => current,
        }
    }

    /// Move the clip straight, no command. The command goes on the undo
    /// stack once, at the end.
    ///
    /// `centre` is where the gesture wants the box's centre now, but it
    /// is only honoured for a move. A scale or a rotate happens about the
    /// centre the gesture started from, which is what makes the sticker
    /// grow in place instead of sliding away from the pointer.
    fn drag_sticker(&mut self, clip_id: uuid::Uuid, g: StickerGeom, mode: StickerMode) {
        let StickerGeom {
            scale,
            centre,
            aspect,
            frame_aspect,
            rotation,
        } = g;
        let origin = self.sticker_drag_origin(clip_id, centre);
        let Some(clip) = self.project.clips.iter_mut().find(|c| c.id == clip_id) else {
            return;
        };
        let Some(current) = clip.overlay.clone() else {
            return;
        };
        // Remember where the gesture began, not where it is now, or undo
        // would have nowhere to travel back to.
        let from = match &self.sticker_drag {
            Some(d) if d.clip_id == clip_id => d.from.clone(),
            _ => current,
        };
        let (cx, cy) = match mode {
            StickerMode::Move => centre,
            StickerMode::Scale | StickerMode::Rotate => origin,
        };
        // While rotating, `rotation` is the absolute angle. For a move
        // or a scale the angle is whatever the clip already had.
        let next_rotation = match mode {
            StickerMode::Rotate => rotation,
            _ => from.rotation,
        };
        // What the live drawing needs on top of the stored angle.
        let extra_degrees = next_rotation - from.rotation;

        let mut next = caprust_core::OverlayPlacement::at_centre(
            cx,
            cy,
            scale,
            aspect,
            frame_aspect,
            next_rotation,
        );
        // Geometry only. The animation belongs to the clip and a drag
        // has no business clearing it.
        next.animation = from.animation.clone();
        clip.overlay = Some(next.clone());
        self.sticker_drag = Some(StickerDrag {
            clip_id,
            from,
            to: next,
            origin_centre: origin,
            extra_degrees,
            mode,
        });
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
            // Not render_hash: a sticker is composited by the preview
            // panel and never reaches ffmpeg, so moving one changes no
            // pixel of this output. Hashing placement here would spawn an
            // encoder on every frame of a drag to produce the same frame.
            let live = self.project.preview_hash();
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

                // Full A/V restart on every seek (not just video):
                // drop the old audio player so it stops playing the
                // pre-seek position during the 0.5-4 s video re-prime,
                // and clear the anchor so the first new frame re-anchors
                // the wall clock and fires the deferred audio start.
                // Without this the old audio kept playing against the
                // new video (or the new PCM never started at all),
                // which is exactly the scrub-while-playing desync.
                self.audio_player = None;
                self.play_anchor_set = false;
                self.audio_baseline_ms = 0;

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
                    // Stickers are drawn by the preview panel, not by
                    // ffmpeg, so they come out of the preview graph. Left
                    // in, every mark appears twice -- once stale from the
                    // last ffmpeg pass, once current from egui.
                    let previewable = self.project.without_overlay_clips();
                    match caprust_media_io::export_graph::plan_from_project(
                        &previewable,
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
                                    if e.to_string().contains("disk full") {
                                        self.toast_error(tr("toast-disk-full"));
                                    }
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
            // Without the timeout this can deadlock: the audio player
            // waits for PCM from the same ffmpeg child whose stdout we
            // are holding because we are waiting for the audio player.
            // A silent 10 % CPU preview and no frames is the symptom.
            let audio_playing = self
                .audio_player
                .as_ref()
                .is_none_or(|ap| ap.playhead_ms() > 0)
                || self
                    .playback_started_at
                    .map(|t| t.elapsed() >= std::time::Duration::from_millis(3_000))
                    .unwrap_or(false);
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
                        // publish_frame, not a bare texture assignment:
                        // RemoteSession::preview_frame reads the pixels
                        // from the player, so a frame shown without being
                        // captured is invisible over MCP -- which is what
                        // made preview_frame answer null while the
                        // desktop played happily.
                        self.preview_player.publish_frame(
                            &ctx,
                            "preview-timeline",
                            w,
                            h,
                            &buf[..expected],
                        );

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

            // The comment above said "stop audio" and the code did not.
            // `stop_stream` only ends the video decoder; the cpal
            // player keeps pulling from its own PCM file on its own
            // thread, so pausing left the audio running against a frozen
            // frame. Dropping it signals the reader and joins it.
            // Recreated on the next play, so nothing is lost.
            self.audio_player = None;

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
                    // Stickers are drawn by the preview panel, not by
                    // ffmpeg, so they come out of the preview graph. Left
                    // in, every mark appears twice -- once stale from the
                    // last ffmpeg pass, once current from egui.
                    let previewable = self.project.without_overlay_clips();
                    match caprust_media_io::export_graph::plan_from_project(
                        &previewable,
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
                                    if e.to_string().contains("disk full") {
                                        self.toast_error(tr("toast-disk-full"));
                                    }
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
                        self.preview_player.publish_frame(
                            &ctx,
                            "preview-timeline",
                            w,
                            h,
                            &buf[..expected],
                        );
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
                "FFmpeg not detected — set it in Settings → Paths".to_string()
            } else if clip_info.is_none() {
                if self.project.clips.is_empty() {
                    "No clips on timeline".to_string()
                } else {
                    "Playhead is not over a video clip".to_string()
                }
            } else if let Some(err) = &self.preview_player.last_error {
                // Surface the ffmpeg failure (missing decoder, bad
                // path, …) instead of sitting on "Decoding…" forever.
                format!("Decode failed: {err}")
            } else {
                "Decoding…".to_string()
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

        // ---- Stickers, drawn over the picture ----
        // After the frame, never before: the frame is painted into the
        // same clip region and covers anything already there. Drawing
        // the sticker at this point in the panel made it invisible on
        // every project, because it landed first and the video painted
        // straight over it.
        //
        // Positioned against `draw_rect` rather than `rect`: the video is
        // letterboxed inside the container, and a sticker's x/y are
        // fractions of the frame -- the picture -- not of the padding
        // around it. Against `rect` it drifted by the letterbox offset,
        // which is the same class of mistake as treating `scale` as a
        // fraction of both axes.
        let sticker_rect = overlay_ctx.map(|(r, _)| r).unwrap_or(rect);
        self.preview_sticker_drag(ui, sticker_rect);

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
        let can_save_frame = !self.preview.playing && self.preview_player.has_frame;
        let ev = crate::panels::preview_window::show_transport(
            ui,
            &mut self.preview,
            self.playhead_ms,
            total_ms,
            &mut self.project.aspect_ratio,
            self.settings.muted,
            self.settings.master_volume,
            can_save_frame,
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
                // Same omission as the pause branch: reaching the end
                // stopped the video and left the audio playing past the
                // last frame.
                self.audio_player = None;
                if let Some(mut r) = self.preview_renderer.take() {
                    r.kill();
                }
            }
        }
    }

    /// Start the encoder-capability probe if it has not run yet, and
    /// collect its result when it lands.
    ///
    /// Each candidate encoder costs one ffmpeg spawn (a 1-frame test
    /// encode). Nine of those inline blocked the UI thread for long
    /// enough that the export window never appeared, so the work
    /// happens on a detached thread and the panel simply shows a
    /// placeholder until the list arrives.
    fn poll_encoder_probe(&mut self) {
        use caprust_core::project::VideoEncoder;

        if let Some(rx) = &self.encoder_probe_rx {
            match rx.try_recv() {
                Ok(list) => {
                    tracing::info!("export: probed encoders available: {list:?}");
                    self.available_encoders = Some(list);
                    self.encoder_probe_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // Probe thread died; fall back to the CPU codecs
                    // so the export is still usable.
                    self.available_encoders = Some(vec![
                        VideoEncoder::H264Cpu,
                        VideoEncoder::H265Cpu,
                        VideoEncoder::Av1Cpu,
                    ]);
                    self.encoder_probe_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
            return;
        }

        if self.available_encoders.is_some() {
            return;
        }
        let Some(ffmpeg) = self.ffmpeg_status.ffmpeg.clone() else {
            self.available_encoders = Some(vec![
                VideoEncoder::H264Cpu,
                VideoEncoder::H265Cpu,
                VideoEncoder::Av1Cpu,
            ]);
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        self.encoder_probe_rx = Some(rx);
        std::thread::spawn(move || {
            let mut available = vec![
                VideoEncoder::H264Cpu,
                VideoEncoder::H265Cpu,
                VideoEncoder::Av1Cpu,
            ];
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
            let _ = tx.send(available);
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
                let project_dims = self.project.project_dimensions();
                // Encoder detection spawns ffmpeg once per candidate.
                // Doing that inline froze the UI for the whole probe and
                // the window never painted, so it runs on a worker and
                // the panel renders as soon as the result lands.
                self.poll_encoder_probe();
                let available = self.available_encoders.clone().unwrap_or_default();
                let probing = self.encoder_probe_rx.is_some();

                start_clicked = crate::panels::export_window::show(
                    ui,
                    &mut self.export_state,
                    &available,
                    total_ms,
                    clip_count,
                    project_dims,
                    probing,
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

        let dir = self.settings.effective_recordings_dir();
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

    fn show_template_save_window(&mut self, ctx: &egui::Context) {
        let Some(mut buf) = self.template_save.clone() else {
            return;
        };

        let mut commit = false;
        let mut cancel = false;

        egui::Window::new(tr("template-save-title"))
            .id(egui::Id::new("template_save_modal"))
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(360.0)
            .show(ctx, |ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut buf)
                        .desired_width(f32::INFINITY)
                        .hint_text(tr("template-save-hint")),
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
                        egui::RichText::new(tr("template-save-ok"))
                            .color(egui::Color32::WHITE)
                            .strong(),
                    )
                    .fill(egui::Color32::from_rgb(34, 139, 230));
                    if ui.add(ok).clicked() {
                        commit = true;
                    }
                    if ui.button(tr("template-save-cancel")).clicked() {
                        cancel = true;
                    }
                });
            });

        if cancel {
            self.template_save = None;
            return;
        }
        if commit {
            match caprust_core::template::save_template(&self.project, &buf) {
                Ok(path) => self.toast(format!(
                    "{}: {}",
                    tr("toast-template-saved"),
                    path.display()
                )),
                Err(e) => self.toast_error(format!("{}: {e:#}", tr("toast-template-failed"))),
            }
            self.template_save = None;
        } else {
            self.template_save = Some(buf);
        }
    }

    fn show_template_confirm_window(&mut self, ctx: &egui::Context) {
        let Some(req) = self.template_confirm.clone() else {
            return;
        };
        use crate::panels::asset_browser::TemplateRequest;

        let label = match &req {
            TemplateRequest::Bundled(id) => caprust_core::template::bundled_templates()
                .into_iter()
                .find(|t| t.id == *id)
                .map(|t| t.display.to_string())
                .unwrap_or_else(|| id.to_string()),
            TemplateRequest::User(p) => p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("template")
                .to_string(),
        };

        let mut apply = false;
        let mut cancel = false;

        egui::Window::new(tr("template-confirm-title"))
            .id(egui::Id::new("template_confirm_modal"))
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.label(format!("{}: {label}", tr("template-confirm-body")));
                ui.add_space(space::M);
                ui.horizontal(|ui| {
                    let ok = egui::Button::new(
                        egui::RichText::new(tr("template-confirm-apply"))
                            .color(egui::Color32::WHITE)
                            .strong(),
                    )
                    .fill(egui::Color32::from_rgb(34, 139, 230));
                    if ui.add(ok).clicked() {
                        apply = true;
                    }
                    if ui.button(tr("template-confirm-cancel")).clicked() {
                        cancel = true;
                    }
                });
            });

        if cancel {
            self.template_confirm = None;
            return;
        }
        if apply {
            let loaded = match &req {
                TemplateRequest::Bundled(id) => {
                    match caprust_core::template::bundled_templates()
                        .into_iter()
                        .find(|t| t.id == *id)
                    {
                        Some(t) => Some(caprust_core::template::parse_bundled(&t)),
                        None => {
                            tracing::error!("unknown bundled template {id}");
                            None
                        }
                    }
                }
                TemplateRequest::User(p) => Some(caprust_core::template::load_template(p)),
            };
            match loaded {
                Some(Ok(mut state)) => {
                    // The template becomes the timeline. Fresh undo
                    // history (same as opening a project), unsaved
                    // path so the next save asks where to write.
                    state.project_path = None;
                    // The media bin is the user's, not the template's.
                    // Assigning `state` wholesale dropped every
                    // imported item along with its cached probe,
                    // thumbnail and waveform flags, so the bin came
                    // back empty and the clips pointed at nothing.
                    // Merge the template's items in instead; the
                    // existing entries keep their cache flags.
                    let mut media = std::mem::take(&mut self.project.media);
                    media.merge_missing_from(&state.media);
                    state.media = media;
                    self.project = state;
                    self.undo_stack = UndoStack::new();
                    self.selected_clips.clear();
                    self.check_missing_media_on_load();
                    self.toast(format!("{}: {label}", tr("toast-template-applied")));
                }
                Some(Err(e)) => self.toast_error(format!("{}: {e:#}", tr("toast-template-failed"))),
                None => {}
            }
            self.template_confirm = None;
        }
    }

    fn show_clip_rename_window(&mut self, ctx: &egui::Context) {
        let Some((id, mut buf)) = self.clip_rename.clone() else {
            return;
        };

        let mut commit = false;
        let mut cancel = false;

        egui::Window::new(tr("clip-rename-title"))
            .id(egui::Id::new("clip_rename_modal"))
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(360.0)
            .show(ctx, |ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut buf)
                        .desired_width(f32::INFINITY)
                        .hint_text(tr("clip-rename-hint")),
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
                        egui::RichText::new(tr("clip-rename-ok"))
                            .color(egui::Color32::WHITE)
                            .strong(),
                    )
                    .fill(egui::Color32::from_rgb(34, 139, 230));
                    if ui.add(ok).clicked() {
                        commit = true;
                    }
                    if ui.button(tr("clip-rename-cancel")).clicked() {
                        cancel = true;
                    }
                });
            });

        if cancel {
            self.clip_rename = None;
            return;
        }
        if commit {
            let trimmed = buf.trim().to_string();
            let new_name = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            };
            let cmd = caprust_core::commands::set_clip::SetClipCommand::new(id).name(new_name);
            if let Err(e) = self.undo_stack.execute(Box::new(cmd), &mut self.project) {
                tracing::error!("rename clip failed: {e}");
            }
            self.clip_rename = None;
        } else {
            self.clip_rename = Some((id, buf));
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

        let out_dir = out
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(std::env::temp_dir);
        if matches!(
            caprust_media_io::disk::free_bytes(&out_dir),
            Some(free) if free < caprust_media_io::disk::EXPORT_MIN_FREE_BYTES
        ) {
            tracing::error!("export: disk full, aborting before ffmpeg spawn");
            self.toast_error(tr("toast-disk-full"));
            return;
        }
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
        if let Some(parent) = path.parent() {
            if matches!(
                caprust_media_io::disk::free_bytes(parent),
                Some(free) if free < caprust_media_io::disk::AUDIO_CACHE_MIN_FREE_BYTES
            ) {
                tracing::warn!("audio cache: disk full, skipping render");
                return;
            }
        }
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
        // A recovery offer is a decision about which session the user
        // ends up editing. Opening the relink dialog at the same time
        // gave the user two modals stacked on the centre of the
        // screen, and relinking first then restoring threw the relink
        // away. Hold the scan until the offer is resolved.
        if self.recovery_prompt.is_some() {
            tracing::info!("relink: deferring scan behind recovery prompt");
            self.pending_missing_scan = true;
            return;
        }
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
                    &mut self.settings_status,
                );
                if ev.remote_pair_requested {
                    self.start_remote_pairing();
                }
                if ev.remote_revoke_requested {
                    self.revoke_remote_access();
                }
                if let Some(id) = ev.download_requested.clone() {
                    self.start_model_download(&id);
                }
                if ev.export_requested {
                    self.handle_settings_export();
                }
                if ev.import_requested {
                    self.handle_settings_import();
                }
                if ev.theme_export_requested {
                    self.handle_theme_export();
                }
                if ev.theme_import_requested {
                    self.handle_theme_import();
                }
                if let Some(p) = ev.layout_preset_requested {
                    self.apply_layout_preset(p);
                }
                if ev.layout_reset_requested {
                    self.reset_layout();
                }
                if ev.save {
                    // Re-detect ffmpeg with new paths
                    self.ffmpeg_status = caprust_core::detect_ffmpeg(&self.settings);
                    // Trigger a save next frame (eframe persists via save())
                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                        egui::UserAttentionType::Informational,
                    ));
                    self.settings_open = false;
                    self.settings_status.autosave_applied = None;
                }
                if ev.close {
                    self.settings_open = false;
                    self.settings_status.autosave_applied = None;
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

    /// Write the current Theme to a user-chosen JSON file.
    fn handle_theme_export(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(tr("set-appearance-theme-save"))
            .add_filter("JSON", &["json"])
            .set_file_name("caprust-theme.json")
            .save_file()
        else {
            return;
        };

        let result = serde_json::to_string_pretty(&self.theme)
            .map_err(|e| e.to_string())
            .and_then(|json| std::fs::write(&path, json).map_err(|e| e.to_string()));
        match result {
            Ok(()) => {
                tracing::info!("theme exported to {}", path.display());
                self.toast(tr("toast-theme-exported"));
            }
            Err(e) => {
                tracing::warn!("theme export failed: {e}");
                self.toast_error(tr("toast-theme-export-failed"));
            }
        }
    }

    /// Replace the current Theme from a user-chosen JSON file. A bad
    /// file toasts and keeps the running theme untouched.
    fn handle_theme_import(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(tr("set-appearance-theme-load"))
            .add_filter("JSON", &["json"])
            .pick_file()
        else {
            return;
        };

        let loaded: Result<crate::theme::Theme, String> = (|| {
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            serde_json::from_str(&text).map_err(|e| e.to_string())
        })();
        match loaded {
            Ok(theme) => {
                self.theme = theme;
                tracing::info!("theme imported from {}", path.display());
                self.toast(tr("toast-theme-imported"));
            }
            Err(e) => {
                tracing::warn!("theme import failed: {e}");
                self.toast_error(tr("toast-theme-import-failed"));
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

/// Wall-clock "14:05" for a Unix timestamp, in the user's local zone.
/// Used only to say when a recovery snapshot was written.
fn format_clock(unix_secs: u64) -> String {
    let Ok(ts) = jiff::Timestamp::from_second(i64::try_from(unix_secs).unwrap_or(0)) else {
        return "--:--".into();
    };
    let zoned = ts.to_zoned(jiff::tz::TimeZone::system());
    format!("{:02}:{:02}", zoned.hour(), zoned.minute())
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
    /// Open the clip rename modal for `id`. The dispatcher also
    /// selects the clip (plain click semantics).
    RenameClip(uuid::Uuid),
    /// Per-clip waveform layout override. Some(true) = split display,
    /// Some(false) = legacy overlay. Set from the clip context menu
    /// ("Clip display" submenu); undoable via SetClipCommand.
    SetSplitDisplay(uuid::Uuid, Option<bool>),
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
        self.drain_sfx_download();
        self.drain_translate_job();
        self.drain_caption_job();
        self.drain_narration_job();
        self.drain_reframe_job();
        self.drain_beat_job();
        self.drain_bg_removal_job();
        self.drain_anon_job();
        self.drain_stab_job();
        self.drain_save_frame();
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
        // Crash-recovery autosave. Cheap when disabled or idle: the
        // interval and dirty checks bail before any hashing.
        self.tick_autosave();
        // Remote control. Also cheap: no listener means an early
        // return before any borrowing.
        self.tick_remote_control();
        if self.recovery_prompt.is_some() {
            self.show_recovery_prompt_window(ctx);
        } else if self.pending_missing_scan {
            self.pending_missing_scan = false;
            self.check_missing_media_on_load();
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
        if self.clip_rename.is_some() {
            self.show_clip_rename_window(ctx);
        }
        if self.template_save.is_some() {
            self.show_template_save_window(ctx);
        }
        if self.template_confirm.is_some() {
            self.show_template_confirm_window(ctx);
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
        if let Ok(json) = serde_json::to_string(&MediaBinLayout {
            tab_layout: self.media_bin.tab_layout,
            sidebar_width: self.media_bin.sidebar_width,
        }) {
            storage.set_string("media_bin_layout", json);
        }
    }
}

/// i18n key for a layout mode's menu label. Lives here because the View
/// menu and the Settings dialog both need it and neither owns it.
pub(crate) fn layout_mode_key(mode: caprust_core::LayoutMode) -> &'static str {
    match mode {
        caprust_core::LayoutMode::Default => "menu-view-layout-default",
        caprust_core::LayoutMode::Modern => "menu-view-layout-modern",
    }
}

pub(crate) fn layout_mode_hint_key(mode: caprust_core::LayoutMode) -> &'static str {
    match mode {
        caprust_core::LayoutMode::Default => "menu-view-layout-default-hint",
        caprust_core::LayoutMode::Modern => "menu-view-layout-modern-hint",
    }
}

pub(crate) fn timeline_size_key(size: caprust_core::TimelineSize) -> &'static str {
    match size {
        caprust_core::TimelineSize::S => "menu-timeline-size-s",
        caprust_core::TimelineSize::M => "menu-timeline-size-m",
        caprust_core::TimelineSize::L => "menu-timeline-size-l",
    }
}

/// The dock tree the editor opens with, given the persisted settings.
///
/// `Dockable` rebuilds from the named preset every launch, so a user who
/// picked "wide timeline" on a 13-inch laptop gets it on a 27-inch one
/// too. An id that no longer resolves -- a preset renamed or removed in a
/// later release -- falls through to the saved tree rather than to the
/// built-in one, because a machine that already has a saved arrangement
/// should keep it.
///
/// `Default` reads the saved tree, or the built-in starting arrangement
/// on a fresh install. This is the path every existing install takes and
/// Which of the four side zones a call is about.
///
/// An enum rather than four `Tab` fields passed around: the picker needs
/// to read and write one of them through a single `&mut ModernState`, and
/// an index into a struct with named fields is what that looks like when
/// it is written out by hand four times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    LeftTop,
    LeftBottom,
    RightTop,
    RightBottom,
}

impl Zone {
    /// The active panel this zone is showing (first in the stack).
    pub fn tab(self, state: &ModernState) -> crate::dock::Tab {
        match self {
            Self::LeftTop => state
                .left_top
                .first()
                .cloned()
                .unwrap_or(crate::dock::Tab::AssetMedia),
            Self::LeftBottom => state
                .left_bottom
                .first()
                .cloned()
                .unwrap_or(crate::dock::Tab::AssetEffects),
            Self::RightTop => state
                .right_top
                .first()
                .cloned()
                .unwrap_or(crate::dock::Tab::Properties),
            Self::RightBottom => state
                .right_bottom
                .first()
                .cloned()
                .unwrap_or(crate::dock::Tab::AssetPlugins),
        }
    }

    /// Get all stacked tabs in this zone (up to 3, like poker cards).
    pub fn stacked_tabs(self, state: &ModernState) -> Vec<crate::dock::Tab> {
        match self {
            Self::LeftTop => state.left_top.clone(),
            Self::LeftBottom => state.left_bottom.clone(),
            Self::RightTop => state.right_top.clone(),
            Self::RightBottom => state.right_bottom.clone(),
        }
    }

    /// Replace the active panel in this zone (first in stack, up to 3 max).
    pub fn set(self, state: &mut ModernState, tab: crate::dock::Tab) {
        match self {
            Self::LeftTop => {
                if !state.left_top.contains(&tab) {
                    state.left_top.insert(0, tab);
                    if state.left_top.len() > 3 {
                        state.left_top.pop();
                    }
                } else {
                    state.left_top.retain(|t| *t != tab);
                    state.left_top.insert(0, tab);
                }
            }
            Self::LeftBottom => {
                if !state.left_bottom.contains(&tab) {
                    state.left_bottom.insert(0, tab);
                    if state.left_bottom.len() > 3 {
                        state.left_bottom.pop();
                    }
                } else {
                    state.left_bottom.retain(|t| *t != tab);
                    state.left_bottom.insert(0, tab);
                }
            }
            Self::RightTop => {
                if !state.right_top.contains(&tab) {
                    state.right_top.insert(0, tab);
                    if state.right_top.len() > 3 {
                        state.right_top.pop();
                    }
                } else {
                    state.right_top.retain(|t| *t != tab);
                    state.right_top.insert(0, tab);
                }
            }
            Self::RightBottom => {
                if !state.right_bottom.contains(&tab) {
                    state.right_bottom.insert(0, tab);
                    if state.right_bottom.len() > 3 {
                        state.right_bottom.pop();
                    }
                } else {
                    state.right_bottom.retain(|t| *t != tab);
                    state.right_bottom.insert(0, tab);
                }
            }
        }
    }

    /// A stable number for this zone, used to give its scroll area its own
    /// remembered scroll offset.
    pub fn slot(self) -> u8 {
        match self {
            Self::LeftTop => 0,
            Self::LeftBottom => 1,
            Self::RightTop => 2,
            Self::RightBottom => 3,
        }
    }

    /// Put the first panel this zone does not already hold at the front.
    ///
    /// Used by View -> Add container. An empty zone takes the editor's
    /// default for that slot, so adding to a cleared zone gets the zone back
    /// rather than leaving it blank.
    pub fn add_next(self, state: &mut ModernState) {
        let fallback = self.default_tab();
        let existing: Vec<crate::dock::Tab> = match self {
            Self::LeftTop => state.left_top.clone(),
            Self::LeftBottom => state.left_bottom.clone(),
            Self::RightTop => state.right_top.clone(),
            Self::RightBottom => state.right_bottom.clone(),
        };
        let next = crate::dock::Tab::all()
            .into_iter()
            .find(|t| !existing.contains(t))
            .unwrap_or(fallback);
        self.set(state, next);
    }

    /// The panel a zone falls back to, and what `None` leaves behind.
    pub fn default_tab(self) -> crate::dock::Tab {
        match self {
            Self::LeftTop => crate::dock::Tab::AssetMedia,
            Self::LeftBottom => crate::dock::Tab::AssetEffects,
            Self::RightTop => crate::dock::Tab::Properties,
            Self::RightBottom => crate::dock::Tab::AssetPlugins,
        }
    }

    /// Take the back card off the stack.
    ///
    /// Without this the stack could only grow: `+` disappeared at three and
    /// there was no way back to a single card.
    pub fn drop_back(self, state: &mut ModernState) {
        let stack = match self {
            Self::LeftTop => &mut state.left_top,
            Self::LeftBottom => &mut state.left_bottom,
            Self::RightTop => &mut state.right_top,
            Self::RightBottom => &mut state.right_bottom,
        };
        if stack.len() > 1 {
            stack.pop();
        }
    }

    /// Bring a card that is already in this zone to the front.
    pub fn promote(self, state: &mut ModernState, tab: crate::dock::Tab) {
        let stack = match self {
            Self::LeftTop => &mut state.left_top,
            Self::LeftBottom => &mut state.left_bottom,
            Self::RightTop => &mut state.right_top,
            Self::RightBottom => &mut state.right_bottom,
        };
        if let Some(at) = stack.iter().position(|t| *t == tab) {
            let tab = stack.remove(at);
            stack.insert(0, tab);
        }
    }

    /// Clear this zone (set to empty stack) — used by the "None" picker option.
    pub fn clear_zone(self, state: &mut ModernState) {
        match self {
            Self::LeftTop => state.left_top.clear(),
            Self::LeftBottom => state.left_bottom.clear(),
            Self::RightTop => state.right_top.clear(),
            Self::RightBottom => state.right_bottom.clear(),
        }
    }

    /// Label for zone header (for submenu).
    pub fn title_text(self) -> &'static str {
        match self {
            Self::LeftTop => "Left Top",
            Self::LeftBottom => "Left Bottom",
            Self::RightTop => "Right Top",
            Self::RightBottom => "Right Bottom",
        }
    }
}

/// What each zone of the fixed editor is holding.
///
/// One panel per zone, chosen from a picker. There is deliberately no
/// dragging between zones here: this is the editor for someone who wants
/// to say what goes where once and have it stay, and the dock editor is
/// where the moving happens. Two editors, two different jobs.
#[derive(Debug, Clone, PartialEq)]
pub struct ModernState {
    /// Top-left: stacked up to 3 tabs (like poker cards).
    pub left_top: Vec<crate::dock::Tab>,
    /// Bottom-left: stacked up to 3 tabs.
    pub left_bottom: Vec<crate::dock::Tab>,
    /// Top-right: stacked up to 3 tabs.
    pub right_top: Vec<crate::dock::Tab>,
    /// Bottom-right: stacked up to 3 tabs.
    pub right_bottom: Vec<crate::dock::Tab>,
}

impl Default for ModernState {
    /// The starting zones, in the order a clip editor is expected to
    /// have them.
    fn default() -> Self {
        use crate::dock::Tab;
        // Only the two upper slots start filled. The lower ones are empty
        // slots the user fills from the dropdown or from View -> Modern
        // zones -> Add container, which is a choice rather than a default.
        Self {
            left_top: vec![Tab::AssetMedia],
            left_bottom: Vec::new(),
            right_top: vec![Tab::Properties],
            right_bottom: Vec::new(),
        }
    }
}

/// The dock tree, ready for the editor to render.
///
/// The dock exists in both editors -- `Modern` draws its own fixed
/// panels, but the tree is still kept so switching between them does not
/// cost anyone the arrangement they made. This picks the tree to start
/// from.
///
/// A named preset is rebuilt on every launch, so a user who picked "wide
/// timeline" on a 13-inch laptop gets it on a 27-inch one too. An id that
/// no longer resolves -- a preset renamed or removed in a later release --
/// falls through to the saved tree rather than to the built-in one,
/// because a machine that already has a saved arrangement should keep it.
///
/// The preset belongs to `Default`. `Modern` has fixed panels, so reading
/// one there would be a choice the user did not make; the saved tree
/// stands so going to Modern and back restores what was there.
fn starting_dock_state(
    settings: &caprust_core::AppSettings,
) -> egui_dock::DockState<crate::dock::Tab> {
    let saved = || {
        settings.dock_layout.as_ref().and_then(|json| {
            serde_json::from_str::<egui_dock::DockState<crate::dock::Tab>>(json).ok()
        })
    };

    if settings.layout == caprust_core::LayoutMode::Default {
        if let Some(id) = settings.layout_preset.as_deref() {
            match crate::dock::LayoutPreset::from_id(id) {
                Some(p) => return crate::dock::preset_dock_state(p),
                None => tracing::warn!(
                    "saved dock preset {id:?} is not a preset this build knows; using the saved layout"
                ),
            }
        }
    }

    saved().unwrap_or_else(crate::dock::default_dock_state)
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

#[cfg(test)]
mod preview_encoding_tests {
    use super::encode_jpeg_base64;
    use crate::preview_player::{PreviewPlayer, RawFrame};

    /// The bug this pins: `RemoteSession::preview_frame` answers from
    /// `player.raw_frame()`, while every rendering path in app.rs sets
    /// `texture` and `has_frame` on the player. Nothing set the pixels.
    ///
    /// So a remote client got `{"frame": null}` for the whole time
    /// playback was running -- and playback on the desktop was plainly
    /// working, which is what made it look like the phone rather than
    /// the bridge.
    ///
    /// `poll_stream` did set `frame`, correctly, and was called from
    /// nowhere. That asymmetry is the whole defect: a correct helper
    /// that nothing invoked, and an inline path that half-updated the
    /// struct.
    #[test]
    fn a_preview_frame_survives_to_the_remote_reader() {
        let mut player = PreviewPlayer::default();
        assert!(
            player.raw_frame().is_none(),
            "nothing decoded yet, so there is no frame to send"
        );

        let w = 64;
        let h = 32;
        let rgba = vec![200u8; w * h * 4];
        // publish_frame is the only path that sets the texture, so
        // showing a frame and keeping it are the same act and cannot
        // come apart. This is the invariant that was missing: three
        // rendering sites assigned `texture` directly and left `frame`
        // as None.
        player.publish_frame(&egui::Context::default(), "test", w, h, &rgba);

        let held = player.raw_frame().expect("frame kept for the reader");
        assert_eq!((held.width, held.height), (w, h));
        assert_eq!(held.rgba.len(), w * h * 4);

        // And it must survive the encode the remote path runs.
        let jpeg = encode_jpeg_base64(held, 320).expect("encode");
        assert!(!jpeg.is_empty());
        assert!(!jpeg.contains("null"));
    }

    fn frame(w: usize, h: usize) -> RawFrame {
        let mut rgba = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                rgba[i] = (x * 255 / w.max(1)) as u8;
                rgba[i + 1] = (y * 255 / h.max(1)) as u8;
                rgba[i + 2] = 128;
                rgba[i + 3] = 255;
            }
        }
        RawFrame {
            width: w,
            height: h,
            rgba,
        }
    }

    #[test]
    fn encoded_frame_is_a_jpeg_the_image_crate_can_read_back() {
        // The phone decodes with BitmapFactory, which is as strict as
        // this. If the bytes are not a real JPEG the phone shows
        // "could not decode" and nothing else explains why.
        let f = frame(320, 180);
        let b64 = encode_jpeg_base64(&f, 320).expect("encode");
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &b64)
            .expect("valid base64");

        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "no JPEG SOI marker");
        assert_eq!(&bytes[bytes.len() - 2..], &[0xFF, 0xD9], "no EOI marker");

        let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
            .expect("image crate could not decode the frame");
        assert_eq!(img.width(), 320);
        assert_eq!(img.height(), 180);
    }

    #[test]
    fn downscaling_respects_the_requested_width() {
        let f = frame(640, 360);
        let b64 = encode_jpeg_base64(&f, 320).unwrap();
        let bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &b64).unwrap();
        let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg).unwrap();
        assert_eq!(img.width(), 320);
        // Aspect kept: 640x360 into 320 wide is 180 high.
        assert_eq!(img.height(), 180);
    }

    #[test]
    fn a_wide_frame_does_not_collapse_to_nothing() {
        // A very wide frame scaled to a small width can round the
        // height to zero, which produces a zero-length image that no
        // decoder will accept.
        let f = frame(4000, 8);
        let b64 = encode_jpeg_base64(&f, 64).unwrap();
        let bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &b64).unwrap();
        let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg).unwrap();
        assert!(img.height() >= 1, "height collapsed to {}", img.height());
    }

    #[test]
    fn a_frame_that_needs_no_scaling_keeps_its_size() {
        let f = frame(160, 90);
        let b64 = encode_jpeg_base64(&f, 320).unwrap();
        let bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &b64).unwrap();
        let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((img.width(), img.height()), (160, 90));
    }
}

#[cfg(test)]
mod track_height_drag_tests {
    /// Track-height drag must move the divider by the pointer, at every
    /// timeline size.
    ///
    /// The row is drawn at `track.height * factor` pixels but the drag
    /// delta is in pixels, so one has to be converted into the other or
    /// the factor is fed back into its own value on every frame of the
    /// drag. That grew the stored height without bound, the clamp pinned
    /// it at the maximum, and from there the drag could not lower it
    /// again -- the reported "I can make it bigger but never smaller".
    ///
    /// These went through a local copy of the arithmetic for a while,
    /// which is the same mistake as testing a hand-written graph: the
    /// copy and the real code could agree while the real code was
    /// wrong. They call the production functions now.
    use super::lane_layout;
    use caprust_core::TimelineSize;

    const SIZES: [(TimelineSize, f32); 3] = [
        (TimelineSize::S, 1.0),
        (TimelineSize::M, 1.3),
        (TimelineSize::L, 1.7),
    ];

    /// One frame of the drag, the way the panel does it.
    fn drag_frame(
        stored: f32,
        dy: f32,
        size: TimelineSize,
        heights: &[f32],
        available: f32,
    ) -> f32 {
        let layout = lane_layout(size, heights, available);
        let per_stored = layout.row_height_per_stored(stored);
        (stored + dy / per_stored.max(1e-3)).clamp(40.0, 160.0)
    }

    #[test]
    fn the_divider_follows_the_pointer() {
        for (size, factor) in SIZES {
            // One track, so the layout is the nominal one.
            let heights = [46.0f32];
            let mut stored = 46.0f32;
            let start_row = stored * factor;
            let dy = 10.0f32;

            stored = drag_frame(stored, dy, size, &heights, 900.0);
            assert!(
                (stored * factor - start_row - dy).abs() < 0.01,
                "{size:?}: dragged down 10px, the row moved {}",
                stored * factor - start_row
            );

            let after_down = stored * factor;
            stored = drag_frame(stored, -dy, size, &heights, 900.0);
            assert!(
                (after_down - stored * factor - dy).abs() < 0.01,
                "{size:?}: dragged back up 10px, the row moved {}",
                stored * factor - after_down
            );
        }
    }

    #[test]
    fn sixty_frames_of_dragging_does_not_run_the_height_away() {
        for (size, factor) in SIZES {
            let heights = [160.0f32];
            // Shrinking from the maximum has to actually reach the
            // minimum. The runaway pinned it at the maximum forever.
            let mut stored = 160.0f32;
            let mut before = stored * factor;
            for _ in 0..60 {
                stored = drag_frame(stored, -6.0, size, &heights, 900.0);
                assert!(
                    stored * factor <= before + 0.01,
                    "{size:?}: dragging up made the row taller"
                );
                before = stored * factor;
            }
            assert!(
                stored < 41.0,
                "{size:?}: could not shrink below the minimum, stuck at {stored}"
            );

            // And growing from the minimum has to reach the maximum
            // without overshooting past the clamp.
            let mut stored = 40.0f32;
            for _ in 0..60 {
                stored = drag_frame(stored, 6.0, size, &heights, 900.0);
            }
            assert!(stored <= 160.0 + f32::EPSILON, "{size:?}: ran to {stored}");
        }
    }

    /// The bug itself, stated as a fact: the old line fed the scaled
    /// height back into the stored value, so it grew by the factor on
    /// every frame and could not be reversed.
    #[test]
    fn the_old_line_grew_by_the_factor_per_frame() {
        for (name, factor) in [("M", 1.3f32), ("L", 1.7)] {
            let mut stored = 46.0f32;
            let mut grew_past_max = false;
            for _ in 0..60 {
                let row = stored * factor;
                stored = (row + 6.0).clamp(40.0, 160.0);
                if stored >= 160.0 {
                    grew_past_max = true;
                }
            }
            assert!(grew_past_max, "{name}: the old line did not diverge");
            // And from the pinned maximum, up-dragging cannot help.
            let stuck = (160.0f32 * factor - 6.0).clamp(40.0, 160.0);
            assert_eq!(stuck, 160.0, "{name}: the old line could not shrink");
        }
    }
}

#[cfg(test)]
mod app_layout_tests {
    use super::starting_dock_state;
    use crate::dock::{default_dock_state, preset_dock_state, LayoutPreset};
    use caprust_core::{AppSettings, LayoutMode};
    use egui_dock::DockState;
    use serde_json::Value;

    type Tree = DockState<crate::dock::Tab>;

    /// A dock tree carries its rectangles, and those are written by
    /// egui_dock during layout rather than by the code that builds the
    /// tree. A tree that has never been shown has `null` rectangles,
    /// which do not survive a round trip through serde -- and neither do
    /// they have any business being compared. Strip them and compare the
    /// structure: which panels sit where, and how the space is divided.
    fn shape(state: &Tree) -> Value {
        fn strip(v: &mut Value) {
            if let Some(map) = v.as_object_mut() {
                map.remove("rect");
                map.remove("viewport");
                for (_k, child) in map.iter_mut() {
                    strip(child);
                }
            } else if let Some(arr) = v.as_array_mut() {
                for child in arr.iter_mut() {
                    strip(child);
                }
            }
        }
        let mut v = serde_json::to_value(state).expect("tree serialises");
        strip(&mut v);
        v
    }

    /// Stand in for a settings file written by a previous session.
    ///
    /// A real one has laid-out rectangles, so the nulls a fresh tree
    /// carries are filled in here too. Without this the test would be
    /// measuring egui_dock's layout pass rather than the layout choice.
    fn stored(state: &Tree) -> String {
        fn fill(v: &mut Value) {
            if let Some(map) = v.as_object_mut() {
                for (k, child) in map.iter_mut() {
                    if k == "rect" || k == "viewport" {
                        fill_rect(child);
                    } else {
                        fill(child);
                    }
                }
            } else if let Some(arr) = v.as_array_mut() {
                for child in arr.iter_mut() {
                    fill(child);
                }
            }
        }
        // Rectangles are `{"min": {"x": null, "y": null}, ...}`, so the
        // nulls sit inside the object rather than replacing it.
        fn fill_rect(v: &mut Value) {
            if v.is_null() {
                *v = serde_json::json!(0.0);
                return;
            }
            if let Some(map) = v.as_object_mut() {
                for child in map.values_mut() {
                    fill_rect(child);
                }
            } else if let Some(arr) = v.as_array_mut() {
                for child in arr.iter_mut() {
                    fill_rect(child);
                }
            }
        }
        let mut v = serde_json::to_value(state).expect("tree serialises");
        fill(&mut v);
        serde_json::to_string(&v).expect("stored tree serialises")
    }

    fn settings_with(
        layout: LayoutMode,
        preset: Option<&str>,
        saved: Option<&Tree>,
    ) -> AppSettings {
        AppSettings {
            dock_layout: saved.map(stored),
            layout,
            layout_preset: preset.map(str::to_string),
            ..Default::default()
        }
    }

    /// The promise the default mode makes: nothing moves under someone
    /// who has already arranged their panels. Without this the feature
    /// would reset every existing install's layout on first launch.
    #[test]
    fn default_mode_opens_the_saved_arrangement() {
        let saved = preset_dock_state(LayoutPreset::WideTimeline);
        let settings = settings_with(LayoutMode::Default, None, Some(&saved));
        assert_eq!(
            shape(&starting_dock_state(&settings)),
            shape(&saved),
            "default mode must not rearrange what the user saved"
        );
    }

    /// A fresh install has nothing saved, so it gets the built-in tree.
    #[test]
    fn default_mode_with_nothing_saved_uses_the_built_in_tree() {
        let settings = AppSettings::default();
        assert_eq!(
            shape(&starting_dock_state(&settings)),
            shape(&default_dock_state())
        );
    }

    /// A preset wins over the saved tree, so the arrangement is the same
    /// on every launch and every screen. This is what made the old View
    /// menu buttons useless: they applied a preset once and then let the
    /// save tick store that tree as if the user had dragged it there.
    #[test]
    fn a_preset_is_rebuilt_on_every_launch() {
        let settings = settings_with(
            LayoutMode::Default,
            Some("wide-timeline"),
            Some(&preset_dock_state(LayoutPreset::PreviewFocus)),
        );
        assert_eq!(
            shape(&starting_dock_state(&settings)),
            shape(&preset_dock_state(LayoutPreset::WideTimeline)),
            "the preset must win over the saved tree"
        );
    }

    /// A preset id from a future or hand-edited settings file. Falling
    /// back to the saved tree keeps the user's layout; falling back to
    /// the built-in one would throw away an arrangement they made.
    #[test]
    fn an_unknown_preset_keeps_the_saved_arrangement() {
        let saved = preset_dock_state(LayoutPreset::TimelineFocus);
        let settings = settings_with(LayoutMode::Default, Some("cinerama"), Some(&saved));
        assert_eq!(shape(&starting_dock_state(&settings)), shape(&saved));
    }

    /// Unparseable saved JSON is not a reason to lose the mode: the
    /// preset still resolves.
    #[test]
    fn dockable_survives_unparseable_saved_json() {
        let mut settings = settings_with(LayoutMode::Default, Some("timeline-focus"), None);
        settings.dock_layout = Some("{ not json".into());
        assert_eq!(
            shape(&starting_dock_state(&settings)),
            shape(&preset_dock_state(LayoutPreset::TimelineFocus))
        );
    }

    /// A hand-edited settings file can leave a preset named while the
    /// mode is the fixed editor. There the preset does not apply, and the
    /// saved tree stands so going to Modern and back restores what the
    /// user made rather than a preset they picked months ago.
    #[test]
    fn a_preset_is_ignored_in_the_modern_editor() {
        let saved = preset_dock_state(LayoutPreset::WideTimeline);
        let settings = settings_with(LayoutMode::Modern, Some("timeline-focus"), Some(&saved));
        assert_eq!(
            shape(&starting_dock_state(&settings)),
            shape(&saved),
            "Modern has fixed panels; a dock preset is not a choice it made"
        );
    }

    /// Without a preset named the dock is the saved tree, or the built-in
    /// one on a fresh install.
    #[test]
    fn without_a_preset_the_saved_arrangement_stands() {
        let saved = preset_dock_state(LayoutPreset::WideTimeline);
        let settings = settings_with(LayoutMode::Default, None, Some(&saved));
        assert_eq!(shape(&starting_dock_state(&settings)), shape(&saved));
    }

    /// Every preset must survive a save and reload, or `Dockable` would
    /// quietly become `Default` after one restart. This is the check
    /// that the null-rectangle problem cannot come back unnoticed.
    #[test]
    fn every_preset_survives_a_store_and_reload() {
        for p in LayoutPreset::all() {
            let tree = preset_dock_state(p);
            let settings = settings_with(LayoutMode::Default, Some(p.id()), Some(&tree));
            let opened = starting_dock_state(&settings);
            assert_eq!(
                shape(&opened),
                shape(&preset_dock_state(p)),
                "{p:?} did not come back as itself"
            );
        }
    }
}

#[cfg(test)]
mod export_settings_tests {
    use super::*;

    #[test]
    fn available_lists_are_never_empty() {
        // A setting whose option list is empty renders as a Change
        // button that opens nothing. That is exactly what a phone
        // cannot explain, so it is asserted here rather than found on
        // a device.
        let mirror =
            ExportSettingsMirror::of(&crate::panels::export_window::ExportState::default());
        let v = export_settings_value(&mirror);
        let available = v["available"].as_object().expect("no `available` key");
        for key in [
            "resolution",
            "frame_rate",
            "quality",
            "rate_mode",
            "color_range",
        ] {
            let n = available
                .get(key)
                .and_then(|a| a.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            assert!(n > 0, "`{key}` has no accepted values");
        }
    }

    #[test]
    fn a_current_value_is_always_one_of_the_accepted_ones() {
        // The row shows the current value and the option list shows
        // what is allowed. If the current value is not in the list,
        // nothing is ticked and the user cannot tell what is set.
        let state = crate::panels::export_window::ExportState::default();
        let mirror = ExportSettingsMirror::of(&state);
        let v = export_settings_value(&mirror);
        let available = v["available"].as_object().unwrap();
        for key in [
            "resolution",
            "frame_rate",
            "quality",
            "rate_mode",
            "color_range",
        ] {
            let current = v[key].as_str().unwrap_or_default();
            let ids: Vec<String> = available[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| o["id"].as_str().unwrap_or_default().to_string())
                .collect();
            assert!(
                ids.iter().any(|i| i == current),
                "`{key}` is `{current}`, which is not among {ids:?}"
            );
        }
    }

    #[test]
    fn a_mirror_applies_back_onto_a_real_state() {
        // The round trip the phone relies on: read, change, write, and
        // the desktop's export window agrees.
        let mut state = crate::panels::export_window::ExportState::default();
        let mut mirror = ExportSettingsMirror::of(&state);
        mirror.set("quality", "large").unwrap();
        mirror.set("resolution", "hd").unwrap();
        apply_export_mirror(&mut state, &mirror).expect("valid values");

        assert_eq!(
            state.quality,
            crate::panels::export_window::QualityTier::Large
        );
        assert_eq!(
            state.resolution,
            caprust_media_io::export::ExportResolution::Hd
        );
    }

    #[test]
    fn a_bad_value_is_refused_and_changes_nothing() {
        let mut state = crate::panels::export_window::ExportState::default();
        let before = state.quality;
        let mut mirror = ExportSettingsMirror::of(&state);
        mirror.set("quality", "enormous").unwrap();
        let err = apply_export_mirror(&mut state, &mirror).unwrap_err();
        assert!(err.contains("export_settings"), "{err}");
        assert_eq!(
            state.quality, before,
            "a rejected value still changed the state"
        );
    }

    #[test]
    fn an_unknown_key_names_the_valid_ones() {
        let mut mirror = ExportSettingsMirror::of(&Default::default());
        let err = mirror.set("sharpness", "high").unwrap_err();
        assert!(err.contains("resolution"), "{err}");
    }
}
/// One format card: a rectangle in the real proportions, the name, and
/// what the format is for. Returns true when clicked.
fn format_card(ui: &mut egui::Ui, preset: &AspectRatio, selected: bool) -> bool {
    let (w, h) = preset.dimensions(1080);
    let ratio = w as f32 / h.max(1) as f32;

    // Fixed card size, with the thumbnail fitted into a reserved area
    // above a reserved band for the two label lines. Fixed sizes keep
    // the grid even, which is what makes the rectangles comparable --
    // the whole reason for showing them side by side.
    const CARD_W: f32 = 210.0;
    const CARD_H: f32 = 176.0;
    const PAD: f32 = 10.0;
    /// Enough for the name and the one line under it.
    const LABEL_H: f32 = 42.0;

    // The rect is allocated first and everything is painted onto it.
    //
    // A Frame::show allocates its own rect and returns a response that
    // only covers what was allocated inside, so a separate interaction
    // on top of it lands in the wrong place -- the previous version
    // drew a second zero-height rect after the frame, which is why
    // only the default card was ever clickable.
    let (rect, response) = ui.allocate_exact_size(egui::vec2(CARD_W, CARD_H), egui::Sense::click());
    let p = ui.painter().with_clip_rect(rect);
    let accent = ui.visuals().selection.stroke.color;

    p.rect_filled(
        rect,
        radius::cr(radius::MD),
        if selected {
            ui.visuals().selection.bg_fill
        } else {
            ui.visuals().faint_bg_color
        },
    );
    p.rect_stroke(
        rect,
        radius::cr(radius::MD),
        egui::Stroke::new(
            if selected { 2.0 } else { 1.0 },
            if selected {
                accent
            } else {
                ui.visuals().widgets.noninteractive.bg_stroke.color
            },
        ),
        egui::StrokeKind::Inside,
    );

    // Two bands, top and bottom, and the rectangle is fitted into the
    // top one. Deriving both from CARD_H rather than positioning by
    // hand is what stops a tall 9:16 from growing past the card edge and
    // the labels from landing on top of it.
    let thumb_area = egui::Rect::from_min_max(
        egui::pos2(rect.left() + PAD, rect.top() + PAD),
        egui::pos2(rect.right() - PAD, rect.bottom() - PAD - LABEL_H),
    );
    let label_area = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.bottom() - PAD - LABEL_H),
        egui::pos2(rect.right(), rect.bottom() - PAD),
    );

    // Fit inside the band, preserving the ratio. A portrait 9:16 comes
    // out as a tall sliver and a 21:9 as a wide bar, which is the whole
    // point of showing them side by side.
    let aw = thumb_area.width();
    let ah = thumb_area.height();
    let (bw, bh) = if ratio >= 1.0 {
        let h = ah;
        let w = h * ratio;
        if w > aw {
            (aw, aw / ratio)
        } else {
            (w, h)
        }
    } else {
        let w = aw;
        let h = w / ratio;
        if h > ah {
            (ah * ratio, ah)
        } else {
            (w, h)
        }
    };
    let thumb = egui::Rect::from_center_size(thumb_area.center(), egui::vec2(bw, bh));
    p.rect_filled(
        thumb,
        radius::cr(radius::SM),
        if selected {
            accent.gamma_multiply(0.35)
        } else {
            egui::Color32::from_gray(70)
        },
    );
    p.rect_stroke(
        thumb,
        radius::cr(radius::SM),
        egui::Stroke::new(1.0, accent),
        egui::StrokeKind::Inside,
    );
    // Only while it fits. On a tall sliver the text is wider than the
    // rectangle and spills over the card edge.
    if bw > 70.0 {
        p.text(
            thumb.center(),
            egui::Align2::CENTER_CENTER,
            format!("{w}x{h}"),
            egui::FontId::proportional(11.0),
            ui.visuals().text_color(),
        );
    }

    // Labels in their own band, so they cannot collide with whatever
    // shape the rectangle turned out to be.
    p.text(
        egui::pos2(label_area.center().x, label_area.top() + 12.0),
        egui::Align2::CENTER_CENTER,
        preset.label(),
        egui::FontId::proportional(14.0),
        if selected {
            ui.visuals().text_color()
        } else {
            ui.visuals().weak_text_color()
        },
    );
    p.text(
        egui::pos2(label_area.center().x, label_area.top() + 29.0),
        egui::Align2::CENTER_CENTER,
        preset.description(),
        egui::FontId::proportional(10.0),
        ui.visuals().weak_text_color(),
    );

    response.clicked()
        || ui
            .interact(rect, ui.id().with("card"), egui::Sense::click())
            .clicked()
}

/// The chosen format, drawn to scale against the panel.
fn new_project_preview(ui: &mut egui::Ui, draft: &NewProjectDraft) {
    ui.label(egui::RichText::new(tr("new-preview-header")).strong());
    ui.add_space(space::M);

    let (w, h) = draft.aspect_ratio.dimensions(draft.base_resolution.max(1));
    let ratio = w as f32 / h.max(1) as f32;

    let avail_w = ui.available_width() - 20.0;
    // Fit inside the panel in both directions, taking the whole width
    // for a wide format and the full height for a tall one. The height
    // is the cap rather than the width, so a 9:16 uses the full
    // column instead of sitting in a narrow strip beside it.
    let max_h = 420.0f32;
    let (bw, bh) = if ratio >= 1.0 {
        let w = avail_w.min(max_h * ratio);
        (w, w / ratio)
    } else {
        let h = max_h;
        (h * ratio, h)
    };
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(bw.max(20.0), bh.max(20.0)), egui::Sense::hover());
    let accent = ui.visuals().selection.stroke.color;
    ui.painter()
        .rect_filled(rect, radius::cr(radius::MD), ui.visuals().faint_bg_color);
    ui.painter().rect_stroke(
        rect,
        radius::cr(radius::MD),
        egui::Stroke::new(1.5, accent),
        egui::StrokeKind::Inside,
    );

    // Safe-area guides, so the box reads as a frame rather than a
    // rectangle of nothing. Title and action zones are where people put
    // things, and seeing them before the first clip exists is the
    // point of the panel.
    let inset = rect.height() * 0.05;
    let guide = egui::Stroke::new(1.0, egui::Color32::from_gray(90));
    for frac in [0.1f32, 0.9] {
        let y = rect.top() + rect.height() * frac;
        ui.painter().add(egui::Shape::line_segment(
            [
                egui::pos2(rect.left() + inset, y),
                egui::pos2(rect.right() - inset, y),
            ],
            guide,
        ));
    }
    let x = rect.left() + rect.width() * 0.05;
    ui.painter().add(egui::Shape::line_segment(
        [
            egui::pos2(x, rect.top() + inset),
            egui::pos2(x, rect.bottom() - inset),
        ],
        guide,
    ));

    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        format!("{} x {}", w, h),
        egui::FontId::proportional(20.0),
        ui.visuals().text_color(),
    );

    ui.add_space(space::M);
    ui.label(egui::RichText::new(draft.aspect_ratio.label()).strong());
    ui.label(
        egui::RichText::new(draft.aspect_ratio.description())
            .small()
            .color(ui.visuals().weak_text_color()),
    );
}

#[cfg(test)]
mod sticker_overlay_tests {
    /// The bug this file's comment keeps referring to: the preview drew
    /// stickers only when one was selected, while ffmpeg had already
    /// stopped drawing them, so the preview went blank.
    ///
    /// `sticker_quads` takes the project and the playhead. There is no
    /// selection argument and no way to reach one from inside it, which
    /// is the point: a sticker with nothing selected still has to come
    /// out of it, because in the editor nothing is selected most of the
    /// time.
    #[test]
    fn stickers_are_drawn_with_nothing_selected() {
        use caprust_core::clip::{Clip, ClipType, OverlayPlacement};
        use caprust_core::track::{Track, TrackKind};

        let mut p = caprust_core::ProjectState::default();
        p.tracks.clear();
        p.tracks.push(Track::new("V1", TrackKind::Video));
        p.tracks.push(Track::new("V2", TrackKind::Video));
        p.tracks.last_mut().unwrap().visible = false;

        let sticker = |id: u128, track: usize, start: u64| {
            let mut c = Clip::new_video("s.png", track, start, 4000);
            c.id = uuid::Uuid::from_u128(id);
            c.clip_type = ClipType::Image {
                path: format!("s{id}.png"),
                duration_ms: 4000,
            };
            c.overlay = Some(OverlayPlacement::default_sticker());
            c
        };
        p.clips.push(sticker(1, 0, 0)); // on screen, visible
        p.clips.push(sticker(2, 1, 0)); // on screen, hidden track
        p.clips.push(sticker(3, 0, 8000)); // off screen for now
        let mut footage = Clip::new_video("a.mp4", 0, 0, 4000);
        footage.id = uuid::Uuid::from_u128(4);
        p.clips.push(footage);

        let frame = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 360.0));
        let quads = super::sticker_quads(&p, 1000, frame);

        assert_eq!(
            quads.len(),
            1,
            "expected the one visible sticker under the playhead, got {}",
            quads.len()
        );
        assert_eq!(quads[0].path, "s1.png");
    }

    /// The quad has to describe the sticker the export will composite,
    /// so it is derived from `frame_box` and checked against the numbers
    /// real ffmpeg produced for a 320x180 frame earlier.
    #[test]
    fn a_sticker_quad_lands_where_frame_box_says() {
        use caprust_core::clip::{Clip, ClipType, OverlayPlacement};
        use caprust_core::track::{Track, TrackKind};

        let mut p = caprust_core::ProjectState::default();
        p.tracks.clear();
        p.tracks.push(Track::new("V1", TrackKind::Video));

        let mut c = Clip::new_video("s.png", 0, 0, 4000);
        c.clip_type = ClipType::Image {
            path: String::from("s.png"),
            duration_ms: 4000,
        };
        c.overlay = Some(OverlayPlacement {
            scale: 0.25,
            x: 0.3,
            y: 0.25,
            aspect: 1.0,
            rotation: 0.0,
            animation: None,
        });
        p.clips.push(c);

        let frame = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(320.0, 180.0));
        let quads = super::sticker_quads(&p, 0, frame);
        assert_eq!(quads.len(), 1);
        let q = &quads[0];

        let (l, t, w, h) = OverlayPlacement {
            scale: 0.25,
            x: 0.3,
            y: 0.25,
            aspect: 1.0,
            rotation: 0.0,
            animation: None,
        }
        .frame_box(320.0 / 180.0);

        let near = |got: egui::Pos2, want: f32| (got.x - want).abs() < 0.01;
        assert!(near(q.corners[0], l * 320.0), "tl.x is {}", q.corners[0].x);
        assert!(
            near(q.corners[2], (l + w) * 320.0),
            "br.x is {}",
            q.corners[2].x
        );
        assert!((q.corners[0].y - t * 180.0).abs() < 0.01);
        assert!((q.corners[2].y - (t + h) * 180.0).abs() < 0.01);
    }

    /// A turned sticker's quad is a turned rectangle, not a bounding box:
    /// the four corners are off the axes, and opposite corners stay
    /// opposite.
    #[test]
    fn a_turned_sticker_quad_is_turned_not_axis_aligned() {
        use caprust_core::clip::{Clip, ClipType, OverlayPlacement};
        use caprust_core::track::{Track, TrackKind};

        let mut p = caprust_core::ProjectState::default();
        p.tracks.clear();
        p.tracks.push(Track::new("V1", TrackKind::Video));

        let mut c = Clip::new_video("s.png", 0, 0, 4000);
        c.clip_type = ClipType::Image {
            path: String::from("s.png"),
            duration_ms: 4000,
        };
        c.overlay = Some(OverlayPlacement {
            scale: 0.3,
            x: 0.5,
            y: 0.5,
            aspect: 1.0,
            rotation: 30.0,
            animation: None,
        });
        p.clips.push(c);

        let frame = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(320.0, 180.0));
        let q = &super::sticker_quads(&p, 0, frame)[0];
        let [tl, tr, br, bl] = q.corners;

        // A rectangle: every edge is perpendicular to the next.
        let e1 = tr - tl;
        let e2 = bl - tl;
        assert!(
            (e1.dot(e2)).abs() < 0.5,
            "the quad is not a rectangle: edges dot to {}",
            e1.dot(e2)
        );
        // And turned: the top edge is not horizontal.
        assert!(e1.y.abs() > 1.0, "the top edge is still flat ({:?})", e1);
        // Same height and width as the un-turned one times sqrt(2)-ish,
        // and the centre is where it was.
        let centre = tl.to_vec2() + (br - tl) / 2.0;
        assert!(
            (centre.x - 160.0).abs() < 0.01 && (centre.y - 90.0).abs() < 0.01,
            "centre moved to {centre:?}"
        );
    }
    /// Stickers are painted by this panel, into the same clip region as
    /// the video frame. Painting the sticker first works exactly as well
    /// as not painting it at all, because the frame then covers it -- and
    /// that is what happened: the call sat near the top of the panel,
    /// ninety lines above the frame, so every sticker in every project
    /// was invisible while the drag handles worked fine.
    ///
    /// Nothing about the geometry catches this. A sticker can have the
    /// right box, the right corners, the right texture and still be
    /// underneath the picture. So the only check available is that the
    /// call comes after the frame is painted.
    ///
    /// Deliberately source-order rather than behavioural: driving egui
    /// and reading back the shape list is not something this crate can
    /// do, and this is the third UI-only defect in a row that a real
    /// unit test could not have seen.
    #[test]
    fn the_sticker_overlay_is_painted_after_the_frame() {
        let src = include_str!("app.rs");
        let panel = src
            .split_once("fn render_preview_panel")
            .expect("no render_preview_panel")
            .1;
        // Stop at the next `fn` so this cannot accidentally match a call
        // in some later method.
        let panel = &panel[..panel.find("\n    fn ").unwrap_or(panel.len())];

        let frame = panel.find("ui.painter().image(").expect("no frame paint");
        let sticker = panel
            .find("self.preview_sticker_drag(")
            .expect("no sticker overlay in the preview panel");

        assert!(
            sticker > frame,
            "the sticker overlay is painted at {sticker}, the frame at {frame}: \
             the picture covers it"
        );
    }

    /// And it has to be positioned against the drawn picture, not the
    /// container around it. The video is letterboxed inside `rect`, so a
    /// sticker's x/y -- fractions of the frame -- land in the wrong place
    /// by however much padding there is.
    #[test]
    fn the_sticker_overlay_is_anchored_to_the_drawn_picture() {
        let src = include_str!("app.rs");
        let panel = src
            .split_once("fn render_preview_panel")
            .expect("no render_preview_panel")
            .1;
        let panel = &panel[..panel.find("\n    fn ").unwrap_or(panel.len())];

        assert!(
            panel.contains("let sticker_rect = overlay_ctx.map(|(r, _)| r).unwrap_or(rect);"),
            "the sticker overlay is not anchored to draw_rect"
        );
    }
    /// The middle of a sticker moves it, a corner sizes it and the
    /// handle above turns it. All three have to be reachable: the
    /// previous version let egui's overlapping-rect priority decide, and
    /// the outcome was a control where resize worked, move did not start
    /// and rotate needed a 16px hit.
    #[test]
    fn each_part_of_the_overlay_is_reachable() {
        const GRIP: f32 = 12.0;
        let place = caprust_core::OverlayPlacement {
            scale: 0.33,
            x: 0.5,
            y: 0.5,
            aspect: 1.0,
            rotation: 0.0,
            animation: None,
        };
        let frame = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 360.0));
        let corners = super::sticker_corners(&place, frame, 0.0);
        let box_rect = super::corners_bounds(&corners);
        let rot_rect = super::rotation_grip_rect(box_rect);

        let at = |p: egui::Pos2| super::sticker_handle_at(p, corners, GRIP, rot_rect);

        // Middle, and the edges away from the corners, are all a move.
        for p in [
            box_rect.center(),
            box_rect.left_top() + egui::vec2(30.0, 30.0),
            box_rect.center_top() + egui::vec2(-20.0, 0.0),
            box_rect.right_bottom() + egui::vec2(-30.0, -30.0),
        ] {
            assert_eq!(at(p), super::StickerMode::Move, "at {p:?}");
        }

        // Every corner is a size, including the ones where the grip only
        // overlaps the body.
        for c in [
            box_rect.left_top(),
            box_rect.right_top(),
            box_rect.right_bottom(),
            box_rect.left_bottom(),
        ] {
            assert_eq!(at(c), super::StickerMode::Scale, "at {c:?}");
            // And just off the exact corner, still a size.
            assert_eq!(
                at(c + egui::vec2(4.0, 0.0)),
                super::StickerMode::Scale,
                "just off {c:?}"
            );
        }

        // The rotation handle.
        assert_eq!(at(rot_rect.center()), super::StickerMode::Rotate);
    }

    /// The handle has to be somewhere reachable rather than tucked under
    /// the pointer or off the top of the picture, and it must not sit on
    /// the sticker itself -- a rotation target inside the box is one the
    /// body wins.
    #[test]
    fn the_rotation_handle_sits_above_the_box_and_off_it() {
        let tall = egui::Rect::from_min_size(egui::pos2(100.0, 200.0), egui::vec2(200.0, 300.0));
        let flat = egui::Rect::from_min_size(egui::pos2(100.0, 200.0), egui::vec2(200.0, 6.0));
        for box_rect in [tall, flat] {
            let r = super::rotation_grip_rect(box_rect);
            assert!(
                r.max.y < box_rect.min.y,
                "the handle overlaps the box: {r:?} vs {box_rect:?}"
            );
            assert!(
                (r.center().x - box_rect.center().x).abs() < 0.01,
                "the handle is off the centre line"
            );
            assert!(r.width() >= 20.0, "the handle is a {}px target", r.width());
        }
    }

    /// A sticker scaled almost to nothing still needs a handle that is
    /// not sitting on the picture, or every drag is a rotate by accident.
    #[test]
    fn a_tiny_sticker_still_has_a_hand_stand_off() {
        let tiny = egui::Rect::from_min_size(egui::pos2(300.0, 300.0), egui::vec2(4.0, 4.0));
        let r = super::rotation_grip_rect(tiny);
        assert!(r.max.y < tiny.min.y, "{r:?} vs {tiny:?}");
        assert!(tiny.min.y - r.center().y >= 18.0);
    }

    /// The overlay registers one interact rect covering both the box and
    /// the handle. More than one means egui decides who gets the pointer,
    /// and that is what went wrong before.
    #[test]
    fn the_overlay_registers_exactly_one_interact_rect() {
        let src = include_str!("app.rs");
        let panel = src
            .split_once("fn preview_sticker_handles")
            .expect("no preview_sticker_handles")
            .1;
        let panel = &panel[..panel.find("\n    fn ").unwrap_or(panel.len())];

        let n = panel.matches("ui.interact(").count();
        assert_eq!(n, 1, "the sticker overlay registers {n} interact rects");
        assert!(
            panel.contains("egui::Sense::click_and_drag()"),
            "the overlay is not draggable; Sense::click() alone never sets dragged()"
        );
        // Checked with the draggable variant masked out, since
        // `click_and_drag` contains `click` as a substring -- and with
        // comments stripped, since this function explains the mistake in
        // prose that quotes it.
        let code: String = panel
            .lines()
            .map(|l| match l.find("//") {
                Some(i) => &l[..i],
                None => l,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let code = code.replace("Sense::click_and_drag()", "");
        assert!(
            !code.contains("Sense::click()"),
            "Sense::click() on its own cannot start a drag"
        );
    }
    /// The grips have to be on the sticker, not on the box around it.
    ///
    /// After a rotation the bounding box's corners sit in empty space, up
    /// to half a sticker-width away from the mark, so aiming at the thing
    /// you can see reached nothing. The handles follow `sticker_corners`,
    /// which is the same turned rectangle the preview paints and the
    /// export composites.
    #[test]
    fn the_grips_sit_on_the_sticker_at_any_angle() {
        const GRIP: f32 = 12.0;
        let frame = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 360.0));

        for deg in [0.0f32, 7.0, 30.0, 45.0, 90.0, 137.0, 210.0, 300.0] {
            let place = caprust_core::OverlayPlacement {
                scale: 0.3,
                x: 0.5,
                y: 0.5,
                aspect: 1.0,
                rotation: deg,
                animation: None,
            };
            let corners = super::sticker_corners(&place, frame, 0.0);
            let bounds = super::corners_bounds(&corners);
            let rot_rect = super::rotation_grip_rect(bounds);
            let at = |p: egui::Pos2| super::sticker_handle_at(p, corners, GRIP, rot_rect);

            for c in corners {
                assert_eq!(
                    at(c),
                    super::StickerMode::Scale,
                    "at {deg} deg the corner {c:?} is not a size handle"
                );
            }

            // Where the turn is wide enough that the bounding box's
            // corners are somewhere else entirely, those corners are not
            // handles. They are what the grips used to be drawn on, which
            // is why aiming at the sticker after rotating reached nothing.
            //
            // At 0, 90 and 180 degrees the two sets coincide and there is
            // nothing to say, so they are skipped rather than asserted.
            let stray = [
                bounds.left_top(),
                bounds.right_top(),
                bounds.right_bottom(),
                bounds.left_bottom(),
            ];
            for p in stray {
                let nearest = corners
                    .iter()
                    .map(|c| p.distance(*c))
                    .fold(f32::MAX, f32::min);
                if nearest < 20.0 {
                    continue; // the turn is square: same corner
                }
                assert_ne!(
                    at(p),
                    super::StickerMode::Scale,
                    "at {deg} deg the bounding-box corner {p:?} is {nearest}px from \
                     the sticker and still catches the pointer"
                );
            }
        }
    }

    /// The drawn mark is a GRIP square; the target has to cover all of
    /// it. It used to be a circle inscribed in the square, so the corners
    /// of every visible handle were dead.
    #[test]
    fn the_target_covers_the_whole_drawn_mark() {
        const GRIP: f32 = 12.0;
        let place = caprust_core::OverlayPlacement {
            scale: 0.3,
            x: 0.5,
            y: 0.5,
            aspect: 1.0,
            rotation: 23.0,
            animation: None,
        };
        let frame = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 360.0));
        let corners = super::sticker_corners(&place, frame, 0.0);
        let rot_rect = super::rotation_grip_rect(super::corners_bounds(&corners));

        for c in corners {
            // Every corner of the drawn square, and a nudge beyond it.
            for (dx, dy) in [
                (GRIP / 2.0, GRIP / 2.0),
                (-GRIP / 2.0, GRIP / 2.0),
                (GRIP / 2.0, -GRIP / 2.0),
                (-GRIP / 2.0, -GRIP / 2.0),
            ] {
                let p = c + egui::vec2(dx, dy);
                assert_eq!(
                    super::sticker_handle_at(p, corners, GRIP, rot_rect),
                    super::StickerMode::Scale,
                    "the corner of the drawn handle at {p:?} is dead"
                );
            }
        }
    }

    /// The rotation handle must not sit inside the sticker's own turns,
    /// or dragging near the top edge turns instead of moving. Above the
    /// bounding box is the only place that is always free.
    #[test]
    fn the_rotation_handle_never_overlaps_the_sticker() {
        let frame = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 360.0));
        for deg in [0.0f32, 15.0, 45.0, 90.0, 135.0, 180.0, 225.0, 315.0] {
            let place = caprust_core::OverlayPlacement {
                scale: 0.4,
                x: 0.5,
                y: 0.5,
                aspect: 1.0,
                rotation: deg,
                animation: None,
            };
            let corners = super::sticker_corners(&place, frame, 0.0);
            let bounds = super::corners_bounds(&corners);
            let rot_rect = super::rotation_grip_rect(bounds);
            assert!(
                rot_rect.max.y < bounds.min.y,
                "at {deg} deg the handle overlaps the sticker: {rot_rect:?} vs {bounds:?}"
            );
        }
    }

    /// The rotation stem has to reach the sticker's topmost corner, or it
    /// appears to come off nothing after a turn.
    #[test]
    fn the_stem_reaches_the_topmost_corner() {
        let frame = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 360.0));
        for deg in [0.0f32, 20.0, 45.0, 90.0, 160.0] {
            let place = caprust_core::OverlayPlacement {
                scale: 0.3,
                x: 0.5,
                y: 0.5,
                aspect: 1.0,
                rotation: deg,
                animation: None,
            };
            let corners = super::sticker_corners(&place, frame, 0.0);
            let bounds = super::corners_bounds(&corners);
            let rot_rect = super::rotation_grip_rect(bounds);
            let top = corners
                .iter()
                .copied()
                .fold(corners[0], |a, b| if b.y < a.y { b } else { a });
            // The handle sits above the topmost corner on the vertical
            // line through the centre, so the stem is never zero-length
            // and never starts from empty space.
            assert!(
                rot_rect.max.y < top.y,
                "at {deg} deg the handle is below the sticker"
            );
            assert!(
                (rot_rect.center().x - bounds.center().x).abs() < 0.01,
                "the handle drifted off the centre line at {deg} deg"
            );
        }
    }

    /// `sticker_corners` has to be turned, checked against the geometry
    /// rather than through the hit test.
    ///
    /// The reachability test cannot see this on its own: it feeds
    /// `sticker_corners`' output to `sticker_handle_at`, so if the corners
    /// stopped being turned both sides would move together and the test
    /// would still pass. Two functions agreeing on the same wrong answer
    /// is not a check.
    #[test]
    fn sticker_corners_are_turned_when_the_sticker_is() {
        let frame = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 360.0));
        let base = caprust_core::OverlayPlacement {
            scale: 0.3,
            x: 0.5,
            y: 0.5,
            aspect: 1.5, // deliberately not square
            rotation: 0.0,
            animation: None,
        };

        let flat = super::sticker_corners(&base, frame, 0.0);
        let (_, _, bw, bh) = base.frame_box(640.0 / 360.0);
        assert!(
            (flat[2].x - flat[0].x - frame.width() * bw).abs() < 0.01,
            "the top edge is {} wide, the frame says {}",
            flat[2].x - flat[0].x,
            frame.width() * bw
        );
        assert!(
            (flat[3].y - flat[0].y - frame.height() * bh).abs() < 0.01,
            "the side edge is {} tall, the frame says {}",
            flat[3].y - flat[0].y,
            frame.height() * bh
        );

        // Turn it. Both edges must leave the axes, and the top edge must
        // be the one that grows on a wide sticker.
        let turned = super::sticker_corners(
            &caprust_core::OverlayPlacement {
                rotation: 30.0,
                ..base.clone()
            },
            frame,
            0.0,
        );
        let top = turned[1] - turned[0];
        assert!(top.y.abs() > 1.0, "the top edge is still flat: {top:?}");
        let side = turned[3] - turned[0];
        assert!(side.x.abs() > 1.0, "the side edge is still flat: {side:?}");

        // A rectangle: the turn is rigid, so the edges stay perpendicular
        // and the same length.
        assert!(
            top.dot(side).abs() < 1.0,
            "the turned corners are not a rectangle"
        );
        let turned_place = caprust_core::OverlayPlacement {
            rotation: 30.0,
            ..base.clone()
        };
        let (_, _, bw2, bh2) = turned_place.frame_box(640.0 / 360.0);
        assert!(
            (top.length() - frame.width() * bw2).abs() < 0.5,
            "turning changed the width: {} vs {}",
            top.length(),
            frame.width() * bw2
        );
        assert!(
            (side.length() - frame.height() * bh2).abs() < 0.5,
            "turning changed the height: {} vs {}",
            side.length(),
            frame.height() * bh2
        );

        // And the centre did not move.
        let centre = flat[0].to_vec2() + (flat[2] - flat[0]) / 2.0;
        let centre2 = turned[0].to_vec2() + (turned[2] - turned[0]) / 2.0;
        assert!(
            (centre - centre2).length() < 0.01,
            "turning moved the centre: {centre:?} -> {centre2:?}"
        );
    }

    /// The four corners are ordered around the sticker, not scrambled.
    /// A grip drawn on the wrong pair is a grip that scales the wrong way.
    #[test]
    fn the_corners_are_in_order_around_the_sticker() {
        let frame = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 360.0));
        for deg in [0.0f32, 25.0, 90.0, 180.0, 270.0] {
            let c = super::sticker_corners(
                &caprust_core::OverlayPlacement {
                    scale: 0.3,
                    x: 0.5,
                    y: 0.5,
                    aspect: 1.0,
                    rotation: deg,
                    animation: None,
                },
                frame,
                0.0,
            );
            for i in 0..4 {
                let a = c[i];
                let b = c[(i + 1) % 4];
                assert!(
                    a.distance(b) > 1.0,
                    "at {deg} deg two corners collapsed: {a:?} {b:?}"
                );
            }
            // Opposite corners are diagonal to each other, so their
            // midpoint is the centre and they are not adjacent.
            for i in 0..2 {
                let o1 = c[i];
                let o2 = c[(i + 2) % 4];
                assert!(
                    o1.distance(o2) > 10.0,
                    "at {deg} deg corners {i} and {} are adjacent",
                    (i + 2) % 4
                );
            }
        }
    }

    /// The handles have to come from `sticker_corners`.
    ///
    /// This one cannot be caught geometrically. The reachability test
    /// above compares the bounding box's corners against whatever the
    /// hit test says, so if the grips were moved back onto the bounding
    /// box both sides move together and every assertion still holds --
    /// two pieces of code agreeing on the same wrong answer. It takes a
    /// look at the source to catch, which is not a great test and is here
    /// because this is the fourth UI-only defect in a row that no
    /// geometric one would have seen.
    #[test]
    fn the_handles_are_built_from_the_sticker_corners() {
        let src = include_str!("app.rs");
        let panel = src
            .split_once("fn preview_sticker_handles")
            .expect("no preview_sticker_handles")
            .1;
        let panel = &panel[..panel.find("\n    fn ").unwrap_or(panel.len())];

        assert!(
            panel.contains("sticker_corners(&place, frame_rect"),
            "the handles no longer use the sticker's own corners"
        );
        assert!(
            !panel.contains("left_top()") && !panel.contains("right_bottom()"),
            "the handles reconstruct corners from the bounding box instead"
        );
        assert!(
            panel.contains("for corner in corners"),
            "the grips are not drawn on the sticker's corners"
        );
    }
    /// A sticker whose centre leaves the frame cannot be dragged back,
    /// because there is nothing on the picture to grab. The drag keeps it
    /// just inside.
    #[test]
    fn dragging_out_past_the_edge_keeps_the_sticker_grabbable() {
        let over = super::OVERDRAG;
        for (cx, cy) in [
            (over, over),
            (1.0 + over, 1.0 + over),
            (-over, 1.0 + over),
            (0.5, 0.5),
        ] {
            let p = caprust_core::OverlayPlacement::at_centre(cx, cy, 0.34, 1.0, 16.0 / 9.0, 0.0);
            let (l, t, w, h) = p.frame_box(16.0 / 9.0);
            // Not "the centre is inside" -- it deliberately is not, that
            // is the overhang. The property that matters is that the mark
            // still overlaps the frame, so it is on screen and there is
            // something to grab to bring it back.
            assert!(
                l < 1.0 && l + w > 0.0 && t < 1.0 && t + h > 0.0,
                "a centre of ({cx}, {cy}) put the mark at ({l}, {t}) {:.2}x{:.2}, \
                 entirely off the frame",
                w,
                h
            );
            // And the centre is never more than the overhang out, so the
            // handle stays on the picture to drag back. The tolerance is
            // for the solve's own rounding, not for a real excursion.
            const EPS: f32 = 1e-4;
            let bcx = l + w / 2.0;
            assert!(
                bcx >= -over - EPS && bcx <= 1.0 + over + EPS,
                "the centre of a mark dragged to ({cx}, {cy}) landed at {bcx}"
            );
        }
        // And the overhang is real: the mark can be pushed off the edge
        // on purpose.
        assert!(over > 0.0, "no overhang at all");
    }
    /// The clamp the drag actually applies, not just the constant it is
    /// built from.
    #[test]
    fn the_drag_holds_the_centre_within_reach() {
        let over = super::OVERDRAG;
        for (cx, cy) in [
            (0.5f32, 0.5f32),
            (-5.0, 5.0),
            (5.0, -5.0),
            (1.0, 0.0),
            (0.0, 1.0),
            (0.08, 1.08),
            (-0.08, -0.08),
        ] {
            let p = super::clamp_centre(cx, cy);
            assert!(
                (-over..=1.0 + over).contains(&p.x),
                "x={} survived from {cx}",
                p.x
            );
            assert!(
                (-over..=1.0 + over).contains(&p.y),
                "y={} survived from {cy}",
                p.y
            );
            assert!(p.x.is_finite() && p.y.is_finite());
        }
    }

    /// ...and the move gesture goes through it rather than clamping by
    /// hand, which is the part that is otherwise only visible by reading
    /// the branch.
    #[test]
    fn the_move_gesture_uses_the_shared_clamp() {
        let src = include_str!("app.rs");
        let panel = src
            .split_once("fn preview_sticker_handles")
            .expect("no preview_sticker_handles")
            .1;
        let panel = &panel[..panel.find("\n    fn ").unwrap_or(panel.len())];
        assert!(
            panel.contains("clamp_centre("),
            "the move branch clamps by hand instead of through clamp_centre"
        );
        assert!(
            !panel.contains(".clamp(-OVERDRAG"),
            "a second clamp crept in beside the shared one"
        );
    }
    /// The size slider's top of range is what fits, not a constant.
    ///
    /// `at_centre` clamps the stored value whatever the panel offers, so a
    /// slider that went past the limit would not corrupt anything -- it
    /// would just show a number the render cannot use, and snap back. No
    /// core test can see a panel's slider, so this looks.
    #[test]
    fn the_size_slider_stops_where_the_limit_is() {
        let src = include_str!("../src/panels/clip_properties.rs");
        let seg = src
            .split_once("fn show_sticker_placement")
            .expect("no show_sticker_placement")
            .1;
        let seg = &seg[..seg.find("\nfn ").unwrap_or(seg.len())];

        assert!(
            seg.contains("0.02..=max_scale"),
            "the size slider is not bounded by max_scale"
        );
        assert!(
            seg.contains("let max_scale = place.max_scale(fa);"),
            "the slider's limit is not the placement's own"
        );
        assert!(
            !seg.contains("0.02..=2.0"),
            "the old hard-coded slider range is still here"
        );
    }
    /// The two coordinate systems in the overlay, and where they meet.
    ///
    /// A placement's `x` and `y` are fractions of the frame; the painter,
    /// the handles and `drag_delta` are in screen pixels. The drag used to
    /// keep its frozen centre in pixels and hand it to `at_centre` as
    /// though it were a fraction, so a scale or a rotate solved
    /// `x = (974 - 0.07) / 0.85`, clamped to the maximum, and the sticker
    /// jumped to the corner of the frame every time it was touched -- and
    /// stayed there, because the next touch did the same thing again.
    #[test]
    fn to_unit_turns_screen_pixels_into_frame_fractions() {
        let frame = egui::Rect::from_min_size(egui::pos2(450.0, 180.0), egui::vec2(1048.0, 442.0));
        // A point that is a sane pixel coordinate and an insane fraction.
        let p = egui::pos2(974.0, 401.0);
        let (x, y) = super::to_unit(p, frame);
        assert!((x - 0.5).abs() < 1e-3, "x = {x}");
        assert!((y - 0.5).abs() < 1e-3, "y = {y}");
        assert!(
            (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
            "a point inside the frame did not land inside the fractions"
        );

        // The corners map to the corners.
        let (l, t) = super::to_unit(frame.left_top(), frame);
        assert!(l.abs() < 1e-6 && t.abs() < 1e-6, "{l}, {t}");
        let (r, b) = super::to_unit(frame.right_bottom(), frame);
        assert!((r - 1.0).abs() < 1e-6 && (b - 1.0).abs() < 1e-6, "{r}, {b}");

        // And back out: the placement built from a frame-fraction centre
        // puts its box where the painter says, which is the whole point
        // of keeping the two straight.
        let place =
            caprust_core::OverlayPlacement::at_centre(0.5, 0.5, 0.34, 1.0, 1048.0 / 442.0, 0.0);
        let (bl, bt, bw, bh) = place.frame_box(1048.0 / 442.0);
        assert!(
            ((bl + bw / 2.0) * 1048.0 - 524.0).abs() < 0.5,
            "the box's centre is {} px from the frame's centre",
            ((bl + bw / 2.0) * 1048.0 - 524.0).abs()
        );
        let _ = bt;
        let _ = bh;
    }

    /// A centre that has been frozen in pixels must not survive as one.
    #[test]
    fn a_pixel_centre_never_reaches_at_centre() {
        // The arithmetic the bug used to perform, spelled out: 974 is a
        // screen x, and feeding it to the fraction solver gives a
        // position past the end of the frame. If `at_centre` is handed a
        // fraction this small there is nothing to see; if the UI hands it
        // a pixel, this is what comes out, and the clamp hides the
        // damage -- which is why the test is on the clamp as well.
        let place =
            caprust_core::OverlayPlacement::at_centre(974.0, 400.0, 0.34, 1.0, 16.0 / 9.0, 0.0);
        let (l, t, w, h) = place.frame_box(16.0 / 9.0);
        // It is clamped to the export range, so it stays finite and
        // representable -- which is the best that can be asked of a
        // garbage input.
        assert!((l - (1.0 - w) * place.x).abs() < 1e-6);
        assert!((t - (1.0 - h) * place.y).abs() < 1e-6);
        assert!(place.x.is_finite() && place.y.is_finite());
    }

    /// The drag state stores fractions. This is the actual defect and the
    /// only thing that catches it: a source look, because the type
    /// system cannot distinguish a fraction from a pixel once both are
    /// f32s.
    #[test]
    fn the_drag_state_and_geometry_carry_fractions() {
        let src = include_str!("app.rs");
        let panel = src
            .split_once("fn preview_sticker_handles")
            .expect("no preview_sticker_handles")
            .1;
        let panel = &panel[..panel.find("\n    fn ").unwrap_or(panel.len())];
        let code: String = panel
            .lines()
            .map(|l| match l.find("//") {
                Some(i) => &l[..i],
                None => l,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            code.contains("let centre = to_unit(bounds.center(), frame_rect);"),
            "the gesture centre is not converted to frame fractions"
        );
        assert!(
            !code.contains("bounds.center(),") || !code.contains("centre: bounds.center()"),
            "a pixel centre is still being handed over as a fraction"
        );
    }
    /// The four animation names mean the same thing in the preview as in
    /// the filtergraph, and they do not exist there by accident.
    #[test]
    fn every_animation_name_is_one_the_render_knows() {
        let src = include_str!("app.rs");
        // The preview's own list.
        let preview: Vec<&str> = {
            let seg = src
                .split_once("pub fn sticker_motion")
                .expect("no sticker_motion")
                .1;
            let seg = &seg[..seg.find("\n}\n").unwrap_or(seg.len())];
            ["pop", "bounce", "spin", "drift"]
                .into_iter()
                .filter(|n| seg.contains(&format!("\"{n}\"")))
                .collect()
        };
        // The render's list, from the same source of truth.
        let render_src = include_str!("../../media-io/src/export_graph.rs");
        let frag = render_src
            .split_once("fn sticker_animation_fragment")
            .expect("no sticker_animation_fragment")
            .1;
        let frag = &frag[..frag.find("\n}\n").unwrap_or(frag.len())];

        for name in ["pop", "bounce", "spin", "drift"] {
            assert!(
                preview.contains(&name),
                "`{name}` is animated in the preview but not in the render, or the source check missed it"
            );
            assert!(
                frag.contains(&format!("\"{name}\" =>")),
                "`{name}` is not one of the render's animations"
            );
        }
    }

    /// A static sticker does not repaint on its own, and an animated one
    /// does. The second is the whole mechanism -- without the request the
    /// motion plays once and stops.
    #[test]
    fn the_quad_carries_its_animation_and_clock() {
        let src = include_str!("app.rs");
        let seg = src
            .split_once("pub struct StickerQuad")
            .expect("no StickerQuad")
            .1;
        let seg = &seg[..seg.find("\n}\n").unwrap_or(seg.len())];
        assert!(seg.contains("pub animation: Option<String>"));
        assert!(seg.contains("pub t_sec: f32"));
    }

    /// `sticker_motion` has to move the sticker where the filtergraph
    /// puts it, and a name it does not know has to mean "still".
    #[test]
    fn sticker_motion_matches_the_filtergraph() {
        // 256 source px, 128 px on screen, so offsets halve.
        let k = 0.5;

        // Static and unknown names are still.
        assert_eq!(
            super::sticker_motion(None, 3.0, 256.0, 128.0),
            super::StickerMotion::default()
        );
        assert_eq!(
            super::sticker_motion(Some("nonsense"), 3.0, 256.0, 128.0),
            super::StickerMotion::default()
        );

        // Pop is a fade over 0.2 s, nothing else.
        let at0 = super::sticker_motion(Some("pop"), 0.0, 256.0, 128.0);
        assert!(
            (at0.alpha - 0.0).abs() < 1e-6,
            "alpha at t=0 is {}",
            at0.alpha
        );
        assert_eq!(at0.angle, 0.0);
        let at02 = super::sticker_motion(Some("pop"), 0.2, 256.0, 128.0);
        assert!((at02.alpha - 1.0).abs() < 1e-6);
        let at1 = super::sticker_motion(Some("pop"), 1.0, 256.0, 128.0);
        assert!((at1.alpha - 1.0).abs() < 1e-6, "the fade must not reverse");

        // Spin turns at 1.6 rad/s and moves nothing.
        let s = super::sticker_motion(Some("spin"), 1.0, 256.0, 128.0);
        assert!((s.angle - 1.6).abs() < 1e-6, "angle is {}", s.angle);
        assert_eq!(s.dx, 0.0);
        assert_eq!(s.dy, 0.0);
        assert_eq!(s.alpha, 1.0);

        // Bounce is the decaying hop: 18 source px up at the peak, at
        // 1.6 Hz, scaled to screen.
        let b = super::sticker_motion(Some("bounce"), 0.0, 256.0, 128.0);
        assert_eq!(b.dy, 0.0, "no hop at t=0");
        assert!((b.scale - 1.25).abs() < 1e-6);
        let peak_t = std::f32::consts::FRAC_PI_2 / (2.0 * std::f32::consts::PI * 1.6);
        let b2 = super::sticker_motion(Some("bounce"), peak_t, 256.0, 128.0);
        let expected = -18.0 * 1.0 * (-3.2 * peak_t).exp() * k;
        assert!(
            (b2.dy - expected).abs() < 0.05,
            "the hop is {} at the peak, the filtergraph says {}",
            b2.dy,
            expected
        );
        // And it decays.
        let late = super::sticker_motion(Some("bounce"), 3.0, 256.0, 128.0);
        assert!(
            late.dy.abs() < 1.0,
            "the hop should have decayed, it is {}",
            late.dy
        );

        // Drift is the figure of eight: 26 across over 6 s, 16 down over
        // 9 s, with the 1.15 the filtergraph uses.
        let d = super::sticker_motion(Some("drift"), 0.0, 256.0, 128.0);
        assert!((d.scale - 1.15).abs() < 1e-6);
        assert!((d.dx - 0.0).abs() < 1e-6, "sin(0) is 0");
        assert!((d.dy - 16.0 * k).abs() < 1e-6, "cos(0) is 1: dy={}", d.dy);
        let d2 = super::sticker_motion(Some("drift"), 3.0, 256.0, 128.0);
        let want = 26.0 * (2.0 * std::f32::consts::PI * 3.0 / 6.0).sin() * k;
        assert!((d2.dx - want).abs() < 1e-6, "{} != {}", d2.dx, want);
    }

    /// The tab state must be normalised once per clip, not every frame.
    /// As a per-frame rule it undid a click on the next frame, which is
    /// what "Efekti is not clickable" was.
    #[test]
    fn the_tab_is_not_forced_back_every_frame() {
        let src = include_str!("app.rs");
        // The normalisation lives in the properties panel.
        let panel = include_str!("../src/panels/clip_properties.rs");
        let seg = panel
            .split_once("let clip_is_audio = matches!")
            .expect("no clip_is_audio")
            .1;
        let seg = &seg[..seg.find("egui::ScrollArea").unwrap_or(seg.len())];

        assert!(
            seg.contains("tab_normalised_for"),
            "the tab normalisation is not scoped to the selection"
        );
        assert!(
            !seg.contains("if clip_is_sticker && state.tab != PropertiesTab::Sticker {\n        state.tab = PropertiesTab::Sticker;\n    }"),
            "the per-frame forced reset is still there"
        );
        let _ = src;
    }

    /// `sticker_quads` has to carry the clip's animation and its clock.
    ///
    /// Checking the struct has the fields is not the same as checking it
    /// fills them: setting either back to a default still compiled and
    /// still passed a field-presence test, so three mutations went
    /// through. This builds a sticker that is on screen and looks.
    #[test]
    fn sticker_quads_carries_the_animation_and_the_clock() {
        use caprust_core::clip::{Clip, ClipType, OverlayPlacement};
        use caprust_core::track::{Track, TrackKind};

        let mut p = caprust_core::ProjectState::default();
        p.tracks.clear();
        p.tracks.push(Track::new("V1", TrackKind::Video));

        for (id, anim) in [(1u128, Some("pop")), (2, Some("spin")), (3, None)] {
            let mut c = Clip::new_video("s.png", 0, 0, 4000);
            c.id = uuid::Uuid::from_u128(id);
            c.clip_type = ClipType::Image {
                path: format!("s{id}.png"),
                duration_ms: 4000,
            };
            c.overlay = Some(OverlayPlacement {
                scale: 0.3,
                x: 0.5,
                y: 0.5,
                aspect: 1.0,
                rotation: 0.0,
                animation: anim.map(str::to_string),
            });
            p.clips.push(c);
        }

        let frame = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 360.0));
        let quads = super::sticker_quads(&p, 2500, frame);
        assert_eq!(quads.len(), 3);

        let anims: Vec<Option<String>> = quads.iter().map(|q| q.animation.clone()).collect();
        assert_eq!(anims[0].as_deref(), Some("pop"));
        assert_eq!(anims[1].as_deref(), Some("spin"));
        assert_eq!(anims[2], None);

        // 2.5 s in, so the clock has moved and an animation would have
        // visible progress.
        for q in &quads {
            assert!(
                (q.t_sec - 2.5).abs() < 1e-3,
                "the clock is {} instead of 2.5 s",
                q.t_sec
            );
        }

        // And off the playhead there is nothing to animate.
        assert!(super::sticker_quads(&p, 9000, frame).is_empty());
    }

    /// One implementation of the geometry. A second copy inline in
    /// `sticker_quads` had already appeared once, and two copies of the
    /// same arithmetic are how the handle and the artwork drift apart.
    #[test]
    fn sticker_quads_has_no_geometry_of_its_own() {
        let src = include_str!("app.rs");
        let seg = src
            .split_once("pub fn sticker_quads")
            .expect("no sticker_quads")
            .1;
        let seg = &seg[..seg.find("\n}\n").unwrap_or(seg.len())];
        assert!(
            seg.contains("sticker_corners(place, frame_rect, 0.0)"),
            "sticker_quads is not built from sticker_corners"
        );
        assert!(
            !seg.contains("frame_box_at(fa"),
            "sticker_quads has its own copy of the geometry"
        );
    }

    /// An animated sticker asks to be repainted.
    ///
    /// Without it the motion is computed correctly for exactly one frame
    /// and then the sticker sits still, because nothing makes egui draw
    /// again. No unit test can observe a repaint being scheduled, so this
    /// reads the source -- which is a weaker check, and is here because
    /// the alternative was to leave the mechanism untested and assume it
    /// worked.
    #[test]
    fn an_animated_sticker_keeps_asking_to_be_repainted() {
        let src = include_str!("app.rs");
        let seg = src
            .split_once("fn paint_sticker_quad")
            .expect("no paint_sticker_quad")
            .1;
        let seg = &seg[..seg.find("\n    fn ").unwrap_or(seg.len())];

        assert!(
            seg.contains("request_repaint_after"),
            "the painter does not ask to be repainted, so an animation plays once"
        );
        assert!(
            seg.contains("q.animation.is_some()"),
            "the repaint is not gated on the sticker being animated"
        );
    }
    /// Every track has to be on screen at every timeline size.
    ///
    /// In L the rows are 1.7 times taller, five of them did not fit, and
    /// the last one was cut off with no way to scroll to it. The set is
    /// now scaled down together to fit, which keeps the proportions
    /// between the tracks -- a track twice as tall stays twice as tall.
    #[test]
    fn every_track_fits_at_every_timeline_size() {
        use caprust_core::TimelineSize;

        let heights = [46.0f32, 46.0, 46.0, 46.0, 46.0];
        for available in [150.0f32, 300.0, 442.0, 900.0] {
            for size in [TimelineSize::S, TimelineSize::M, TimelineSize::L] {
                let layout = super::lane_layout(size, &heights, available);
                let drawn: f32 = heights.iter().map(|h| h * layout.factor).sum();
                assert!(
                    drawn <= available + 0.01,
                    "{size:?} at {available}px: the rows add up to {drawn}"
                );
                // And when they do not fit, they fill it exactly --
                // there is no wasted space at the bottom.
                if layout.fitted {
                    assert!(
                        (drawn - available).abs() < 0.01,
                        "{size:?} at {available}px: the rows add up to {drawn}"
                    );
                }
            }
        }
    }

    /// A size that fits is left alone. Squeezing the rows when there is
    /// room would make the size setting mean nothing.
    #[test]
    fn a_size_that_fits_is_left_alone() {
        use caprust_core::TimelineSize;
        let heights = [46.0f32, 46.0];
        assert_eq!(
            super::lane_layout(TimelineSize::S, &heights, 900.0).factor,
            1.0
        );
        assert_eq!(
            super::lane_layout(TimelineSize::M, &heights, 900.0).factor,
            1.3
        );
        assert_eq!(
            super::lane_layout(TimelineSize::L, &heights, 900.0).factor,
            1.7
        );
        // And it says so, so the drag can use the plain rate.
        assert!(!super::lane_layout(TimelineSize::L, &heights, 900.0).fitted);
    }

    /// The proportions between the tracks survive the scaling. A track
    /// twice as tall as another is still twice as tall on screen.
    #[test]
    fn the_proportions_between_tracks_survive() {
        use caprust_core::TimelineSize;
        let heights = [92.0f32, 46.0, 46.0];
        let layout = super::lane_layout(TimelineSize::L, &heights, 200.0);
        assert!(layout.fitted, "this should not have fitted");
        let drawn: Vec<f32> = heights.iter().map(|h| h * layout.factor).collect();
        assert!(
            (drawn[0] / drawn[1] - 2.0).abs() < 1e-4,
            "the tall track is {} times the short one",
            drawn[0] / drawn[1]
        );
    }

    /// The divider has to stay under the pointer, which is what the
    /// manual resize is for.
    ///
    /// When the rows are scaled to fit, the row height is
    /// `height * available / total` and the total moves as the dragged
    /// height moves, so the marginal rate is not the factor. Dividing by
    /// the plain factor left the divider trailing the pointer by the
    /// dragged track's share of the height -- on a five-track timeline
    /// that is a fifth of the movement lost on every frame.
    #[test]
    fn the_divider_follows_the_pointer_whether_or_not_the_rows_are_scaled() {
        use caprust_core::TimelineSize;

        for (heights, available) in [
            (vec![46.0f32, 46.0, 46.0], 900.0),             // fits
            (vec![80.0f32, 80.0, 80.0, 80.0, 80.0], 200.0), // scaled
        ] {
            let layout = super::lane_layout(TimelineSize::L, &heights, available);
            for (i, h) in heights.iter().enumerate() {
                // Small enough that the 40..160 clamp on the stored
                // height is not what is being measured. The clamp has
                // its own test.
                for dy in [3.0f32, -3.0, 6.0, -6.0] {
                    // Through the production function, not a copy of
                    // it: the drag has to be tested where it happens.
                    let after = super::stored_height_after_drag(*h, dy, &layout);
                    let per_stored = layout.row_height_per_stored(*h);
                    let moved = per_stored * (after - *h);
                    assert!(
                        (moved - dy).abs() < 0.01,
                        "track {i}: a {dy}px drag moved the divider {moved}"
                    );
                }
            }
        }
    }

    /// The stored height is bounded, so a drag that would take it past
    /// the bound stops there. The divider then does not move the full
    /// distance, which is correct: the row cannot be shorter than the
    /// minimum.
    #[test]
    fn the_drag_stops_at_the_bounds() {
        use caprust_core::TimelineSize;
        let heights = [46.0f32, 46.0, 46.0];
        let layout = super::lane_layout(TimelineSize::L, &heights, 900.0);

        // Down, hard, from just above the minimum.
        let after = super::stored_height_after_drag(41.0, -100.0, &layout);
        assert!(
            (after - 40.0).abs() < 1e-3,
            "the height went to {after}, not the 40 floor"
        );
        // And up, from just below the maximum.
        let after = super::stored_height_after_drag(159.0, 100.0, &layout);
        assert!(
            (after - 160.0).abs() < 1e-3,
            "the height went to {after}, not the 160 ceiling"
        );
    }

    /// The marginal rate has to be what the layout says it is, and not
    /// the plain factor when the rows are scaled.
    #[test]
    fn the_marginal_rate_is_not_the_plain_factor_when_scaled() {
        use caprust_core::TimelineSize;
        let heights = [92.0f32, 46.0, 46.0, 46.0, 46.0];
        let layout = super::lane_layout(TimelineSize::L, &heights, 200.0);
        assert!(layout.fitted);
        // The tall track's share is bigger, so its rows move less per
        // unit of stored height.
        let tall = layout.row_height_per_stored(92.0);
        let short = layout.row_height_per_stored(46.0);
        assert!(
            tall < short,
            "the tall track moves {tall} per unit, the short one {short}"
        );
        assert!(tall < layout.factor, "the tall one is not discounted");
    }
}
