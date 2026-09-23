//! Missing media relink — updates media paths when files move.

use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct RelinkCommand {
    pub clip_id: Uuid,
    pub old_path: String,
    pub new_path: String,
}

impl RelinkCommand {
    pub fn new(clip_id: Uuid, new_path: impl Into<String>) -> Self {
        Self {
            clip_id,
            old_path: String::new(),
            new_path: new_path.into(),
        }
    }
}

impl Command for RelinkCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) {
            self.old_path = match &clip.clip_type {
                crate::clip::ClipType::Video { path, .. }
                | crate::clip::ClipType::Audio { path, .. }
                | crate::clip::ClipType::Image { path, .. } => path.clone(),
                _ => return Err(anyhow::anyhow!("Cannot relink text overlay")),
            };
            match &mut clip.clip_type {
                crate::clip::ClipType::Video { path, .. }
                | crate::clip::ClipType::Audio { path, .. }
                | crate::clip::ClipType::Image { path, .. } => {
                    *path = self.new_path.clone();
                }
                _ => unreachable!(),
            }
        }
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) {
            match &mut clip.clip_type {
                crate::clip::ClipType::Video { path, .. }
                | crate::clip::ClipType::Audio { path, .. }
                | crate::clip::ClipType::Image { path, .. } => {
                    *path = self.old_path.clone();
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Relink clip to {}", self.new_path)
    }
}

#[derive(Debug, Clone)]
pub struct MissingMedia {
    pub clip_id: Uuid,
    pub name: String,
    pub path: String,
}

pub fn find_missing_media(state: &ProjectState) -> Vec<MissingMedia> {
    state
        .clips
        .iter()
        .filter_map(|clip| {
            let path = match &clip.clip_type {
                crate::clip::ClipType::Video { path, .. }
                | crate::clip::ClipType::Audio { path, .. }
                | crate::clip::ClipType::Image { path, .. } => path,
                _ => return None,
            };
            if !std::path::Path::new(path).exists() {
                Some(MissingMedia {
                    clip_id: clip.id,
                    name: path.clone(),
                    path: path.clone(),
                })
            } else {
                None
            }
        })
        .collect()
}
