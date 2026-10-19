//! Insert a clip at a position, shifting only its own track.
//!
//! `RippleInsertCommand` shifts every audio, caption and text clip that
//! overlapped what it moved, which is right when a cut lands in the
//! middle of an edit and wrong for anything that is not editing the cut.
//! A sticker is laid on top of the frame at a moment; inserting it moved
//! the whole timeline sideways.
//!
//! This one shifts the clips after the insertion point on the same track
//! by exactly as much as it needs to, and touches nothing else.

use crate::clip::Clip;
use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct AddClipCommand {
    new_clip: Clip,
    /// (clip id, its start before the shift) for the clips this moved.
    affected: Vec<(Uuid, u64)>,
    created_id: Option<Uuid>,
}

impl AddClipCommand {
    pub fn new(clip: Clip) -> Self {
        Self {
            new_clip: clip,
            affected: Vec::new(),
            created_id: None,
        }
    }

    /// The id of the clip this created, readable after `execute`.
    pub fn created_id(&self) -> Option<Uuid> {
        self.created_id
    }
}

impl Command for AddClipCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let ns = self.new_clip.start_time_ms;
        let nd = self.new_clip.duration_ms;
        let nt = self.new_clip.track_index;
        let end = ns + nd;

        // How far the insertion reaches past what is already there. The
        // insert occupies [ns, end), so a clip has to move by however much
        // of that range overlaps it -- not by its own length, which left a
        // gap the size of the rest of the clip it displaced.
        let mut push = 0u64;
        for c in state.clips.iter() {
            if c.track_index != nt {
                continue;
            }
            if c.start_time_ms < end && ns < c.start_time_ms + c.duration_ms {
                push = push.max(end.saturating_sub(c.start_time_ms));
            }
        }
        if push == 0 {
            self.created_id = Some(self.new_clip.id);
            state.add_clip(self.new_clip.clone());
            return Ok(());
        }

        // Only the clips that actually overlap the insert move. Shifting
        // everything from `ns` onward pushed the rest of the timeline
        // along by the insert's length for no reason -- a clip already
        // starting after the insert's new end has room to stay put.
        let until = ns + push;
        self.affected = state
            .clips
            .iter()
            .filter(|c| c.track_index == nt && c.start_time_ms < until)
            .map(|c| (c.id, c.start_time_ms))
            .collect();
        for c in state.clips.iter_mut() {
            if c.track_index == nt && c.start_time_ms < until {
                c.start_time_ms += push;
            }
        }
        self.new_clip.start_time_ms = ns;
        self.created_id = Some(self.new_clip.id);
        state.add_clip(self.new_clip.clone());
        Ok(())
    }

    fn description(&self) -> String {
        match &self.new_clip.clip_type {
            crate::clip::ClipType::TextOverlay { content, .. } => content.clone(),
            _ => "clip".to_string(),
        }
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(id) = self.created_id {
            state.clips.retain(|c| c.id != id);
        }
        for (id, before) in std::mem::take(&mut self.affected) {
            if let Some(c) = state.clips.iter_mut().find(|c| c.id == id) {
                c.start_time_ms = before;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::{Track, TrackKind};

    fn project_with_two_clips() -> ProjectState {
        let mut p = ProjectState::default();
        p.tracks.push(Track::new("V1", TrackKind::Video));
        p.tracks.push(Track::new("A1", TrackKind::Audio));
        for (i, start) in [(0u64, 0u64), (1, 4000)] {
            let mut c = Clip::new_video("a.mp4", 0, start, 4000);
            c.id = Uuid::from_u128(10 + i as u128);
            p.clips.push(c);
        }
        let mut a = Clip::new_audio("a.mp3", 1, 0, 8000);
        a.id = Uuid::from_u128(20);
        p.clips.push(a);
        p
    }

    #[test]
    fn inserting_does_not_move_other_tracks() {
        let mut p = project_with_two_clips();
        let audio_before: Vec<u64> = p
            .clips
            .iter()
            .filter(|c| c.track_index == 1)
            .map(|c| c.start_time_ms)
            .collect();

        let mut sticker = Clip::new_image("s.png", 0, 0, 3000);
        sticker.id = Uuid::from_u128(30);
        let mut cmd = AddClipCommand::new(sticker);
        cmd.execute(&mut p).expect("insert");

        let audio_after: Vec<u64> = p
            .clips
            .iter()
            .filter(|c| c.track_index == 1)
            .map(|c| c.start_time_ms)
            .collect();
        assert_eq!(
            audio_before, audio_after,
            "an insert moved an audio clip, which is what the ripple did"
        );
    }

    #[test]
    fn same_track_shifts_right_only_as_far_as_it_must() {
        let mut p = project_with_two_clips();
        let mut sticker = Clip::new_image("s.png", 0, 0, 3000);
        sticker.id = Uuid::from_u128(30);
        let mut cmd = AddClipCommand::new(sticker);
        cmd.execute(&mut p).expect("insert");

        let at = |id: u128| {
            p.clips
                .iter()
                .find(|c| c.id == Uuid::from_u128(id))
                .map(|c| c.start_time_ms)
                .expect("clip present")
        };
        assert_eq!(p.clips.len(), 4);
        assert_eq!(at(30), 0, "the insert stays where it was asked for");
        // The clip at 4000 has to clear the new one, which ends at 3000.
        // Insert is [0, 3000). The clip already at 0 has to clear 3000,
        // so it moves to 3000 -- not to 4000, its own length later.
        assert_eq!(at(10), 3000, "the overlapping clip moves just clear");
        assert_eq!(at(11), 4000, "a clip after the insert is untouched");
        assert_eq!(at(20), 0, "the audio clip is not on this track");
    }

    #[test]
    fn undo_puts_everything_back() {
        let mut p = project_with_two_clips();
        let before: Vec<(Uuid, u64)> = p.clips.iter().map(|c| (c.id, c.start_time_ms)).collect();

        let mut sticker = Clip::new_image("s.png", 0, 0, 3000);
        sticker.id = Uuid::from_u128(30);
        let mut cmd = AddClipCommand::new(sticker);
        cmd.execute(&mut p).expect("insert");
        cmd.undo(&mut p).expect("undo");

        let after: Vec<(Uuid, u64)> = p.clips.iter().map(|c| (c.id, c.start_time_ms)).collect();
        assert_eq!(before, after);
    }
}
