//! Write computed sync offsets into a MultiCamGroup.
//!
//! Pure data command: no I/O, no threading. The caller (UI) is
//! responsible for running media-io::multicam_sync first and
//! handing the resulting Vec<i64> here.

use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct SetMultiCamSyncCommand {
    group_id: Uuid,
    offsets_ms: Vec<i64>,
    previous: Option<Vec<i64>>,
}

impl SetMultiCamSyncCommand {
    pub fn new(group_id: Uuid, offsets_ms: Vec<i64>) -> Self {
        Self {
            group_id,
            offsets_ms,
            previous: None,
        }
    }
}

impl Command for SetMultiCamSyncCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(g) = state
            .multicam_groups
            .iter_mut()
            .find(|g| g.id == self.group_id)
        else {
            anyhow::bail!("multicam group {} not found", self.group_id);
        };
        if self.offsets_ms.len() != g.angle_clip_ids.len() {
            anyhow::bail!(
                "offset count {} does not match angle count {}",
                self.offsets_ms.len(),
                g.angle_clip_ids.len()
            );
        }
        self.previous = Some(std::mem::replace(
            &mut g.sync_offsets_ms,
            self.offsets_ms.clone(),
        ));
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(prev) = self.previous.take() {
            if let Some(g) = state
                .multicam_groups
                .iter_mut()
                .find(|g| g.id == self.group_id)
            {
                g.sync_offsets_ms = prev;
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Set multicam sync for {}", self.group_id)
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
    fn sets_offsets_and_undo_restores() {
        let (mut p, gid) = project_with_group();
        assert!(p.multicam_groups[0].sync_offsets_ms.is_empty());
        let mut stack = UndoStack::default();
        stack
            .execute(
                Box::new(SetMultiCamSyncCommand::new(gid, vec![0, -240])),
                &mut p,
            )
            .unwrap();
        assert_eq!(p.multicam_groups[0].sync_offsets_ms, vec![0, -240]);
        stack.undo(&mut p).unwrap();
        assert!(p.multicam_groups[0].sync_offsets_ms.is_empty());
    }

    #[test]
    fn rejects_offset_count_mismatch() {
        let (mut p, gid) = project_with_group();
        let mut cmd = SetMultiCamSyncCommand::new(gid, vec![0]);
        assert!(cmd.execute(&mut p).is_err());
    }

    #[test]
    fn rejects_unknown_group() {
        let (mut p, _gid) = project_with_group();
        let ghost = Uuid::new_v4();
        let mut cmd = SetMultiCamSyncCommand::new(ghost, vec![0, 0]);
        assert!(cmd.execute(&mut p).is_err());
    }
}
