//! Reattach a separated audio clip back into its source video.
//!
//! Symmetric to SeparateAudioCommand. Given a video whose
//! `audio_detached` is true, this deletes the detached Audio clip
//! (via `detached_audio_clip_id`) and flips `audio_detached` back to
//! false. If the audio clip was already deleted by the user, the
//! back-reference is None and we only flip the flag.

use crate::clip::Clip;
use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct ReattachAudioCommand {
    video_id: Uuid,
    /// Snapshot of the video clip for undo (audio_detached +
    /// detached_audio_clip_id).
    video_before: Option<(bool, Option<Uuid>)>,
    /// Snapshot of the removed audio clip, for undo.
    removed_audio: Option<Clip>,
}

impl ReattachAudioCommand {
    pub fn new(video_id: Uuid) -> Self {
        Self {
            video_id,
            video_before: None,
            removed_audio: None,
        }
    }
}

impl Command for ReattachAudioCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(video) = state.clips.iter().find(|c| c.id == self.video_id).cloned() else {
            return Ok(());
        };
        self.video_before = Some((video.audio_detached, video.detached_audio_clip_id));

        // Remove the detached audio clip if it still exists. If the
        // back-reference points at a clip that the user already
        // deleted, we just clear the flag and move on.
        if let Some(audio_id) = video.detached_audio_clip_id {
            self.removed_audio = state.remove_clip(audio_id);
        }

        if let Some(v) = state.clips.iter_mut().find(|c| c.id == self.video_id) {
            v.audio_detached = false;
            v.detached_audio_clip_id = None;
        }
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some((detached, back_ref)) = self.video_before.take() {
            if let Some(v) = state.clips.iter_mut().find(|c| c.id == self.video_id) {
                v.audio_detached = detached;
                v.detached_audio_clip_id = back_ref;
            }
        }
        if let Some(clip) = self.removed_audio.take() {
            state.add_clip(clip);
        }
        Ok(())
    }

    fn description(&self) -> String {
        "Reattach audio".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::Clip;
    use crate::commands::separate_audio::SeparateAudioCommand;
    use crate::track::{Track, TrackKind};

    fn prep() -> (ProjectState, Uuid) {
        let mut s = ProjectState::default();
        s.tracks.clear();
        s.clips.clear();
        s.tracks.push(Track::new("V1", TrackKind::Video));
        let v = Clip::new_video("clip.mp4", 0, 0, 1000);
        let id = v.id;
        s.add_clip(v);
        (s, id)
    }

    #[test]
    fn reattach_removes_audio_clip_and_clears_flags() {
        let (mut s, vid) = prep();
        SeparateAudioCommand::new(vid).execute(&mut s).unwrap();
        assert_eq!(s.clips.len(), 2);

        let mut cmd = ReattachAudioCommand::new(vid);
        cmd.execute(&mut s).unwrap();

        assert_eq!(s.clips.len(), 1, "audio clip removed");
        let v = s.clips.iter().find(|c| c.id == vid).unwrap();
        assert!(!v.audio_detached);
        assert_eq!(v.detached_audio_clip_id, None);
    }

    #[test]
    fn reattach_undo_restores_audio_clip_and_flags() {
        let (mut s, vid) = prep();
        SeparateAudioCommand::new(vid).execute(&mut s).unwrap();
        let audio_id = s.clips.iter().find(|c| c.id != vid).map(|c| c.id).unwrap();

        let mut cmd = ReattachAudioCommand::new(vid);
        cmd.execute(&mut s).unwrap();
        cmd.undo(&mut s).unwrap();

        assert_eq!(s.clips.len(), 2, "audio restored");
        assert!(s.clips.iter().any(|c| c.id == audio_id));
        let v = s.clips.iter().find(|c| c.id == vid).unwrap();
        assert!(v.audio_detached);
        assert_eq!(v.detached_audio_clip_id, Some(audio_id));
    }

    #[test]
    fn reattach_when_audio_already_deleted_just_flips_flag() {
        let (mut s, vid) = prep();
        SeparateAudioCommand::new(vid).execute(&mut s).unwrap();
        // Delete the audio clip out from under the video.
        let audio_id = s.clips.iter().find(|c| c.id != vid).map(|c| c.id).unwrap();
        s.remove_clip(audio_id);

        let mut cmd = ReattachAudioCommand::new(vid);
        cmd.execute(&mut s).unwrap();

        assert_eq!(s.clips.len(), 1);
        let v = s.clips.iter().find(|c| c.id == vid).unwrap();
        assert!(!v.audio_detached);
        assert_eq!(v.detached_audio_clip_id, None);
    }

    #[test]
    fn reattach_on_missing_video_is_noop() {
        let mut s = ProjectState::default();
        let mut cmd = ReattachAudioCommand::new(Uuid::new_v4());
        cmd.execute(&mut s).unwrap();
        assert!(s.clips.is_empty());
    }
}
