//! Batch media relink -- updates `MediaItem.path` and every clip that
//! references the same file, in one undoable command.
//!
//! Used on project load when one or more source files are missing.
//! The UI prompts the user to locate a folder; files in it whose
//! basename matches a missing `MediaItem.name` (case-insensitive)
//! become the new paths.

use crate::clip::{Clip, ClipType};
use crate::commands::Command;
use crate::media::MediaItem;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

/// One file's worth of relink: the media item id, its old path, and
/// the new path it should point at.
#[derive(Debug, Clone)]
pub struct RelinkMapping {
    pub media_id: Uuid,
    pub old_path: String,
    pub new_path: String,
}

pub struct RelinkManyCommand {
    mappings: Vec<RelinkMapping>,
    /// `(media_id, old_path)` -- captured in `execute()`, consumed in `undo()`.
    applied_media: Vec<(Uuid, String)>,
    /// `(clip_id, old_inline_path)` -- captured in `execute()`, consumed in `undo()`.
    applied_clips: Vec<(Uuid, String)>,
}

impl RelinkManyCommand {
    pub fn new(mappings: Vec<RelinkMapping>) -> Self {
        Self {
            mappings,
            applied_media: Vec::new(),
            applied_clips: Vec::new(),
        }
    }

    pub fn mappings(&self) -> &[RelinkMapping] {
        &self.mappings
    }
}

impl Command for RelinkManyCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        self.applied_media.clear();
        self.applied_clips.clear();

        // Pass 1 -- media library.
        for m in &self.mappings {
            if let Some(item) = state.media.items.iter_mut().find(|i| i.id == m.media_id) {
                if item.path != m.new_path {
                    self.applied_media.push((item.id, item.path.clone()));
                    item.path = m.new_path.clone();
                }
            } else {
                tracing::warn!(
                    "relink_many: media id {} not present in library (skipped)",
                    m.media_id
                );
            }
        }

        // Pass 2 -- clips. Match by media_id first, then by inline
        // path for projects saved before media_id was added.
        for clip in state.clips.iter_mut() {
            let mapping = match clip.media_id {
                Some(mid) => self.mappings.iter().find(|m| m.media_id == mid),
                None => {
                    let inline = match &clip.clip_type {
                        ClipType::Video { path, .. }
                        | ClipType::Audio { path, .. }
                        | ClipType::Image { path, .. } => path.as_str(),
                        _ => continue,
                    };
                    self.mappings.iter().find(|m| m.old_path == inline)
                }
            };

            let Some(m) = mapping else { continue };

            match &mut clip.clip_type {
                ClipType::Video { path, .. }
                | ClipType::Audio { path, .. }
                | ClipType::Image { path, .. } => {
                    if *path != m.new_path {
                        self.applied_clips.push((clip.id, path.clone()));
                        *path = m.new_path.clone();
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        // Reverse order -- clips first, then media items.
        for (clip_id, old_path) in self.applied_clips.drain(..).rev() {
            if let Some(clip) = state.clips.iter_mut().find(|c| c.id == clip_id) {
                match &mut clip.clip_type {
                    ClipType::Video { path, .. }
                    | ClipType::Audio { path, .. }
                    | ClipType::Image { path, .. } => *path = old_path,
                    _ => {}
                }
            }
        }
        for (media_id, old_path) in self.applied_media.drain(..).rev() {
            if let Some(item) = state.media.items.iter_mut().find(|i| i.id == media_id) {
                item.path = old_path;
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Relink {} media file(s)", self.mappings.len())
    }
}

// ---------------------------------------------------------------------------
// Scan helpers
// ---------------------------------------------------------------------------

/// A single media library entry whose file no longer exists on disk.
#[derive(Debug, Clone)]
pub struct MissingRef {
    pub media_id: Uuid,
    pub name: String,
    pub old_path: String,
    /// How many timeline clips reference this item (via media_id, or
    /// via inline path for legacy clips). Informational.
    pub clip_count: usize,
}

/// Scan the media library for items whose file is missing on disk.
/// Deduplicates by media id. Synchronous -- call on project load only.
pub fn find_missing_media_items(state: &ProjectState) -> Vec<MissingRef> {
    state
        .media
        .items
        .iter()
        .filter(|item| !std::path::Path::new(&item.path).exists())
        .map(|item| {
            let clip_count = state
                .clips
                .iter()
                .filter(|c| clip_references(c, item))
                .count();
            MissingRef {
                media_id: item.id,
                name: item.name.clone(),
                old_path: item.path.clone(),
                clip_count,
            }
        })
        .collect()
}

/// Does `clip` reference `item`? Primary: media_id equality.
/// Fallback: media_id.is_none() and inline path equals item.path.
fn clip_references(clip: &Clip, item: &MediaItem) -> bool {
    if let Some(mid) = clip.media_id {
        return mid == item.id;
    }
    match &clip.clip_type {
        ClipType::Video { path, .. }
        | ClipType::Audio { path, .. }
        | ClipType::Image { path, .. } => path == &item.path,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::MediaKind;

    fn inline_path(c: &Clip) -> &str {
        match &c.clip_type {
            ClipType::Video { path, .. }
            | ClipType::Audio { path, .. }
            | ClipType::Image { path, .. } => path.as_str(),
            _ => panic!("not a media clip"),
        }
    }

    fn make_video_clip(path: &str, media_id: Option<Uuid>) -> Clip {
        let mut c = Clip::new_video(path, 0, 0, 1000);
        c.media_id = media_id;
        c
    }

    fn make_media(path: &str) -> MediaItem {
        MediaItem::new(path, MediaKind::Video, 1)
    }

    #[test]
    fn relink_many_updates_media_and_clips() {
        let mut state = ProjectState::default();
        let a = make_media("C:/old/a.mp4");
        let b = make_media("C:/old/b.mp4");
        state.media.items.push(a.clone());
        state.media.items.push(b.clone());
        state
            .clips
            .push(make_video_clip("C:/old/a.mp4", Some(a.id)));
        state
            .clips
            .push(make_video_clip("C:/old/b.mp4", Some(b.id)));
        state
            .clips
            .push(make_video_clip("C:/old/b.mp4", Some(b.id)));

        let mut cmd = RelinkManyCommand::new(vec![
            RelinkMapping {
                media_id: a.id,
                old_path: "C:/old/a.mp4".into(),
                new_path: "D:/new/a.mp4".into(),
            },
            RelinkMapping {
                media_id: b.id,
                old_path: "C:/old/b.mp4".into(),
                new_path: "D:/new/b.mp4".into(),
            },
        ]);
        cmd.execute(&mut state).unwrap();

        assert_eq!(state.media.items[0].path, "D:/new/a.mp4");
        assert_eq!(state.media.items[1].path, "D:/new/b.mp4");
        assert_eq!(inline_path(&state.clips[0]), "D:/new/a.mp4");
        assert_eq!(inline_path(&state.clips[1]), "D:/new/b.mp4");
        assert_eq!(inline_path(&state.clips[2]), "D:/new/b.mp4");

        cmd.undo(&mut state).unwrap();

        assert_eq!(state.media.items[0].path, "C:/old/a.mp4");
        assert_eq!(state.media.items[1].path, "C:/old/b.mp4");
        assert_eq!(inline_path(&state.clips[0]), "C:/old/a.mp4");
        assert_eq!(inline_path(&state.clips[1]), "C:/old/b.mp4");
        assert_eq!(inline_path(&state.clips[2]), "C:/old/b.mp4");
    }

    #[test]
    fn relink_by_inline_path_when_media_id_missing() {
        let mut state = ProjectState::default();
        let a = make_media("C:/old/a.mp4");
        state.media.items.push(a.clone());
        state.clips.push(make_video_clip("C:/old/a.mp4", None));

        let mut cmd = RelinkManyCommand::new(vec![RelinkMapping {
            media_id: a.id,
            old_path: "C:/old/a.mp4".into(),
            new_path: "D:/new/a.mp4".into(),
        }]);
        cmd.execute(&mut state).unwrap();

        assert_eq!(state.media.items[0].path, "D:/new/a.mp4");
        assert_eq!(inline_path(&state.clips[0]), "D:/new/a.mp4");
    }

    #[test]
    fn execute_undo_execute_undo_is_idempotent() {
        let mut state = ProjectState::default();
        let a = make_media("C:/old/a.mp4");
        state.media.items.push(a.clone());
        state
            .clips
            .push(make_video_clip("C:/old/a.mp4", Some(a.id)));

        let mut cmd = RelinkManyCommand::new(vec![RelinkMapping {
            media_id: a.id,
            old_path: "C:/old/a.mp4".into(),
            new_path: "D:/new/a.mp4".into(),
        }]);

        for _ in 0..2 {
            cmd.execute(&mut state).unwrap();
            assert_eq!(state.media.items[0].path, "D:/new/a.mp4");
            assert_eq!(inline_path(&state.clips[0]), "D:/new/a.mp4");
            cmd.undo(&mut state).unwrap();
            assert_eq!(state.media.items[0].path, "C:/old/a.mp4");
            assert_eq!(inline_path(&state.clips[0]), "C:/old/a.mp4");
        }
    }

    #[test]
    fn empty_mappings_is_noop() {
        let mut state = ProjectState::default();
        let a = make_media("C:/old/a.mp4");
        state.media.items.push(a.clone());
        state
            .clips
            .push(make_video_clip("C:/old/a.mp4", Some(a.id)));

        let mut cmd = RelinkManyCommand::new(vec![]);
        cmd.execute(&mut state).unwrap();
        assert_eq!(state.media.items[0].path, "C:/old/a.mp4");
        assert_eq!(inline_path(&state.clips[0]), "C:/old/a.mp4");
        cmd.undo(&mut state).unwrap();
        assert_eq!(state.media.items[0].path, "C:/old/a.mp4");
    }

    #[test]
    fn find_missing_reports_clip_count() {
        let mut state = ProjectState::default();
        // Path that definitely does not exist on Windows.
        let missing = make_media("C:/nope/caprust-missing-xyz.mp4");
        // Path that does exist -- crate root dir always exists at test time.
        let present = make_media(env!("CARGO_MANIFEST_DIR"));
        state.media.items.push(missing.clone());
        state.media.items.push(present.clone());
        state.clips.push(make_video_clip(
            "C:/nope/caprust-missing-xyz.mp4",
            Some(missing.id),
        ));
        state.clips.push(make_video_clip(
            "C:/nope/caprust-missing-xyz.mp4",
            Some(missing.id),
        ));
        state.clips.push(make_video_clip(
            env!("CARGO_MANIFEST_DIR"),
            Some(present.id),
        ));

        let refs = find_missing_media_items(&state);
        assert_eq!(refs.len(), 1, "only the missing item should be reported");
        assert_eq!(refs[0].media_id, missing.id);
        assert_eq!(refs[0].clip_count, 2);
    }
}
