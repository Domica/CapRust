//! Commands for attaching/removing effects and transitions on a clip.

use crate::clip::{Clip, EffectInstance};
use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct AddEffectCommand {
    pub clip_id: Uuid,
    pub effect_id: String,
    before: Option<Clip>,
}

impl AddEffectCommand {
    pub fn new(clip_id: Uuid, effect_id: impl Into<String>) -> Self {
        Self {
            clip_id,
            effect_id: effect_id.into(),
            before: None,
        }
    }
}

impl Command for AddEffectCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) else {
            return Ok(());
        };
        self.before = Some(clip.clone());
        if !clip.effects.iter().any(|e| e.effect_id == self.effect_id) {
            clip.effects.push(EffectInstance {
                effect_id: self.effect_id.clone(),
                amount: 1.0,
                enabled: true,
            });
        }
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(orig) = self.before.take() {
            if let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) {
                *clip = orig;
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Add effect '{}'", self.effect_id)
    }
}

pub struct RemoveEffectCommand {
    pub clip_id: Uuid,
    pub effect_id: String,
    before: Option<Clip>,
}

impl RemoveEffectCommand {
    pub fn new(clip_id: Uuid, effect_id: impl Into<String>) -> Self {
        Self {
            clip_id,
            effect_id: effect_id.into(),
            before: None,
        }
    }
}

impl Command for RemoveEffectCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) else {
            return Ok(());
        };
        self.before = Some(clip.clone());
        clip.effects.retain(|e| e.effect_id != self.effect_id);
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(orig) = self.before.take() {
            if let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) {
                *clip = orig;
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Remove effect '{}'", self.effect_id)
    }
}

/// Tolerance for detecting that a previous clip on the same track
/// touches the current one. Matches the render planner's adjacency
/// rule (ADJACENCY_TOL_SEC).
const ADJACENCY_TOL_MS: u64 = 200;

pub struct SetTransitionCommand {
    pub clip_id: Uuid,
    pub in_edge: bool,
    pub transition_id: Option<String>,
    before: Option<Clip>,
    /// Every clip the command shifted, with its ORIGINAL start_ms.
    /// Includes the target clip. Undo restores each in reverse.
    shifted: Vec<(Uuid, u64)>,
}

impl SetTransitionCommand {
    pub fn new(clip_id: Uuid, in_edge: bool, transition_id: Option<String>) -> Self {
        Self {
            clip_id,
            in_edge,
            transition_id,
            before: None,
            shifted: Vec::new(),
        }
    }
}

impl Command for SetTransitionCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        self.shifted.clear();

        let Some(clip) = state.clips.iter().find(|c| c.id == self.clip_id).cloned() else {
            return Ok(());
        };
        self.before = Some(clip.clone());

        let track_idx = clip.track_index;
        let current_start = clip.start_time_ms;
        let old_shift = clip.applied_xfade_shift_ms;

        // Should this transition render as an xfade between two
        // clips? Only an in-edge xfade that actually has a touching
        // predecessor does. Fade-in on a lone clip (fade-from-black)
        // does not shift anything.
        let wants_xfade = self.in_edge
            && self
                .transition_id
                .as_deref()
                .map(crate::clip::is_xfade_transition)
                .unwrap_or(false);

        let has_predecessor = if wants_xfade {
            state
                .clips
                .iter()
                .filter(|c| c.track_index == track_idx && c.id != self.clip_id)
                .filter_map(|c| {
                    let end = c.start_time_ms + c.duration_ms;
                    if end <= current_start {
                        Some(end)
                    } else {
                        None
                    }
                })
                .max()
                .map(|end| current_start.saturating_sub(end) <= ADJACENCY_TOL_MS)
                .unwrap_or(false)
        } else {
            false
        };

        let target_shift_ms = if wants_xfade && has_predecessor {
            clip.transition_duration_ms
        } else {
            0
        };

        // Update the transition field and the recorded shift.
        if let Some(c) = state.clips.iter_mut().find(|c| c.id == self.clip_id) {
            if self.in_edge {
                c.transition_in = self.transition_id.clone();
            } else {
                c.transition_out = self.transition_id.clone();
            }
            c.applied_xfade_shift_ms = target_shift_ms;
        }

        // Compute the delta and shift every clip on this track that
        // starts at or after the target's ORIGINAL position.
        let delta = old_shift as i64 - target_shift_ms as i64;
        if delta != 0 {
            // Snapshot before mutating, and shift.
            let affected: Vec<(Uuid, u64)> = state
                .clips
                .iter()
                .filter(|c| c.track_index == track_idx && c.start_time_ms >= current_start)
                .map(|c| (c.id, c.start_time_ms))
                .collect();
            for (id, orig) in &affected {
                if let Some(c) = state.clips.iter_mut().find(|c| c.id == *id) {
                    let new_ms = (*orig as i64 + delta).max(0) as u64;
                    c.start_time_ms = new_ms;
                }
            }
            self.shifted = affected;
        }

        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        // Restore shifted clips in reverse so partial states are
        // consistent if anything ever fails mid-undo.
        for (id, orig_ms) in self.shifted.drain(..).rev() {
            if let Some(c) = state.clips.iter_mut().find(|c| c.id == id) {
                c.start_time_ms = orig_ms;
            }
        }
        if let Some(orig) = self.before.take() {
            if let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) {
                *clip = orig;
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        let edge = if self.in_edge { "in" } else { "out" };
        format!("Set transition {edge} = {:?}", self.transition_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with_clip() -> (ProjectState, Uuid) {
        let mut s = ProjectState::default();
        let clip = Clip::new_video("a.mp4", 0, 0, 3000);
        let id = clip.id;
        s.add_clip(clip);
        (s, id)
    }

    #[test]
    fn add_effect_appends_once() {
        let (mut s, id) = project_with_clip();
        AddEffectCommand::new(id, "glitch").execute(&mut s).unwrap();
        AddEffectCommand::new(id, "glitch").execute(&mut s).unwrap();
        assert_eq!(s.clips[0].effects.len(), 1);
    }

    #[test]
    fn remove_effect_drops_it() {
        let (mut s, id) = project_with_clip();
        AddEffectCommand::new(id, "glitch").execute(&mut s).unwrap();
        RemoveEffectCommand::new(id, "glitch")
            .execute(&mut s)
            .unwrap();
        assert!(s.clips[0].effects.is_empty());
    }

    #[test]
    fn set_transition_undo_restores() {
        let (mut s, id) = project_with_clip();
        let mut cmd = SetTransitionCommand::new(id, true, Some("fade".into()));
        cmd.execute(&mut s).unwrap();
        assert_eq!(s.clips[0].transition_in.as_deref(), Some("fade"));
        cmd.undo(&mut s).unwrap();
        assert_eq!(s.clips[0].transition_in, None);
    }

    #[test]
    fn xfade_between_adjacent_clips_shifts_second_left() {
        use crate::clip::Clip;
        let mut s = ProjectState::default();
        let a = Clip::new_video("a.mp4", 0, 0, 10_000);
        let b = Clip::new_video("b.mp4", 0, 10_000, 10_000);
        let aid = a.id;
        let bid = b.id;
        s.add_clip(a);
        s.add_clip(b);

        // Default transition_duration_ms is 500, override to 3000 for
        // the shift math.
        if let Some(c) = s.clips.iter_mut().find(|c| c.id == bid) {
            c.transition_duration_ms = 3000;
        }

        let mut cmd = SetTransitionCommand::new(bid, true, Some("fade".into()));
        cmd.execute(&mut s).unwrap();

        let a2 = s.clips.iter().find(|c| c.id == aid).unwrap();
        let b2 = s.clips.iter().find(|c| c.id == bid).unwrap();
        assert_eq!(a2.start_time_ms, 0, "first clip stays put");
        assert_eq!(b2.start_time_ms, 7000, "second clip shifted left by 3s");
        assert_eq!(b2.applied_xfade_shift_ms, 3000);
        assert_eq!(b2.transition_in.as_deref(), Some("fade"));

        cmd.undo(&mut s).unwrap();
        let b3 = s.clips.iter().find(|c| c.id == bid).unwrap();
        assert_eq!(b3.start_time_ms, 10_000, "undo restores original position");
        assert_eq!(b3.applied_xfade_shift_ms, 0);
        assert_eq!(b3.transition_in, None);
    }

    #[test]
    fn removing_xfade_shifts_back_to_original() {
        use crate::clip::Clip;
        let mut s = ProjectState::default();
        let a = Clip::new_video("a.mp4", 0, 0, 10_000);
        let b = Clip::new_video("b.mp4", 0, 10_000, 10_000);
        let bid = b.id;
        s.add_clip(a);
        s.add_clip(b);
        if let Some(c) = s.clips.iter_mut().find(|c| c.id == bid) {
            c.transition_duration_ms = 3000;
        }

        let mut add = SetTransitionCommand::new(bid, true, Some("fade".into()));
        add.execute(&mut s).unwrap();
        assert_eq!(
            s.clips.iter().find(|c| c.id == bid).unwrap().start_time_ms,
            7000
        );

        // Now remove it.
        let mut rem = SetTransitionCommand::new(bid, true, None);
        rem.execute(&mut s).unwrap();
        let b2 = s.clips.iter().find(|c| c.id == bid).unwrap();
        assert_eq!(b2.start_time_ms, 10_000);
        assert_eq!(b2.applied_xfade_shift_ms, 0);
    }

    #[test]
    fn fade_in_without_predecessor_does_not_shift() {
        use crate::clip::Clip;
        let mut s = ProjectState::default();
        let a = Clip::new_video("a.mp4", 0, 5_000, 10_000);
        let aid = a.id;
        s.add_clip(a);
        if let Some(c) = s.clips.iter_mut().find(|c| c.id == aid) {
            c.transition_duration_ms = 3000;
        }

        let mut cmd = SetTransitionCommand::new(aid, true, Some("fade".into()));
        cmd.execute(&mut s).unwrap();

        let a2 = s.clips.iter().find(|c| c.id == aid).unwrap();
        assert_eq!(a2.start_time_ms, 5_000, "lone clip must not shift");
        assert_eq!(a2.applied_xfade_shift_ms, 0, "no xfade means no shift");
    }
}
