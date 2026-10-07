//! Media library — imported clips, music, images.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    Video,
    Audio,
    Image,
}

impl MediaKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Image => "Image",
        }
    }
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Video => "🎞",
            Self::Audio => "🎵",
            Self::Image => "🖼",
        }
    }
}

fn default_has_audio() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaItem {
    pub id: Uuid,
    pub name: String,
    pub path: String,
    pub kind: MediaKind,
    pub duration_ms: u64,
    pub size_bytes: u64,
    /// Insertion order — used for "Sort by: Added".
    pub added_at: u64,
    /// Probe status — set true once ffprobe has run.
    #[serde(default)]
    pub probe_done: bool,
    /// Thumbnail status — set true once the JPEG is on disk.
    #[serde(default)]
    pub thumb_done: bool,
    /// Waveform peak cache status — set true once the .bin is on disk.
    #[serde(default)]
    pub waveform_done: bool,
    /// True when the source carries at least one audio stream.
    /// Filled by the probe job from ffprobe::MediaProbe::has_audio.
    /// Defaults to true so pre-probe items keep their old
    /// "assume audio exists" behaviour; the flag is corrected
    /// the moment the probe finishes.
    #[serde(default = "default_has_audio")]
    pub has_audio: bool,
}

impl MediaItem {
    pub fn new(path: &str, kind: MediaKind, added_at: u64) -> Self {
        let name = std::path::Path::new(path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string());
        let size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        Self {
            id: Uuid::new_v4(),
            name,
            path: path.to_string(),
            kind,
            duration_ms: 0,
            size_bytes,
            added_at,
            probe_done: false,
            thumb_done: false,
            waveform_done: false,
            has_audio: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaLibrary {
    pub items: Vec<MediaItem>,
    next_added_at: u64,
}

impl MediaLibrary {
    pub fn add(&mut self, path: &str, kind: MediaKind) -> Uuid {
        self.next_added_at += 1;
        let item = MediaItem::new(path, kind, self.next_added_at);
        let id = item.id;
        self.items.push(item);
        id
    }

    pub fn remove(&mut self, id: Uuid) {
        self.items.retain(|m| m.id != id);
    }

    /// Find an item by path, case- and separator-insensitive, so
    /// re-importing the same file does not duplicate the library.
    pub fn find_by_path(&self, path: &str) -> Option<Uuid> {
        let norm = normalize_import_path(path);
        self.items
            .iter()
            .find(|m| normalize_import_path(&m.path) == norm)
            .map(|m| m.id)
    }
}

/// Lowercase + forward slashes: the same file picked twice (or via
/// differently-cased browsing) maps to one key.
fn normalize_import_path(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

/// Guess kind from extension.
pub fn guess_kind(path: &str) -> Option<MediaKind> {
    let ext = std::path::Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())?;
    match ext.as_str() {
        "mp4" | "mov" | "avi" | "mkv" | "webm" | "m4v" | "wmv" | "flv" => Some(MediaKind::Video),
        "mp3" | "wav" | "m4a" | "aac" | "flac" | "ogg" | "opus" => Some(MediaKind::Audio),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tiff" => Some(MediaKind::Image),
        _ => None,
    }
}

/// Supported extensions for file dialogs.
pub const VIDEO_EXTS: &[&str] = &["mp4", "mov", "avi", "mkv", "webm", "m4v", "wmv", "flv"];
pub const AUDIO_EXTS: &[&str] = &["mp3", "wav", "m4a", "aac", "flac", "ogg", "opus"];
pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp", "tiff"];

#[cfg(test)]
mod media_dedup_tests {
    use super::*;

    #[test]
    fn find_by_path_ignores_case_and_separators() {
        let mut lib = MediaLibrary::default();
        let id = lib.add("C:/Music/Song.mp3", MediaKind::Audio);
        assert_eq!(lib.find_by_path("c:\\music\\song.MP3"), Some(id));
        assert_eq!(lib.find_by_path("C:/other.mp3"), None);
    }
}
