//! Export resolution, frame rate, and rate mode.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ExportResolution {
    Uhd4K,
    Qhd,
    FullHd,
    Hd,
    UltraWide21x9,
    Original,
    Scale1_5x,
    Scale2x,
}

impl ExportResolution {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Uhd4K => "4K (3840×2160)",
            Self::Qhd => "QHD (2560×1440)",
            Self::FullHd => "1080p (1920×1080)",
            Self::Hd => "720p (1280×720)",
            Self::UltraWide21x9 => "Wide 21:9 (2560×1080)",
            Self::Original => "Original project resolution",
            Self::Scale1_5x => "1.5× project",
            Self::Scale2x => "2× project",
        }
    }

    pub fn dimensions(&self, project_w: u32, project_h: u32) -> (u32, u32) {
        let even = |side: f64| ((side / 2.0).round() as u32 * 2).max(2);
        let pw = project_w.max(1) as f64;
        let ph = project_h.max(1) as f64;

        match self {
            Self::Uhd4K | Self::Qhd | Self::FullHd | Self::Hd => {
                let short: u32 = match self {
                    Self::Uhd4K => 2160,
                    Self::Qhd => 1440,
                    Self::FullHd => 1080,
                    Self::Hd => 720,
                    _ => unreachable!(),
                };
                if pw >= ph {
                    (even(short as f64 * pw / ph), short)
                } else {
                    (short, even(short as f64 * ph / pw))
                }
            }
            Self::UltraWide21x9 => (2560, 1080),
            Self::Original => (even(pw), even(ph)),
            Self::Scale1_5x => (even(pw * 1.5), even(ph * 1.5)),
            Self::Scale2x => (even(pw * 2.0), even(ph * 2.0)),
        }
    }

    pub fn all() -> Vec<Self> {
        vec![
            Self::Uhd4K,
            Self::Qhd,
            Self::FullHd,
            Self::Hd,
            Self::UltraWide21x9,
            Self::Original,
            Self::Scale1_5x,
            Self::Scale2x,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ExportFrameRate {
    Fps24,
    Fps30,
    Fps60,
    Original,
}

impl ExportFrameRate {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Fps24 => "24 fps",
            Self::Fps30 => "30 fps",
            Self::Fps60 => "60 fps",
            Self::Original => "Original",
        }
    }

    pub fn fraction(&self, project_num: i64, project_den: i64) -> (i64, i64) {
        match self {
            Self::Fps24 => (24, 1),
            Self::Fps30 => (30, 1),
            Self::Fps60 => (60, 1),
            Self::Original => (project_num, project_den),
        }
    }

    pub fn all() -> Vec<Self> {
        vec![Self::Fps24, Self::Fps30, Self::Fps60, Self::Original]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RateMode {
    Vbr,
    Cbr,
}

impl RateMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Vbr => "VBR",
            Self::Cbr => "CBR",
        }
    }
    pub fn all() -> Vec<Self> {
        vec![Self::Vbr, Self::Cbr]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportSpec {
    pub resolution: ExportResolution,
    pub frame_rate: ExportFrameRate,
    pub quality_crf: u8,
    pub preset: String,
    pub rate_mode: RateMode,
    pub bitrate_kbps: u32,
    pub ten_bit: bool,
}

impl Default for ExportSpec {
    fn default() -> Self {
        Self {
            resolution: ExportResolution::FullHd,
            frame_rate: ExportFrameRate::Original,
            quality_crf: 20,
            preset: "veryfast".into(),
            rate_mode: RateMode::Vbr,
            bitrate_kbps: 8000,
            ten_bit: false,
        }
    }
}
