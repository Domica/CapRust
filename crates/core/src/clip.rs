//! Clip model.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ClipType {
    Video {
        path: String,
        duration_ms: u64,
    },
    Audio {
        path: String,
        duration_ms: u64,
    },
    Image {
        path: String,
        duration_ms: u64,
    },
    TextOverlay {
        content: String,
        font_size: f32,
        above: bool,
        /// Preset id from the asset browser Text tab. Empty / "default"
        /// falls back to plain white text. Mapping to drawtext options
        /// lives in build_filtergraph.
        #[serde(default)]
        style: String,
        #[serde(default)]
        motion: TextMotion,
        #[serde(default)]
        effect: Option<TextEffect>,
    },
    /// AI-generated captions spanning the clip's duration.
    Captions {
        model_id: String,
        language: String,
        /// Filled in once the model has run.
        segments: Vec<CaptionSegment>,
        #[serde(default)]
        style: CaptionStyle,
    },
    /// Text-to-speech narration.
    Narration {
        model_id: String,
        voice_id: String,
        text: String,
    },
}

/// Visual style of a Captions clip. Applied to every drawtext the
/// render path expands from the segments. Defaults match the pre-
/// feature hardcoded look (yellow text, black outline, bottom of
/// the frame, size 32).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct CaptionStyle {
    #[serde(default = "default_caption_font_size")]
    pub font_size: f32,
    #[serde(default)]
    pub position: CaptionPosition,
    /// RGB in 0..=255.
    #[serde(default = "default_caption_color")]
    pub color: [u8; 3],
    #[serde(default = "default_caption_outline_color")]
    pub outline_color: [u8; 3],
    /// 0 = no outline.
    #[serde(default = "default_caption_outline_width")]
    pub outline_width: f32,
    #[serde(default)]
    pub bg_enabled: bool,
    #[serde(default = "default_caption_bg_opacity")]
    pub bg_opacity: f32,
}

fn default_caption_font_size() -> f32 {
    32.0
}
fn default_caption_color() -> [u8; 3] {
    [255, 255, 0]
}
fn default_caption_outline_color() -> [u8; 3] {
    [0, 0, 0]
}
fn default_caption_outline_width() -> f32 {
    2.0
}
fn default_caption_bg_opacity() -> f32 {
    0.5
}

impl Default for CaptionStyle {
    fn default() -> Self {
        Self {
            font_size: default_caption_font_size(),
            position: CaptionPosition::default(),
            color: default_caption_color(),
            outline_color: default_caption_outline_color(),
            outline_width: default_caption_outline_width(),
            bg_enabled: false,
            bg_opacity: default_caption_bg_opacity(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CaptionPosition {
    Top,
    Middle,
    #[default]
    Bottom,
}

/// One word inside a caption segment, with its own timing. Used by
/// the P1 progressive-reveal render path: the segment is expanded into
/// one drawtext per word, each showing the accumulated text up to that
/// word, so the caption "types itself" as the speaker talks.
///
/// Populated by whisper.rs when token timestamps are available. Older
/// projects (and any segment where token grouping produced nothing)
/// carry `words: vec![]` and fall back to the pre-P1 single-drawtext
/// render.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordTiming {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptionSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    /// Per-word timings. Empty = no word data, render as one drawtext
    /// for the whole segment (pre-P1 behaviour). Non-empty = render
    /// one drawtext per word in progressive-reveal mode.
    #[serde(default)]
    pub words: Vec<WordTiming>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EffectInstance {
    pub effect_id: String,
    #[serde(default = "default_effect_amount")]
    pub amount: f32,
    #[serde(default)]
    pub enabled: bool,
}

fn default_effect_amount() -> f32 {
    1.0
}

/// Easing applied to a speed ramp's interpolation between `speed` and
/// `speed_end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EaseCurve {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

/// Which portion of a clip the speed ramp occupies. The complementary
/// portion (if any) stays at `speed_end` (FirstN) or `speed` (LastN).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SpeedRampRange {
    /// The ramp spans the entire clip.
    #[default]
    WholeClip,
    /// The ramp occupies the first N milliseconds of the clip; the rest
    /// runs at `speed_end`.
    FirstN(u64),
    /// The ramp occupies the last N milliseconds of the clip; the
    /// earlier portion runs at `speed`.
    LastN(u64),
}

/// Text motion transform. Offsets are normalized fractions of the
/// frame: 0.0 = centered (pre-feature behaviour), -1.0 / +1.0 =
/// flush against the frame edge. Applied on top of the existing
/// above/below baseline, so Default renders identically to before.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct TextMotion {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    /// Degrees. Stored for forward-compat; v1 does NOT render
    /// rotation (ffmpeg drawtext has no rotate). See DIRECTIVES §30.
    #[serde(default)]
    pub rotation: f32,
    /// Multiplier on `font_size`. 1.0 = no scaling.
    #[serde(default = "default_motion_scale")]
    pub scale: f32,
}

fn default_motion_scale() -> f32 {
    1.0
}

/// Default duration of a video transition (fade in/out). 500 ms is
/// what CapCut uses out of the box and reads as a real transition
/// rather than a pop.
fn default_transition_duration_ms() -> u64 {
    500
}

/// The set of transition preset ids that render as an xfade between
/// two adjacent clips. These transitions overlap the two clips and
/// therefore shorten the run by their duration; the model shifts
/// the follower clip left when one is applied. Single-clip fade
/// edges (fade in/out) are not in this set: they apply to one clip
/// and never move their neighbours.
pub fn is_xfade_transition(id: &str) -> bool {
    matches!(
        id,
        "fade"
            | "slide_l"
            | "slide_r"
            | "slide_u"
            | "slide_d"
            | "wipe_l"
            | "wipe_r"
            | "zoom_in"
            | "zoom_out"
            | "rotate"
            | "blur_t"
    )
}

impl Default for TextMotion {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            scale: 1.0,
        }
    }
}

/// Procedural text effect. Period in seconds; amount scales the
/// amplitude for effects that have one (blink ignores it).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextEffectKind {
    /// Hard on/off every `period/2`.
    Blink,
    /// Alpha oscillates between `1-amount` and `1`.
    Pulse,
    /// Hue cycles through the RGB wheel, `period` seconds per turn.
    ColorCycle,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct TextEffect {
    pub kind: TextEffectKind,
    #[serde(default = "default_effect_period")]
    pub period: f32,
    #[serde(default = "default_effect_amount")]
    pub amount: f32,
}

fn default_effect_period() -> f32 {
    1.0
}

/// One point in a clip's volume automation curve. `t_ms` is relative
/// to the clip's own start (0 = clip head). `gain_db` is the target
/// gain at that instant. Between keyframes the renderer interpolates
/// linearly in the dB domain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VolumeKeyframe {
    pub t_ms: u64,
    pub gain_db: f32,
}

/// One auto-reframe keypoint: at time `t_ms` relative to the clip
/// start, the crop rectangle should be centered at
/// `(cx_norm, cy_norm)`, expressed as a fraction of the available
/// horizontal / vertical travel. 0 = crop flush against the left /
/// top edge, 1 = flush against the right / bottom edge, 0.5 = centered.
///
/// Populated by the P2c detector pass. Empty Vec = no reframe; the
/// render graph then uses the source frame as-is.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct ReframeKeypoint {
    pub t_ms: u64,
    pub cx_norm: f32,
    pub cy_norm: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    pub id: Uuid,
    pub track_index: usize,
    pub start_time_ms: u64,
    pub duration_ms: u64,
    pub clip_type: ClipType,
    pub speed: f32,
    pub reversed: bool,
    pub flip_h: bool,
    pub flip_v: bool,
    pub volume_db: f32,
    /// Original source length. 0 = unlimited (images, text).
    #[serde(default)]
    pub source_duration_ms: u64,
    /// Links back to the MediaItem this clip was created from, if any.
    #[serde(default)]
    pub media_id: Option<Uuid>,
    #[serde(default)]
    pub effects: Vec<EffectInstance>,
    #[serde(default)]
    pub transition_in: Option<String>,
    #[serde(default)]
    pub transition_out: Option<String>,
    /// Easing applied to the in transition's interpolation curve.
    /// Only meaningful for transitions that support easing (currently
    /// `fade`; slide/wipe/zoom still use ffmpeg's linear ramp).
    /// `Linear` matches the pre-feature behaviour, so old projects
    /// load unchanged.
    #[serde(default)]
    pub transition_in_easing: EaseCurve,
    /// Easing applied to the out transition. Same semantics as
    /// `transition_in_easing`.
    #[serde(default)]
    pub transition_out_easing: EaseCurve,
    /// Duration of the video fade in/out in milliseconds. Shared by
    /// the in and the out edge, matching how CapCut exposes a single
    /// slider. Ignored when neither `transition_in` nor
    /// `transition_out` is set.
    #[serde(default = "default_transition_duration_ms")]
    pub transition_duration_ms: u64,
    /// Horizontal shift that was applied to this clip when its
    /// `transition_in` xfade was attached. Zero means no shift is in
    /// effect. Used by `SetTransitionCommand` so changing the
    /// transition duration from D1 to D2 shifts the clip and every
    /// following clip on its track by (D1 - D2), and removing the
    /// transition shifts back by the stored value.
    #[serde(default)]
    pub applied_xfade_shift_ms: u64,
    /// True when the user has explicitly separated this video's audio
    /// onto an Audio track. The video continues to render, but its
    /// embedded audio is not harvested into the mix — the detached
    /// Audio clip is the sole audio source.
    #[serde(default)]
    pub audio_detached: bool,
    /// User-editable display name. None = derive from the source file
    /// name at render time.
    #[serde(default)]
    pub name: Option<String>,
    /// Fade-in duration in ms. Applied to the audio stream only (video
    /// uses transition_in for visual fades). 0 = no fade.
    #[serde(default)]
    pub fade_in_ms: u64,
    /// Fade-out duration in ms. Same semantics as fade_in_ms.
    #[serde(default)]
    pub fade_out_ms: u64,
    /// Volume automation curve. Empty = static `volume_db`. Non-empty
    /// = piecewise-linear in dB between sorted keyframes, held flat
    /// before the first and after the last.
    #[serde(default)]
    pub volume_keyframes: Vec<VolumeKeyframe>,
    /// Auto-ducking: id of the clip whose audio drives the sidechain
    /// (typically a narration or spoken-word clip). When set on an
    /// audio-bearing clip, the render sidechains this clip's audio
    /// against that clip's stream, so it drops in level whenever the
    /// control speaks. None = no ducking.
    #[serde(default)]
    pub duck_against: Option<Uuid>,
    /// Speed ramp end. When Some(x), the clip ramps from `speed` to `x`
    /// across the clip (or a sub-range, see speed_range). None = static
    /// speed (uses `speed`).
    #[serde(default)]
    pub speed_end: Option<f32>,
    /// Easing of the ramp between `speed` and `speed_end`.
    #[serde(default)]
    pub speed_ease: EaseCurve,
    /// Where the ramp lives inside the clip.
    #[serde(default)]
    pub speed_range: SpeedRampRange,
    /// Auto-reframe keypoints (Phase P2c). Empty = source is used
    /// as-is. Non-empty = the render graph crops the source to the
    /// project aspect ratio and pans the crop to follow these points.
    #[serde(default)]
    pub auto_reframe: Vec<ReframeKeypoint>,
    /// Path to the per-clip alpha mask (Phase P3c). Relative to the
    /// project directory, e.g. "cache/masks/<clip_id>.mkv". None =
    /// no background removal. Populated by the P3c job, consumed by
    /// the P3d render graph as an `alphamerge` input.
    #[serde(default)]
    pub bg_removal: Option<String>,
}

impl Clip {
    pub fn new_video(path: &str, track: usize, start_ms: u64, dur_ms: u64) -> Self {
        Self {
            id: Uuid::new_v4(),
            track_index: track,
            start_time_ms: start_ms,
            duration_ms: dur_ms,
            clip_type: ClipType::Video {
                path: path.to_string(),
                duration_ms: dur_ms,
            },
            speed: 1.0,
            reversed: false,
            flip_h: false,
            flip_v: false,
            volume_db: 0.0,
            effects: Vec::new(),
            transition_in: None,
            transition_out: None,
            transition_in_easing: EaseCurve::Linear,
            transition_out_easing: EaseCurve::Linear,
            transition_duration_ms: default_transition_duration_ms(),
            applied_xfade_shift_ms: 0,
            audio_detached: false,
            name: None,
            fade_in_ms: 0,
            fade_out_ms: 0,
            volume_keyframes: Vec::new(),
            duck_against: None,
            speed_end: None,
            speed_ease: EaseCurve::default(),
            speed_range: SpeedRampRange::default(),
            auto_reframe: Vec::new(),
            bg_removal: None,
            source_duration_ms: dur_ms,
            media_id: None,
        }
    }

    pub fn new_audio(path: &str, track: usize, start_ms: u64, dur_ms: u64) -> Self {
        Self {
            id: Uuid::new_v4(),
            track_index: track,
            start_time_ms: start_ms,
            duration_ms: dur_ms,
            clip_type: ClipType::Audio {
                path: path.to_string(),
                duration_ms: dur_ms,
            },
            speed: 1.0,
            reversed: false,
            flip_h: false,
            flip_v: false,
            volume_db: 0.0,
            effects: Vec::new(),
            transition_in: None,
            transition_out: None,
            transition_in_easing: EaseCurve::Linear,
            transition_out_easing: EaseCurve::Linear,
            transition_duration_ms: default_transition_duration_ms(),
            applied_xfade_shift_ms: 0,
            audio_detached: false,
            name: None,
            fade_in_ms: 0,
            fade_out_ms: 0,
            volume_keyframes: Vec::new(),
            duck_against: None,
            speed_end: None,
            speed_ease: EaseCurve::default(),
            speed_range: SpeedRampRange::default(),
            auto_reframe: Vec::new(),
            bg_removal: None,
            source_duration_ms: dur_ms,
            media_id: None,
        }
    }

    pub fn new_image(path: &str, track: usize, start_ms: u64, dur_ms: u64) -> Self {
        Self {
            id: Uuid::new_v4(),
            track_index: track,
            start_time_ms: start_ms,
            duration_ms: dur_ms,
            clip_type: ClipType::Image {
                path: path.to_string(),
                duration_ms: dur_ms,
            },
            speed: 1.0,
            reversed: false,
            flip_h: false,
            flip_v: false,
            volume_db: 0.0,
            effects: Vec::new(),
            transition_in: None,
            transition_out: None,
            transition_in_easing: EaseCurve::Linear,
            transition_out_easing: EaseCurve::Linear,
            transition_duration_ms: default_transition_duration_ms(),
            applied_xfade_shift_ms: 0,
            audio_detached: false,
            name: None,
            fade_in_ms: 0,
            fade_out_ms: 0,
            volume_keyframes: Vec::new(),
            duck_against: None,
            speed_end: None,
            speed_ease: EaseCurve::default(),
            speed_range: SpeedRampRange::default(),
            auto_reframe: Vec::new(),
            bg_removal: None,
            source_duration_ms: 0,
            media_id: None,
        }
    }

    pub fn new_text(content: &str, track: usize, start_ms: u64, dur_ms: u64, above: bool) -> Self {
        Self {
            id: Uuid::new_v4(),
            track_index: track,
            start_time_ms: start_ms,
            duration_ms: dur_ms,
            clip_type: ClipType::TextOverlay {
                content: content.to_string(),
                font_size: 24.0,
                above,
                style: "default".to_string(),
                motion: TextMotion::default(),
                effect: None,
            },
            speed: 1.0,
            reversed: false,
            flip_h: false,
            flip_v: false,
            volume_db: 0.0,
            effects: Vec::new(),
            transition_in: None,
            transition_out: None,
            transition_in_easing: EaseCurve::Linear,
            transition_out_easing: EaseCurve::Linear,
            transition_duration_ms: default_transition_duration_ms(),
            applied_xfade_shift_ms: 0,
            audio_detached: false,
            name: None,
            fade_in_ms: 0,
            fade_out_ms: 0,
            volume_keyframes: Vec::new(),
            duck_against: None,
            speed_end: None,
            speed_ease: EaseCurve::default(),
            speed_range: SpeedRampRange::default(),
            auto_reframe: Vec::new(),
            bg_removal: None,
            source_duration_ms: 0,
            media_id: None,
        }
    }

    pub fn new_captions(
        track: usize,
        start_ms: u64,
        dur_ms: u64,
        model_id: &str,
        language: &str,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            track_index: track,
            start_time_ms: start_ms,
            duration_ms: dur_ms,
            clip_type: ClipType::Captions {
                model_id: model_id.to_string(),
                language: language.to_string(),
                segments: Vec::new(),
                style: CaptionStyle::default(),
            },
            speed: 1.0,
            reversed: false,
            flip_h: false,
            flip_v: false,
            volume_db: 0.0,
            effects: Vec::new(),
            transition_in: None,
            transition_out: None,
            transition_in_easing: EaseCurve::Linear,
            transition_out_easing: EaseCurve::Linear,
            transition_duration_ms: default_transition_duration_ms(),
            applied_xfade_shift_ms: 0,
            audio_detached: false,
            name: None,
            fade_in_ms: 0,
            fade_out_ms: 0,
            volume_keyframes: Vec::new(),
            duck_against: None,
            speed_end: None,
            speed_ease: EaseCurve::default(),
            speed_range: SpeedRampRange::default(),
            auto_reframe: Vec::new(),
            bg_removal: None,
            source_duration_ms: 0,
            media_id: None,
        }
    }

    pub fn new_narration(
        track: usize,
        start_ms: u64,
        dur_ms: u64,
        model_id: &str,
        voice_id: &str,
        text: &str,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            track_index: track,
            start_time_ms: start_ms,
            duration_ms: dur_ms,
            clip_type: ClipType::Narration {
                model_id: model_id.to_string(),
                voice_id: voice_id.to_string(),
                text: text.to_string(),
            },
            speed: 1.0,
            reversed: false,
            flip_h: false,
            flip_v: false,
            volume_db: 0.0,
            effects: Vec::new(),
            transition_in: None,
            transition_out: None,
            transition_in_easing: EaseCurve::Linear,
            transition_out_easing: EaseCurve::Linear,
            transition_duration_ms: default_transition_duration_ms(),
            applied_xfade_shift_ms: 0,
            audio_detached: false,
            name: None,
            fade_in_ms: 0,
            fade_out_ms: 0,
            volume_keyframes: Vec::new(),
            duck_against: None,
            speed_end: None,
            speed_ease: EaseCurve::default(),
            speed_range: SpeedRampRange::default(),
            auto_reframe: Vec::new(),
            bg_removal: None,
            source_duration_ms: 0,
            media_id: None,
        }
    }

    pub fn end_time_ms(&self) -> u64 {
        self.start_time_ms + self.duration_ms
    }
}

#[cfg(test)]
mod text_motion_tests {
    use super::*;

    #[test]
    fn text_motion_default_is_identity() {
        let m = TextMotion::default();
        assert_eq!(m.x, 0.0);
        assert_eq!(m.y, 0.0);
        assert_eq!(m.rotation, 0.0);
        assert_eq!(m.scale, 1.0);
    }

    #[test]
    fn text_overlay_deserializes_without_motion_or_effect() {
        let json = r#"{
            "TextOverlay": {
                "content": "hi",
                "font_size": 24.0,
                "above": true,
                "style": "default"
            }
        }"#;
        let ct: ClipType = serde_json::from_str(json).expect("legacy deserializes");
        match ct {
            ClipType::TextOverlay {
                motion,
                effect,
                style,
                ..
            } => {
                assert_eq!(style, "default");
                assert_eq!(motion, TextMotion::default());
                assert!(effect.is_none());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn text_effect_kind_serde_is_snake_case() {
        assert_eq!(
            serde_json::to_string(&TextEffectKind::Blink).unwrap(),
            "\"blink\""
        );
        assert_eq!(
            serde_json::to_string(&TextEffectKind::Pulse).unwrap(),
            "\"pulse\""
        );
        assert_eq!(
            serde_json::to_string(&TextEffectKind::ColorCycle).unwrap(),
            "\"color_cycle\""
        );
    }

    #[test]
    fn text_overlay_round_trip_with_motion_and_effect() {
        let ct = ClipType::TextOverlay {
            content: "x".into(),
            font_size: 32.0,
            above: false,
            style: "bold".into(),
            motion: TextMotion {
                x: 0.25,
                y: -0.1,
                rotation: 15.0,
                scale: 1.5,
            },
            effect: Some(TextEffect {
                kind: TextEffectKind::Pulse,
                period: 0.8,
                amount: 0.5,
            }),
        };
        let s = serde_json::to_string(&ct).unwrap();
        let back: ClipType = serde_json::from_str(&s).unwrap();
        assert_eq!(back, ct);
    }
}
