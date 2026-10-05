//! Create a new multi-camera group from existing clips.
//!
//! The group references clips that are already on the timeline;
//! it does not create tracks or move clips. See DIRECTIVES 28.3.

use crate::commands::Command;
use crate::multicam::MultiCamGroup;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct CreateMultiCamGroupCommand {
    group_id: Uuid,
    name: String,
    angle_clip_ids: Vec<Uuid>,
    /// Filled on execute; used by undo to remove exactly the
    /// group we pushed.
    inserted_index: Option<usize>,
}

impl CreateMultiCamGroupCommand {
    pub fn new(name: impl Into<String>, angle_clip_ids: Vec<Uuid>) -> Self {
        Self {
            group_id: Uuid::new_v4(),
            name: name.into(),
            angle_clip_ids,
            inserted_index: None,
        }
    }

    pub fn group_id(&self) -> Uuid {
        self.group_id
    }
}

impl Command for CreateMultiCamGroupCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        if self.angle_clip_ids.len() < 2 {
            anyhow::bail!(
                "multicam group needs at least 2 angle clips, got {}",
                self.angle_clip_ids.len()
            );
        }
        for id in &self.angle_clip_ids {
            if !state.clips.iter().any(|c| c.id == *id) {
                anyhow::bail!("angle clip {id} not found in project");
            }
        }
        let group = MultiCamGroup {
            id: self.group_id,
            name: self.name.clone(),
            angle_clip_ids: self.angle_clip_ids.clone(),
            sync_offsets_ms: Vec::new(),
        };
        let idx = state.multicam_groups.len();
        state.multicam_groups.push(group);
        self.inserted_index = Some(idx);
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(idx) = self.inserted_index.take() {
            if idx < state.multicam_groups.len() {
                state.multicam_groups.remove(idx);
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Create multicam group {}", self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::Clip;
    use crate::commands::UndoStack;

    fn project_with_two_clips() -> (ProjectState, Uuid, Uuid) {
        let mut p = ProjectState::default();
        let a = Clip::new_video("a.mp4", 0, 0, 1000);
        let b = Clip::new_video("b.mp4", 1, 0, 1000);
        let aid = a.id;
        let bid = b.id;
        p.add_clip(a);
        p.add_clip(b);
        (p, aid, bid)
    }

    #[test]
    fn creates_group_and_undo_removes() {
        let (mut p, a, b) = project_with_two_clips();
        assert!(p.multicam_groups.is_empty());
        let mut stack = UndoStack::default();
        let cmd = CreateMultiCamGroupCommand::new("cam", vec![a, b]);
        let gid = cmd.group_id();
        stack.execute(Box::new(cmd), &mut p).unwrap();
        assert_eq!(p.multicam_groups.len(), 1);
        assert_eq!(p.multicam_groups[0].id, gid);
        assert_eq!(p.multicam_groups[0].angle_clip_ids, vec![a, b]);
        stack.undo(&mut p).unwrap();
        assert!(p.multicam_groups.is_empty());
    }

    #[test]
    fn rejects_fewer_than_two_angles() {
        let (mut p, a, _b) = project_with_two_clips();
        let mut cmd = CreateMultiCamGroupCommand::new("x", vec![a]);
        assert!(cmd.execute(&mut p).is_err());
        assert!(p.multicam_groups.is_empty());
    }

    #[test]
    fn rejects_missing_clip_id() {
        let (mut p, a, _b) = project_with_two_clips();
        let ghost = Uuid::new_v4();
        let mut cmd = CreateMultiCamGroupCommand::new("x", vec![a, ghost]);
        assert!(cmd.execute(&mut p).is_err());
        assert!(p.multicam_groups.is_empty());
    }
}
