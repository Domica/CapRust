use anyhow::Result;
use uuid::Uuid;

use crate::commands::Command;
use crate::plugin::PluginInstance;
use crate::project::ProjectState;

/// Remove a plugin from the master chain. Undo restores it at its
/// original index (chain order matters for audio).
pub struct RemoveMasterPluginCommand {
    plugin_id: Uuid,
    removed: Option<(usize, PluginInstance)>,
}

impl RemoveMasterPluginCommand {
    pub fn new(plugin_id: Uuid) -> Self {
        Self {
            plugin_id,
            removed: None,
        }
    }
}

impl Command for RemoveMasterPluginCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(idx) = state
            .master_plugins
            .iter()
            .position(|p| p.id == self.plugin_id)
        {
            let inst = state.master_plugins.remove(idx);
            self.removed = Some((idx, inst));
        }
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some((idx, inst)) = self.removed.take() {
            let idx = idx.min(state.master_plugins.len());
            state.master_plugins.insert(idx, inst);
        }
        Ok(())
    }

    fn description(&self) -> String {
        "Remove master plugin".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn inst(name: &str) -> PluginInstance {
        PluginInstance::new(format!("com.x.{name}"), PathBuf::from("/x.clap"), name)
    }

    #[test]
    fn removes_and_undo_restores_at_index() {
        let mut p = ProjectState::default();
        let a = inst("A");
        let b = inst("B");
        let c = inst("C");
        let b_id = b.id;
        p.master_plugins = vec![a, b, c];

        let mut cmd = RemoveMasterPluginCommand::new(b_id);
        cmd.execute(&mut p).unwrap();
        assert_eq!(p.master_plugins.len(), 2);
        assert_eq!(p.master_plugins[0].name, "A");
        assert_eq!(p.master_plugins[1].name, "C");

        cmd.undo(&mut p).unwrap();
        assert_eq!(p.master_plugins.len(), 3);
        assert_eq!(p.master_plugins[1].name, "B");
    }

    #[test]
    fn unknown_id_is_noop() {
        let mut p = ProjectState::default();
        p.master_plugins.push(inst("A"));
        let mut cmd = RemoveMasterPluginCommand::new(Uuid::new_v4());
        cmd.execute(&mut p).unwrap();
        assert_eq!(p.master_plugins.len(), 1);
    }

    #[test]
    fn undo_without_execute_is_noop() {
        let mut p = ProjectState::default();
        p.master_plugins.push(inst("A"));
        let mut cmd = RemoveMasterPluginCommand::new(Uuid::new_v4());
        cmd.undo(&mut p).unwrap();
        assert_eq!(p.master_plugins.len(), 1);
    }
}
