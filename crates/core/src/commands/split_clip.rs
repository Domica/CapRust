//! Split a clip at a time position into two clips.

use crate::clip::Clip;
use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct SplitClipCommand {
    pub clip_id: Uuid,
    pub at_ms: u64,
    /// Captured for undo.
    original: Option<Clip>,
    /// Id of the newly created (right-hand) clip.
    new_id: Option<Uuid>,
}

impl SplitClipCommand {
    pub fn new(clip_id: Uuid, at_ms: u64) -> Self {
        Self {
            clip_id,
            at_ms,
            original: None,
            new_id: None,
        }
    }
}

impl Command for SplitClipCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(pos) = state.clips.iter().position(|c| c.id == self.clip_id) else {
            return Ok(());
        };

        let original = state.clips[pos].clone();
        let start = original.start_time_ms;
        let end = start + original.duration_ms;

        if self.at_ms <= start || self.at_ms >= end {
            return Ok(()); // playhead outside clip
        }

        let left_dur = self.at_ms - start;
        let right_dur = end - self.at_ms;

        self.original = Some(original.clone());

        // Shrink left
        state.clips[pos].duration_ms = left_dur;

        // Create right
        let mut right = original.clone();
        right.id = Uuid::new_v4();
        right.start_time_ms = self.at_ms;
        right.duration_ms = right_dur;
        self.new_id = Some(right.id);
        state.clips.push(right);

        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(id) = self.new_id.take() {
            state.remove_clip(id);
        }
        if let Some(orig) = self.original.take() {
            if let Some(c) = state.clips.iter_mut().find(|c| c.id == orig.id) {
                *c = orig;
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Split at {}ms", self.at_ms)
    }
}
