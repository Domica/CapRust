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
            format!("{:?}", c.transition_in_easing).hash(&mut h);
            format!("{:?}", c.transition_out_easing).hash(&mut h);
            c.transition_duration_ms.hash(&mut h);
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

    /// Hash every field that affects the audio mix. Subset of
    /// `render_hash` minus video-only fields. Used by the pre-rendered
    /// audio PCM cache to invalidate without forcing a video respawn.
    ///
    /// Included: clip timing, clip_type Debug (source path),
    /// speed/reverse, volume_db, source_duration_ms, audio_detached,
    /// fades, transitions, duck_against, speed ramp params,
    /// volume_keyframes, track kind + muted.
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
