//! Project state — serializable.

use crate::aspect_ratio::AspectRatio;
use crate::clip::Clip;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectState {
    pub name: String,
    pub clips: Vec<Clip>,
    pub aspect_ratio: AspectRatio,
    pub base_resolution: u32,
    pub export_settings: ExportSettings,
    pub locale: String,
    pub frame_rate: crate::frame_rate::FrameRate,
    pub project_path: Option<String>,
}

impl Default for ProjectState {
    fn default() -> Self {
        Self {
            name: "Untitled".into(),
            clips: Vec::new(),
            aspect_ratio: AspectRatio::default(),
            base_resolution: 1080,
            export_settings: ExportSettings::default(),
            locale: sys_locale::get_locale()
                .map(|l| l.split('-').next().unwrap_or("en").to_string())
                .unwrap_or_else(|| "en".into()),
            frame_rate: crate::frame_rate::FrameRate::default(),
            project_path: None,
        }
    }
}

impl ProjectState {
    pub fn add_clip(&mut self, clip: Clip) {
        self.clips.push(clip);
    }

    pub fn remove_clip(&mut self, id: uuid::Uuid) -> Option<Clip> {
        self.clips
            .iter()
            .position(|c| c.id == id)
            .map(|pos| self.clips.remove(pos))
    }

    pub fn project_dimensions(&self) -> (u32, u32) {
        self.aspect_ratio.dimensions(self.base_resolution)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportSettings {
    pub resolution_index: usize,
    pub rate_index: usize,
    pub quality_index: usize,
    pub codec_index: usize,
    pub ten_bit: bool,
    pub advanced: bool,
    pub rate_mode_index: usize,
    pub bitrate_kbps: u32,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            resolution_index: 2,
            rate_index: 3,
            quality_index: 1,
            codec_index: 0,
            ten_bit: false,
            advanced: false,
            rate_mode_index: 0,
            bitrate_kbps: 8000,
        }
    }
}
