//! Switch which camera angle a multi-camera group renders.

use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct SetActiveAngleCommand {
    group_id: Uuid,
    new_angle: usize,
    previous: Option<usize>,
}

impl SetActiveAngleCommand {
    pub fn new(group_id: Uuid, new_angle: usize) -> Self {
        Self {
            group_id,
            new_angle,
            previous: None,
        }
    }
}

impl Command for SetActiveAngleCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(g) = state
            .multicam_groups
            .iter_mut()
            .find(|g| g.id == self.group_id)
        else {
            anyhow::bail!("multicam group {} not found", self.group_id);
        };
        if self.new_angle >= g.angle_clip_ids.len() {
            anyhow::bail!(
                "angle {} out of range (group has {})",
                self.new_angle,
                g.angle_clip_ids.len()
            );
        }
        self.previous = Some(std::mem::replace(&mut g.active_angle, self.new_angle));
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(prev) = self.previous.take() {
            if let Some(g) = state
                .multicam_groups
                .iter_mut()
                .find(|g| g.id == self.group_id)
            {
                g.active_angle = prev;
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Set active angle for {}", self.group_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::Clip;
    use crate::commands::create_multicam_group::CreateMultiCamGroupCommand;
    use crate::commands::UndoStack;

    fn project_with_group() -> (ProjectState, Uuid) {
        let mut p = ProjectState::default();
        let a = Clip::new_video("a.mp4", 0, 0, 1000);
        let b = Clip::new_video("b.mp4", 1, 0, 1000);
        let aid = a.id;
        let bid = b.id;
        p.add_clip(a);
        p.add_clip(b);
        let cmd = CreateMultiCamGroupCommand::new("cam", vec![aid, bid]);
        let gid = cmd.group_id();
        let mut stack = UndoStack::default();
        stack.execute(Box::new(cmd), &mut p).unwrap();
        (p, gid)
    }

    #[test]
    fn sets_angle_and_undo_restores() {
        let (mut p, gid) = project_with_group();
        assert_eq!(p.multicam_groups[0].active_angle, 0);
        let mut stack = UndoStack::default();
        stack
            .execute(Box::new(SetActiveAngleCommand::new(gid, 1)), &mut p)
            .unwrap();
        assert_eq!(p.multicam_groups[0].active_angle, 1);
        stack.undo(&mut p).unwrap();
        assert_eq!(p.multicam_groups[0].active_angle, 0);
    }

    #[test]
    fn rejects_out_of_range() {
        let (mut p, gid) = project_with_group();
        let mut cmd = SetActiveAngleCommand::new(gid, 5);
        assert!(cmd.execute(&mut p).is_err());
    }

    #[test]
    fn rejects_unknown_group() {
        let (mut p, _gid) = project_with_group();
        let mut cmd = SetActiveAngleCommand::new(Uuid::new_v4(), 0);
        assert!(cmd.execute(&mut p).is_err());
    }
}
