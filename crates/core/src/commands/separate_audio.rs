//! Detach a video clip's embedded audio onto an Audio track.
//!
//! Used by the timeline context menu "Separate audio". The video keeps
//! playing, but `audio_detached = true` tells `plan_from_project` not
//! to harvest its embedded audio into the mix. A new Audio clip with
//! the same source, position, and duration becomes the only audio
//! source for that span.
//!
//! The new clip lands on the first existing Audio track (or a freshly
//! created A1 if none exists), at the same `start_time_ms` as the
//! video. Magnetic / ripple follower logic detects it as a follower of
//! the video by overlap, so it moves with the video on later pack or
//! ripple operations.

use crate::clip::{Clip, ClipType};
use crate::commands::Command;
use crate::project::ProjectState;
use crate::track::{Track, TrackKind};
use anyhow::Result;
use uuid::Uuid;

pub struct SeparateAudioCommand {
    video_id: Uuid,
    /// Id of the new Audio clip, filled on execute.
    new_audio_id: Option<Uuid>,
    /// Previous value of video.audio_detached, for undo.
    previous_detached: Option<bool>,
    /// Index of the audio track used, filled on execute.
    audio_track_index: Option<usize>,
    /// True when execute created a new Audio track, so undo can drop it.
    created_track: bool,
}

impl SeparateAudioCommand {
    pub fn new(video_id: Uuid) -> Self {
        Self {
            video_id,
            new_audio_id: None,
            previous_detached: None,
            audio_track_index: None,
            created_track: false,
        }
    }
}

impl Command for SeparateAudioCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        // 1. Locate and validate the source video clip.
        let Some(video) = state.clips.iter().find(|c| c.id == self.video_id) else {
            return Ok(());
        };
        if video.audio_detached {
            // Already separated — nothing to do.
            return Ok(());
        }
        let ClipType::Video { path, .. } = &video.clip_type else {
            return Ok(());
        };
        let path = path.clone();
        let start_ms = video.start_time_ms;
        let dur_ms = video.duration_ms;
        let source_duration_ms = video.source_duration_ms;
        let media_id = video.media_id;
        let volume_db = video.volume_db;

        // 2. Pick an Audio track, or create one.
        let audio_track_index = match state.tracks.iter().position(|t| t.kind == TrackKind::Audio) {
            Some(i) => i,
            None => {
                let idx = state.tracks.len();
                let track_num = idx + 1;
                let name = format!("A{track_num}");
                state.tracks.push(Track::new(&name, TrackKind::Audio));
                self.created_track = true;
                idx
            }
        };

        // 3. Create the Audio clip.
        let mut audio = Clip::new_audio(&path, audio_track_index, start_ms, dur_ms);
        audio.source_duration_ms = source_duration_ms;
        audio.media_id = media_id;
        audio.volume_db = volume_db;
        let audio_id = audio.id;

        // 4. Mark the video as detached.
        let Some(video_mut) = state.clips.iter_mut().find(|c| c.id == self.video_id) else {
            return Ok(());
        };
        self.previous_detached = Some(video_mut.audio_detached);
        video_mut.audio_detached = true;

        // 5. Insert the Audio clip.
        state.add_clip(audio);

        self.audio_track_index = Some(audio_track_index);
        self.new_audio_id = Some(audio_id);
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        // Remove the audio clip we created.
        if let Some(aid) = self.new_audio_id.take() {
            state.remove_clip(aid);
        }
        // Restore audio_detached on the video.
        if let Some(prev) = self.previous_detached.take() {
            if let Some(video) = state.clips.iter_mut().find(|c| c.id == self.video_id) {
                video.audio_detached = prev;
            }
        }
        // Drop the track we auto-created.
        if self.created_track {
            if let Some(idx) = self.audio_track_index.take() {
                if idx < state.tracks.len() {
                    state.tracks.remove(idx);
                    // Shift any clips above that index down by one. The
                    // only clip on the removed track is our own, already
                    // removed above.
                    for c in state.clips.iter_mut() {
                        if c.track_index > idx {
                            c.track_index -= 1;
                        }
                    }
                }
            }
            self.created_track = false;
        }
        Ok(())
    }

    fn description(&self) -> String {
        "Separate audio".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::{Track, TrackKind};

    #[test]
    fn separate_audio_moves_sound_to_audio_track() {
        let mut state = ProjectState::default();
        state.tracks.clear();
        state.clips.clear();
        state.tracks.push(Track::new("V1", TrackKind::Video));

        let mut video = Clip::new_video("clip.mp4", 0, 5000, 3000);
        video.source_duration_ms = 3000;
        let vid = video.id;
        state.add_clip(video);

        let mut cmd = SeparateAudioCommand::new(vid);
        cmd.execute(&mut state).unwrap();

        assert_eq!(state.tracks.len(), 2, "A1 auto-created");
        let audio = state
            .clips
            .iter()
            .find(|c| matches!(c.clip_type, ClipType::Audio { .. }))
            .unwrap();
        assert_eq!(audio.track_index, 1);
        assert_eq!(audio.start_time_ms, 5000);
        assert_eq!(audio.duration_ms, 3000);

        let v = state.clips.iter().find(|c| c.id == vid).unwrap();
        assert!(v.audio_detached);
    }

    #[test]
    fn undo_restores_original_state() {
        let mut state = ProjectState::default();
        state.tracks.clear();
        state.clips.clear();
        state.tracks.push(Track::new("V1", TrackKind::Video));

        let video = Clip::new_video("clip.mp4", 0, 0, 1000);
        let vid = video.id;
        state.add_clip(video);

        let mut cmd = SeparateAudioCommand::new(vid);
        cmd.execute(&mut state).unwrap();
        cmd.undo(&mut state).unwrap();

        assert_eq!(state.clips.len(), 1);
        assert_eq!(state.tracks.len(), 1, "auto-created track removed");
        let v = state.clips.iter().find(|c| c.id == vid).unwrap();
        assert!(!v.audio_detached);
    }

    #[test]
    fn separate_uses_existing_audio_track() {
        let mut state = ProjectState::default();
        state.tracks.clear();
        state.clips.clear();
        state.tracks.push(Track::new("V1", TrackKind::Video));
        state.tracks.push(Track::new("A1", TrackKind::Audio));

        let video = Clip::new_video("clip.mp4", 0, 0, 1000);
        let vid = video.id;
        state.add_clip(video);

        let mut cmd = SeparateAudioCommand::new(vid);
        cmd.execute(&mut state).unwrap();

        assert_eq!(state.tracks.len(), 2, "no new track created");
        let audio = state
            .clips
            .iter()
            .find(|c| matches!(c.clip_type, ClipType::Audio { .. }))
            .unwrap();
        assert_eq!(audio.track_index, 1);
    }
}
