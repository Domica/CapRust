//! Track model — video/audio/text/overlay lanes.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackKind {
    Video,
    Audio,
    Text,
    Overlay,
}

impl TrackKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Text => "Text",
            Self::Overlay => "Overlay",
        }
    }
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Video => "🎞",
            Self::Audio => "🎵",
            Self::Text => "T",
            Self::Overlay => "✦",
        }
    }
    pub fn default_height(&self) -> f32 {
        // Reduced lane height per request
        match self {
            Self::Video | Self::Overlay => 46.0,
            Self::Audio => 38.0,
            Self::Text => 34.0,
        }
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
    /// Overrides `kind.default_height()` if user resizes.
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
            height: kind.default_height(),
        }
    }
}

/// Default project tracks: 2 video, 1 audio.
pub fn default_tracks() -> Vec<Track> {
    vec![
        Track::new("V1", TrackKind::Video),
        Track::new("V2", TrackKind::Video),
        Track::new("A1", TrackKind::Audio),
    ]
}
