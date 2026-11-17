//! Command pattern (undo/redo).

pub mod add_clip;
pub mod add_master_plugin;
pub mod add_media;
pub mod caption_segments;
pub mod create_multicam_group;
pub mod delete_clip;
pub mod duplicate_track;
pub mod edit_caption_segment;
pub mod macro_command;
pub mod move_clip;
pub mod move_many;
pub mod move_overlay;
pub mod reattach_audio;
pub mod relink;
pub mod relink_many;
pub mod remove_master_plugin;
pub mod remove_multicam_group;
pub mod ripple;
pub mod ripple_move;
pub mod separate_audio;
pub mod set_active_angle;
pub mod set_clip;
pub mod set_effect;
pub mod set_multicam_sync;
pub mod set_plugin_param;
pub mod split_clip;
pub mod translate_captions;

use crate::project::ProjectState;
use anyhow::Result;

pub trait Command: Send + Sync {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()>;
    fn undo(&mut self, state: &mut ProjectState) -> Result<()>;
    fn description(&self) -> String;
}

pub struct UndoStack {
    undo: Vec<Box<dyn Command>>,
    redo: Vec<Box<dyn Command>>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn execute(&mut self, mut cmd: Box<dyn Command>, state: &mut ProjectState) -> Result<()> {
        cmd.execute(state)?;
        self.undo.push(cmd);
        self.redo.clear();
        Ok(())
    }

    /// Execute multiple commands as a single atomic undo step.
    ///
    /// All commands run in order; if any fails, previously executed ones
    /// are rolled back (via `MacroCommand`). The whole batch becomes one
    /// entry on the undo stack, so Ctrl+Z reverts the entire batch at once.
    pub fn execute_batch(
        &mut self,
        commands: Vec<Box<dyn Command>>,
        state: &mut ProjectState,
    ) -> Result<()> {
        if commands.is_empty() {
            return Ok(());
        }
        let macro_cmd = crate::commands::macro_command::MacroCommand::new("batch", commands);
        self.execute(Box::new(macro_cmd), state)
    }

    pub fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(mut cmd) = self.undo.pop() {
            cmd.undo(state)?;
            self.redo.push(cmd);
        }
        Ok(())
    }

    pub fn redo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(mut cmd) = self.redo.pop() {
            cmd.execute(state)?;
            self.undo.push(cmd);
        }
        Ok(())
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod undo_stack_tests {
    use super::*;
    use crate::clip::Clip;
    use crate::commands::ripple::RippleInsertCommand;

    #[test]
    fn execute_batch_is_single_undo_step() {
        let mut state = ProjectState::default();
        let mut stack = UndoStack::new();

        stack
            .execute_batch(
                vec![
                    Box::new(RippleInsertCommand::new(Clip::new_text(
                        "A", 0, 0, 1000, false,
                    ))),
                    Box::new(RippleInsertCommand::new(Clip::new_text(
                        "B", 0, 1000, 1000, false,
                    ))),
                ],
                &mut state,
            )
            .unwrap();

        assert_eq!(state.clips.len(), 2);
        assert!(stack.can_undo());
        assert!(!stack.can_redo());

        // One undo reverts the whole batch.
        stack.undo(&mut state).unwrap();
        assert!(state.clips.is_empty(), "batch undo removes both clips");
        assert!(!stack.can_undo());
        assert!(stack.can_redo());
    }

    #[test]
    fn execute_batch_empty_is_noop() {
        let mut state = ProjectState::default();
        let mut stack = UndoStack::new();

        stack.execute_batch(vec![], &mut state).unwrap();
        assert!(state.clips.is_empty());
        assert!(!stack.can_undo());
    }

    #[test]
    fn execute_batch_rollback_on_failure() {
        // Use a command that always fails to verify atomicity.
        struct FailingCmd;
        impl Command for FailingCmd {
            fn execute(&mut self, _: &mut ProjectState) -> Result<()> {
                Err(anyhow::anyhow!("boom"))
            }
            fn undo(&mut self, _: &mut ProjectState) -> Result<()> {
                Ok(())
            }
            fn description(&self) -> String {
                "failing".into()
            }
        }

        let mut state = ProjectState::default();
        let mut stack = UndoStack::new();

        let err = stack
            .execute_batch(vec![Box::new(FailingCmd), Box::new(FailingCmd)], &mut state)
            .unwrap_err();
        assert!(err.to_string().contains("boom"));
        assert!(!stack.can_undo(), "failed batch leaves no undo entry");
    }
}
