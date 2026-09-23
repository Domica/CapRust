//! Move a clip on the timeline (drag).

use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct MoveClipCommand {
    pub clip_id: Uuid,
    pub from_ms: u64,
    pub to_ms: u64,
}

impl Command for MoveClipCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(c) = state.clips.iter_mut().find(|c| c.id == self.clip_id) {
            c.start_time_ms = self.to_ms;
        }
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(c) = state.clips.iter_mut().find(|c| c.id == self.clip_id) {
            c.start_time_ms = self.from_ms;
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Move clip {} → {}ms", self.from_ms, self.to_ms)
    }
}
