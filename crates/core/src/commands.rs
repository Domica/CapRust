//! Command pattern (undo/redo).

pub mod delete_clip;
pub mod edit_caption_segment;
pub mod move_clip;
pub mod relink;
pub mod ripple;
pub mod separate_audio;
pub mod set_clip;
pub mod set_effect;
pub mod split_clip;

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
