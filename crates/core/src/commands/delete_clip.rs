//! Delete a clip, optionally ripple-shifting later clips left.

use crate::clip::Clip;
use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct DeleteClipCommand {
    pub clip_id: Uuid,
    pub ripple: bool,
    /// Filled on execute — used by undo.
    removed: Option<Clip>,
    /// (clip_id, original_start_ms) for affected clips when rippling.
    affected: Vec<(Uuid, u64)>,
}

impl DeleteClipCommand {
    pub fn new(clip_id: Uuid, ripple: bool) -> Self {
        Self {
            clip_id,
            ripple,
            removed: None,
            affected: Vec::new(),
        }
    }
}

impl Command for DeleteClipCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        self.affected.clear();
        let removed = state.remove_clip(self.clip_id);
        if let Some(removed) = removed {
            if self.ripple {
                let gap = removed.duration_ms;
                let after = removed.start_time_ms + removed.duration_ms;

                // Track kinds: what packed on its own (Video / Overlay)
                // and what follows it (Audio / Captions / Text).
                let is_video_track = |idx: usize| -> bool {
                    state
                        .tracks
                        .get(idx)
                        .map(|t| {
                            matches!(
                                t.kind,
                                crate::track::TrackKind::Video | crate::track::TrackKind::Overlay
                            )
                        })
                        .unwrap_or(false)
                };

                // If we're deleting a Video/Overlay clip, find the
                // follower clips (Audio/Captions/Text) that overlapped
                // it in the OLD timeline and pull them left by the same
                // gap. Otherwise only shift same-track clips.
                let mut follower_ids: Vec<uuid::Uuid> = Vec::new();
                if is_video_track(removed.track_index) {
                    let rs = removed.start_time_ms;
                    let re = removed.start_time_ms + removed.duration_ms;
                    for c in state.clips.iter() {
                        if is_video_track(c.track_index) {
                            continue;
                        }
                        let cs = c.start_time_ms;
                        let ce = c.start_time_ms + c.duration_ms;
                        if cs.max(rs) < ce.min(re) {
                            follower_ids.push(c.id);
                        }
                    }
                }

                for c in state.clips.iter_mut() {
                    let same_track_after =
                        c.track_index == removed.track_index && c.start_time_ms >= after;
                    let is_follower = follower_ids.contains(&c.id);
                    if same_track_after || is_follower {
                        self.affected.push((c.id, c.start_time_ms));
                        c.start_time_ms = c.start_time_ms.saturating_sub(gap);
                    }
                }
            }
            self.removed = Some(removed);
        }
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        for (id, orig) in &self.affected {
            if let Some(c) = state.clips.iter_mut().find(|c| &c.id == id) {
                c.start_time_ms = *orig;
            }
        }
        if let Some(clip) = self.removed.take() {
            state.add_clip(clip);
        }
        Ok(())
    }

    fn description(&self) -> String {
        if self.ripple {
            "Ripple delete".into()
        } else {
            "Delete clip".into()
        }
    }
}
