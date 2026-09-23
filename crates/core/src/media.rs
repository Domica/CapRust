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
