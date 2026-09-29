use anyhow::Result;
use uuid::Uuid;

use crate::commands::Command;
use crate::media::MediaKind;
use crate::project::ProjectState;

/// Append a `MediaItem` to the project's media library.
pub struct AddMediaCommand {
    path: String,
    kind: MediaKind,
    added_id: Option<Uuid>,
}

impl AddMediaCommand {
    pub fn new(path: impl Into<String>, kind: MediaKind) -> Self {
        Self {
            path: path.into(),
            kind,
            added_id: None,
        }
    }
}

impl Command for AddMediaCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let id = state.media.add(&self.path, self.kind);
        self.added_id = Some(id);
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(id) = self.added_id.take() {
            state.media.remove(id);
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Add media: {}", self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_item_and_undo_removes() {
        let mut p = ProjectState::default();
        let before = p.media.items.len();
        let mut cmd = AddMediaCommand::new("clip.mp4", MediaKind::Video);
        cmd.execute(&mut p).unwrap();
        assert_eq!(p.media.items.len(), before + 1);
        cmd.undo(&mut p).unwrap();
        assert_eq!(p.media.items.len(), before);
    }

    #[test]
    fn redo_after_undo_adds_again_with_new_id() {
        let mut p = ProjectState::default();
        let mut cmd = AddMediaCommand::new("clip.mp4", MediaKind::Video);
        cmd.execute(&mut p).unwrap();
        let first = cmd.added_id;
        cmd.undo(&mut p).unwrap();
        cmd.execute(&mut p).unwrap();
        assert_ne!(cmd.added_id, first);
    }

    #[test]
    fn undo_without_execute_is_noop() {
        let mut p = ProjectState::default();
        let before = p.media.items.len();
        let mut cmd = AddMediaCommand::new("clip.mp4", MediaKind::Video);
        cmd.undo(&mut p).unwrap();
        assert_eq!(p.media.items.len(), before);
    }

    #[test]
    fn description_mentions_path() {
        let c = AddMediaCommand::new("foo/bar.mp3", MediaKind::Audio);
        assert!(c.description().contains("bar.mp3"));
    }
}
