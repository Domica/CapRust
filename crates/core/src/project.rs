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
    /// Master-bus CLAP plugin chain (Faza I). Applied to the
    /// final mix, not per-clip. See DIRECTIVES 12 and 28.3.
    #[serde(default)]
    pub master_plugins: Vec<crate::plugin::PluginInstance>,
    /// Multi-camera groups (Faza Q). See crate::multicam.
    #[serde(default)]
    pub multicam_groups: Vec<crate::multicam::MultiCamGroup>,
    /// Beat grids per media id (beat-sync v1). Analysis metadata only,
    /// deliberately NOT part of render_hash.
    #[serde(default)]
    pub beat_grids: std::collections::HashMap<uuid::Uuid, crate::beat::BeatEntry>,
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
            master_plugins: Vec::new(),
            multicam_groups: Vec::new(),
            beat_grids: Default::default(),
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
    /// Fix xfade-related inconsistencies in a single pass. Idempotent.
    ///
    /// Rules (matches `SetTransitionCommand`'s adjacency math and the
    /// render planner's `ADJACENCY_TOL_SEC`):
    ///
    ///   1. Clip has an xfade `transition_in` but no shift, yet a
    ///      touching predecessor exists -> the transition was set
    ///      before the model tracked shifts (old file), or the clip
    ///      was dragged next to its predecessor after the fact.
    ///      Compute the shift, move the clip and every later clip on
    ///      the same track left by that amount, record it.
    ///   2. Clip has an xfade `transition_in` but no touching
    ///      predecessor -> the predecessor was deleted or moved to
    ///      another track. Clear `transition_in` and any stale shift.
    ///   3. Clip has a nonzero shift but `transition_in` is not an
    ///      xfade -> clear the shift marker.
    ///
    /// Called ONLY on project load (migration for older `.caprust`
    /// files). Deliberately NOT called from move / delete commands:
    /// the pass touches every clip on every track, so calling it on
    /// an edit would silently mutate clips the user did not touch,
    /// and those mutations would not be captured by the edit's undo
    /// snapshot (so undo would restore positions but leave cleared
    /// transitions cleared).
    pub fn normalize_xfade_shifts(&mut self) {
        use crate::clip::is_xfade_transition;

        // Must match media-io::export_graph::XFADE_DUR_SEC.
        const XFADE_DUR_SEC: f64 = 0.5;
        // Must match commands::set_effect::ADJACENCY_TOL_MS.
        const ADJACENCY_TOL_MS: u64 = 200;

        // Pass 1: a clip without an xfade transition_in must not carry
        // a nonzero shift. This catches stale markers left over from a
        // transition that was cleared without updating the shift.
        for c in self.clips.iter_mut() {
            let is_xfade = c
                .transition_in
                .as_deref()
                .map(is_xfade_transition)
                .unwrap_or(false);
            if !is_xfade && c.applied_xfade_shift_ms != 0 {
                c.applied_xfade_shift_ms = 0;
            }
        }

        // Group clip indices per track.
        let mut by_track: std::collections::BTreeMap<usize, Vec<usize>> =
            std::collections::BTreeMap::new();
        for (i, c) in self.clips.iter().enumerate() {
            by_track.entry(c.track_index).or_default().push(i);
        }

        for idxs in by_track.values() {
            // Sort by start_time, ties broken by id for determinism.
            let mut sorted = idxs.clone();
            sorted.sort_by_key(|&i| (self.clips[i].start_time_ms, self.clips[i].id));

            // Snapshot original positions BEFORE any movement so the
            // shift math uses the pre-normalization start time of
            // each clip, not a value already moved by an earlier
            // iteration on this track (which would compound shifts
            // across a chain).
            #[allow(unused)]
            let original_start: Vec<u64> = self.clips.iter().map(|c| c.start_time_ms).collect();

            // Pass 2a: the first clip on a track has no predecessor.
            // Any xfade transition_in it carries is orphaned.
            if let Some(&first_i) = sorted.first() {
                let is_xfade = self.clips[first_i]
                    .transition_in
                    .as_deref()
                    .map(is_xfade_transition)
                    .unwrap_or(false);
                if is_xfade {
                    self.clips[first_i].transition_in = None;
                    self.clips[first_i].applied_xfade_shift_ms = 0;
                }
            }

            // Pass 2b: fix or clear each pair.
            for k in 1..sorted.len() {
                let prev_i = sorted[k - 1];
                let cur_i = sorted[k];

                let is_xfade = self.clips[cur_i]
                    .transition_in
                    .as_deref()
                    .map(is_xfade_transition)
                    .unwrap_or(false);
                if !is_xfade {
                    continue;
                }

                let prev_start = self.clips[prev_i].start_time_ms;
                let prev_end = prev_start + self.clips[prev_i].duration_ms;
                let cur_start = self.clips[cur_i].start_time_ms;

                // An xfade clip must OVERLAP its predecessor -- that is
                // the whole point of it, and once the shift has been
                // applied (which is the normal state of any project
                // saved after a transition was set) `cur_start` sits
                // *before* `prev_end`, not after it.
                //
                // The old test here was `prev_end <= unshifted_start`,
                // which only ever held for the pre-migration state where
                // the clips were butt-joined and no shift was recorded.
                // Any project saved after the user arranged a
                // transition failed that test on reload, so the
                // transition was cleared and the overlap thrown away --
                // the clips sprang back apart exactly as the user
                // described.
                //
                // So: adjacent means touching OR overlapping, and not
                // nested so far inside the predecessor that the fade has
                // nothing to cross.
                let overlaps_or_touches = cur_start >= prev_start.saturating_sub(ADJACENCY_TOL_MS)
                    && cur_start <= prev_end.saturating_add(ADJACENCY_TOL_MS);

                if !overlaps_or_touches {
                    self.clips[cur_i].transition_in = None;
                    self.clips[cur_i].applied_xfade_shift_ms = 0;
                    continue;
                }

                let requested = self.clips[cur_i].transition_duration_ms as f64 / 1000.0;
                let prev_dur = self.clips[prev_i].duration_ms as f64 / 1000.0;
                let cur_dur = self.clips[cur_i].duration_ms as f64 / 1000.0;
                let d = requested
                    .max(XFADE_DUR_SEC)
                    .min(prev_dur * 0.5)
                    .min(cur_dur * 0.5)
                    .max(0.05);
                let target_shift = (d * 1000.0).round() as u64;

                // Where the clip belongs: sitting back from the end of its predecessor
                // by exactly the transition length. Anchoring to
                // `prev_end` rather than to the clip's own recorded start
                // is what makes this pass idempotent. The old version
                // subtracted the shift from `original_start`, which
                // already had the shift baked in, so every reload moved
                // the clip a little further left -- and since the
                // predecessor was indexed by `sorted`, which was built
                // from the already-shifted positions, a chain crept
                // further on every open.
                let desired_start = prev_end.saturating_sub(target_shift);
                let shift_needed = self.clips[cur_i].start_time_ms as i64 - desired_start as i64;

                if shift_needed != 0 {
                    for &j in &sorted[k..] {
                        let new_ms =
                            (self.clips[j].start_time_ms as i64 - shift_needed).max(0) as u64;
                        self.clips[j].start_time_ms = new_ms;
                    }
                }
                self.clips[cur_i].applied_xfade_shift_ms = target_shift;
            }
        }
    }

    /// Re-link clips whose `media_id` points at a media item that
    /// no longer exists in the media library. Matching is by the
    /// clip's source `path` against `media.items[].path`. Returns
    /// the number of clips that were fixed.
    ///
    /// The common scenario: the media library was re-imported (or
    /// re-scanned on a different machine) and the media items got
    /// fresh UUIDs. Timeline clips still reference the old ids,
    /// so thumbnails / waveforms / probes look up cache files by
    /// an id that no longer exists and silently fail.
    ///
    /// Runs on project load, before regen/backfill jobs, so those
    /// passes see the corrected references.
    /// Returns `(relinked, created)`.
    pub fn relink_orphan_media_refs(&mut self) -> (usize, usize) {
        use std::collections::{HashMap, HashSet};
        let existing: HashSet<uuid::Uuid> = self.media.items.iter().map(|m| m.id).collect();
        let by_path: HashMap<String, uuid::Uuid> = self
            .media
            .items
            .iter()
            .map(|m| (m.path.clone(), m.id))
            .collect();
        let mut fixed = 0usize;
        let mut created = 0usize;
        for clip in &mut self.clips {
            let Some(mid) = clip.media_id else { continue };
            if existing.contains(&mid) {
                continue;
            }
            let src = match &clip.clip_type {
                crate::clip::ClipType::Video { path, .. }
                | crate::clip::ClipType::Audio { path, .. }
                | crate::clip::ClipType::Image { path, .. } => Some(path.as_str()),
                _ => None,
            };
            let Some(src) = src else { continue };
            if let Some(&new_id) = by_path.get(src) {
                clip.media_id = Some(new_id);
                fixed += 1;
                continue;
            }
            let kind = match &clip.clip_type {
                crate::clip::ClipType::Video { .. } => crate::media::MediaKind::Video,
                crate::clip::ClipType::Audio { .. } => crate::media::MediaKind::Audio,
                crate::clip::ClipType::Image { .. } => crate::media::MediaKind::Image,
                _ => continue,
            };
            let new_id = self.media.add(src, kind);
            clip.media_id = Some(new_id);
            created += 1;
            fixed += 1;
        }
        (fixed, created)
    }

    pub fn render_hash(&self) -> u64 {
        self.hash_with(true)
    }

    /// Same as `render_hash`, minus each sticker's placement.
    ///
    /// This is what decides whether the preview needs re-running. The
    /// preview draws stickers itself -- the editor composites the PNG
    /// over the frame, so a drag is instant and independent of the
    /// preview's quality setting -- which means a placement change does
    /// not change a single pixel of the ffmpeg output. Hashing it here
    /// would spawn an encoder on every frame of a drag to produce the
    /// exact same picture.
    ///
    /// Adding or removing a sticker still changes this: the clip list is
    /// hashed either way, and whether a clip is an overlay is part of how
    /// the preview routes it.
    pub fn preview_hash(&self) -> u64 {
        self.hash_with(false)
    }

    /// A copy with every sticker clip taken out.
    ///
    /// The preview panel composites stickers itself -- it has the PNG, the
    /// geometry and the rotation, and painting one costs the renderer
    /// nothing while changing the filtergraph costs an encoder restart.
    /// So the preview's ffmpeg graph leaves them out.
    ///
    /// Tracks are left alone even when they end up empty, because
    /// `track_index` on the surviving clips refers to positions in the
    /// original list. Renumbering would have to rewrite every clip, and
    /// an empty track renders nothing anyway.
    pub fn without_overlay_clips(&self) -> Self {
        let mut p = self.clone();
        p.clips.retain(|c| c.overlay.is_none());
        p
    }

    fn hash_with(&self, include_placement: bool) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();

        self.clips.len().hash(&mut h);
        self.multicam_groups.len().hash(&mut h);
        for g in &self.multicam_groups {
            g.id.hash(&mut h);
            g.active_angle.hash(&mut h);
            g.angle_clip_ids.len().hash(&mut h);
            for id in &g.angle_clip_ids {
                id.hash(&mut h);
            }
            g.sync_offsets_ms.hash(&mut h);
        }
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
            c.source_offset_ms.hash(&mut h);
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
            format!("{:?}", c.transition_in_easing).hash(&mut h);
            format!("{:?}", c.transition_out_easing).hash(&mut h);
            c.transition_duration_ms.hash(&mut h);
            c.duck_against.hash(&mut h);
            c.duck_reduction_db.to_bits().hash(&mut h);
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
            c.stab_trf.hash(&mut h);
            // The placement is what decides where a sticker lands and how
            // big it is, so a drag that changes it has to reach the
            // preview. Without this the marker is moved in the model and
            // nothing on screen follows.
            // Whether a clip is an overlay changes how the preview
            // routes it, so that part is always hashed. Only the
            // placement itself is preview-invisible.
            match &c.overlay {
                None => 0u8.hash(&mut h),
                Some(o) => {
                    1u8.hash(&mut h);
                    if include_placement {
                        o.scale.to_bits().hash(&mut h);
                        o.x.to_bits().hash(&mut h);
                        o.y.to_bits().hash(&mut h);
                        o.aspect.to_bits().hash(&mut h);
                        o.rotation.to_bits().hash(&mut h);
                        o.animation.hash(&mut h);
                    }
                }
            }
            match &c.chroma_key {
                None => 0u8.hash(&mut h),
                Some(ck) => {
                    1u8.hash(&mut h);
                    ck.key_color.hash(&mut h);
                    ck.similarity.to_bits().hash(&mut h);
                    ck.blend.to_bits().hash(&mut h);
                    ck.spill_suppression.hash(&mut h);
                }
            }
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

        // Master-bus CLAP chain (Faza I). Applied to the final mix,
        // so any change forces a preview respawn. Instance id is
        // excluded (fresh on every add); plugin_id + bypassed +
        // params define the actual sound.
        self.master_plugins.len().hash(&mut h);
        for p in &self.master_plugins {
            p.plugin_id.hash(&mut h);
            p.bypassed.hash(&mut h);
            p.params.len().hash(&mut h);
            for (id, v) in &p.params {
                id.hash(&mut h);
                v.to_bits().hash(&mut h);
            }
        }

        h.finish()
    }

    /// Hash every field that affects the audio mix. Subset of
    /// `render_hash` minus video-only fields. Used by the pre-rendered
    /// audio PCM cache to invalidate without forcing a video respawn.
    ///
    /// Included: clip timing, clip_type Debug (source path),
    /// speed/reverse, volume_db, source_duration_ms, audio_detached,
    /// fades, transitions, duck_against, speed ramp params,
    /// volume_keyframes, track kind + muted, and every audio effect
    /// toggle.
    ///
    /// The effect flags used to be missing here, which meant toggling
    /// denoise / normalize / telephone / bass / echo / chipmunk /
    /// voice boost produced an unchanged hash. The preview audio cache
    /// keys off this hash, so the PCM was never re-rendered and the
    /// effect stayed silent in the preview while appearing in the
    /// export, which builds a fresh plan. Same for video clips and
    /// detached audio clips, not just audio-only ones.
    ///
    /// Excluded (video-only): flip_h/flip_v, effects, auto_reframe,
    /// bg_removal, text/caption styling, visible, pinned, locked,
    /// name, media_id.
    pub fn audio_render_hash(&self) -> u64 {
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
            c.volume_db.to_bits().hash(&mut h);
            c.source_duration_ms.hash(&mut h);
            c.audio_detached.hash(&mut h);
            c.fade_in_ms.hash(&mut h);
            c.fade_out_ms.hash(&mut h);
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
            // Audio effect toggles. These change what the rendered PCM
            // sounds like, so the cache must be invalidated when they
            // flip. Without them an effect is audible in the export
            // (fresh plan) and silent in the preview (stale cache).
            c.audio_normalize.hash(&mut h);
            c.audio_denoise.hash(&mut h);
            c.audio_voice_boost.hash(&mut h);
            c.audio_telephone.hash(&mut h);
            c.audio_bass.hash(&mut h);
            c.audio_echo.hash(&mut h);
            c.audio_chipmunk.hash(&mut h);
        }

        self.tracks.len().hash(&mut h);
        for t in &self.tracks {
            t.kind.hash(&mut h);
            t.muted.hash(&mut h);
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
    fn transition_duration_defaults_to_500ms_on_old_projects() {
        let json = r#"{
            "id": "00000000-0000-0000-0000-000000000001",
            "track_index": 0,
            "start_time_ms": 0,
            "duration_ms": 1000,
            "clip_type": { "Video": { "path": "a.mp4", "duration_ms": 1000 } },
            "speed": 1.0,
            "reversed": false,
            "flip_h": false,
            "flip_v": false,
            "volume_db": 0.0
        }"#;
        let c: Clip = serde_json::from_str(json).expect("legacy clip deserializes");
        assert_eq!(c.transition_duration_ms, 500);
    }

    #[test]
    fn render_hash_changes_on_transition_duration_edit() {
        let mut p = ProjectState::default();
        let clip = Clip::new_video("a.mp4", 0, 0, 1000);
        let id = clip.id;
        p.add_clip(clip);

        let h1 = p.render_hash();
        if let Some(c) = p.clips.iter_mut().find(|c| c.id == id) {
            c.transition_duration_ms = 1200;
        }
        let h2 = p.render_hash();
        assert_ne!(h1, h2, "duration change must invalidate the render hash");
    }

    #[test]
    fn transition_easing_defaults_to_linear_on_old_projects() {
        // A JSON blob without the new fields must deserialize with
        // EaseCurve::Linear on both sides.
        let json = r#"{
            "id": "00000000-0000-0000-0000-000000000001",
            "track_index": 0,
            "start_time_ms": 0,
            "duration_ms": 1000,
            "clip_type": { "Video": { "path": "a.mp4", "duration_ms": 1000 } },
            "speed": 1.0,
            "reversed": false,
            "flip_h": false,
            "flip_v": false,
            "volume_db": 0.0
        }"#;
        let c: Clip = serde_json::from_str(json).expect("legacy clip deserializes");
        assert_eq!(c.transition_in_easing, crate::clip::EaseCurve::Linear);
        assert_eq!(c.transition_out_easing, crate::clip::EaseCurve::Linear);
    }

    #[test]
    fn render_hash_changes_on_transition_easing_edit() {
        let mut p = ProjectState::default();
        let clip = Clip::new_video("a.mp4", 0, 0, 1000);
        let id = clip.id;
        p.add_clip(clip);

        let h1 = p.render_hash();
        if let Some(c) = p.clips.iter_mut().find(|c| c.id == id) {
            c.transition_in_easing = crate::clip::EaseCurve::EaseInOut;
        }
        let h2 = p.render_hash();
        assert_ne!(h1, h2, "easing change must invalidate the render hash");
    }

    #[test]
    fn render_hash_changes_on_master_plugin_add() {
        use crate::plugin::PluginInstance;
        use std::path::PathBuf;

        let a = ProjectState::default();
        let mut b = ProjectState::default();
        b.master_plugins.push(PluginInstance::new(
            "com.example.Reverb",
            PathBuf::from("/reverb.clap"),
            "Reverb",
        ));
        assert_ne!(a.render_hash(), b.render_hash());
    }

    #[test]
    fn render_hash_changes_on_master_plugin_param() {
        use crate::plugin::PluginInstance;
        use std::path::PathBuf;

        let mut a = ProjectState::default();
        a.master_plugins.push(PluginInstance::new(
            "com.example.Reverb",
            PathBuf::from("/reverb.clap"),
            "Reverb",
        ));
        let mut b = a.clone();
        b.master_plugins[0].set_param(1, 0.5);
        assert_ne!(a.render_hash(), b.render_hash());
    }

    #[test]
    fn render_hash_ignores_master_plugin_instance_id() {
        use crate::plugin::PluginInstance;
        use std::path::PathBuf;

        let mut a = ProjectState::default();
        let mut b = ProjectState::default();
        let mut pa = PluginInstance::new(
            "com.example.Reverb",
            PathBuf::from("/reverb.clap"),
            "Reverb",
        );
        let mut pb = PluginInstance::new(
            "com.example.Reverb",
            PathBuf::from("/reverb.clap"),
            "Reverb",
        );
        // Force same id so only id would differ if we hashed it
        pb.id = pa.id;
        pa.set_param(2, 0.7);
        pb.set_param(2, 0.7);
        a.master_plugins.push(pa);
        b.master_plugins.push(pb);
        assert_eq!(
            a.render_hash(),
            b.render_hash(),
            "instance id must not affect render_hash"
        );
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

    #[test]
    fn encoder_round_trips_through_index() {
        use crate::project::VideoEncoder;
        for v in [
            VideoEncoder::H264Cpu,
            VideoEncoder::H265Cpu,
            VideoEncoder::Av1Cpu,
            VideoEncoder::H264Nvenc,
            VideoEncoder::H265Nvenc,
            VideoEncoder::Av1Nvenc,
            VideoEncoder::H264Amf,
            VideoEncoder::H265Amf,
            VideoEncoder::Av1Amf,
        ] {
            assert_eq!(VideoEncoder::from_index(v.to_index()), v);
        }
    }

    #[test]
    fn encoder_from_index_defaults_to_h264_cpu() {
        use crate::project::VideoEncoder;
        assert_eq!(VideoEncoder::from_index(99), VideoEncoder::H264Cpu);
    }

    #[test]
    fn encoder_is_hardware_flags_correct() {
        use crate::project::VideoEncoder;
        assert!(!VideoEncoder::H264Cpu.is_hardware());
        assert!(!VideoEncoder::Av1Cpu.is_hardware());
        assert!(VideoEncoder::H264Nvenc.is_hardware());
        assert!(VideoEncoder::H265Amf.is_hardware());
    }

    #[test]
    fn encoder_ffmpeg_ids_are_distinct() {
        use crate::project::VideoEncoder;
        let ids: std::collections::HashSet<&str> = [
            VideoEncoder::H264Cpu,
            VideoEncoder::H265Cpu,
            VideoEncoder::Av1Cpu,
            VideoEncoder::H264Nvenc,
            VideoEncoder::H265Nvenc,
            VideoEncoder::Av1Nvenc,
            VideoEncoder::H264Amf,
            VideoEncoder::H265Amf,
            VideoEncoder::Av1Amf,
        ]
        .iter()
        .map(|v| v.ffmpeg_id())
        .collect();
        assert_eq!(ids.len(), 9);
    }

    #[test]
    fn audio_render_hash_stable_for_identical_projects() {
        let a = ProjectState::default();
        let b = ProjectState::default();
        assert_eq!(a.audio_render_hash(), b.audio_render_hash());
    }

    #[test]
    fn audio_render_hash_changes_on_volume_edit() {
        let mut p = ProjectState::default();
        let clip = Clip::new_video("a.mp4", 0, 0, 1000);
        let id = clip.id;
        p.add_clip(clip);

        let h1 = p.audio_render_hash();
        if let Some(c) = p.clips.iter_mut().find(|c| c.id == id) {
            c.volume_db = -6.0;
        }
        let h2 = p.audio_render_hash();
        assert_ne!(h1, h2, "volume change must invalidate the audio hash");
    }

    #[test]
    fn audio_render_hash_ignores_video_effect() {
        let mut p = ProjectState::default();
        let mut clip = Clip::new_video("a.mp4", 0, 0, 1000);
        clip.effects.push(EffectInstance {
            effect_id: "blur".into(),
            amount: 1.0,
            enabled: true,
        });
        let id = clip.id;
        p.add_clip(clip);

        let h1 = p.audio_render_hash();
        if let Some(c) = p.clips.iter_mut().find(|c| c.id == id) {
            c.effects[0].amount = 5.0;
        }
        let h2 = p.audio_render_hash();
        assert_eq!(h1, h2, "video effect must not invalidate the audio hash");
    }

    /// Toggling an audio effect has to invalidate the preview PCM
    /// cache. It did not, so the effect played in the export (which
    /// builds a fresh plan) and stayed silent in the preview (which
    /// reuses the cached file for an unchanged hash). Covered video
    /// clips, detached audio clips and audio-only clips alike, since
    /// they all land in the same audio chain.
    #[test]
    fn audio_render_hash_changes_on_every_audio_effect() {
        /// Name and the toggle to flip, so one test covers all seven.
        type EffectToggle = (&'static str, fn(&mut Clip));
        let toggles: Vec<EffectToggle> = vec![
            ("normalize", |c| c.audio_normalize = true),
            ("denoise", |c| c.audio_denoise = true),
            ("voice boost", |c| c.audio_voice_boost = true),
            ("telephone", |c| c.audio_telephone = true),
            ("bass", |c| c.audio_bass = true),
            ("echo", |c| c.audio_echo = true),
            ("chipmunk", |c| c.audio_chipmunk = true),
        ];
        for (name, toggle) in toggles {
            let mut p = ProjectState::default();
            let clip = Clip::new_video("a.mp4", 0, 0, 1000);
            let id = clip.id;
            p.add_clip(clip);

            let before = p.audio_render_hash();
            if let Some(c) = p.clips.iter_mut().find(|c| c.id == id) {
                toggle(c);
            }
            let after = p.audio_render_hash();
            assert_ne!(
                before, after,
                "audio effect `{name}` must invalidate the audio render hash"
            );
        }
    }

    #[test]
    fn audio_render_hash_ignores_flip() {
        let mut p = ProjectState::default();
        let clip = Clip::new_video("a.mp4", 0, 0, 1000);
        let id = clip.id;
        p.add_clip(clip);

        let h1 = p.audio_render_hash();
        if let Some(c) = p.clips.iter_mut().find(|c| c.id == id) {
            c.flip_h = true;
            c.flip_v = true;
        }
        let h2 = p.audio_render_hash();
        assert_eq!(h1, h2, "flip must not invalidate the audio hash");
    }
}

/// Video encoder choice for export.
///
/// `codec_index` (0=H264, 1=Hevc, 2=Av1) is the legacy on-disk field;
/// it now maps to the CPU variants so existing projects load
/// unchanged. New values 3..=8 select hardware encoders.
///
/// Detection is runtime-only: a variant that names an unavailable
/// encoder must not be offered in the UI. See `probe` for the
/// check and `from_index`/`to_index` for the serde bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEncoder {
    H264Cpu,
    H265Cpu,
    Av1Cpu,
    H264Nvenc,
    H265Nvenc,
    Av1Nvenc,
    H264Amf,
    H265Amf,
    Av1Amf,
}

impl VideoEncoder {
    pub fn from_index(i: usize) -> Self {
        match i {
            1 => Self::H265Cpu,
            2 => Self::Av1Cpu,
            3 => Self::H264Nvenc,
            4 => Self::H265Nvenc,
            5 => Self::Av1Nvenc,
            6 => Self::H264Amf,
            7 => Self::H265Amf,
            8 => Self::Av1Amf,
            _ => Self::H264Cpu,
        }
    }

    pub fn to_index(self) -> usize {
        match self {
            Self::H264Cpu => 0,
            Self::H265Cpu => 1,
            Self::Av1Cpu => 2,
            Self::H264Nvenc => 3,
            Self::H265Nvenc => 4,
            Self::Av1Nvenc => 5,
            Self::H264Amf => 6,
            Self::H265Amf => 7,
            Self::Av1Amf => 8,
        }
    }

    /// FTL key for the export dropdown. Callers pass this through
    /// `tr(...)`.
    pub fn ftl_key(self) -> &'static str {
        match self {
            Self::H264Cpu => "exp-codec-h264-cpu",
            Self::H265Cpu => "exp-codec-h265-cpu",
            Self::Av1Cpu => "exp-codec-av1-cpu",
            Self::H264Nvenc => "exp-codec-h264-nvenc",
            Self::H265Nvenc => "exp-codec-h265-nvenc",
            Self::Av1Nvenc => "exp-codec-av1-nvenc",
            Self::H264Amf => "exp-codec-h264-amf",
            Self::H265Amf => "exp-codec-h265-amf",
            Self::Av1Amf => "exp-codec-av1-amf",
        }
    }

    pub fn is_hardware(self) -> bool {
        !matches!(self, Self::H264Cpu | Self::H265Cpu | Self::Av1Cpu)
    }

    /// ffmpeg `-c:v` value.
    pub fn ffmpeg_id(self) -> &'static str {
        match self {
            Self::H264Cpu => "libx264",
            Self::H265Cpu => "libx265",
            Self::Av1Cpu => "libsvtav1",
            Self::H264Nvenc => "h264_nvenc",
            Self::H265Nvenc => "hevc_nvenc",
            Self::Av1Nvenc => "av1_nvenc",
            Self::H264Amf => "h264_amf",
            Self::H265Amf => "hevc_amf",
            Self::Av1Amf => "av1_amf",
        }
    }

    /// Quick runtime probe. Returns true if a 1-frame test encode at
    /// 320x240 succeeds. Slow on first call (~150 ms) but cheap
    /// enough to run once per export dialog open. Returns false on
    /// any error so the UI simply omits the codec.
    pub fn probe(self, ffmpeg: &std::path::Path) -> bool {
        use std::process::{Command, Stdio};
        let mut cmd = Command::new(ffmpeg);
        // Same window suppression as media-io's silent_command, which
        // this crate cannot call because media-io depends on core.
        // Without it the probe flashes a console window nine times as
        // the export dialog opens.
        #[cfg(all(windows, not(debug_assertions)))]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW = 0x08000000
            cmd.creation_flags(0x0800_0000);
        }
        cmd.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=1:size=320x240:rate=24",
            "-c:v",
            self.ffmpeg_id(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

        // Encoder-specific args so the probe exercises the same
        // parameter shape the real export will use.
        match self {
            Self::H264Nvenc | Self::H265Nvenc | Self::Av1Nvenc => {
                cmd.args(["-rc", "vbr", "-cq", "23", "-b:v", "0"]);
            }
            Self::H264Amf | Self::H265Amf | Self::Av1Amf => {
                cmd.args(["-quality", "balanced", "-rc", "cqp"]);
            }
            _ => {}
        }
        cmd.args(["-f", "null", "-"]);
        cmd.status().map(|s| s.success()).unwrap_or(false)
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

impl ExportSettings {
    /// Current encoder derived from the on-disk `codec_index`.
    pub fn encoder(&self) -> VideoEncoder {
        VideoEncoder::from_index(self.codec_index)
    }

    /// Set the encoder, keeping `codec_index` in sync so the field
    /// still round-trips through serde unchanged.
    pub fn set_encoder(&mut self, e: VideoEncoder) {
        self.codec_index = e.to_index();
    }
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

#[cfg(test)]
mod normalize_xfade_tests {
    use crate::clip::Clip;
    use crate::commands::set_effect::SetTransitionCommand;
    use crate::commands::Command;
    use crate::project::ProjectState;

    /// Clip with an xfade transition_in and a touching predecessor
    /// but no recorded shift (the pre-migration state) gets a shift
    /// applied and moved left, and its followers move with it.
    #[test]
    fn fills_in_missing_shift_for_touching_xfade() {
        let mut s = ProjectState::default();
        let a = Clip::new_video("a.mp4", 0, 0, 8000);
        let mut b = Clip::new_video("b.mp4", 0, 8000, 8000);
        let mut c = Clip::new_video("c.mp4", 0, 16000, 8000);
        b.transition_in = Some("fade".into());
        b.transition_duration_ms = 500;
        c.transition_in = Some("fade".into());
        c.transition_duration_ms = 500;
        s.clips.push(a);
        s.clips.push(b);
        s.clips.push(c);

        s.normalize_xfade_shifts();

        assert_eq!(s.clips[1].start_time_ms, 7500, "b shifted left by 500 ms");
        assert_eq!(s.clips[1].applied_xfade_shift_ms, 500);
        assert_eq!(s.clips[1].transition_in.as_deref(), Some("fade"));

        // b now ends at 15500, so c sits at 15000 and overlaps b by its
        // own 500 ms.
        //
        // This used to assert 15500, which was the bug rather than the
        // intent: c had only followed b's shift instead of taking its
        // own, and the old adjacency test then cleared c's transition
        // outright (it read as "not touching"). A third of a chain was
        // silently losing its fade on every project load. Asserted here
        // explicitly so the regression cannot come back quietly.
        assert_eq!(s.clips[2].start_time_ms, 15000, "c overlaps b by 500 ms");
        assert_eq!(s.clips[2].applied_xfade_shift_ms, 500);
        assert_eq!(
            s.clips[2].transition_in.as_deref(),
            Some("fade"),
            "every link in the chain keeps its own transition"
        );
    }

    /// Clip with xfade but no touching predecessor gets the transition
    /// cleared.
    #[test]
    fn clears_orphaned_xfade() {
        let mut s = ProjectState::default();
        let a = Clip::new_video("a.mp4", 0, 0, 8000);
        let mut b = Clip::new_video("b.mp4", 0, 20000, 8000);
        b.transition_in = Some("fade".into());
        s.clips.push(a);
        s.clips.push(b);

        s.normalize_xfade_shifts();

        assert_eq!(s.clips[1].transition_in, None);
        assert_eq!(s.clips[1].applied_xfade_shift_ms, 0);
        assert_eq!(s.clips[1].start_time_ms, 20000, "position unchanged");
    }

    /// Shift on a clip with no xfade transition_in gets cleared.
    #[test]
    fn clears_stale_shift_without_transition() {
        let mut s = ProjectState::default();
        let mut a = Clip::new_video("a.mp4", 0, 0, 8000);
        a.applied_xfade_shift_ms = 500;
        s.clips.push(a);

        s.normalize_xfade_shifts();
        assert_eq!(s.clips[0].applied_xfade_shift_ms, 0);
    }

    /// Running the pass twice produces the same state (idempotent).
    #[test]
    fn is_idempotent() {
        let mut s = ProjectState::default();
        let a = Clip::new_video("a.mp4", 0, 0, 8000);
        let mut b = Clip::new_video("b.mp4", 0, 8000, 8000);
        b.transition_in = Some("fade".into());
        s.clips.push(a);
        s.clips.push(b);

        s.normalize_xfade_shifts();
        let first: Vec<u64> = s.clips.iter().map(|c| c.start_time_ms).collect();
        s.normalize_xfade_shifts();
        let second: Vec<u64> = s.clips.iter().map(|c| c.start_time_ms).collect();
        assert_eq!(first, second);
    }

    /// The user's actual report: apply a transition, it overlaps
    /// correctly on screen, save, quit, reopen — and the transition is
    /// gone and the clips sit butt-joined again.
    ///
    /// Goes through the real command and a real JSON round trip, because
    /// the failure is not in the command (the shift is applied and looks
    /// right) but in what survives serialisation. `diag_bl` is the id the
    /// user named.
    #[test]
    fn applied_transition_survives_save_and_reload() {
        let mut s = ProjectState::default();
        let a = Clip::new_video("a.mp4", 0, 0, 8000);
        let b = Clip::new_video("b.mp4", 0, 8000, 8000);
        let c = Clip::new_video("c.mp4", 0, 16000, 8000);
        let b_id = b.id;
        s.clips.push(a);
        s.clips.push(b);
        s.clips.push(c);

        let mut cmd = SetTransitionCommand::new(b_id, true, Some("diag_bl".into()));
        cmd.execute(&mut s).expect("apply transition");

        // What the user sees right after applying: b pulled left so it
        // overlaps a, and the marker records how far.
        assert!(s.clips[1].start_time_ms < 8000, "b should overlap a");
        let shifted_start = s.clips[1].start_time_ms;
        let recorded = s.clips[1].applied_xfade_shift_ms;
        assert_eq!(recorded, 8000 - shifted_start, "marker matches the move");

        // Save and reopen, exactly as the app does it.
        let json = serde_json::to_string(&s).expect("serialize");
        let mut loaded: ProjectState = serde_json::from_str(&json).expect("deserialize");
        loaded.normalize_xfade_shifts();

        assert_eq!(
            loaded.clips[1].transition_in.as_deref(),
            Some("diag_bl"),
            "the transition must survive a reload"
        );
        assert_eq!(
            loaded.clips[1].start_time_ms, shifted_start,
            "and the overlap must be the same one the user arranged"
        );
        assert_eq!(loaded.clips[1].applied_xfade_shift_ms, recorded);
        // The follower was shifted with b and must stay with it, or the
        // gap opens up at the tail.
        assert_eq!(loaded.clips[2].start_time_ms, 16000 - recorded);
    }

    /// Reopening the same project must not move anything. This is the
    /// half of the bug that a single save/load assertion would miss: the
    /// old `desired_start` subtracted the shift from a position that
    /// already had the shift in it, so each open crept the clips further
    /// left. One round trip can look right; five cannot.
    #[test]
    fn repeated_reloads_do_not_creep() {
        let mut s = ProjectState::default();
        let a = Clip::new_video("a.mp4", 0, 0, 8000);
        let b = Clip::new_video("b.mp4", 0, 8000, 8000);
        let c = Clip::new_video("c.mp4", 0, 16000, 8000);
        let b_id = b.id;
        let c_id = c.id;
        s.clips.push(a);
        s.clips.push(b);
        s.clips.push(c);

        SetTransitionCommand::new(b_id, true, Some("diag_bl".into()))
            .execute(&mut s)
            .expect("apply b");

        let mut positions = Vec::new();
        for round in 0..5 {
            let json = serde_json::to_string(&s).expect("serialize");
            let mut loaded: ProjectState = serde_json::from_str(&json).expect("deserialize");
            loaded.normalize_xfade_shifts();

            let got: Vec<(u64, Option<String>, u64)> = loaded
                .clips
                .iter()
                .map(|c| {
                    (
                        c.start_time_ms,
                        c.transition_in.clone(),
                        c.applied_xfade_shift_ms,
                    )
                })
                .collect();
            positions.push(got);

            // Carry the loaded state forward so the next round re-saves
            // exactly what a user's file would contain.
            s = loaded;
            let _ = round;
        }

        for (i, w) in positions.windows(2).enumerate() {
            assert_eq!(w[0], w[1], "round {} moved the timeline", i + 1);
        }
        let final_state = positions.last().unwrap();
        assert_eq!(final_state[1].1.as_deref(), Some("diag_bl"));
        let _ = c_id;
    }

    /// Cross-track independence: a clip with an xfade whose predecessor
    /// is on a different track must be cleared, not accidentally
    /// considered touching.
    #[test]
    fn cross_track_predecessor_does_not_count() {
        let mut s = ProjectState::default();
        let a = Clip::new_video("a.mp4", 0, 0, 8000);
        let mut b = Clip::new_video("b.mp4", 1, 8000, 8000);
        b.transition_in = Some("fade".into());
        s.clips.push(a);
        s.clips.push(b);

        s.normalize_xfade_shifts();

        assert_eq!(s.clips[1].transition_in, None);
        assert_eq!(s.clips[1].applied_xfade_shift_ms, 0);
    }
}

#[cfg(test)]
mod chroma_key_hash_tests {
    use super::*;
    use crate::clip::{ChromaKeySpec, Clip};

    #[test]
    fn render_hash_changes_on_chroma_key_edit() {
        let mut p = ProjectState::default();
        let clip = Clip::new_video("test.mp4", 0, 0, 1000);
        let id = clip.id;
        p.add_clip(clip);
        let h1 = p.render_hash();
        if let Some(c) = p.clips.iter_mut().find(|c| c.id == id) {
            c.chroma_key = Some(ChromaKeySpec::default());
        }
        let h2 = p.render_hash();
        assert_ne!(h1, h2, "chroma_key must invalidate the render hash");
    }
}

#[cfg(test)]
mod multicam_hash_tests {
    use super::*;
    use crate::clip::Clip;
    use crate::commands::create_multicam_group::CreateMultiCamGroupCommand;
    use crate::commands::set_active_angle::SetActiveAngleCommand;
    use crate::commands::UndoStack;

    fn project_with_group() -> (ProjectState, uuid::Uuid) {
        let mut p = ProjectState::default();
        let a = Clip::new_video("a.mp4", 0, 0, 1000);
        let b = Clip::new_video("b.mp4", 1, 0, 1000);
        let aid = a.id;
        let bid = b.id;
        p.add_clip(a);
        p.add_clip(b);
        let cmd = CreateMultiCamGroupCommand::new("cam", vec![aid, bid]);
        let gid = cmd.group_id();
        let mut stack = UndoStack::default();
        stack.execute(Box::new(cmd), &mut p).unwrap();
        (p, gid)
    }

    #[test]
    fn hash_changes_on_active_angle_edit() {
        let (mut p, gid) = project_with_group();
        let h1 = p.render_hash();
        let mut stack = UndoStack::default();
        stack
            .execute(Box::new(SetActiveAngleCommand::new(gid, 1)), &mut p)
            .unwrap();
        let h2 = p.render_hash();
        assert_ne!(h1, h2, "active angle change must invalidate preview");
    }
}

#[cfg(test)]
mod relink_orphan_tests {
    use super::*;
    use crate::clip::Clip;
    use crate::media::{MediaItem, MediaKind};

    #[test]
    fn relinks_clip_by_path_when_media_id_is_stale() {
        let mut p = ProjectState::default();
        let path = "F:/x/clip.mp4";
        // Media item has one UUID...
        let item = MediaItem::new(path, MediaKind::Video, 0);
        let real_id = item.id;
        p.media.items.push(item);
        // ...but the clip points at a different (stale) one.
        let mut clip = Clip::new_video(path, 0, 0, 1000);
        clip.media_id = Some(uuid::Uuid::new_v4());
        let clip_id = clip.id;
        p.add_clip(clip);

        let (fixed, _created) = p.relink_orphan_media_refs();
        assert_eq!(fixed, 1);
        let c = p.clips.iter().find(|c| c.id == clip_id).unwrap();
        assert_eq!(c.media_id, Some(real_id));
    }

    #[test]
    fn leaves_valid_media_ids_alone() {
        let mut p = ProjectState::default();
        let path = "F:/x/clip.mp4";
        let item = MediaItem::new(path, MediaKind::Video, 0);
        let real_id = item.id;
        p.media.items.push(item);
        let mut clip = Clip::new_video(path, 0, 0, 1000);
        clip.media_id = Some(real_id);
        p.add_clip(clip);

        let (fixed, _created) = p.relink_orphan_media_refs();
        assert_eq!(fixed, 0);
    }

    #[test]
    fn creates_missing_media_item_from_clip_path() {
        let mut p = ProjectState::default();
        let mut clip = Clip::new_video("F:/missing.mp4", 0, 0, 1000);
        let ghost = uuid::Uuid::new_v4();
        clip.media_id = Some(ghost);
        p.add_clip(clip);

        let (fixed, created) = p.relink_orphan_media_refs();
        assert_eq!(fixed, 1);
        assert_eq!(created, 1);
        let new_id = p.clips[0].media_id.unwrap();
        assert_ne!(new_id, ghost);
        assert!(p.media.items.iter().any(|m| m.id == new_id));
    }

    #[test]
    fn is_idempotent_across_repeated_loads() {
        let mut p = ProjectState::default();
        let mut clip = Clip::new_video("F:/missing2.mp4", 0, 0, 1000);
        clip.media_id = Some(uuid::Uuid::new_v4());
        p.add_clip(clip);

        let (_, c1) = p.relink_orphan_media_refs();
        assert_eq!(c1, 1);
        let (fixed2, created2) = p.relink_orphan_media_refs();
        assert_eq!(fixed2, 0);
        assert_eq!(created2, 0);
    }

    /// Every field of a placement has to reach `render_hash`. The preview
    /// respawns off this hash, so a field left out is a drag that moves
    /// the model and nothing on screen -- which is exactly how the sticker
    /// handles read before this was noticed.
    #[test]
    fn render_hash_notices_a_sticker_being_moved() {
        use crate::clip::OverlayPlacement;

        let base = OverlayPlacement {
            scale: 0.34,
            x: 0.5,
            y: 0.72,
            aspect: 1.0,
            rotation: 0.0,
            animation: None,
        };

        let variants: [(&str, OverlayPlacement); 6] = [
            (
                "scale",
                OverlayPlacement {
                    scale: 0.5,
                    ..base.clone()
                },
            ),
            (
                "x",
                OverlayPlacement {
                    x: 0.8,
                    ..base.clone()
                },
            ),
            (
                "y",
                OverlayPlacement {
                    y: 0.1,
                    ..base.clone()
                },
            ),
            (
                "aspect",
                OverlayPlacement {
                    aspect: 2.0,
                    ..base.clone()
                },
            ),
            (
                "rotation",
                OverlayPlacement {
                    rotation: 45.0,
                    ..base.clone()
                },
            ),
            (
                "animation",
                OverlayPlacement {
                    animation: Some(String::from("spin")),
                    ..base.clone()
                },
            ),
        ];

        for (field, changed) in variants {
            let mut p = ProjectState::default();
            let mut clip = Clip::new_video("a.mp4", 0, 0, 4000);
            clip.overlay = Some(base.clone());
            p.clips.push(clip);

            let before = p.render_hash();
            p.clips[0].overlay = Some(changed);
            assert_ne!(
                before,
                p.render_hash(),
                "changing {field} did not change the render hash"
            );
        }
    }

    /// And no placement at all has to hash differently from a placement,
    /// so dropping a sticker into a project is seen as a render change
    /// rather than an invisible edit.
    #[test]
    fn render_hash_distinguishes_a_placement_from_none() {
        use crate::clip::OverlayPlacement;

        let mut p = ProjectState::default();
        let clip = Clip::new_video("a.mp4", 0, 0, 4000);
        p.clips.push(clip);
        let plain = p.render_hash();

        p.clips[0].overlay = Some(OverlayPlacement::default_sticker());
        assert_ne!(
            plain,
            p.render_hash(),
            "adding a sticker did not change the render hash"
        );
    }

    /// Moving a sticker must not restart the preview encoder: the preview
    /// paints stickers itself and no pixel of the ffmpeg output changes.
    /// Hashing placement here would spawn an encoder on every frame of a
    /// drag, which is the delay this split exists to remove.
    #[test]
    fn preview_hash_ignores_a_sticker_being_moved() {
        use crate::clip::{Clip, OverlayPlacement};

        let mut p = ProjectState::default();
        let mut clip = Clip::new_video("a.mp4", 0, 0, 4000);
        clip.overlay = Some(OverlayPlacement::default_sticker());
        p.clips.push(clip);

        let before = p.preview_hash();
        for change in [
            OverlayPlacement {
                scale: 0.9,
                ..OverlayPlacement::default_sticker()
            },
            OverlayPlacement {
                x: 0.1,
                ..OverlayPlacement::default_sticker()
            },
            OverlayPlacement {
                y: 0.9,
                ..OverlayPlacement::default_sticker()
            },
            OverlayPlacement {
                rotation: 45.0,
                ..OverlayPlacement::default_sticker()
            },
            OverlayPlacement {
                animation: Some(String::from("spin")),
                ..OverlayPlacement::default_sticker()
            },
        ] {
            p.clips[0].overlay = Some(change);
            assert_eq!(
                before,
                p.preview_hash(),
                "moving a sticker restarted the preview encoder"
            );
        }

        // But the clip itself appearing or going is still a change: the
        // preview routes on it, and the timeline has to redraw.
        p.clips.clear();
        assert_ne!(before, p.preview_hash());
    }

    /// The split is only useful if the two hashes really do differ, so
    /// pin that too rather than trusting that the omission happened.
    #[test]
    fn preview_hash_and_render_hash_disagree_on_placement() {
        use crate::clip::{Clip, OverlayPlacement};

        let mut p = ProjectState::default();
        let mut clip = Clip::new_video("a.mp4", 0, 0, 4000);
        clip.overlay = Some(OverlayPlacement::default_sticker());
        p.clips.push(clip);

        p.clips[0].overlay = Some(OverlayPlacement {
            x: 0.8,
            ..OverlayPlacement::default_sticker()
        });
        assert_ne!(
            p.preview_hash(),
            p.render_hash(),
            "the two hashes agree on a changed placement"
        );
    }

    /// Taking stickers out for the preview graph has to leave the
    /// survivors alone -- renumbering tracks would move every remaining
    /// clip onto the wrong track.
    #[test]
    fn without_overlay_clips_keeps_the_other_clips_where_they_were() {
        use crate::clip::{Clip, OverlayPlacement};
        use crate::track::{Track, TrackKind};

        let mut p = ProjectState::default();
        // Relative, not absolute: a default project may already ship
        // tracks and the assertion is about them surviving untouched.
        let tracks_before = p.tracks.len();
        p.tracks.push(Track::new("V1", TrackKind::Video));
        p.tracks.push(Track::new("V2", TrackKind::Video));
        p.tracks.push(Track::new("V3", TrackKind::Video));

        let mut a = Clip::new_video("a.mp4", 0, 0, 4000);
        a.id = uuid::Uuid::from_u128(1);
        a.track_index = 0;
        let mut b = Clip::new_video("b.mp4", 0, 4000, 4000);
        b.id = uuid::Uuid::from_u128(2);
        b.track_index = 2;
        let mut s = Clip::new_video("s.png", 0, 0, 4000);
        s.id = uuid::Uuid::from_u128(3);
        s.track_index = 1;
        s.overlay = Some(OverlayPlacement::default_sticker());
        p.clips = vec![a, b, s];

        let stripped = p.without_overlay_clips();
        assert_eq!(stripped.clips.len(), 2, "the sticker survived");
        for c in &stripped.clips {
            assert!(c.overlay.is_none());
            assert!(
                c.id != uuid::Uuid::from_u128(3),
                "a sticker is still in the preview graph"
            );
        }
        // The whole point: track indices are positions in the ORIGINAL
        // list, so they must not be rewritten.
        let b2 = stripped
            .clips
            .iter()
            .find(|c| c.id == uuid::Uuid::from_u128(2))
            .expect("b is missing");
        assert_eq!(b2.track_index, 2, "b moved to the wrong track");
        assert_eq!(
            stripped.tracks.len(),
            tracks_before + 3,
            "tracks were renumbered"
        );
        // And the original is untouched.
        assert_eq!(p.clips.len(), 3);
    }
}
