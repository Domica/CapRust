//! Track model — video/audio/text/captions/overlay lanes.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackKind {
    /// Regular video content. Multiple allowed.
    Video,
    /// Audio lane. Multiple allowed.
    Audio,
    /// Text overlays (lower thirds, titles) — user placed.
    Text,
    /// Pinned overlay lane (watermark, logo, global effect). Only one.
    Overlay,
    /// Dedicated captions lane (burn-in + sidecar SRT/VTT export). Only one.
    Captions,
}

impl TrackKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Text => "Text",
            Self::Overlay => "Overlay",
            Self::Captions => "Captions",
        }
    }
    pub fn icon(&self) -> &'static str {
        // Phosphor constants (see crates/ui/src/timeline/track_header.rs).
        // Core doesn't link egui-phosphor; keep the raw glyphs here.
        match self {
            Self::Video => "\u{E4C2}",    // FILM_STRIP
            Self::Audio => "\u{E3D5}",    // WAVEFORM
            Self::Text => "\u{E4F0}",     // TEXT_T
            Self::Overlay => "\u{E4F1}",  // SPARKLE
            Self::Captions => "\u{E2BC}", // CHAT_TEXT
        }
    }
    pub fn default_height(&self) -> f32 {
        // Two-row header needs ~44px. Give comfortable room.
        match self {
            Self::Video => 56.0,
            Self::Overlay => 50.0,
            Self::Audio => 50.0,
            Self::Text => 46.0,
            Self::Captions => 46.0,
        }
    }
    /// Overlay + Captions lanes never spawn more than one of their kind.
    pub fn is_singleton(&self) -> bool {
        matches!(self, Self::Overlay | Self::Captions)
    }
    /// Pinned lanes are always sorted to the top of the timeline.
    pub fn is_pinned_by_default(&self) -> bool {
        matches!(self, Self::Overlay)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: Uuid,
    pub name: String,
    pub kind: TrackKind,
    pub locked: bool,
    pub visible: bool,
    pub muted: bool,
    /// If true, this lane always renders above all non-pinned lanes,
    /// regardless of array order. Used by the Overlay lane.
    #[serde(default)]
    pub pinned: bool,
    /// Overrides `kind.default_height()` if the user resizes.
    pub height: f32,
}

impl Track {
    pub fn new(name: &str, kind: TrackKind) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            kind,
            locked: false,
            visible: true,
            muted: false,
            pinned: kind.is_pinned_by_default(),
            height: kind.default_height(),
        }
    }
}

/// Default project tracks:
///  - Overlay (pinned, top)  ← watermark / global effect
///  - V1, V2                 ← main video content
///  - A1                     ← audio
///  - Captions               ← dedicated captions lane
pub fn default_tracks() -> Vec<Track> {
    vec![
        Track::new("Overlay", TrackKind::Overlay),
        Track::new("V1", TrackKind::Video),
        Track::new("V2", TrackKind::Video),
        Track::new("A1", TrackKind::Audio),
        Track::new("Captions", TrackKind::Captions),
    ]
}

/// Returns the visible track order: pinned first (in insertion order),
/// then non-pinned in their array order.
pub fn display_order(tracks: &[Track]) -> Vec<usize> {
    let mut pinned: Vec<usize> = Vec::new();
    let mut normal: Vec<usize> = Vec::new();
    for (i, t) in tracks.iter().enumerate() {
        if t.pinned {
            pinned.push(i);
        } else {
            normal.push(i);
        }
    }
    pinned.into_iter().chain(normal).collect()
}
