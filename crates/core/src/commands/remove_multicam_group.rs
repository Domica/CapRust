//! Remove a multi-camera group from the project.

use crate::commands::Command;
use crate::multicam::MultiCamGroup;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct RemoveMultiCamGroupCommand {
    group_id: Uuid,
    removed: Option<(usize, MultiCamGroup)>,
}

impl RemoveMultiCamGroupCommand {
    pub fn new(group_id: Uuid) -> Self {
        Self {
            group_id,
            removed: None,
        }
    }
}

impl Command for RemoveMultiCamGroupCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(idx) = state
            .multicam_groups
            .iter()
            .position(|g| g.id == self.group_id)
        else {
            anyhow::bail!("multicam group {} not found", self.group_id);
        };
        let g = state.multicam_groups.remove(idx);
        self.removed = Some((idx, g));
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some((idx, g)) = self.removed.take() {
            let at = idx.min(state.multicam_groups.len());
            state.multicam_groups.insert(at, g);
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Remove multicam group {}", self.group_id)
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
    fn remove_and_undo_restores_position() {
        let (mut p, gid) = project_with_group();
        assert_eq!(p.multicam_groups.len(), 1);
        let mut stack = UndoStack::default();
        stack
            .execute(Box::new(RemoveMultiCamGroupCommand::new(gid)), &mut p)
            .unwrap();
        assert!(p.multicam_groups.is_empty());
        stack.undo(&mut p).unwrap();
        assert_eq!(p.multicam_groups.len(), 1);
        assert_eq!(p.multicam_groups[0].id, gid);
    }

    #[test]
    fn rejects_unknown() {
        let (mut p, _gid) = project_with_group();
        let mut cmd = RemoveMultiCamGroupCommand::new(Uuid::new_v4());
        assert!(cmd.execute(&mut p).is_err());
    }
}
