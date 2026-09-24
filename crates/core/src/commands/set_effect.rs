//! Commands for attaching/removing effects and transitions on a clip.

use crate::clip::{Clip, EffectInstance};
use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct AddEffectCommand {
    pub clip_id: Uuid,
    pub effect_id: String,
    before: Option<Clip>,
}

impl AddEffectCommand {
    pub fn new(clip_id: Uuid, effect_id: impl Into<String>) -> Self {
        Self {
            clip_id,
            effect_id: effect_id.into(),
            before: None,
        }
    }
}

impl Command for AddEffectCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) else {
            return Ok(());
        };
        self.before = Some(clip.clone());
        if !clip.effects.iter().any(|e| e.effect_id == self.effect_id) {
            clip.effects.push(EffectInstance {
                effect_id: self.effect_id.clone(),
                amount: 1.0,
                enabled: true,
            });
        }
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(orig) = self.before.take() {
            if let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) {
                *clip = orig;
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Add effect '{}'", self.effect_id)
    }
}

pub struct RemoveEffectCommand {
    pub clip_id: Uuid,
    pub effect_id: String,
    before: Option<Clip>,
}

impl RemoveEffectCommand {
    pub fn new(clip_id: Uuid, effect_id: impl Into<String>) -> Self {
        Self {
            clip_id,
            effect_id: effect_id.into(),
            before: None,
        }
    }
}

impl Command for RemoveEffectCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) else {
            return Ok(());
        };
        self.before = Some(clip.clone());
        clip.effects.retain(|e| e.effect_id != self.effect_id);
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(orig) = self.before.take() {
            if let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) {
                *clip = orig;
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Remove effect '{}'", self.effect_id)
    }
}

pub struct SetTransitionCommand {
    pub clip_id: Uuid,
    pub in_edge: bool,
    pub transition_id: Option<String>,
    before: Option<Clip>,
}

impl SetTransitionCommand {
    pub fn new(clip_id: Uuid, in_edge: bool, transition_id: Option<String>) -> Self {
        Self {
            clip_id,
            in_edge,
            transition_id,
            before: None,
        }
    }
}

impl Command for SetTransitionCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) else {
            return Ok(());
        };
        self.before = Some(clip.clone());
        if self.in_edge {
            clip.transition_in = self.transition_id.clone();
        } else {
            clip.transition_out = self.transition_id.clone();
        }
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(orig) = self.before.take() {
            if let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) {
                *clip = orig;
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        let edge = if self.in_edge { "in" } else { "out" };
        format!("Set transition {edge} = {:?}", self.transition_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with_clip() -> (ProjectState, Uuid) {
        let mut s = ProjectState::default();
        let clip = Clip::new_video("a.mp4", 0, 0, 3000);
        let id = clip.id;
        s.add_clip(clip);
        (s, id)
    }

    #[test]
    fn add_effect_appends_once() {
        let (mut s, id) = project_with_clip();
        AddEffectCommand::new(id, "glitch").execute(&mut s).unwrap();
        AddEffectCommand::new(id, "glitch").execute(&mut s).unwrap();
        assert_eq!(s.clips[0].effects.len(), 1);
    }

    #[test]
    fn remove_effect_drops_it() {
        let (mut s, id) = project_with_clip();
        AddEffectCommand::new(id, "glitch").execute(&mut s).unwrap();
        RemoveEffectCommand::new(id, "glitch")
            .execute(&mut s)
            .unwrap();
        assert!(s.clips[0].effects.is_empty());
    }

    #[test]
    fn set_transition_undo_restores() {
        let (mut s, id) = project_with_clip();
        let mut cmd = SetTransitionCommand::new(id, true, Some("fade".into()));
        cmd.execute(&mut s).unwrap();
        assert_eq!(s.clips[0].transition_in.as_deref(), Some("fade"));
        cmd.undo(&mut s).unwrap();
        assert_eq!(s.clips[0].transition_in, None);
    }
}
