use anyhow::Result;
use uuid::Uuid;

use crate::commands::Command;
use crate::plugin::PluginInstance;
use crate::project::ProjectState;

/// Append a plugin to the master chain. Undo removes exactly that entry.
pub struct AddMasterPluginCommand {
    instance: PluginInstance,
    /// Set on execute so undo can find the exact index even if the
    /// chain was mutated in between (it cannot be, but the pattern
    /// matches AddMediaCommand).
    added_id: Option<Uuid>,
}

impl AddMasterPluginCommand {
    pub fn new(instance: PluginInstance) -> Self {
        Self {
            instance,
            added_id: None,
        }
    }
}

impl Command for AddMasterPluginCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        self.added_id = Some(self.instance.id);
        state.master_plugins.push(self.instance.clone());
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(id) = self.added_id.take() {
            state.master_plugins.retain(|p| p.id != id);
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Add master plugin: {}", self.instance.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn inst() -> PluginInstance {
        PluginInstance::new("com.x.A", PathBuf::from("/a.clap"), "A")
    }

    #[test]
    fn adds_and_undo_removes() {
        let mut p = ProjectState::default();
        let mut c = AddMasterPluginCommand::new(inst());
        c.execute(&mut p).unwrap();
        assert_eq!(p.master_plugins.len(), 1);
        c.undo(&mut p).unwrap();
        assert!(p.master_plugins.is_empty());
    }

    #[test]
    fn undo_without_execute_is_noop() {
        let mut p = ProjectState::default();
        let mut c = AddMasterPluginCommand::new(inst());
        c.undo(&mut p).unwrap();
        assert!(p.master_plugins.is_empty());
    }

    #[test]
    fn undo_removes_only_target() {
        let mut p = ProjectState::default();
        let a = inst();
        let a_id = a.id;
        let b = inst();
        p.master_plugins.push(b);
        let mut c = AddMasterPluginCommand::new(a);
        c.execute(&mut p).unwrap();
        assert_eq!(p.master_plugins.len(), 2);
        c.undo(&mut p).unwrap();
        assert_eq!(p.master_plugins.len(), 1);
        assert_ne!(p.master_plugins[0].id, a_id);
    }
}
