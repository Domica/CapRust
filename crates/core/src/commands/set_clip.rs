//! Generic command to edit any subset of clip fields.

use crate::clip::Clip;
use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct SetClipCommand {
    pub clip_id: Uuid,
    pub start_time_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub speed: Option<f32>,
    pub reversed: Option<bool>,
    pub flip_h: Option<bool>,
    pub flip_v: Option<bool>,
    pub volume_db: Option<f32>,
    pub track_index: Option<usize>,
    pub name: Option<Option<String>>,
    /// Only meaningful on TextOverlay clips: sets the drawtext style id.
    /// No-op on other clip kinds.
    pub text_style: Option<String>,
    pub fade_in_ms: Option<u64>,
    pub fade_out_ms: Option<u64>,
    pub volume_keyframes: Option<Vec<crate::clip::VolumeKeyframe>>,
    before: Option<Clip>,
}

impl SetClipCommand {
    pub fn new(clip_id: Uuid) -> Self {
        Self {
            clip_id,
            start_time_ms: None,
            duration_ms: None,
            speed: None,
            reversed: None,
            flip_h: None,
            flip_v: None,
            volume_db: None,
            track_index: None,
            name: None,
            text_style: None,
            fade_in_ms: None,
            fade_out_ms: None,
            volume_keyframes: None,
            before: None,
        }
    }

    pub fn speed(mut self, v: f32) -> Self {
        self.speed = Some(v);
        self
    }
    pub fn reversed(mut self, v: bool) -> Self {
        self.reversed = Some(v);
        self
    }
    pub fn flip_h(mut self, v: bool) -> Self {
        self.flip_h = Some(v);
        self
    }
    pub fn flip_v(mut self, v: bool) -> Self {
        self.flip_v = Some(v);
        self
    }
    pub fn volume_db(mut self, v: f32) -> Self {
        self.volume_db = Some(v);
        self
    }
    pub fn start_time_ms(mut self, v: u64) -> Self {
        self.start_time_ms = Some(v);
        self
    }
    pub fn duration_ms(mut self, v: u64) -> Self {
        self.duration_ms = Some(v);
        self
    }
    pub fn track_index(mut self, v: usize) -> Self {
        self.track_index = Some(v);
        self
    }
    /// Set the clip name. Pass `None` to clear it back to file-derived.
    pub fn name(mut self, v: Option<String>) -> Self {
        self.name = Some(v);
        self
    }
    /// Set the TextOverlay drawtext style. No-op on other clip kinds.
    pub fn text_style(mut self, v: impl Into<String>) -> Self {
        self.text_style = Some(v.into());
        self
    }
    pub fn fade_in_ms(mut self, v: u64) -> Self {
        self.fade_in_ms = Some(v);
        self
    }
    pub fn fade_out_ms(mut self, v: u64) -> Self {
        self.fade_out_ms = Some(v);
        self
    }
    /// Replace the whole automation curve. Pass an empty Vec to fall
    /// back to the static `volume_db`.
    pub fn volume_keyframes(mut self, v: Vec<crate::clip::VolumeKeyframe>) -> Self {
        self.volume_keyframes = Some(v);
        self
    }
}

impl Command for SetClipCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(c) = state.clips.iter_mut().find(|c| c.id == self.clip_id) else {
            return Ok(());
        };
        self.before = Some(c.clone());
        if let Some(v) = self.start_time_ms {
            c.start_time_ms = v;
        }
        if let Some(v) = self.duration_ms {
            c.duration_ms = v;
        }
        if let Some(v) = self.speed {
            c.speed = v;
        }
        if let Some(v) = self.reversed {
            c.reversed = v;
        }
        if let Some(v) = self.flip_h {
            c.flip_h = v;
        }
        if let Some(v) = self.flip_v {
            c.flip_v = v;
        }
        if let Some(v) = self.volume_db {
            c.volume_db = v;
        }
        if let Some(v) = self.track_index {
            c.track_index = v;
        }
        if let Some(v) = self.name.clone() {
            c.name = v;
        }
        if let Some(v) = self.text_style.clone() {
            if let crate::clip::ClipType::TextOverlay { style, .. } = &mut c.clip_type {
                *style = v;
            }
        }
        if let Some(v) = self.fade_in_ms {
            c.fade_in_ms = v;
        }
        if let Some(v) = self.fade_out_ms {
            c.fade_out_ms = v;
        }
        if let Some(v) = self.volume_keyframes.clone() {
            c.volume_keyframes = v;
        }
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(orig) = self.before.take() {
            if let Some(c) = state.clips.iter_mut().find(|c| c.id == self.clip_id) {
                *c = orig;
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Edit clip {}", self.clip_id)
    }
}
