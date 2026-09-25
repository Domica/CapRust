//! Edit the text of a single caption segment in place.

use crate::clip::ClipType;
use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct EditCaptionSegmentCommand {
    clip_id: Uuid,
    idx: usize,
    new_text: String,
    /// Text captured on `execute`, restored on `undo`.
    before_text: Option<String>,
}

impl EditCaptionSegmentCommand {
    pub fn new(clip_id: Uuid, idx: usize, new_text: impl Into<String>) -> Self {
        Self {
            clip_id,
            idx,
            new_text: new_text.into(),
            before_text: None,
        }
    }
}

impl Command for EditCaptionSegmentCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) else {
            return Ok(());
        };
        let ClipType::Captions { segments, .. } = &mut clip.clip_type else {
            return Ok(());
        };
        let Some(seg) = segments.get_mut(self.idx) else {
            return Ok(());
        };
        self.before_text = Some(seg.text.clone());
        seg.text = self.new_text.clone();
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(before) = self.before_text.clone() else {
            return Ok(());
        };
        let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) else {
            return Ok(());
        };
        let ClipType::Captions { segments, .. } = &mut clip.clip_type else {
            return Ok(());
        };
        let Some(seg) = segments.get_mut(self.idx) else {
            return Ok(());
        };
        seg.text = before;
        Ok(())
    }

    fn description(&self) -> String {
        format!("Edit caption segment {}", self.idx + 1)
    }
}
