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
}

fn default_true() -> bool {
    true
}

fn default_master_volume() -> f32 {
    1.0
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

/// Try to locate ffmpeg/ffprobe — respects overrides, then PATH.
pub fn detect_ffmpeg(settings: &AppSettings) -> FfmpegStatus {
    FfmpegStatus {
        ffmpeg: which_or_override(settings.ffmpeg_path.as_deref(), "ffmpeg"),
        ffprobe: which_or_override(settings.ffprobe_path.as_deref(), "ffprobe"),
    }
}

fn which_or_override(override_path: Option<&str>, name: &str) -> Option<String> {
    if let Some(p) = override_path {
        if !p.trim().is_empty() && std::path::Path::new(p).exists() {
            return Some(p.to_string());
        }
    }
    which_in_path(name)
}

fn which_in_path(name: &str) -> Option<String> {
    let path_var = std::env::var_os("PATH")?;
    let exe_names: Vec<String> = if cfg!(windows) {
        vec![format!("{name}.exe"), name.to_string()]
    } else {
        vec![name.to_string()]
    };
    for dir in std::env::split_paths(&path_var) {
        for exe in &exe_names {
            let candidate = dir.join(exe);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }
    None
}
