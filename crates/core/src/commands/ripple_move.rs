//! Ripple move: move a clip and shift every later same-track clip
//! by the same delta. Used when the toolbar's Ripple toggle is on:
//! dragging one clip pushes everything to its right out of the way
//! (positive delta) or pulls them in (negative delta), instead of
//! leaving a gap or overlapping.

use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct RippleMoveCommand {
    pub clip_id: Uuid,
    pub from_ms: u64,
    pub to_ms: u64,
    /// (clip_id, original_start_ms) for every clip this command moved,
    /// including the dragged clip itself. Used by undo.
    shifted: Vec<(Uuid, u64)>,
}

impl RippleMoveCommand {
    pub fn new(clip_id: Uuid, from_ms: u64, to_ms: u64) -> Self {
        Self {
            clip_id,
            from_ms,
            to_ms,
            shifted: Vec::new(),
        }
    }
}

impl Command for RippleMoveCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        self.shifted.clear();

        let Some(clip) = state.clips.iter().find(|c| c.id == self.clip_id).cloned() else {
            return Ok(());
        };
        let track_idx = clip.track_index;
        let delta = self.to_ms as i64 - self.from_ms as i64;
        if delta == 0 {
            return Ok(());
        }

        // Every clip on the same track whose start is >= the dragged
        // clip's original start. This includes the dragged clip itself
        // (its start == from_ms, and from_ms >= from_ms).
        let affected: Vec<(Uuid, u64)> = state
            .clips
            .iter()
            .filter(|c| c.track_index == track_idx && c.start_time_ms >= self.from_ms)
            .map(|c| (c.id, c.start_time_ms))
            .collect();

        for (id, orig) in &affected {
            if let Some(c) = state.clips.iter_mut().find(|c| c.id == *id) {
                let new_ms = (*orig as i64 + delta).max(0) as u64;
                c.start_time_ms = new_ms;
            }
        }
        self.shifted = affected;
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        for (id, orig) in &self.shifted {
            if let Some(c) = state.clips.iter_mut().find(|c| c.id == *id) {
                c.start_time_ms = *orig;
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!(
            "Ripple move clip {} \u{2192} {}ms",
            self.from_ms, self.to_ms
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::Clip;

    #[test]
    fn shifts_followers_right_when_moving_right() {
        let mut s = ProjectState::default();
        let a = Clip::new_video("a.mp4", 0, 0, 5000);
        let b = Clip::new_video("b.mp4", 0, 5000, 5000);
        let c = Clip::new_video("c.mp4", 0, 10000, 5000);
        let (aid, bid, cid) = (a.id, b.id, c.id);
        s.clips.push(a);
        s.clips.push(b);
        s.clips.push(c);

        let mut cmd = RippleMoveCommand::new(aid, 0, 2000);
        cmd.execute(&mut s).unwrap();

        assert_eq!(
            s.clips.iter().find(|c| c.id == aid).unwrap().start_time_ms,
            2000
        );
        assert_eq!(
            s.clips.iter().find(|c| c.id == bid).unwrap().start_time_ms,
            7000
        );
        assert_eq!(
            s.clips.iter().find(|c| c.id == cid).unwrap().start_time_ms,
            12000
        );
    }

    #[test]
    fn shifts_followers_left_when_moving_left() {
        let mut s = ProjectState::default();
        let a = Clip::new_video("a.mp4", 0, 0, 5000);
        let b = Clip::new_video("b.mp4", 0, 5000, 5000);
        let c = Clip::new_video("c.mp4", 0, 10000, 5000);
        let (aid, bid, cid) = (a.id, b.id, c.id);
        s.clips.push(a);
        s.clips.push(b);
        s.clips.push(c);

        let mut cmd = RippleMoveCommand::new(bid, 5000, 3000);
        cmd.execute(&mut s).unwrap();

        assert_eq!(
            s.clips.iter().find(|c| c.id == aid).unwrap().start_time_ms,
            0
        );
        assert_eq!(
            s.clips.iter().find(|c| c.id == bid).unwrap().start_time_ms,
            3000
        );
        assert_eq!(
            s.clips.iter().find(|c| c.id == cid).unwrap().start_time_ms,
            8000
        );
    }

    #[test]
    fn undo_restores_all_positions() {
        let mut s = ProjectState::default();
        let a = Clip::new_video("a.mp4", 0, 0, 5000);
        let b = Clip::new_video("b.mp4", 0, 5000, 5000);
        let (aid, bid) = (a.id, b.id);
        s.clips.push(a);
        s.clips.push(b);

        let mut cmd = RippleMoveCommand::new(aid, 0, 2000);
        cmd.execute(&mut s).unwrap();
        cmd.undo(&mut s).unwrap();

        assert_eq!(
            s.clips.iter().find(|c| c.id == aid).unwrap().start_time_ms,
            0
        );
        assert_eq!(
            s.clips.iter().find(|c| c.id == bid).unwrap().start_time_ms,
            5000
        );
    }

    #[test]
    fn does_not_touch_clips_on_other_tracks() {
        let mut s = ProjectState::default();
        let a = Clip::new_video("a.mp4", 0, 0, 5000);
        let b = Clip::new_video("b.mp4", 0, 5000, 5000);
        let c = Clip::new_video("c.mp4", 1, 5000, 5000);
        let (aid, bid, cid) = (a.id, b.id, c.id);
        s.clips.push(a);
        s.clips.push(b);
        s.clips.push(c);

        let mut cmd = RippleMoveCommand::new(aid, 0, 2000);
        cmd.execute(&mut s).unwrap();

        assert_eq!(
            s.clips.iter().find(|c| c.id == bid).unwrap().start_time_ms,
            7000
        );
        assert_eq!(
            s.clips.iter().find(|c| c.id == cid).unwrap().start_time_ms,
            5000
        );
    }

    #[test]
    fn zero_delta_is_noop() {
        let mut s = ProjectState::default();
        let a = Clip::new_video("a.mp4", 0, 0, 5000);
        let aid = a.id;
        s.clips.push(a);

        let mut cmd = RippleMoveCommand::new(aid, 0, 0);
        cmd.execute(&mut s).unwrap();
        assert_eq!(s.clips[0].start_time_ms, 0);
    }
}
