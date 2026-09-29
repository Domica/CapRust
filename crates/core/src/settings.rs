//! App-wide settings persisted across sessions (not project-specific).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Current version of the on-disk settings export format.
///
/// Bump when the shape of `AppSettings` changes in a way that is not
/// backward-compatible. `import_from_file` refuses to load any file
/// with a higher number so a user cannot accidentally downgrade a
/// newer export into an older binary.
pub const SETTINGS_FILE_VERSION: u32 = 1;

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

    /// Write this settings snapshot to `path` as JSON.
    ///
    /// Creates parent directories. The file is wrapped in a
    /// [`SettingsFile`] envelope so future migrations have a version
    /// field and the metadata to reason about (app version, format
    /// version). Overwrites an existing file.
    pub fn export_to_file(&self, path: &Path) -> Result<()> {
        let file = SettingsFile {
            caprust_settings_version: SETTINGS_FILE_VERSION,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            settings: self.clone(),
        };
        let json = serde_json::to_string_pretty(&file).context("serialize AppSettings to JSON")?;

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create dir {}", parent.display()))?;
            }
        }
        fs::write(path, json).with_context(|| format!("write {}", path.display()))?;
        tracing::info!("settings exported to {}", path.display());
        Ok(())
    }

    /// Read a settings snapshot from `path`.
    ///
    /// Returns a fresh `AppSettings`; the caller decides whether to
    /// apply it. Errors if the file is not valid JSON, if the
    /// envelope is missing `settings`, or if the file was written by
    /// a newer format version than this binary knows about.
    ///
    /// A missing `caprust_settings_version` field is treated as
    /// version 1: hand-authored files and exports from before the
    /// version field existed still load.
    pub fn import_from_file(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let file: SettingsFile = serde_json::from_str(&text).context("parse settings JSON")?;

        if file.caprust_settings_version > SETTINGS_FILE_VERSION {
            bail!(
                "settings file is version {} but this build knows only up to {}; \
                 upgrade CapRust to import it",
                file.caprust_settings_version,
                SETTINGS_FILE_VERSION
            );
        }

        tracing::info!(
            "settings imported from {} (version {}, app {})",
            path.display(),
            file.caprust_settings_version,
            file.app_version,
        );
        Ok(file.settings)
    }
}

/// On-disk envelope for a settings export.
///
/// The `settings` field is the payload. The two fields above it are
/// metadata: the format version lets a future build detect and migrate
/// older exports, and `app_version` records which release wrote the
/// file. Keeping these separate from `AppSettings` means the payload
/// struct does not carry versioning noise of its own.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsFile {
    #[serde(default = "default_settings_version")]
    pub caprust_settings_version: u32,
    #[serde(default)]
    pub app_version: String,
    pub settings: AppSettings,
}

fn default_settings_version() -> u32 {
    SETTINGS_FILE_VERSION
}

/// Path helper: where the app would put a user-facing export by
/// default if the user does not choose a location. Not used by the
/// export itself (that goes through the file dialog) but handy for
/// tests and future CLI tooling.
pub fn default_export_path() -> PathBuf {
    let base = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("caprust-settings.json")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "caprust_settings_{}_{}.json",
            label,
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn export_import_roundtrip_preserves_all_fields() {
        let a = AppSettings {
            language: "hr".into(),
            master_volume: 0.42,
            muted: true,
            translate_source_lang: "en".into(),
            translate_target_lang: "de".into(),
            translate_email: Some("user@example.com".into()),
            dock_layout: Some(r#"{"x":1}"#.into()),
            docked_panels: vec!["text".into(), "effects".into()],
            ..Default::default()
        };

        let p = tmp_path("roundtrip");
        a.export_to_file(&p).unwrap();
        let b = AppSettings::import_from_file(&p).unwrap();

        assert_eq!(a.language, b.language);
        assert_eq!(a.master_volume, b.master_volume);
        assert_eq!(a.muted, b.muted);
        assert_eq!(a.translate_source_lang, b.translate_source_lang);
        assert_eq!(a.translate_target_lang, b.translate_target_lang);
        assert_eq!(a.translate_email, b.translate_email);
        assert_eq!(a.dock_layout, b.dock_layout);
        assert_eq!(a.docked_panels, b.docked_panels);

        let _ = fs::remove_file(&p);
    }

    #[test]
    fn export_creates_parent_directories() {
        let dir =
            std::env::temp_dir().join(format!("caprust_settings_nested_{}", uuid::Uuid::new_v4()));
        let p = dir.join("sub").join("settings.json");

        let a = AppSettings::default();
        a.export_to_file(&p).unwrap();
        assert!(p.is_file(), "file should exist after export");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_rejects_newer_version() {
        let p = tmp_path("newer");
        let body = format!(
            r#"{{"caprust_settings_version": 999, "app_version": "future", "settings": {}}}"#,
            serde_json::to_string(&AppSettings::default()).unwrap(),
        );
        fs::write(&p, body).unwrap();

        let err = AppSettings::import_from_file(&p).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("version 999") && msg.contains("upgrade"),
            "unexpected message: {msg}"
        );

        let _ = fs::remove_file(&p);
    }

    #[test]
    fn import_accepts_missing_version_field() {
        let p = tmp_path("noversion");
        let body = format!(
            r#"{{"settings": {}}}"#,
            serde_json::to_string(&AppSettings::default()).unwrap(),
        );
        fs::write(&p, body).unwrap();

        let s = AppSettings::import_from_file(&p).unwrap();
        assert_eq!(s.language, AppSettings::default().language);

        let _ = fs::remove_file(&p);
    }

    #[test]
    fn import_rejects_malformed_json() {
        let p = tmp_path("malformed");
        fs::write(&p, b"this is not json").unwrap();

        let err = AppSettings::import_from_file(&p).unwrap_err();
        assert!(
            err.to_string().contains("parse settings JSON"),
            "unexpected message: {err}"
        );

        let _ = fs::remove_file(&p);
    }

    #[test]
    fn import_rejects_missing_settings_key() {
        let p = tmp_path("nosettings");
        fs::write(&p, br#"{"caprust_settings_version": 1}"#).unwrap();

        // `settings` has no #[serde(default)] — the field is mandatory.
        let err = AppSettings::import_from_file(&p).unwrap_err();
        assert!(err.to_string().contains("parse settings JSON"));

        let _ = fs::remove_file(&p);
    }

    #[test]
    fn import_rejects_nonexistent_file() {
        let err =
            AppSettings::import_from_file(Path::new("Z:/caprust_missing_xyz.json")).unwrap_err();
        assert!(err.to_string().contains("read"));
    }

    #[test]
    fn default_export_path_is_under_home() {
        let p = default_export_path();
        assert!(p.to_string_lossy().ends_with("caprust-settings.json"));
    }
}
