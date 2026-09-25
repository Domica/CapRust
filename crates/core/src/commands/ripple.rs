//! Ripple insert: shifts clips right when a drop would overlap.

use crate::clip::Clip;
use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct RippleInsertCommand {
    new_clip: Clip,
    affected: Vec<(Uuid, u64)>,
}

impl RippleInsertCommand {
    pub fn new(clip: Clip) -> Self {
        Self {
            new_clip: clip,
            affected: Vec::new(),
        }
    }
}

impl Command for RippleInsertCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let ns = self.new_clip.start_time_ms;
        let nd = self.new_clip.duration_ms;
        let nt = self.new_clip.track_index;
        let end = ns + nd;

        let overlaps = state.clips.iter().any(|c| {
            c.track_index == nt && c.start_time_ms < end && ns < c.start_time_ms + c.duration_ms
        });

        if overlaps {
            // Is the insertion landing on a Video / Overlay track?
            // If so, the Audio / Captions / Text clips that overlap
            // the shifted video clips must follow them right by the
            // same amount. Otherwise only same-track clips move.
            let is_video_track = state
                .tracks
                .get(nt)
                .map(|t| {
                    matches!(
                        t.kind,
                        crate::track::TrackKind::Video | crate::track::TrackKind::Overlay
                    )
                })
                .unwrap_or(false);

            // Same-track shifts (all clips at/after ns on this track).
            let same_track: Vec<(Uuid, u64)> = state
                .clips
                .iter()
                .filter(|c| c.track_index == nt && c.start_time_ms >= ns)
                .map(|c| (c.id, c.start_time_ms))
                .collect();

            // Followers: any Audio / Captions / Text clip that
            // overlapped one of the same-track clips in the OLD
            // timeline. Dedup because a follower may overlap several.
            let mut followers: Vec<(Uuid, u64)> = Vec::new();
            if is_video_track {
                for vid in state.clips.iter() {
                    if vid.track_index != nt || vid.start_time_ms < ns {
                        continue;
                    }
                    let vs = vid.start_time_ms;
                    let ve = vid.start_time_ms + vid.duration_ms;
                    for c in state.clips.iter() {
                        if c.track_index == nt {
                            continue;
                        }
                        let kind = state.tracks.get(c.track_index).map(|t| t.kind);
                        if !matches!(
                            kind,
                            Some(
                                crate::track::TrackKind::Audio
                                    | crate::track::TrackKind::Captions
                                    | crate::track::TrackKind::Text
                            )
                        ) {
                            continue;
                        }
                        let cs = c.start_time_ms;
                        let ce = c.start_time_ms + c.duration_ms;
                        if cs.max(vs) < ce.min(ve) {
                            followers.push((c.id, c.start_time_ms));
                        }
                    }
                }
                followers.sort_by_key(|(id, _)| *id);
                followers.dedup_by_key(|(id, _)| *id);
            }

            self.affected.extend(same_track.iter().cloned());
            self.affected.extend(followers.iter().cloned());

            let shifted_ids: std::collections::HashSet<Uuid> = same_track
                .iter()
                .chain(followers.iter())
                .map(|(id, _)| *id)
                .collect();

            for clip in state.clips.iter_mut() {
                if shifted_ids.contains(&clip.id) {
                    clip.start_time_ms += nd;
                }
            }
        }

        state.add_clip(self.new_clip.clone());
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        for (id, orig_start) in &self.affected {
            if let Some(c) = state.clips.iter_mut().find(|c| &c.id == id) {
                c.start_time_ms = *orig_start;
            }
        }
        state.remove_clip(self.new_clip.id);
        Ok(())
    }

    fn description(&self) -> String {
        let label = match &self.new_clip.clip_type {
            crate::clip::ClipType::TextOverlay { content, .. } => content.clone(),
            _ => "clip".to_string(),
        };
        format!(
            "Ripple insert '{}' at {}ms",
            label, self.new_clip.start_time_ms
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ripple_shifts_overlapping_clips() {
        let mut state = ProjectState::default();
        state.add_clip(Clip::new_text("A", 0, 1000, 2000, false));
        state.add_clip(Clip::new_text("B", 0, 3000, 1000, false));

        let new_clip = Clip::new_text("X", 0, 2000, 1500, false);
        let mut cmd = RippleInsertCommand::new(new_clip);
        cmd.execute(&mut state).unwrap();

        let b = state
            .clips
            .iter()
            .find(|c| matches!(&c.clip_type, crate::clip::ClipType::TextOverlay { content, .. } if content == "B"))
            .unwrap();
        assert_eq!(b.start_time_ms, 4500);
    }

    #[test]
    fn no_ripple_when_no_overlap() {
        let mut state = ProjectState::default();
        state.add_clip(Clip::new_text("A", 0, 0, 1000, false));
        let new_clip = Clip::new_text("X", 0, 2000, 1000, false);
        let mut cmd = RippleInsertCommand::new(new_clip);
        cmd.execute(&mut state).unwrap();
        let a = state
            .clips
            .iter()
            .find(|c| matches!(&c.clip_type, crate::clip::ClipType::TextOverlay { content, .. } if content == "A"))
            .unwrap();
        assert_eq!(a.start_time_ms, 0);
    }

    #[test]
    fn ripple_insert_shifts_followers_of_video_clips() {
        use crate::clip::Clip;
        use crate::track::{Track, TrackKind};

        let mut state = ProjectState::default();
        // Drop the default tracks so indices 0/1/2 are exactly
        // Video / Audio / Captions. Default tracks are
        // [Overlay, V1, V2, A1, Captions] — all Video-kind except the
        // last two, so the follower detection below needs a clean
        // slate to see Audio and Captions as follower kinds.
        state.tracks.clear();
        state.clips.clear();
        state.tracks.push(Track::new("V1", TrackKind::Video));
        state.tracks.push(Track::new("A1", TrackKind::Audio));
        state
            .tracks
            .push(Track::new("Captions", TrackKind::Captions));

        // Video B on V1 (index 0), starts at 3000.
        let mut b = Clip::new_text("B", 0, 3000, 2000, false);
        b.track_index = 0;
        state.add_clip(b.clone());

        // Audio follower of B, at 3000-5000 on A1 (index 1).
        let mut audio = Clip::new_text("aB", 1, 3000, 2000, false);
        audio.track_index = 1;
        state.add_clip(audio.clone());

        // Captions follower of B, at 3000-5000 on Captions (index 2).
        let mut cap = Clip::new_text("capB", 2, 3000, 2000, false);
        cap.track_index = 2;
        state.add_clip(cap.clone());

        // Insert a new video X on V1 at 2500, duration 500. This
        // overlaps B (2500-3000 vs 3000+) — actually no, X is
        // 2500..3000, B starts at 3000. So no overlap by our test.
        // Move X to overlap B: X at 3000, dur 500.
        let mut x = Clip::new_text("X", 0, 3000, 500, false);
        x.track_index = 0;
        let mut cmd = RippleInsertCommand::new(x);
        cmd.execute(&mut state).unwrap();

        let b2 = state.clips.iter().find(|c| c.id == b.id).unwrap();
        let a2 = state.clips.iter().find(|c| c.id == audio.id).unwrap();
        let c2 = state.clips.iter().find(|c| c.id == cap.id).unwrap();

        assert_eq!(b2.start_time_ms, 3500, "video shifted right by 500");
        assert_eq!(a2.start_time_ms, 3500, "audio followed its video");
        assert_eq!(c2.start_time_ms, 3500, "captions followed its video");
    }

    #[test]
    fn undo_restores_original_positions() {
        let mut state = ProjectState::default();
        state.add_clip(Clip::new_text("A", 0, 1000, 2000, false));
        state.add_clip(Clip::new_text("B", 0, 3000, 1000, false));

        let new_clip = Clip::new_text("X", 0, 2000, 1500, false);
        let mut cmd = RippleInsertCommand::new(new_clip);
        cmd.execute(&mut state).unwrap();
        cmd.undo(&mut state).unwrap();

        let b = state
            .clips
            .iter()
            .find(|c| matches!(&c.clip_type, crate::clip::ClipType::TextOverlay { content, .. } if content == "B"))
            .unwrap();
        assert_eq!(b.start_time_ms, 3000);
        assert_eq!(state.clips.len(), 2);
    }
}
