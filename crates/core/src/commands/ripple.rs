//! Ripple insert. Ported from jub0t/Concat#141.

use crate::clip::Clip;
use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct RippleInsertCommand {
    new_clip: Clip,
    affected: Vec<(Uuid, u64)>,
}

impl RippleInsertCommand {
    pub fn new(clip: Clip) -> Self {
        Self {
            new_clip: clip,
            affected: Vec::new(),
        }
    }
}

impl Command for RippleInsertCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let end = self.new_clip.start_time_ms + self.new_clip.duration_ms;
        let overlaps = state.clips.iter().any(|c| {
            c.track_index == self.new_clip.track_index
                && c.start_time_ms < end
                && self.new_clip.start_time_ms < c.start_time_ms + c.duration_ms
        });

        if overlaps {
            self.affected = state
                .clips
                .iter()
                .filter(|c| {
                    c.track_index == self.new_clip.track_index
                        && c.start_time_ms >= self.new_clip.start_time_ms
                })
                .map(|c| (c.id, c.start_time_ms))
                .collect();

            for clip in state.clips.iter_mut() {
                if clip.track_index == self.new_clip.track_index
                    && clip.start_time_ms >= self.new_clip.start_time_ms
                {
                    clip.start_time_ms += self.new_clip.duration_ms;
                }
            }
        }

        state.add_clip(self.new_clip.clone());
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        for (id, orig_start) in &self.affected {
            if let Some(c) = state.clips.iter_mut().find(|c| &c.id == id) {
                c.start_time_ms = *orig_start;
            }
        }
        state.remove_clip(self.new_clip.id);
        Ok(())
    }

    fn description(&self) -> String {
        let label = match &self.new_clip.clip_type {
            crate::clip::ClipType::TextOverlay { content, .. } => content.clone(),
            _ => "clip".to_string(),
        };
        format!(
            "Ripple insert '{}' at {}ms",
            label, self.new_clip.start_time_ms
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ripple_shifts_overlapping_clips() {
        let mut state = ProjectState::default();
        state.add_clip(Clip::new_text("A", 0, 1000, 2000, false));
        state.add_clip(Clip::new_text("B", 0, 3000, 1000, false));

        let new_clip = Clip::new_text("X", 0, 2000, 1500, false);
        let mut cmd = RippleInsertCommand::new(new_clip);
        cmd.execute(&mut state).unwrap();

        let b = state
            .clips
            .iter()
            .find(|c| matches!(&c.clip_type, crate::clip::ClipType::TextOverlay { content, .. } if content == "B"))
            .unwrap();
        assert_eq!(b.start_time_ms, 4500);
    }

    #[test]
    fn no_ripple_when_no_overlap() {
        let mut state = ProjectState::default();
        state.add_clip(Clip::new_text("A", 0, 0, 1000, false));
        let new_clip = Clip::new_text("X", 0, 2000, 1000, false);
        let mut cmd = RippleInsertCommand::new(new_clip);
        cmd.execute(&mut state).unwrap();
        let a = state
            .clips
            .iter()
            .find(|c| matches!(&c.clip_type, crate::clip::ClipType::TextOverlay { content, .. } if content == "A"))
            .unwrap();
        assert_eq!(a.start_time_ms, 0);
    }

    #[test]
    fn undo_restores_original_positions() {
        let mut state = ProjectState::default();
        state.add_clip(Clip::new_text("A", 0, 1000, 2000, false));
        state.add_clip(Clip::new_text("B", 0, 3000, 1000, false));

        let new_clip = Clip::new_text("X", 0, 2000, 1500, false);
        let mut cmd = RippleInsertCommand::new(new_clip);
        cmd.execute(&mut state).unwrap();
        cmd.undo(&mut state).unwrap();

        let b = state
            .clips
            .iter()
            .find(|c| matches!(&c.clip_type, crate::clip::ClipType::TextOverlay { content, .. } if content == "B"))
            .unwrap();
        assert_eq!(b.start_time_ms, 3000);
        assert_eq!(state.clips.len(), 2);
    }
}
