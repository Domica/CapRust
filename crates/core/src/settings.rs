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
    /// Waveform-at-bottom clip layout. When true, video clips with
    /// audio shrink the thumbnail strip to the top ~70% and draw the
    /// waveform in a dedicated bottom band; video without audio shows
    /// a darker empty band. Off = legacy overlay (waveform centered
    /// over the full clip). On by default; toggle in Settings →
    /// Appearance → Timeline.
    #[serde(default = "default_true")]
    pub waveform_bottom: bool,
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
    /// Optional folder that cloud clients (Google Drive, OneDrive,
    /// Dropbox, Box, iCloud Drive, Syncthing, Nextcloud) keep in
    /// sync between machines. If set, `save()` writes a snapshot of
    /// this AppSettings to `<sync_folder>/caprust-settings.json` so
    /// the other machine can pick it up on next launch. CapRust does
    /// not talk to any cloud API; the client's folder sync is the
    /// entire transport.
    #[serde(default)]
    pub sync_folder: Option<String>,
    /// Unix seconds of the last successful sync write or load. Used
    /// to decide whether the sync file is newer than what this
    /// machine already knows about. `None` means "this machine has
    /// never synced", which makes any existing sync file newer.
    #[serde(default)]
    pub last_synced_at: Option<u64>,
    /// Optional folder for saved preview frames (PNG). None or empty
    /// = `<Pictures>/CapRust` (see `effective_screenshots_dir`).
    #[serde(default)]
    pub screenshots_dir: Option<String>,
    /// Optional folder for screen recordings. None or empty =
    /// `%APPDATA%/CapRust/recordings` (see
    /// `effective_recordings_dir`).
    #[serde(default)]
    pub recordings_dir: Option<String>,
    /// Optional folder for cached custom LUT (.cube) files. None or
    /// empty = `%APPDATA%/CapRust/luts` (see `effective_lut_cache_dir`).
    #[serde(default)]
    pub lut_cache_dir: Option<String>,
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
                .filter(|l| matches!(l.as_str(), "en" | "hr" | "es" | "it" | "de"))
                .unwrap_or_else(|| "en".into()),
            models_dir: default_models_dir(),
            ffmpeg_path: None,
            ffprobe_path: None,
            enable_shortcuts: true,
            master_volume: default_master_volume(),
            muted: false,
            check_for_updates: true,
            trim_follow: false,
            waveform_bottom: true,
            managed_ffmpeg_dir: None,
            ffmpeg_prompt_dismissed: false,
            docked_panels: Vec::new(),
            dock_layout: None,
            translate_source_lang: default_translate_source(),
            translate_target_lang: default_translate_target(),
            translate_email: None,
            sync_folder: None,
            last_synced_at: None,
            screenshots_dir: None,
            recordings_dir: None,
            lut_cache_dir: None,
        }
    }
}

fn default_models_dir() -> String {
    let base = std::env::var("APPDATA")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    format!("{base}/CapRust/models")
}

/// User-visible pictures folder: `%USERPROFILE%/Pictures` on Windows,
/// `~/Pictures` elsewhere, temp dir as a last resort.
fn default_pictures_dir() -> String {
    std::env::var("USERPROFILE")
        .map(|u| format!("{u}/Pictures"))
        .or_else(|_| std::env::var("HOME").map(|h| format!("{h}/Pictures")))
        .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned())
}

/// Historical screen-recordings home. Kept as the default so existing
/// installs keep writing where they always did.
fn default_recordings_dir() -> std::path::PathBuf {
    let base = std::env::var("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("CapRust").join("recordings")
}

fn default_lut_cache_dir() -> std::path::PathBuf {
    let base = std::env::var("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("CapRust").join("luts")
}

/// Parsed 3D LUT from a `.cube` file.
#[derive(Debug, Clone)]
pub struct CubeLut {
    pub size: u32,
    pub data: Vec<[f32; 3]>,
}

impl CubeLut {
    /// Parse a `.cube` 3D LUT file.
    ///
    /// Format reference: https://wwwimages.adobe.com/content/dam/acom/en/products/speedgrade/cc/pdfs/cube-lut-specification-1.0.pdf
    ///
    /// Lines starting with `#` are comments, `TITLE` is optional,
    /// `LUT_3D_SIZE N` is required, `DOMAIN_MIN`/`DOMAIN_MAX` default
    /// to `[0.0, 1.0]`. The remaining lines are N*N*N RGB triples
    /// ordered B-major (blue fastest).
    pub fn parse(content: &str) -> Result<Self> {
        let mut size: Option<u32> = None;
        let mut domain_min = [0.0_f32; 3];
        let mut domain_max = [1.0_f32; 3];
        let mut data = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // TITLE is optional; skip it.
            if line.starts_with("TITLE") {
                continue;
            }
            if let Some(rest) = line.strip_prefix("LUT_3D_SIZE") {
                size = Some(rest.trim().parse().context("invalid LUT_3D_SIZE")?);
                continue;
            }
            if let Some(rest) = line.strip_prefix("DOMAIN_MIN") {
                let parts: Vec<f32> = rest
                    .split_whitespace()
                    .map(|s| s.parse::<f32>().context("invalid DOMAIN_MIN value"))
                    .collect::<Result<Vec<_>, _>>()?;
                if parts.len() != 3 {
                    anyhow::bail!("DOMAIN_MIN must have 3 components");
                }
                domain_min[0] = parts[0];
                domain_min[1] = parts[1];
                domain_min[2] = parts[2];
                continue;
            }
            if let Some(rest) = line.strip_prefix("DOMAIN_MAX") {
                let parts: Vec<f32> = rest
                    .split_whitespace()
                    .map(|s| s.parse::<f32>().context("invalid DOMAIN_MAX value"))
                    .collect::<Result<Vec<_>, _>>()?;
                if parts.len() != 3 {
                    anyhow::bail!("DOMAIN_MAX must have 3 components");
                }
                domain_max[0] = parts[0];
                domain_max[1] = parts[1];
                domain_max[2] = parts[2];
                continue;
            }
            // RGB triple line.
            let parts: Vec<f32> = line
                .split_whitespace()
                .map(|s| s.parse::<f32>().context("invalid LUT data value"))
                .collect::<Result<Vec<_>, _>>()?;
            if parts.len() != 3 {
                anyhow::bail!("LUT data line must have 3 components, got {}", parts.len());
            }
            data.push([parts[0], parts[1], parts[2]]);
        }

        let size = size.context("missing LUT_3D_SIZE")?;
        let expected = (size * size * size) as usize;
        if data.len() != expected {
            anyhow::bail!(
                "LUT data has {} entries, expected {} for size {}",
                data.len(),
                expected,
                size
            );
        }
        let _ = (domain_min, domain_max); // validated but not stored yet

        Ok(Self { size, data })
    }
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

    /// Resolved screenshots path: user override, else
    /// `<Pictures>/CapRust`, else the OS temp dir. Never empty.
    pub fn effective_screenshots_dir(&self) -> std::path::PathBuf {
        if let Some(d) = self.screenshots_dir.as_ref() {
            if !d.trim().is_empty() {
                return std::path::PathBuf::from(d);
            }
        }
        std::path::PathBuf::from(default_pictures_dir()).join("CapRust")
    }

    /// Resolved screen-recordings path: user override, else
    /// `%APPDATA%/CapRust/recordings` (temp dir fallback).
    pub fn effective_recordings_dir(&self) -> std::path::PathBuf {
        if let Some(d) = self.recordings_dir.as_ref() {
            if !d.trim().is_empty() {
                return std::path::PathBuf::from(d);
            }
        }
        default_recordings_dir()
    }

    /// Custom LUT (.cube) cache directory. Falls back to
    /// `%APPDATA%/CapRust/luts` when unset.
    pub fn effective_lut_cache_dir(&self) -> std::path::PathBuf {
        if let Some(d) = self.lut_cache_dir.as_ref() {
            if !d.trim().is_empty() {
                return std::path::PathBuf::from(d);
            }
        }
        default_lut_cache_dir()
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
            exported_at: now_unix_secs(),
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

    /// Absolute path to the sync file when a sync folder is set.
    /// `None` if `sync_folder` is unset or blank.
    pub fn sync_path(&self) -> Option<PathBuf> {
        let f = self.sync_folder.as_ref()?.trim();
        if f.is_empty() {
            return None;
        }
        Some(PathBuf::from(f).join("caprust-settings.json"))
    }

    /// Write the current settings snapshot to the sync folder, if
    /// one is set. Updates `last_synced_at` on success. Safe to call
    /// on every save tick; callers can guard with a hash check to
    /// avoid spamming a cloud client with identical writes.
    ///
    /// Returns the `exported_at` value on success, or `None` when no
    /// sync folder is configured.
    pub fn sync_now(&mut self) -> Result<Option<u64>> {
        let Some(path) = self.sync_path() else {
            return Ok(None);
        };
        let now = now_unix_secs();
        let file = SettingsFile {
            caprust_settings_version: SETTINGS_FILE_VERSION,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            exported_at: now,
            settings: self.clone(),
        };
        let json = serde_json::to_string_pretty(&file).context("serialize sync snapshot")?;

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create sync dir {}", parent.display()))?;
            }
        }
        fs::write(&path, json).with_context(|| format!("write sync {}", path.display()))?;
        self.last_synced_at = Some(now);
        Ok(Some(now))
    }

    /// Return the sync file path and its `exported_at` if the file
    /// exists and is newer than `last_synced_at`. `None` when no
    /// sync folder is set, the file is missing, unparseable, or not
    /// newer. Failures are silent: a broken sync file must not block
    /// startup.
    pub fn check_sync_newer(&self) -> Option<(PathBuf, u64)> {
        let path = self.sync_path()?;
        if !path.is_file() {
            return None;
        }
        let text = fs::read_to_string(&path).ok()?;
        let file: SettingsFile = serde_json::from_str(&text).ok()?;
        let last = self.last_synced_at.unwrap_or(0);
        if file.exported_at > last {
            Some((path, file.exported_at))
        } else {
            None
        }
    }

    /// Load the settings snapshot from a sync file, ignoring nothing.
    /// Returns the parsed AppSettings and the file's `exported_at` so
    /// the caller can stamp `last_synced_at` after applying.
    pub fn load_sync_file(path: &Path) -> Result<(Self, u64)> {
        let text =
            fs::read_to_string(path).with_context(|| format!("read sync {}", path.display()))?;
        let file: SettingsFile = serde_json::from_str(&text).context("parse sync JSON")?;
        if file.caprust_settings_version > SETTINGS_FILE_VERSION {
            bail!(
                "sync file is version {} but this build knows only up to {}",
                file.caprust_settings_version,
                SETTINGS_FILE_VERSION
            );
        }
        Ok((file.settings, file.exported_at))
    }

    /// Replace `self` with settings loaded from an untrusted file (a
    /// backup import or the sync folder), keeping this machine's
    /// executable paths. `ffmpeg_path` / `ffprobe_path` name binaries
    /// we spawn, so a settings file must not be able to choose them.
    pub fn replace_from_untrusted(&mut self, mut loaded: AppSettings) {
        loaded.ffmpeg_path = self.ffmpeg_path.take();
        loaded.ffprobe_path = self.ffprobe_path.take();
        // Capture folders are machine-local absolute paths; keep ours.
        loaded.screenshots_dir = self.screenshots_dir.take();
        loaded.recordings_dir = self.recordings_dir.take();
        *self = loaded;
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
    /// Unix seconds when this snapshot was written. Used by the
    /// sync feature to decide whether a file is newer than the
    /// local `last_synced_at`. Older exports without this field
    /// load with 0, which means "never newer than anything else".
    #[serde(default)]
    pub exported_at: u64,
    pub settings: AppSettings,
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
    fn cube_lut_parses_minimal_file() {
        let content = "# comment\nTITLE \"test\"\nLUT_3D_SIZE 2\n1.0 0.0 0.0\n0.0 1.0 0.0\n0.0 0.0 1.0\n1.0 1.0 1.0\n0.5 0.5 0.5\n0.0 0.0 0.0\n1.0 0.5 0.0\n0.0 1.0 0.5\n";
        let lut = CubeLut::parse(content).expect("valid 2x2x2 cube");
        assert_eq!(lut.size, 2);
        assert_eq!(lut.data.len(), 8);
        assert_eq!(lut.data[0], [1.0, 0.0, 0.0]);
    }

    #[test]
    fn cube_lut_rejects_wrong_entry_count() {
        let content = "LUT_3D_SIZE 2\n1.0 0.0 0.0\n";
        assert!(CubeLut::parse(content).is_err());
    }

    #[test]
    fn cube_lut_rejects_missing_size() {
        let content = "1.0 0.0 0.0\n";
        assert!(CubeLut::parse(content).is_err());
    }

    #[test]
    fn cube_lut_parses_generated_builtin() {
        // The generator emits 17^3 = 4913 entries after 4 header lines.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("app")
            .join("assets")
            .join("luts")
            .join("film_kodak.cube");
        let content = fs::read_to_string(&path).expect("bundled film_kodak.cube exists");
        let lut = CubeLut::parse(&content).expect("generated cube must parse");
        assert_eq!(lut.size, 17);
        assert_eq!(lut.data.len(), 17 * 17 * 17);
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
    fn untrusted_replace_keeps_local_executable_paths() {
        let mut local = AppSettings {
            ffmpeg_path: Some("/usr/bin/ffmpeg".into()),
            ffprobe_path: None,
            ..Default::default()
        };
        let loaded = AppSettings {
            language: "hr".into(),
            ffmpeg_path: Some("\\\\evil\\share\\ffmpeg.exe".into()),
            ffprobe_path: Some("/tmp/evil".into()),
            ..Default::default()
        };
        local.replace_from_untrusted(loaded);
        assert_eq!(local.language, "hr", "other settings are taken");
        assert_eq!(local.ffmpeg_path.as_deref(), Some("/usr/bin/ffmpeg"));
        assert_eq!(local.ffprobe_path, None);
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

    fn tmp_sync_dir(label: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("caprust_sync_{}_{}", label, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn sync_path_none_when_unset_or_blank() {
        let mut a = AppSettings::default();
        assert!(a.sync_path().is_none());
        a.sync_folder = Some("".into());
        assert!(a.sync_path().is_none());
        a.sync_folder = Some("   ".into());
        assert!(a.sync_path().is_none());
    }

    #[test]
    fn sync_now_writes_file_and_stamps_last_synced() {
        let dir = tmp_sync_dir("write");
        let mut a = AppSettings {
            sync_folder: Some(dir.to_string_lossy().to_string()),
            ..Default::default()
        };

        assert!(a.last_synced_at.is_none());
        let stamp = a.sync_now().unwrap().expect("stamp on success");
        assert!(stamp > 0);
        assert_eq!(a.last_synced_at, Some(stamp));
        assert!(dir.join("caprust-settings.json").is_file());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_now_noop_without_folder() {
        let mut a = AppSettings::default();
        assert!(a.sync_now().unwrap().is_none());
        assert!(a.last_synced_at.is_none());
    }

    #[test]
    fn check_sync_newer_detects_external_write() {
        let dir = tmp_sync_dir("newer");
        let mut a = AppSettings {
            sync_folder: Some(dir.to_string_lossy().to_string()),
            ..Default::default()
        };

        // No file yet.
        assert!(a.check_sync_newer().is_none());

        // Simulate a snapshot written by another machine: future stamp.
        let future = now_unix_secs() + 10_000;
        let body = format!(
            r#"{{"caprust_settings_version": 1, "app_version": "x", "exported_at": {}, "settings": {}}}"#,
            future,
            serde_json::to_string(&AppSettings::default()).unwrap(),
        );
        fs::write(dir.join("caprust-settings.json"), body).unwrap();

        let (path, stamp) = a.check_sync_newer().expect("newer file detected");
        assert!(path.to_string_lossy().ends_with("caprust-settings.json"));
        assert_eq!(stamp, future);

        // Once we acknowledge (stamp last_synced_at), it stops prompting.
        a.last_synced_at = Some(future);
        assert!(a.check_sync_newer().is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_sync_file_round_trips_settings_and_stamp() {
        let dir = tmp_sync_dir("load");
        let mut a = AppSettings {
            language: "hr".into(),
            sync_folder: Some(dir.to_string_lossy().to_string()),
            ..Default::default()
        };
        a.sync_now().unwrap();

        let path = dir.join("caprust-settings.json");
        let (b, stamp) = AppSettings::load_sync_file(&path).unwrap();
        assert_eq!(b.language, "hr");
        assert!(stamp > 0);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_sync_newer_ignores_corrupt_file() {
        let dir = tmp_sync_dir("corrupt");
        let a = AppSettings {
            sync_folder: Some(dir.to_string_lossy().to_string()),
            ..Default::default()
        };
        fs::write(dir.join("caprust-settings.json"), b"not json").unwrap();

        // Corrupt file -> silent None, no panic.
        assert!(a.check_sync_newer().is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn capture_dirs_fall_back_to_defaults() {
        let s = AppSettings::default();
        let shots = s.effective_screenshots_dir().to_string_lossy().into_owned();
        assert!(shots.contains("CapRust"), "screenshots default: {shots}");
        let rec = s.effective_recordings_dir().to_string_lossy().into_owned();
        assert!(rec.contains("recordings"), "recordings default: {rec}");
    }

    #[test]
    fn capture_dir_override_wins_blank_falls_back() {
        let s = AppSettings {
            screenshots_dir: Some("D:/shots".into()),
            recordings_dir: Some("   ".into()),
            ..Default::default()
        };
        assert_eq!(
            s.effective_screenshots_dir(),
            std::path::PathBuf::from("D:/shots")
        );
        assert!(s
            .effective_recordings_dir()
            .to_string_lossy()
            .contains("recordings"));
    }

    #[test]
    fn import_keeps_local_capture_dirs() {
        let mut local = AppSettings {
            screenshots_dir: Some("D:/shots".into()),
            recordings_dir: Some("D:/rec".into()),
            ..Default::default()
        };
        let loaded = AppSettings {
            screenshots_dir: Some("/tmp/evil".into()),
            recordings_dir: None,
            ..Default::default()
        };
        local.replace_from_untrusted(loaded);
        assert_eq!(local.screenshots_dir.as_deref(), Some("D:/shots"));
        assert_eq!(local.recordings_dir.as_deref(), Some("D:/rec"));
    }
}
