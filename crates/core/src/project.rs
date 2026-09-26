//! Project state — serializable.

use crate::aspect_ratio::AspectRatio;
use crate::clip::Clip;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectState {
    pub name: String,
    pub clips: Vec<Clip>,
    pub aspect_ratio: AspectRatio,
    pub base_resolution: u32,
    pub export_settings: ExportSettings,
    pub locale: String,
    pub frame_rate: crate::frame_rate::FrameRate,
    pub project_path: Option<String>,
    pub media: crate::media::MediaLibrary,
    pub models: crate::models::ModelRegistry,
    pub tracks: Vec<crate::track::Track>,
}

impl Default for ProjectState {
    fn default() -> Self {
        Self {
            name: "Untitled".into(),
            clips: Vec::new(),
            aspect_ratio: AspectRatio::default(),
            base_resolution: 1080,
            export_settings: ExportSettings::default(),
            locale: sys_locale::get_locale()
                .map(|l| l.split('-').next().unwrap_or("en").to_string())
                .unwrap_or_else(|| "en".into()),
            frame_rate: crate::frame_rate::FrameRate::default(),
            project_path: None,
            media: crate::media::MediaLibrary::default(),
            models: crate::models::ModelRegistry::default(),
            tracks: crate::track::default_tracks(),
        }
    }
}

impl ProjectState {
    pub fn add_clip(&mut self, clip: Clip) {
        self.clips.push(clip);
    }

    pub fn remove_clip(&mut self, id: uuid::Uuid) -> Option<Clip> {
        self.clips
            .iter()
            .position(|c| c.id == id)
            .map(|pos| self.clips.remove(pos))
    }

    pub fn project_dimensions(&self) -> (u32, u32) {
        self.aspect_ratio.dimensions(self.base_resolution)
    }

    /// Hash every clip/track field that influences the rendered
    /// filtergraph. Used by the UI to detect when the preview renderer
    /// must be respawned so the user sees effect / transition / speed /
    /// volume edits without a manual seek (Phase K, K1).
    ///
    /// Deliberately over-broad: fields like `clip.id` and `media_id`
    /// are included even though they do not change the graph, because
    /// a missed field would silently leave the preview stale -- far
    /// worse than one extra respawn. `name` is excluded because a
    /// rename is a pure label change and respawning on it would look
    /// like a glitch.
    ///
    /// Cost: one DefaultHasher pass over every clip. At ~20 clips this
    /// is well under 100 microseconds, negligible at UI frame rates.
    pub fn render_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();

        self.clips.len().hash(&mut h);
        for c in &self.clips {
            c.id.hash(&mut h);
            c.track_index.hash(&mut h);
            c.start_time_ms.hash(&mut h);
            c.duration_ms.hash(&mut h);
            format!("{:?}", c.clip_type).hash(&mut h);
            c.speed.to_bits().hash(&mut h);
            c.reversed.hash(&mut h);
            c.flip_h.hash(&mut h);
            c.flip_v.hash(&mut h);
            c.volume_db.to_bits().hash(&mut h);
            c.source_duration_ms.hash(&mut h);
            c.audio_detached.hash(&mut h);
            c.fade_in_ms.hash(&mut h);
            c.fade_out_ms.hash(&mut h);
            c.effects.len().hash(&mut h);
            for e in &c.effects {
                e.effect_id.hash(&mut h);
                e.amount.to_bits().hash(&mut h);
                e.enabled.hash(&mut h);
            }
            c.transition_in.hash(&mut h);
            c.transition_out.hash(&mut h);
            c.duck_against.hash(&mut h);
            c.speed_end.map(|s| s.to_bits()).hash(&mut h);
            format!("{:?}", c.speed_ease).hash(&mut h);
            format!("{:?}", c.speed_range).hash(&mut h);
            c.volume_keyframes.len().hash(&mut h);
            for k in &c.volume_keyframes {
                k.t_ms.hash(&mut h);
                k.gain_db.to_bits().hash(&mut h);
            }
            c.auto_reframe.len().hash(&mut h);
            for k in &c.auto_reframe {
                k.t_ms.hash(&mut h);
                k.cx_norm.to_bits().hash(&mut h);
                k.cy_norm.to_bits().hash(&mut h);
            }
            c.bg_removal.hash(&mut h);
            // NOTE: c.name and c.media_id intentionally excluded.
        }

        self.tracks.len().hash(&mut h);
        for t in &self.tracks {
            // Hash only the fields that affect the render. Using
            // Debug on the whole Track would pull in `Track.id` (a
            // fresh UUID on every ProjectState::default()) and make
            // the hash non-deterministic across otherwise identical
            // projects.
            t.kind.hash(&mut h);
            t.muted.hash(&mut h);
            t.visible.hash(&mut h);
            t.pinned.hash(&mut h);
            // t.locked is UI-only (blocks clip drag); t.name, t.id
            // and t.height are render-neutral. Excluded so a rename
            // or a lock toggle does not trigger a preview respawn.
        }

        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{Clip, EffectInstance};

    #[test]
    fn render_hash_stable_for_identical_projects() {
        let a = ProjectState::default();
        let b = ProjectState::default();
        assert_eq!(a.render_hash(), b.render_hash());
    }

    #[test]
    fn render_hash_changes_on_effect_amount() {
        let mut p = ProjectState::default();
        let mut clip = Clip::new_video("test.mp4", 0, 0, 1000);
        let id = clip.id;
        clip.effects.push(EffectInstance {
            effect_id: "blur".into(),
            amount: 1.0,
            enabled: true,
        });
        p.add_clip(clip);

        let h1 = p.render_hash();
        if let Some(c) = p.clips.iter_mut().find(|c| c.id == id) {
            c.effects[0].amount = 2.0;
        }
        let h2 = p.render_hash();
        assert_ne!(h1, h2, "amount change must invalidate the render hash");
    }

    #[test]
    fn render_hash_changes_on_reframe_keypoint_edit() {
        use crate::clip::ReframeKeypoint;
        let mut p = ProjectState::default();
        let clip = Clip::new_video("test.mp4", 0, 0, 1000);
        let id = clip.id;
        p.add_clip(clip);

        let h1 = p.render_hash();
        if let Some(c) = p.clips.iter_mut().find(|c| c.id == id) {
            c.auto_reframe = vec![
                ReframeKeypoint {
                    t_ms: 0,
                    cx_norm: 0.3,
                    cy_norm: 0.5,
                },
                ReframeKeypoint {
                    t_ms: 1000,
                    cx_norm: 0.7,
                    cy_norm: 0.5,
                },
            ];
        }
        let h2 = p.render_hash();
        assert_ne!(h1, h2, "reframe keypoints must invalidate the render hash");
    }

    #[test]
    fn render_hash_ignores_name_change() {
        let mut p = ProjectState::default();
        let mut clip = Clip::new_video("test.mp4", 0, 0, 1000);
        let id = clip.id;
        clip.name = Some("original".into());
        p.add_clip(clip);

        let h1 = p.render_hash();
        if let Some(c) = p.clips.iter_mut().find(|c| c.id == id) {
            c.name = Some("renamed".into());
        }
        let h2 = p.render_hash();
        assert_eq!(h1, h2, "rename must not force a preview respawn");
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportSettings {
    pub resolution_index: usize,
    pub rate_index: usize,
    pub quality_index: usize,
    pub codec_index: usize,
    pub ten_bit: bool,
    pub advanced: bool,
    pub rate_mode_index: usize,
    pub bitrate_kbps: u32,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            resolution_index: 2,
            rate_index: 3,
            quality_index: 1,
            codec_index: 0,
            ten_bit: false,
            advanced: false,
            rate_mode_index: 0,
            bitrate_kbps: 8000,
        }
    }
}
