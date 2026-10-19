//! Move or resize a sticker on the frame (preview drag).

use crate::clip::OverlayPlacement;
use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

/// Carries both placements rather than capturing one when it runs.
///
/// `SetClipCommand` records its undo point the moment it executes, which
/// is the wrong moment for a drag: by the time the pointer is released the
/// clip has already been mutated live for visual feedback, so "before"
/// would be the dragged position and undo would be a no-op. Naming both
/// ends makes the command safe to execute once at the end of a gesture
/// that already mutated the model.
pub struct MoveOverlayCommand {
    pub clip_id: Uuid,
    pub from: OverlayPlacement,
    pub to: OverlayPlacement,
}

impl MoveOverlayCommand {
    pub fn new(clip_id: Uuid, from: OverlayPlacement, to: OverlayPlacement) -> Self {
        Self { clip_id, from, to }
    }
}

impl Command for MoveOverlayCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(c) = state.clips.iter_mut().find(|c| c.id == self.clip_id) {
            c.overlay = Some(self.to.clone());
        }
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(c) = state.clips.iter_mut().find(|c| c.id == self.clip_id) {
            c.overlay = Some(self.from.clone());
        }
        Ok(())
    }

    fn description(&self) -> String {
        "Move sticker".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::Clip;

    fn project_with_overlay() -> (ProjectState, Uuid, OverlayPlacement) {
        let place = OverlayPlacement {
            scale: 0.34,
            x: 0.5,
            y: 0.72,
            aspect: 1.0,
            rotation: 0.0,
            animation: None,
        };
        let mut clip = Clip::new_video("a.mp4", 0, 0, 4000);
        clip.id = Uuid::from_u128(7);
        clip.overlay = Some(place.clone());
        let mut state = ProjectState::default();
        state.clips.push(clip);
        (state, Uuid::from_u128(7), place)
    }

    #[test]
    fn undo_returns_the_sticker_to_where_the_drag_started() {
        // The live drag mutates the clip before the command ever runs, so
        // "before" has to be the start of the gesture rather than
        // whatever the clip happened to look like at release.
        let (mut state, id, from) = project_with_overlay();
        let to = OverlayPlacement {
            scale: 0.8,
            x: 0.1,
            y: 0.2,
            rotation: 30.0,
            ..from.clone()
        };
        state.clips[0].overlay = Some(to.clone());

        let mut cmd = MoveOverlayCommand::new(id, from.clone(), to.clone());
        cmd.execute(&mut state).unwrap();
        assert_eq!(state.clips[0].overlay.as_ref().unwrap().scale, 0.8);

        cmd.undo(&mut state).unwrap();
        let back = state.clips[0].overlay.as_ref().unwrap();
        assert_eq!(back.scale, from.scale);
        assert_eq!(back.x, from.x);
        assert_eq!(back.y, from.y);
        assert_eq!(back.rotation, from.rotation);
        assert!((to.rotation - 30.0).abs() < 1e-6);
    }

    #[test]
    fn redo_reapplies_the_dropped_position() {
        let (mut state, id, from) = project_with_overlay();
        let to = OverlayPlacement {
            x: 0.9,
            ..from.clone()
        };
        let mut cmd = MoveOverlayCommand::new(id, from, to.clone());
        cmd.undo(&mut state).unwrap();
        cmd.execute(&mut state).unwrap();
        assert_eq!(state.clips[0].overlay.as_ref().unwrap().x, 0.9);
    }

    #[test]
    fn a_drag_that_ends_where_it_started_leaves_no_trace() {
        let (mut state, id, from) = project_with_overlay();
        let mut cmd = MoveOverlayCommand::new(id, from.clone(), from.clone());
        cmd.execute(&mut state).unwrap();
        cmd.undo(&mut state).unwrap();
        assert_eq!(state.clips[0].overlay.as_ref().unwrap().scale, from.scale);
    }

    #[test]
    fn a_missing_clip_is_not_an_error() {
        // A clip can vanish mid-gesture (deleted from the timeline while a
        // preview drag is in flight). Dropping it silently beats making
        // the whole undo stack error out.
        let mut state = ProjectState::default();
        let place = OverlayPlacement::default_sticker();
        let mut cmd = MoveOverlayCommand::new(Uuid::from_u128(99), place.clone(), place);
        cmd.execute(&mut state).unwrap();
        cmd.undo(&mut state).unwrap();
        assert!(state.clips.is_empty());
    }
}
