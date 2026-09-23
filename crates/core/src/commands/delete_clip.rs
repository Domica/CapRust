//! Delete a clip, optionally ripple-shifting later clips left.

use crate::clip::Clip;
use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct DeleteClipCommand {
    pub clip_id: Uuid,
    pub ripple: bool,
    /// Filled on execute — used by undo.
    removed: Option<Clip>,
    /// (clip_id, original_start_ms) for affected clips when rippling.
    affected: Vec<(Uuid, u64)>,
}

impl DeleteClipCommand {
    pub fn new(clip_id: Uuid, ripple: bool) -> Self {
        Self {
            clip_id,
            ripple,
            removed: None,
            affected: Vec::new(),
        }
    }
}

impl Command for DeleteClipCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        self.affected.clear();
        let removed = state.remove_clip(self.clip_id);
        if let Some(removed) = removed {
            if self.ripple {
                let gap = removed.duration_ms;
                let after = removed.start_time_ms + removed.duration_ms;
                for c in state.clips.iter_mut() {
                    if c.track_index == removed.track_index && c.start_time_ms >= after {
                        self.affected.push((c.id, c.start_time_ms));
                        c.start_time_ms = c.start_time_ms.saturating_sub(gap);
                    }
                }
            }
            self.removed = Some(removed);
        }
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        for (id, orig) in &self.affected {
            if let Some(c) = state.clips.iter_mut().find(|c| &c.id == id) {
                c.start_time_ms = *orig;
            }
        }
        if let Some(clip) = self.removed.take() {
            state.add_clip(clip);
        }
        Ok(())
    }

    fn description(&self) -> String {
        if self.ripple {
            "Ripple delete".into()
        } else {
            "Delete clip".into()
        }
    }
}
