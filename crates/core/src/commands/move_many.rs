//! Move a group of clips in one undoable step.

use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

/// One clip's before/after position inside a batch move.
#[derive(Debug, Clone)]
pub struct ClipMove {
    pub clip_id: Uuid,
    pub from_ms: u64,
    pub to_ms: u64,
    pub from_track: usize,
    pub to_track: usize,
}

pub struct MoveManyCommand {
    moves: Vec<ClipMove>,
}

impl MoveManyCommand {
    pub fn new(moves: Vec<ClipMove>) -> Self {
        Self { moves }
    }

    pub fn len(&self) -> usize {
        self.moves.len()
    }

    pub fn is_empty(&self) -> bool {
        self.moves.is_empty()
    }
}

impl Command for MoveManyCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        for m in &self.moves {
            if let Some(c) = state.clips.iter_mut().find(|c| c.id == m.clip_id) {
                c.start_time_ms = m.to_ms;
                c.track_index = m.to_track;
            }
        }
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        for m in self.moves.iter().rev() {
            if let Some(c) = state.clips.iter_mut().find(|c| c.id == m.clip_id) {
                c.start_time_ms = m.from_ms;
                c.track_index = m.from_track;
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Move {} clips", self.moves.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::Clip;

    #[test]
    fn moves_all_clips_and_undo_restores() {
        let mut state = ProjectState::default();
        let a = Clip::new_video("a.mp4", 0, 0, 1000);
        let b = Clip::new_video("b.mp4", 0, 5000, 1000);
        let c = Clip::new_video("c.mp4", 1, 10_000, 1000);
        let (aid, bid, cid) = (a.id, b.id, c.id);
        state.clips.push(a);
        state.clips.push(b);
        state.clips.push(c);

        let mut cmd = MoveManyCommand::new(vec![
            ClipMove {
                clip_id: aid,
                from_ms: 0,
                to_ms: 2000,
                from_track: 0,
                to_track: 0,
            },
            ClipMove {
                clip_id: bid,
                from_ms: 5000,
                to_ms: 7000,
                from_track: 0,
                to_track: 0,
            },
            ClipMove {
                clip_id: cid,
                from_ms: 10_000,
                to_ms: 12_000,
                from_track: 1,
                to_track: 2,
            },
        ]);
        cmd.execute(&mut state).unwrap();

        assert_eq!(
            state
                .clips
                .iter()
                .find(|c| c.id == aid)
                .unwrap()
                .start_time_ms,
            2000
        );
        assert_eq!(
            state
                .clips
                .iter()
                .find(|c| c.id == bid)
                .unwrap()
                .start_time_ms,
            7000
        );
        let cc = state.clips.iter().find(|c| c.id == cid).unwrap();
        assert_eq!(cc.start_time_ms, 12_000);
        assert_eq!(cc.track_index, 2);

        cmd.undo(&mut state).unwrap();
        assert_eq!(
            state
                .clips
                .iter()
                .find(|c| c.id == aid)
                .unwrap()
                .start_time_ms,
            0
        );
        assert_eq!(
            state
                .clips
                .iter()
                .find(|c| c.id == bid)
                .unwrap()
                .start_time_ms,
            5000
        );
        let cc = state.clips.iter().find(|c| c.id == cid).unwrap();
        assert_eq!(cc.start_time_ms, 10_000);
        assert_eq!(cc.track_index, 1);
    }

    #[test]
    fn empty_move_is_noop() {
        let mut state = ProjectState::default();
        let mut cmd = MoveManyCommand::new(vec![]);
        cmd.execute(&mut state).unwrap();
        cmd.undo(&mut state).unwrap();
        assert!(state.clips.is_empty());
    }
}
