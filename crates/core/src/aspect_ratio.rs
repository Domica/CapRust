//! Aspect ratio presets for social video.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum AspectRatio {
    Landscape16x9,
    #[default]
    Portrait9x16,
    Portrait4x5,
    Square1x1,
    Cinema2x1,
    Widescreen21x9,
    Custom(u32, u32),
}

impl AspectRatio {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Landscape16x9 => "16:9 Landscape",
            Self::Portrait9x16 => "Vertical 9:16",
            Self::Portrait4x5 => "Portrait 4:5",
            Self::Square1x1 => "Square 1:1",
            Self::Cinema2x1 => "Cinema 2:1",
            Self::Widescreen21x9 => "Widescreen 21:9",
            Self::Custom(_, _) => "Custom",
        }
    }

    pub fn dimensions(&self, base: u32) -> (u32, u32) {
        match self {
            Self::Landscape16x9 => (base * 16 / 9, base),
            Self::Portrait9x16 => (base, base * 16 / 9),
            Self::Portrait4x5 => (base, base * 5 / 4),
            Self::Square1x1 => (base, base),
            Self::Cinema2x1 => (base * 2, base),
            Self::Widescreen21x9 => (2560, 1080),
            Self::Custom(w, h) => (*w, *h),
        }
    }

    pub fn presets() -> Vec<Self> {
        vec![
            Self::Portrait9x16,
            Self::Landscape16x9,
            Self::Portrait4x5,
            Self::Square1x1,
            Self::Cinema2x1,
            Self::Widescreen21x9,
        ]
    }
}
