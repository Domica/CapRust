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
    /// Add every item from `other` whose path is not already present.
    ///
    /// Used when a template is applied. The media bin belongs to the
    /// user's session, not to the template, and replacing it wholesale
    /// dropped every imported item along with its cached probe,
    /// thumbnail and waveform flags, so the bin came back empty and
    /// every thumbnail had to be regenerated. Existing entries win:
    /// they already carry the cache flags, the incoming ones start
    /// unprobed.
    pub fn merge_missing_from(&mut self, other: &MediaLibrary) {
        for item in &other.items {
            if !self.items.iter().any(|i| i.path == item.path) {
                self.items.push(item.clone());
            }
        }
    }

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
    // The name comes first: it is free, and it is right most of the
    // time.
    let ext = std::path::Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase());
    if let Some(ext) = ext {
        match ext.as_str() {
            "mp4" | "mov" | "avi" | "mkv" | "webm" | "m4v" | "wmv" | "flv" => {
                return Some(MediaKind::Video)
            }
            "mp3" | "wav" | "m4a" | "aac" | "flac" | "ogg" | "opus" => {
                return Some(MediaKind::Audio)
            }
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tiff" | "heic" | "heif" | "avif" => {
                return Some(MediaKind::Image)
            }
            _ => {}
        }
    }
    // The name says nothing, so the file is asked. Every format has a
    // few bytes at the front that say what it is, and a file knows
    // better than its name does: a phone's file picker hands over
    // names like "import-1759000000.bin", and refusing those was
    // refusing perfectly good videos.
    kind_from_bytes(path)
}

/// What a file says it is, from its first bytes.
fn kind_from_bytes(path: &str) -> Option<MediaKind> {
    use std::io::Read;
    let mut head = [0u8; 16];
    let mut file = std::fs::File::open(path).ok()?;
    let read = file.read(&mut head).ok()?;
    if read == 0 {
        return None;
    }
    let head = &head[..read];
    let starts = |magic: &[u8]| head.len() >= magic.len() && &head[..magic.len()] == magic;
    // An MP4 starts with a four byte size and then "ftyp", so the
    // four bytes are looked for inside the head, not at the front of
    // it.
    let has_ftyp = head.len() >= 8 && (0..=head.len() - 8).any(|i| &head[i..i + 4] == b"ftyp");
    let kind = if has_ftyp || starts(&[0x1a, 0x45, 0xdf, 0xa3]) || starts(b"RIFF") {
        // An MP4 or a Matroska/WebM header.
        MediaKind::Video
    } else if starts(b"OggS")
        || starts(b"fLaC")
        || starts(b"ID3")
        || starts(&[0xff, 0xfb])
        || starts(&[0xff, 0xf3])
    {
        // Ogg, FLAC, MP3/ID3.
        MediaKind::Audio
    } else if starts(&[0x89, b'P', b'N', b'G'])   // png
        || starts(&[0xff, 0xd8])                  // jpeg
        || starts(b"GIF8")                        // gif
        || starts(b"BM")
    // bmp
    {
        MediaKind::Image
    } else {
        return None;
    };
    Some(kind)
}

/// Supported extensions for file dialogs.
pub const VIDEO_EXTS: &[&str] = &["mp4", "mov", "avi", "mkv", "webm", "m4v", "wmv", "flv"];
pub const AUDIO_EXTS: &[&str] = &["mp3", "wav", "m4a", "aac", "flac", "ogg", "opus"];
pub const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "gif", "bmp", "tiff", "heic", "heif", "avif",
];

#[cfg(test)]
mod kind_from_bytes_tests {
    use super::*;
    use std::io::Write;

    fn write(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("caprust-kind-{name}"));
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(bytes).unwrap();
        p
    }

    /// The name is not always a name. A phone's file picker hands over
    /// things like "import-1759000000.bin", and refusing those was
    /// refusing perfectly good videos.
    #[test]
    fn a_file_without_a_useful_name_still_knows_what_it_is() {
        let mp4 = write("movie.bin", b"\x00\x00\x00\x18ftypmp42");
        assert_eq!(guess_kind(&mp4.to_string_lossy()), Some(MediaKind::Video));

        let png = write("picture.bin", b"\x89PNG\r\n\x1a\n");
        assert_eq!(guess_kind(&png.to_string_lossy()), Some(MediaKind::Image));

        let mp3 = write("sound.bin", b"ID3\x04\x00\x00\x00");
        assert_eq!(guess_kind(&mp3.to_string_lossy()), Some(MediaKind::Audio));

        let _ = std::fs::remove_file(mp4);
        let _ = std::fs::remove_file(png);
        let _ = std::fs::remove_file(mp3);
    }

    /// The name still wins: a .png that is really a png is a png.
    #[test]
    fn the_name_is_asked_first() {
        assert_eq!(guess_kind("a.mp4"), Some(MediaKind::Video));
        assert_eq!(guess_kind("a.PNG"), Some(MediaKind::Image));
        assert_eq!(guess_kind("a.txt"), None);
    }
}

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
