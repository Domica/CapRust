//! App-wide settings persisted across sessions (not project-specific).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// UI language: "en" or "hr".
    pub language: String,
    /// Where AI models are downloaded.
    pub models_dir: String,
    /// Optional override for ffmpeg binary path. If None, we look in PATH.
    pub ffmpeg_path: Option<String>,
    /// Optional override for ffprobe binary path.
    pub ffprobe_path: Option<String>,
    /// Keyboard shortcuts on/off (mirrors the old flag).
    pub enable_shortcuts: bool,
    /// Master audio volume for preview playback, 0.0..=1.0.
    /// Applied to the cpal stream on top of any per-clip volume.
    /// Does not affect export (export uses each clip's own volume_db).
    #[serde(default = "default_master_volume")]
    pub master_volume: f32,
    /// When true, the preview audio stream is muted regardless of
    /// master_volume. Persisted so the state survives restarts.
    #[serde(default)]
    pub muted: bool,
    /// When true, the app checks GitHub Releases for a newer version on
    /// startup (throttled to once per 24 h, silent on failure).
    #[serde(default = "default_true")]
    pub check_for_updates: bool,
    /// When true, dragging a clip's trim edge moves the playhead along
    /// with the edge (head → new start, tail → new end). Off by
    /// default; mirrors the "trim follow" toggle in the timeline tray.
    #[serde(default)]
    pub trim_follow: bool,
    /// Where the auto-downloaded FFmpeg lives. None or empty =
    /// `%APPDATA%/CapRust/ffmpeg` (see `ffmpeg::managed_dir`).
    #[serde(default)]
    pub managed_ffmpeg_dir: Option<String>,
    /// Set when the user clicks "Odustani" in the first-run FFmpeg
    /// prompt. Suppresses the modal on subsequent launches until the
    /// user opens Settings → Paths → Download again.
    #[serde(default)]
    pub ffmpeg_prompt_dismissed: bool,
    /// Asset browser tabs that are open as their own floating window,
    /// in addition to the main tab strip. Stored as `AssetTab::id()`
    /// strings ("text", "effects", ...) so the list survives future
    /// enum reordering.
    #[serde(default)]
    pub docked_panels: Vec<String>,
    /// Serialized egui_dock layout tree (`DockState<crate::dock::Tab>`).
    /// Stored as JSON so the core crate does not depend on egui_dock.
    /// None or an unparseable value falls back to
    /// `dock::default_dock_state()`.
    #[serde(default)]
    pub dock_layout: Option<String>,
    /// Captions translation source language. "auto" lets MyMemory
    /// guess; otherwise a two-letter code.
    #[serde(default = "default_translate_source")]
    pub translate_source_lang: String,
    /// Captions translation target language. Defaults to "hr".
    #[serde(default = "default_translate_target")]
    pub translate_target_lang: String,
    /// Optional contact email for MyMemory. The anonymous quota is
    /// ~5k words/day; with a valid address it is ~50k words/day.
    /// Never sent anywhere else.
    #[serde(default)]
    pub translate_email: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_master_volume() -> f32 {
    1.0
}

fn default_translate_source() -> String {
    "auto".to_string()
}

fn default_translate_target() -> String {
    "hr".to_string()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: sys_locale::get_locale()
                .map(|l| l.split('-').next().unwrap_or("en").to_string())
                .filter(|l| matches!(l.as_str(), "en" | "hr"))
                .unwrap_or_else(|| "en".into()),
            models_dir: default_models_dir(),
            ffmpeg_path: None,
            ffprobe_path: None,
            enable_shortcuts: true,
            master_volume: default_master_volume(),
            muted: false,
            check_for_updates: true,
            trim_follow: false,
            managed_ffmpeg_dir: None,
            ffmpeg_prompt_dismissed: false,
            docked_panels: Vec::new(),
            dock_layout: None,
            translate_source_lang: default_translate_source(),
            translate_target_lang: default_translate_target(),
            translate_email: None,
        }
    }
}

fn default_models_dir() -> String {
    let base = std::env::var("APPDATA")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    format!("{base}/CapRust/models")
}

impl AppSettings {
    /// Resolved models path (falls back to default if empty).
    pub fn effective_models_dir(&self) -> std::path::PathBuf {
        if self.models_dir.trim().is_empty() {
            std::path::PathBuf::from(default_models_dir())
        } else {
            std::path::PathBuf::from(&self.models_dir)
        }
    }
}

/// Detect `ffmpeg` and `ffprobe` on the system.
#[derive(Debug, Clone, Default)]
pub struct FfmpegStatus {
    pub ffmpeg: Option<String>,
    pub ffprobe: Option<String>,
}

impl FfmpegStatus {
    pub fn is_available(&self) -> bool {
        self.ffmpeg.is_some() && self.ffprobe.is_some()
    }
}

/// Try to locate ffmpeg/ffprobe. Priority: user override, PATH, then
/// the auto-downloaded managed dir (`crate::ffmpeg::managed_dir`).
pub fn detect_ffmpeg(settings: &AppSettings) -> FfmpegStatus {
    FfmpegStatus {
        ffmpeg: crate::ffmpeg::find_ffmpeg(settings).map(|p| p.to_string_lossy().to_string()),
        ffprobe: crate::ffmpeg::find_ffprobe(settings).map(|p| p.to_string_lossy().to_string()),
    }
}
