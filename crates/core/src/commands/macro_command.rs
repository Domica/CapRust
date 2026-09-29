//! Run a list of commands as a single undoable step.

use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;

/// Executes inner commands in order; undo runs them in reverse so
/// state is restored exactly as the pre-execute snapshot. Use this
/// whenever a single user gesture produces N mutations (batch drop,
/// batch delete, batch effect) so Ctrl+Z is one step, not N.
pub struct MacroCommand {
    commands: Vec<Box<dyn Command>>,
    description: String,
}

impl MacroCommand {
    pub fn new(description: impl Into<String>, commands: Vec<Box<dyn Command>>) -> Self {
        Self {
            commands,
            description: description.into(),
        }
    }

    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

impl Command for MacroCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        // Track how many succeeded so a mid-batch failure does not
        // leave the state half-applied without a matching undo path.
        // If a command errors, we roll back everything already
        // executed and propagate the error.
        let mut applied = 0usize;
        for cmd in &mut self.commands {
            match cmd.execute(state) {
                Ok(()) => applied += 1,
                Err(e) => {
                    for prev in self.commands[..applied].iter_mut().rev() {
                        let _ = prev.undo(state);
                    }
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        for cmd in self.commands.iter_mut().rev() {
            cmd.undo(state)?;
        }
        Ok(())
    }

    fn description(&self) -> String {
        self.description.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::Clip;
    use crate::commands::ripple::RippleInsertCommand;

    #[test]
    fn macro_runs_in_order_and_undo_reverses() {
        let mut state = ProjectState::default();
        let mut m = MacroCommand::new(
            "two inserts",
            vec![
                Box::new(RippleInsertCommand::new(Clip::new_text(
                    "A", 0, 0, 1000, false,
                ))),
                Box::new(RippleInsertCommand::new(Clip::new_text(
                    "B", 0, 1000, 1000, false,
                ))),
            ],
        );
        m.execute(&mut state).unwrap();
        assert_eq!(state.clips.len(), 2);
        // Both are on the timeline with the correct labels.
        assert!(state.clips.iter().any(|c| matches!(
            &c.clip_type,
            crate::clip::ClipType::TextOverlay { content, .. } if content == "A"
        )));
        assert!(state.clips.iter().any(|c| matches!(
            &c.clip_type,
            crate::clip::ClipType::TextOverlay { content, .. } if content == "B"
        )));

        m.undo(&mut state).unwrap();
        assert!(state.clips.is_empty(), "undo removes both clips");
    }

    #[test]
    fn empty_macro_is_noop() {
        let mut state = ProjectState::default();
        let mut m = MacroCommand::new("empty", vec![]);
        assert!(m.is_empty());
        m.execute(&mut state).unwrap();
        m.undo(&mut state).unwrap();
        assert!(state.clips.is_empty());
    }

    #[test]
    fn len_matches_input() {
        let m = MacroCommand::new(
            "x",
            vec![
                Box::new(RippleInsertCommand::new(Clip::new_text(
                    "A", 0, 0, 1000, false,
                ))),
                Box::new(RippleInsertCommand::new(Clip::new_text(
                    "B", 0, 2000, 1000, false,
                ))),
                Box::new(RippleInsertCommand::new(Clip::new_text(
                    "C", 0, 4000, 1000, false,
                ))),
            ],
        );
        assert_eq!(m.len(), 3);
    }
}
