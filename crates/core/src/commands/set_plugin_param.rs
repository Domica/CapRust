use anyhow::Result;
use uuid::Uuid;

use crate::commands::Command;
use crate::project::ProjectState;

/// Set one normalized parameter on one master plugin instance.
///
/// `before` captures the previous value (or None if unset) so undo
/// restores it exactly. Values outside 0..=1 are rejected before
/// touching state — `PluginInstance::set_param` returns false.
pub struct SetPluginParamCommand {
    plugin_id: Uuid,
    param_id: u32,
    value: f32,
    before: Option<f32>,
    executed: bool,
}

impl SetPluginParamCommand {
    pub fn new(plugin_id: Uuid, param_id: u32, value: f32) -> Self {
        Self {
            plugin_id,
            param_id,
            value,
            before: None,
            executed: false,
        }
    }
}

impl Command for SetPluginParamCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(inst) = state
            .master_plugins
            .iter_mut()
            .find(|p| p.id == self.plugin_id)
        else {
            return Ok(());
        };
        self.before = inst.get_param(self.param_id);
        if inst.set_param(self.param_id, self.value) {
            self.executed = true;
        }
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if !self.executed {
            return Ok(());
        }
        let Some(inst) = state
            .master_plugins
            .iter_mut()
            .find(|p| p.id == self.plugin_id)
        else {
            return Ok(());
        };
        match self.before {
            Some(v) => {
                inst.set_param(self.param_id, v);
            }
            None => {
                inst.params.retain(|(id, _)| *id != self.param_id);
            }
        }
        self.executed = false;
        Ok(())
    }

    fn description(&self) -> String {
        format!("Set plugin param {} = {}", self.param_id, self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::PluginInstance;
    use std::path::PathBuf;

    fn setup() -> (ProjectState, Uuid) {
        let mut p = ProjectState::default();
        let inst = PluginInstance::new("x", PathBuf::from("/x.clap"), "X");
        let id = inst.id;
        p.master_plugins.push(inst);
        (p, id)
    }

    #[test]
    fn sets_new_param_and_undo_removes_it() {
        let (mut p, id) = setup();
        let mut c = SetPluginParamCommand::new(id, 5, 0.5);
        c.execute(&mut p).unwrap();
        assert_eq!(p.master_plugins[0].get_param(5), Some(0.5));
        c.undo(&mut p).unwrap();
        assert_eq!(p.master_plugins[0].get_param(5), None);
    }

    #[test]
    fn replaces_existing_and_undo_restores() {
        let (mut p, id) = setup();
        p.master_plugins[0].set_param(5, 0.2);

        let mut c = SetPluginParamCommand::new(id, 5, 0.8);
        c.execute(&mut p).unwrap();
        assert_eq!(p.master_plugins[0].get_param(5), Some(0.8));

        c.undo(&mut p).unwrap();
        assert_eq!(p.master_plugins[0].get_param(5), Some(0.2));
    }

    #[test]
    fn rejects_out_of_range_value() {
        let (mut p, id) = setup();
        let mut c = SetPluginParamCommand::new(id, 5, 2.0);
        c.execute(&mut p).unwrap();
        assert_eq!(p.master_plugins[0].get_param(5), None);
        // undo is a no-op because nothing was executed
        c.undo(&mut p).unwrap();
        assert_eq!(p.master_plugins[0].get_param(5), None);
    }

    #[test]
    fn unknown_plugin_id_is_noop() {
        let (mut p, _) = setup();
        let mut c = SetPluginParamCommand::new(Uuid::new_v4(), 5, 0.5);
        c.execute(&mut p).unwrap();
        assert_eq!(p.master_plugins[0].params.len(), 0);
    }
}
