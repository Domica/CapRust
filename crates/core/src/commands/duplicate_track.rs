//! Duplicate a track together with all of its clips.
//!
//! Used by the track-header context menu. Only Video, Audio, and Text
//! tracks can be duplicated — Overlay and Captions are singletons per
//! DIRECTIVES §7.1 and duplicating them would break the layout.
//!
//! The new track is appended to `tracks`. Clips on the source track are
//! copied 1:1 (same start, duration, effects, transitions) but get new
//! UUIDs, so the two tracks render independently afterwards.

use crate::clip::Clip;
use crate::commands::Command;
use crate::project::ProjectState;
use crate::track::{Track, TrackKind};
use anyhow::Result;
use uuid::Uuid;

pub struct DuplicateTrackCommand {
    source_index: usize,
    /// Filled on execute.
    new_track_index: Option<usize>,
    new_clip_ids: Vec<Uuid>,
}

impl DuplicateTrackCommand {
    pub fn new(source_index: usize) -> Self {
        Self {
            source_index,
            new_track_index: None,
            new_clip_ids: Vec::new(),
        }
    }
}

fn next_name(state: &ProjectState, kind: TrackKind) -> String {
    let letter = match kind {
        TrackKind::Video => "V",
        TrackKind::Audio => "A",
        TrackKind::Text => "T",
        TrackKind::Overlay => "Overlay",
        TrackKind::Captions => "Captions",
    };
    if matches!(kind, TrackKind::Overlay | TrackKind::Captions) {
        return letter.to_string();
    }
    let mut n = 1;
    loop {
        let candidate = format!("{letter}{n}");
        if !state.tracks.iter().any(|t| t.name == candidate) {
            return candidate;
        }
        n += 1;
    }
}

impl Command for DuplicateTrackCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(src) = state.tracks.get(self.source_index).cloned() else {
            return Ok(());
        };
        if matches!(src.kind, TrackKind::Overlay | TrackKind::Captions) {
            // Singleton kinds — refuse silently.
            return Ok(());
        }

        let name = next_name(state, src.kind);
        let new_track = Track::new(&name, src.kind);
        let new_index = state.tracks.len();
        state.tracks.push(new_track);
        self.new_track_index = Some(new_index);

        // Copy clips on the source track.
        let copies: Vec<Clip> = state
            .clips
            .iter()
            .filter(|c| c.track_index == self.source_index)
            .map(|c| {
                let mut copy = c.clone();
                copy.id = Uuid::new_v4();
                copy.track_index = new_index;
                copy
            })
            .collect();

        for c in copies {
            self.new_clip_ids.push(c.id);
            state.add_clip(c);
        }
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        for id in self.new_clip_ids.drain(..) {
            state.remove_clip(id);
        }
        if let Some(idx) = self.new_track_index.take() {
            if idx < state.tracks.len() {
                state.tracks.remove(idx);
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Duplicate track {}", self.source_index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicates_video_track_with_clips() {
        let mut state = ProjectState::default();
        state.tracks.clear();
        state.clips.clear();
        state.tracks.push(Track::new("V1", TrackKind::Video));
        state.add_clip(Clip::new_video("a.mp4", 0, 0, 1000));
        state.add_clip(Clip::new_video("b.mp4", 0, 2000, 1000));

        let mut cmd = DuplicateTrackCommand::new(0);
        cmd.execute(&mut state).unwrap();

        assert_eq!(state.tracks.len(), 2);
        assert_eq!(state.tracks[1].kind, TrackKind::Video);
        assert_eq!(state.clips.len(), 4);
        let new_clips: Vec<&Clip> = state.clips.iter().filter(|c| c.track_index == 1).collect();
        assert_eq!(new_clips.len(), 2);
    }

    #[test]
    fn undo_removes_track_and_clips() {
        let mut state = ProjectState::default();
        state.tracks.clear();
        state.clips.clear();
        state.tracks.push(Track::new("V1", TrackKind::Video));
        state.add_clip(Clip::new_video("a.mp4", 0, 0, 1000));

        let mut cmd = DuplicateTrackCommand::new(0);
        cmd.execute(&mut state).unwrap();
        cmd.undo(&mut state).unwrap();

        assert_eq!(state.tracks.len(), 1);
        assert_eq!(state.clips.len(), 1);
    }

    #[test]
    fn refuses_singleton_kinds() {
        let mut state = ProjectState::default();
        state.tracks.clear();
        state.clips.clear();
        state
            .tracks
            .push(Track::new("Captions", TrackKind::Captions));

        let mut cmd = DuplicateTrackCommand::new(0);
        cmd.execute(&mut state).unwrap();
        assert_eq!(state.tracks.len(), 1, "captions must not duplicate");
    }
}
