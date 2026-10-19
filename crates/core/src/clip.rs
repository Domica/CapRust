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
        /// Motion transform shared with TextOverlay: normalized x/y
        /// offset (fraction of frame), uniform scale. Applies to
        /// every drawtext the render path expands from the segments.
        #[serde(default)]
        motion: TextMotion,
        /// Procedural effect shared with TextOverlay: Blink, Pulse,
        /// or ColorCycle. None = static captions.
        #[serde(default)]
        effect: Option<TextEffect>,
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
    /// Caption height as a percentage of frame height, when set.
    ///
    /// `Some(p)` means "this many percent of the frame height" and
    /// wins over `font_size`. `None` is the pixel mode, where
    /// `font_size` is authored against a 1080-line frame and scaled
    /// proportionally from there, so a 720-line export renders at
    /// 2/3 the size and a 2160-line export at twice.
    ///
    /// Both modes produce the same on-screen size at 1080 lines;
    /// percent just makes the intent explicit and stops the number
    /// from silently meaning something different per project.
    #[serde(default)]
    pub font_size_pct: Option<f32>,
    /// Caption height in pixels at a 1080-line frame. Ignored when
    /// `font_size_pct` is set.
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
    /// Reveal the caption word by word in step with the speech.
    ///
    /// Off by default: the plain behaviour is to hold the whole
    /// segment text on screen for its duration, which is what a reader
    /// expects from a subtitle. Karaoke is there for social-video
    /// captions where the tracking effect is the point.
    #[serde(default)]
    pub karaoke: bool,
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
            font_size_pct: None,
            font_size: default_caption_font_size(),
            position: CaptionPosition::default(),
            color: default_caption_color(),
            outline_color: default_caption_outline_color(),
            outline_width: default_caption_outline_width(),
            bg_enabled: false,
            bg_opacity: default_caption_bg_opacity(),
            karaoke: false,
        }
    }
}

impl CaptionStyle {
    /// The authored size expressed in pixels for a frame of `height`
    /// lines.
    ///
    /// This is what makes preview and export agree. The preview
    /// renders at a reduced size and is then scaled up for display,
    /// while the export renders at full size and is not, so an
    /// absolute pixel value used to come out visibly smaller in the
    /// export. Every path goes through here.
    pub fn resolved_font_size(&self, height: u32) -> f32 {
        let px = match self.font_size_pct {
            Some(pct) => height as f32 * pct / 100.0,
            None => self.font_size * height as f32 / 1080.0,
        };
        // libass chokes on Fontsize 0 and renders nothing at all, so
        // keep a floor rather than letting a 0% entry blank the text.
        px.max(4.0)
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct ChromaKeySpec {
    #[serde(default = "default_chroma_key_color")]
    pub key_color: [u8; 3],
    #[serde(default = "default_chroma_similarity")]
    pub similarity: f32,
    #[serde(default = "default_chroma_blend")]
    pub blend: f32,
    #[serde(default)]
    pub spill_suppression: bool,
}

fn default_chroma_key_color() -> [u8; 3] {
    [0, 255, 0]
}
fn default_chroma_similarity() -> f32 {
    0.10
}
fn default_chroma_blend() -> f32 {
    0.10
}

impl Default for ChromaKeySpec {
    fn default() -> Self {
        Self {
            key_color: default_chroma_key_color(),
            similarity: default_chroma_similarity(),
            blend: default_chroma_blend(),
            spill_suppression: false,
        }
    }
}

/// Per-clip color grade (lift / gamma / gain + white balance).
/// All fields are neutral at default, so `is_identity()` grades
/// render nothing and the filtergraph stays byte-identical.
///
/// Mapping to ffmpeg (see `build_color_grade_chain` in media-io):
/// lift → colorbalance shadows, gain → highlights as (gain − 1),
/// gamma → midtones as (1 − gamma), temperature → colortemperature
/// (Kelvin), tint → green/magenta midtones, saturation + contrast
/// → eq. Ranges are clamped at the UI; the renderer clamps again.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct ColorGrade {
    /// Shadows offset per channel, -1..=1. 0 = neutral.
    #[serde(default)]
    pub lift: [f32; 3],
    /// Midtones multiplier per channel, 1 = neutral.
    #[serde(default = "default_grade_gamma")]
    pub gamma: [f32; 3],
    /// Highlights multiplier per channel, 1 = neutral.
    #[serde(default = "default_grade_gain")]
    pub gain: [f32; 3],
    /// White balance in Kelvin. 6500 = neutral.
    #[serde(default = "default_grade_temperature")]
    pub temperature: f32,
    /// Green (+) / magenta (−) balance, -1..=1. 0 = neutral.
    #[serde(default)]
    pub tint: f32,
    /// Saturation multiplier, 1 = neutral.
    #[serde(default = "default_grade_unit")]
    pub saturation: f32,
    /// Contrast multiplier, 1 = neutral.
    #[serde(default = "default_grade_unit")]
    pub contrast: f32,
}

fn default_grade_gamma() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}
fn default_grade_gain() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}
fn default_grade_temperature() -> f32 {
    6500.0
}
fn default_grade_unit() -> f32 {
    1.0
}

impl Default for ColorGrade {
    fn default() -> Self {
        Self {
            lift: [0.0, 0.0, 0.0],
            gamma: default_grade_gamma(),
            gain: default_grade_gain(),
            temperature: default_grade_temperature(),
            tint: 0.0,
            saturation: 1.0,
            contrast: 1.0,
        }
    }
}

impl ColorGrade {
    /// True when every field is neutral — the render emits nothing.
    pub fn is_identity(&self) -> bool {
        self.lift == [0.0, 0.0, 0.0]
            && self.gamma == [1.0, 1.0, 1.0]
            && self.gain == [1.0, 1.0, 1.0]
            && (self.temperature - 6500.0).abs() < 1.0
            && self.tint == 0.0
            && self.saturation == 1.0
            && self.contrast == 1.0
    }

    /// Cinematic starter: warm highlights, cool shadows, snap + sat.
    pub fn preset_cinematic() -> Self {
        Self {
            lift: [0.0, 0.0, 0.06],
            gamma: [1.0, 1.0, 1.0],
            gain: [1.08, 1.0, 0.94],
            temperature: 5600.0,
            tint: 0.0,
            saturation: 1.12,
            contrast: 1.1,
        }
    }

    /// Warm starter: golden-hour white balance + gentle lift.
    pub fn preset_warm() -> Self {
        Self {
            lift: [0.03, 0.01, 0.0],
            gamma: [1.0, 1.0, 1.0],
            gain: [1.05, 1.0, 0.96],
            temperature: 7500.0,
            tint: 0.0,
            saturation: 1.05,
            contrast: 1.0,
        }
    }
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

/// One caption cue window in absolute project milliseconds. Shared by
/// the render overlap clamp and the SRT writer so both agree on what
/// "no overlap" means.
#[derive(Debug, Clone, PartialEq)]
pub struct CueWindow {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// Sort by start; clamp each end to the next start; drop fully covered
/// and empty windows. Whisper segments routinely overlap in time and
/// unclamped windows render as stacked rows (preview and export share
/// the same plan, so one fix covers both).
pub fn normalize_cue_windows(mut cues: Vec<CueWindow>) -> Vec<CueWindow> {
    cues.retain(|c| !c.text.trim().is_empty() && c.end_ms > c.start_ms);
    cues.sort_by_key(|c| (c.start_ms, c.end_ms));
    let mut out: Vec<CueWindow> = Vec::with_capacity(cues.len());
    for mut c in cues {
        if let Some(last) = out.last() {
            if c.start_ms < last.end_ms {
                if c.end_ms <= last.end_ms {
                    continue;
                }
                c.start_ms = last.end_ms;
            }
        }
        out.push(c);
    }
    out
}

/// Format cue windows as SubRip (.srt). Input is assumed normalized;
/// each cue becomes its index, `HH:MM:SS,mmm --> ...`, text, blank line.
pub fn cues_to_srt(cues: &[CueWindow]) -> String {
    fn ts(ms: u64) -> String {
        format!(
            "{:02}:{:02}:{:02},{:03}",
            ms / 3_600_000,
            ms / 60_000 % 60,
            ms / 1_000 % 60,
            ms % 1_000
        )
    }
    let mut out = String::new();
    for (i, c) in cues.iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            ts(c.start_ms),
            ts(c.end_ms),
            c.text.trim()
        ));
    }
    out
}

/// Collect caption cues from project clips in absolute project
/// milliseconds, normalized (no overlaps). Render equivalent lives in
/// export_graph (same rule); times here are raw timeline positions.
pub fn collect_caption_cues(clips: &[Clip]) -> Vec<CueWindow> {
    let mut cues = Vec::new();
    for c in clips {
        if let ClipType::Captions { segments, .. } = &c.clip_type {
            for seg in segments {
                if seg.text.trim().is_empty() {
                    continue;
                }
                cues.push(CueWindow {
                    start_ms: c.start_time_ms.saturating_add(seg.start_ms),
                    end_ms: c.start_time_ms.saturating_add(seg.end_ms),
                    text: seg.text.clone(),
                });
            }
        }
    }
    normalize_cue_windows(cues)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EffectInstance {
    pub effect_id: String,
    #[serde(default = "default_effect_amount")]
    pub amount: f32,
    #[serde(default)]
    pub enabled: bool,
}

fn default_duck_reduction_db() -> f32 {
    -12.0
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
    // MUST stay in sync with media-io `xfade_name`: every id mapped
    // there must be listed here, otherwise load normalization strips
    // it and the apply command computes no shift.
    matches!(
        id,
        "fade"
            | "slide_l"
            | "slide_r"
            | "slide_u"
            | "slide_d"
            | "wipe_l"
            | "wipe_r"
            | "wipe_u"
            | "wipe_d"
            | "fadewhite"
            | "zoom_in"
            | "zoom_out"
            | "rotate"
            | "blur_t"
            | "dissolve"
            | "smooth_l"
            | "smooth_r"
            | "smooth_u"
            | "smooth_d"
            | "circle_close"
            | "pixelize"
            | "cover_l"
            | "cover_r"
            | "cover_u"
            | "cover_d"
            | "reveal_l"
            | "reveal_r"
            | "reveal_u"
            | "reveal_d"
            | "diag_tl"
            | "diag_tr"
            | "diag_bl"
            | "diag_br"
            | "wipe_tl"
            | "wipe_tr"
            | "wipe_bl"
            | "wipe_br"
            | "vert_open"
            | "vert_close"
            | "horz_open"
            | "horz_close"
            | "fadegrays"
            | "distance"
            | "hblur"
            | "circle_crop"
            | "rect_crop"
            | "zoom_blur"
            | "squeeze_h"
            | "squeeze_v"
            | "fade_fast"
            | "fade_slow"
            | "slice_hl"
            | "slice_hr"
            | "slice_vu"
            | "slice_vd"
            | "wind_right"
            | "wind_up"
            | "wind_down"
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

/// Sample the volume automation curve at `t_ms` (ms relative to clip
/// start). Linearly interpolates in the **dB domain** between sorted
/// keyframes, matching the ffmpeg expression built in
/// `media-io::export_graph` (`seg_db = a + (b-a)*(t-ta)/dt` then
/// `pow(10, db/20)`). Do NOT interpolate in the linear gain domain:
/// the two produce different curves and the timeline overlay would
/// drift from the exported audio.
///
/// Clamps to the first / last keyframe when `t_ms` is outside the
/// keyframe range. Returns `default_db` when `keyframes` is empty.
pub fn sample_volume_at(keyframes: &[VolumeKeyframe], default_db: f32, t_ms: i64) -> f32 {
    if keyframes.is_empty() {
        return default_db;
    }
    let mut sorted: Vec<&VolumeKeyframe> = keyframes.iter().collect();
    sorted.sort_by_key(|k| k.t_ms);
    let first = sorted[0];
    let last = sorted[sorted.len() - 1];
    if t_ms <= first.t_ms as i64 {
        return first.gain_db;
    }
    if t_ms >= last.t_ms as i64 {
        return last.gain_db;
    }
    for w in sorted.windows(2) {
        let a = w[0];
        let b = w[1];
        if t_ms >= a.t_ms as i64 && t_ms <= b.t_ms as i64 {
            let span = (b.t_ms - a.t_ms) as f32;
            if span <= 0.0 {
                return b.gain_db;
            }
            let t = (t_ms - a.t_ms as i64) as f32 / span;
            return a.gain_db + (b.gain_db - a.gain_db) * t;
        }
    }
    default_db
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

/// Square, the shape every built-in sticker is. Projects written before
/// `aspect` existed carry no value, and square is what they rendered as.
fn default_aspect() -> f32 {
    1.0
}

/// Where a clip sits on the frame when it is an overlay rather than the
/// frame itself.
///
/// Every clip used to be scaled to fill the output frame, because every
/// clip was footage. A sticker is not footage: it is a 256px PNG that has
/// to sit in a corner at a fraction of the frame, over the top of the
/// video. `None` on a clip keeps the old behaviour, so this costs nothing
/// for existing projects.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverlayPlacement {
    /// Height as a fraction of the output frame. 0.25 = a quarter as
    /// tall as the frame. The width follows the source's aspect ratio.
    pub scale: f32,
    /// Position along the sticker's range of travel, as a fraction of
    /// the space the sticker has to move in. 0 is flush against the left
    /// edge, 1 is flush against the right, and 0.5 is centred. The width
    /// of the sticker is already taken out, so this is not the same as
    /// a fraction of the frame -- see `frame_box`.
    pub x: f32,
    /// As `x`, top to bottom.
    pub y: f32,
    /// Source width divided by height. Needed to draw the marker's box on
    /// screen: `scale` fixes the height, and the width follows the source,
    /// which is not always square.
    #[serde(default = "default_aspect")]
    pub aspect: f32,
    /// Preset id from the sticker animation list. `None` = static.
    #[serde(default)]
    pub animation: Option<String>,
}

impl OverlayPlacement {
    /// The rectangle the sticker covers, in unit-frame coordinates:
    /// `(left, top, width, height)`, all fractions of the frame with the
    /// top-left at 0,0.
    ///
    /// This is the single description of a sticker's geometry. The export
    /// graph emits `overlay=x=(W-w)*x:y=(H-h)*y`, which places the box's
    /// *left edge* at `(W-w)*x`, and this function is that same
    /// arithmetic in unit coordinates. The preview handler multiplies the
    /// result by the on-screen frame size, so the box the user drags is
    /// the box that renders.
    ///
    /// Deriving one from the other by hand is how they drift apart: an
    /// earlier version of the preview treated `x` as the centre and was
    /// wrong by half the sticker's size anywhere except the middle.
    /// `frame_aspect` is the frame's width divided by its height.
    ///
    /// That division is the whole reason this takes an argument.
    /// `scale` is a fraction of the frame's *height*, which is what
    /// `scale=-2:<height>` in the graph asks for, so on a 16:9 frame a
    /// square sticker at 0.2 is 36 px wide on a 320 px frame -- 11% of
    /// the width, not 20%. Reading `scale` as a fraction of both axes
    /// drew a box nearly twice as wide as the sticker and put the drag
    /// handle in empty space beside it.
    pub fn frame_box(&self, frame_aspect: f32) -> (f32, f32, f32, f32) {
        let h = self.scale;
        let w = h * self.aspect / frame_aspect.max(1e-6);
        ((1.0 - w) * self.x, (1.0 - h) * self.y, w, h)
    }

    /// The placement that puts the box's centre at `(cx, cy)`, in the
    /// same unit-frame coordinates `frame_box` reports.
    ///
    /// The inverse of `frame_box`, and the reason the preview drag goes
    /// through this rather than adding the pointer delta to `x`. `x` is a
    /// fraction of the sticker's travel, not of the frame, so adding
    /// `dx / frame_width` moves the sticker by `dx * (1 - width)` -- a
    /// third of the pointer travel for a third-frame-wide sticker, and
    /// the handle would trail behind the cursor.
    ///
    /// The caller supplies the animation id separately; this only
    /// concerns geometry.
    pub fn at_centre(cx: f32, cy: f32, scale: f32, aspect: f32, frame_aspect: f32) -> Self {
        let w = scale * aspect / frame_aspect.max(1e-6);
        // A sticker as wide as the frame has no travel left, so there is
        // no fraction that can centre it. The comparison is on the
        // magnitude because a sticker *wider* than the frame gives a
        // negative denominator that is perfectly solvable -- treating
        // that as "no travel" would pin an oversized sticker to the
        // centre instead of letting it be dragged.
        let denom_x = 1.0 - w;
        let denom_y = 1.0 - scale;
        let x = if denom_x.abs() > 1e-6 {
            (cx - w / 2.0) / denom_x
        } else {
            0.5
        };
        let y = if denom_y.abs() > 1e-6 {
            (cy - scale / 2.0) / denom_y
        } else {
            0.5
        };
        Self {
            scale,
            x,
            y,
            aspect,
            animation: None,
        }
    }

    /// What the properties panel starts a freshly dropped sticker at:
    /// a third of the frame tall, in the lower third, where a caption
    /// or a callout usually goes.
    pub fn default_sticker() -> Self {
        Self {
            scale: 0.34,
            x: 0.5,
            y: 0.72,
            aspect: default_aspect(),
            animation: None,
        }
    }
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
    /// Offset into the source file where this clip's visible
    /// window begins, in source milliseconds. 0 = clip starts at
    /// the beginning of the file. Split-at-playhead sets this on
    /// the right half so the second half plays from where the
    /// first stopped, not from 0. `#[serde(default)]` keeps old
    /// projects loading (everything defaulted to 0).
    #[serde(default)]
    pub source_offset_ms: u64,
    /// Links back to the MediaItem this clip was created from, if any.
    #[serde(default)]
    pub media_id: Option<Uuid>,
    /// Set when the clip is an overlay -- a sticker, a logo, a watermark
    /// -- rather than footage filling the frame. `None` is every clip
    /// that existed before stickers, which is the behaviour they keep.
    #[serde(default)]
    pub overlay: Option<OverlayPlacement>,
    #[serde(default)]
    pub effects: Vec<EffectInstance>,
    /// Color LUT applied to this clip. `None` = no LUT.
    /// `Some("id")` for a built-in LUT, or an absolute path to a custom `.cube` file.
    #[serde(default)]
    pub lut: Option<String>,
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
    /// Back-reference from a video clip to the Audio clip created
    /// by SeparateAudioCommand. `Some(id)` means the video's
    /// embedded audio was separated and the Audio clip still
    /// exists; Reattach can move it back. Cleared when the audio
    /// clip is deleted on its own, so reattach degrades to "just
    /// flip audio_detached back to false".
    #[serde(default)]
    pub detached_audio_clip_id: Option<Uuid>,
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
    /// Apply EBU R128 loudness normalization (ffmpeg `loudnorm`) to
    /// this clip's audio during export and preview. Target -16 LUFS,
    /// -1.5 dBTP, LRA 11, matching the streaming loudness standard.
    #[serde(default)]
    pub audio_normalize: bool,
    /// Apply spectral denoise (ffmpeg `afftdn`, noise floor -25 dB).
    /// Runs before voice boost and normalize.
    #[serde(default)]
    pub audio_denoise: bool,
    /// Apply dynamic range compression tuned for speech (ffmpeg
    /// `dynaudnorm`, peak target 0.9, max gain 10x). Runs before
    /// normalize.
    #[serde(default)]
    pub audio_voice_boost: bool,
    /// Telephone/radio voice (ffmpeg `highpass=f=300,lowpass=f=3400`).
    /// Creative FX stage, runs after normalize.
    #[serde(default)]
    pub audio_telephone: bool,
    /// Bass boost (ffmpeg `bass=g=8`). Creative FX stage.
    #[serde(default)]
    pub audio_bass: bool,
    /// Short slapback echo (ffmpeg `aecho`). Creative FX stage.
    #[serde(default)]
    pub audio_echo: bool,
    /// Chipmunk pitch-up (ffmpeg `asetrate` + `aresample`). Shortens
    /// the audio slightly; a fun effect, not sample-accurate.
    #[serde(default)]
    pub audio_chipmunk: bool,
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
    /// Target gain reduction in dB when the ducking control
    /// clip is playing. Negative value (e.g. -12.0 = drop 12 dB).
    /// Only meaningful when `duck_against` is Some. Clamped at
    /// render time to [-60, 0].
    #[serde(default = "default_duck_reduction_db")]
    pub duck_reduction_db: f32,
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
    /// Path to the per-clip face-anonymize mask. Relative to the
    /// project directory, e.g. "cache/anon/<clip_id>.mkv". None = no
    /// anonymization. Populated by the anonymize job (YuNet boxes),
    /// consumed by the render graph, which merges a blurred copy
    /// through this mask.
    #[serde(default)]
    pub anon_mask: Option<String>,
    /// Path to the per-clip vidstab transforms file (stabilization
    /// pass 1). Relative to the project directory, e.g.
    /// "cache/stab/<clip_id>.trf". None = no stabilization.
    /// Written by the stab detect job, consumed by the render graph
    /// as `vidstabtransform=input=...`. Cleared whenever the source
    /// window changes (trim/speed/reverse), because the stored
    /// transforms would no longer align.
    #[serde(default)]
    pub stab_trf: Option<String>,
    #[serde(default)]
    pub chroma_key: Option<ChromaKeySpec>,
    /// Per-clip waveform layout override. None = follow the global
    /// `waveform_bottom` setting. Some(true) = split display (strip on
    /// top, waveform band at the bottom) for this clip; Some(false) =
    /// legacy overlay. Set from the clip context menu ("Clip display").
    #[serde(default)]
    pub split_display: Option<bool>,
    /// Blend mode for compositing this clip over lower z-order layers.
    /// None (or "normal") = plain overlay. Otherwise one of the ffmpeg
    /// `blend=all_mode=` keywords (multiply, screen, ...). Only the
    /// first set mode in a z-order run takes effect (see
    /// `blend_mode_name`); mixed-mode runs use the first clip's mode.
    #[serde(default)]
    pub blend_mode: Option<String>,
    /// Per-clip color grade. None = ungraded (no filter stages).
    /// Some(g) with g.is_identity() also renders nothing.
    #[serde(default)]
    pub color_grade: Option<ColorGrade>,
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
            detached_audio_clip_id: None,
            name: None,
            fade_in_ms: 0,
            source_offset_ms: 0,
            fade_out_ms: 0,
            audio_normalize: false,
            audio_denoise: false,
            audio_voice_boost: false,
            audio_telephone: false,
            audio_bass: false,
            audio_echo: false,
            audio_chipmunk: false,
            volume_keyframes: Vec::new(),
            duck_against: None,
            duck_reduction_db: -12.0,
            speed_end: None,
            speed_ease: EaseCurve::default(),
            speed_range: SpeedRampRange::default(),
            auto_reframe: Vec::new(),
            bg_removal: None,
            stab_trf: None,
            chroma_key: None,
            lut: None,
            split_display: None,
            blend_mode: None,
            color_grade: None,
            anon_mask: None,
            source_duration_ms: dur_ms,
            media_id: None,
            overlay: None,
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
            detached_audio_clip_id: None,
            name: None,
            fade_in_ms: 0,
            source_offset_ms: 0,
            fade_out_ms: 0,
            audio_normalize: false,
            audio_denoise: false,
            audio_voice_boost: false,
            audio_telephone: false,
            audio_bass: false,
            audio_echo: false,
            audio_chipmunk: false,
            volume_keyframes: Vec::new(),
            duck_against: None,
            duck_reduction_db: -12.0,
            speed_end: None,
            speed_ease: EaseCurve::default(),
            speed_range: SpeedRampRange::default(),
            auto_reframe: Vec::new(),
            bg_removal: None,
            stab_trf: None,
            chroma_key: None,
            lut: None,
            split_display: None,
            blend_mode: None,
            color_grade: None,
            anon_mask: None,
            source_duration_ms: dur_ms,
            media_id: None,
            overlay: None,
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
            detached_audio_clip_id: None,
            name: None,
            fade_in_ms: 0,
            source_offset_ms: 0,
            fade_out_ms: 0,
            audio_normalize: false,
            audio_denoise: false,
            audio_voice_boost: false,
            audio_telephone: false,
            audio_bass: false,
            audio_echo: false,
            audio_chipmunk: false,
            volume_keyframes: Vec::new(),
            duck_against: None,
            duck_reduction_db: -12.0,
            speed_end: None,
            speed_ease: EaseCurve::default(),
            speed_range: SpeedRampRange::default(),
            auto_reframe: Vec::new(),
            bg_removal: None,
            stab_trf: None,
            chroma_key: None,
            lut: None,
            split_display: None,
            blend_mode: None,
            color_grade: None,
            anon_mask: None,
            source_duration_ms: 0,
            media_id: None,
            overlay: None,
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
            detached_audio_clip_id: None,
            name: None,
            fade_in_ms: 0,
            source_offset_ms: 0,
            fade_out_ms: 0,
            audio_normalize: false,
            audio_denoise: false,
            audio_voice_boost: false,
            audio_telephone: false,
            audio_bass: false,
            audio_echo: false,
            audio_chipmunk: false,
            volume_keyframes: Vec::new(),
            duck_against: None,
            duck_reduction_db: -12.0,
            speed_end: None,
            speed_ease: EaseCurve::default(),
            speed_range: SpeedRampRange::default(),
            auto_reframe: Vec::new(),
            bg_removal: None,
            stab_trf: None,
            chroma_key: None,
            lut: None,
            split_display: None,
            blend_mode: None,
            color_grade: None,
            anon_mask: None,
            source_duration_ms: 0,
            media_id: None,
            overlay: None,
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
            detached_audio_clip_id: None,
            name: None,
            fade_in_ms: 0,
            source_offset_ms: 0,
            fade_out_ms: 0,
            audio_normalize: false,
            audio_denoise: false,
            audio_voice_boost: false,
            audio_telephone: false,
            audio_bass: false,
            audio_echo: false,
            audio_chipmunk: false,
            volume_keyframes: Vec::new(),
            duck_against: None,
            duck_reduction_db: -12.0,
            speed_end: None,
            speed_ease: EaseCurve::default(),
            speed_range: SpeedRampRange::default(),
            auto_reframe: Vec::new(),
            bg_removal: None,
            stab_trf: None,
            chroma_key: None,
            lut: None,
            split_display: None,
            blend_mode: None,
            color_grade: None,
            anon_mask: None,
            source_duration_ms: 0,
            media_id: None,
            overlay: None,
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
            detached_audio_clip_id: None,
            name: None,
            fade_in_ms: 0,
            source_offset_ms: 0,
            fade_out_ms: 0,
            audio_normalize: false,
            audio_denoise: false,
            audio_voice_boost: false,
            audio_telephone: false,
            audio_bass: false,
            audio_echo: false,
            audio_chipmunk: false,
            volume_keyframes: Vec::new(),
            duck_against: None,
            duck_reduction_db: -12.0,
            speed_end: None,
            speed_ease: EaseCurve::default(),
            speed_range: SpeedRampRange::default(),
            auto_reframe: Vec::new(),
            bg_removal: None,
            stab_trf: None,
            chroma_key: None,
            lut: None,
            split_display: None,
            blend_mode: None,
            color_grade: None,
            anon_mask: None,
            source_duration_ms: 0,
            media_id: None,
            overlay: None,
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
    fn captions_deserializes_without_motion_or_effect() {
        // Old projects carry Captions clips without motion/effect.
        // serde(default) must fill them in.
        let raw = r#"{
            "Captions": {
                "model_id": "whisper-tiny",
                "language": "en",
                "segments": [],
                "style": {
                    "font_size": 32.0,
                    "position": "bottom",
                    "color": [255, 255, 0],
                    "outline_color": [0, 0, 0],
                    "outline_width": 2.0,
                    "bg_enabled": false,
                    "bg_opacity": 0.5
                }
            }
        }"#;
        let ct: ClipType = serde_json::from_str(raw).unwrap();
        match ct {
            ClipType::Captions { motion, effect, .. } => {
                assert_eq!(motion, TextMotion::default());
                assert!(effect.is_none());
            }
            _ => panic!("expected Captions"),
        }
    }

    #[test]
    fn captions_round_trip_with_motion_and_effect() {
        let mut c = Clip::new_captions(0, 0, 3000, "whisper-tiny", "en");
        if let ClipType::Captions { motion, effect, .. } = &mut c.clip_type {
            motion.x = 0.25;
            motion.y = -0.1;
            motion.scale = 1.5;
            *effect = Some(TextEffect {
                kind: TextEffectKind::Pulse,
                period: 0.8,
                amount: 0.5,
            });
        }
        let j = serde_json::to_string(&c).unwrap();
        let back: Clip = serde_json::from_str(&j).unwrap();
        match back.clip_type {
            ClipType::Captions { motion, effect, .. } => {
                assert_eq!(motion.x, 0.25);
                assert_eq!(motion.scale, 1.5);
                let e = effect.unwrap();
                assert_eq!(e.kind, TextEffectKind::Pulse);
            }
            _ => panic!("expected Captions"),
        }
    }

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

#[cfg(test)]
mod chroma_key_tests {
    use super::*;

    #[test]
    fn defaults() {
        let s = ChromaKeySpec::default();
        assert_eq!(s.key_color, [0, 255, 0]);
        assert!((s.similarity - 0.10).abs() < 1e-6);
        assert!((s.blend - 0.10).abs() < 1e-6);
    }

    #[test]
    fn serde_round_trip() {
        let s = ChromaKeySpec {
            key_color: [10, 200, 30],
            similarity: 0.25,
            blend: 0.05,
            spill_suppression: false,
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: ChromaKeySpec = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn empty_object_is_defaults() {
        let back: ChromaKeySpec = serde_json::from_str("{}").unwrap();
        assert_eq!(back.key_color, [0, 255, 0]);
    }
}

/// Maximum clip duration (seconds) for which Reverse is enabled in
/// the UI. Above this, the ffmpeg `reverse` filter buffers the
/// whole trimmed window in RAM before emitting frame 0, and peak
/// memory on 1080p30 approaches 6 GB at 60 s. Enforced in
/// clip_properties; the render layer accepts any duration.
pub const REVERSE_MAX_SEC: f64 = 60.0;

#[cfg(test)]
mod cue_window_tests {
    use super::*;

    fn cue(start_ms: u64, end_ms: u64, text: &str) -> CueWindow {
        CueWindow {
            start_ms,
            end_ms,
            text: text.into(),
        }
    }

    #[test]
    fn overlapping_windows_clamp_to_next_start() {
        let out = normalize_cue_windows(vec![cue(0, 1000, "a"), cue(800, 2000, "b")]);
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].start_ms, out[0].end_ms), (0, 1000));
        assert_eq!((out[1].start_ms, out[1].end_ms), (1000, 2000));
    }

    #[test]
    fn fully_covered_empty_and_zero_windows_drop() {
        let out = normalize_cue_windows(vec![
            cue(0, 1000, "a"),
            cue(200, 500, "b"),
            cue(1000, 1000, "c"),
            cue(1000, 2000, "  "),
            cue(1000, 2000, "d"),
        ]);
        let texts: Vec<&str> = out.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, vec!["a", "d"]);
    }

    #[test]
    fn srt_formats_index_timestamps_and_text() {
        let out = cues_to_srt(&[cue(0, 1000, "hello"), cue(3723004, 3724999, "world")]);
        assert_eq!(
            out,
            "1\n00:00:00,000 --> 00:00:01,000\nhello\n\n\
             2\n01:02:03,004 --> 01:02:04,999\nworld\n\n"
        );
    }

    #[test]
    fn srt_empty_is_empty() {
        assert_eq!(cues_to_srt(&[]), "");
    }

    /// 16:9, the aspect almost every project is.
    const FRAME_ASPECT: f32 = 16.0 / 9.0;

    /// `frame_box` is the one description of where a sticker sits, and it
    /// has to agree with what ffmpeg composites. These numbers are
    /// measured, not derived: the box below is where a 64x64 white square
    /// at scale 0.2 on a 320x180 frame actually turned up when the real
    /// graph was run.
    #[test]
    fn frame_box_matches_the_filtergraph_placement() {
        use super::OverlayPlacement;

        let place = OverlayPlacement {
            scale: 0.20,
            x: 0.30,
            y: 0.25,
            aspect: 1.0,
            animation: None,
        };
        let (left, top, w, h) = place.frame_box(FRAME_ASPECT);

        // scale is a fraction of the frame's height: 0.2 of 180 = 36px.
        // The width follows the source aspect, so 36px too -- which on a
        // 320px-wide frame is 11% of the width, not 20%.
        assert!(
            (h * 180.0 - 36.0).abs() < 1e-4,
            "height in px is {}",
            h * 180.0
        );
        assert!(
            (w * 320.0 - 36.0).abs() < 1e-4,
            "width in px is {}",
            w * 320.0
        );

        // The graph emits overlay=x=(W-w)*x with W and w in pixels.
        assert!(
            ((left * 320.0) - 85.2).abs() < 0.2,
            "left px is {}",
            left * 320.0
        );
        assert!(
            ((top * 180.0) - 36.0).abs() < 0.2,
            "top px is {}",
            top * 180.0
        );
    }

    /// The two ends of the travel are flush against the frame, and the
    /// middle is the middle. These are readings a user can check by eye,
    /// so a change that breaks one is a change nobody wants.
    #[test]
    fn frame_box_edges_are_flush_and_the_middle_is_centred() {
        use super::OverlayPlacement;

        let base = OverlayPlacement {
            scale: 0.4,
            x: 0.5,
            y: 0.5,
            aspect: 1.0,
            animation: None,
        };

        let (l, t, w, h) = base.frame_box(FRAME_ASPECT);
        assert!((l + w / 2.0 - 0.5).abs() < 1e-6, "not centred at 0.5");
        assert!(
            (t + h / 2.0 - 0.5).abs() < 1e-6,
            "not centred at 0.5 vertically"
        );

        let (l, _, _, _) = OverlayPlacement {
            x: 0.0,
            ..base.clone()
        }
        .frame_box(FRAME_ASPECT);
        assert!(l.abs() < 1e-6, "not flush left at 0");

        let (l, _, w, _) = OverlayPlacement {
            x: 1.0,
            ..base.clone()
        }
        .frame_box(FRAME_ASPECT);
        assert!((l + w - 1.0).abs() < 1e-6, "not flush right at 1");

        let (_, t, _, _) = OverlayPlacement {
            y: 0.0,
            ..base.clone()
        }
        .frame_box(FRAME_ASPECT);
        assert!(t.abs() < 1e-6, "not flush top at 0");

        let (_, t, _, h) = OverlayPlacement {
            y: 1.0,
            ..base.clone()
        }
        .frame_box(FRAME_ASPECT);
        assert!((t + h - 1.0).abs() < 1e-6, "not flush bottom at 1");
    }

    /// A drag works by choosing a centre and asking for the placement
    /// that puts it there. If the two are not inverses the handle drifts
    /// away from the sticker as it moves.
    #[test]
    fn at_centre_and_frame_box_are_inverses() {
        use super::OverlayPlacement;

        for (cx, cy) in [(0.5, 0.5), (0.2, 0.8), (0.9, 0.1), (0.0, 1.0), (1.0, 0.0)] {
            for scale in [0.1f32, 0.34, 0.75] {
                for aspect in [1.0f32, 1.5, 0.666] {
                    for fa in [FRAME_ASPECT, 1.0, 2.35] {
                        let p = OverlayPlacement::at_centre(cx, cy, scale, aspect, fa);
                        let (l, t, w, h) = p.frame_box(fa);
                        assert!(
                            (l + w / 2.0 - cx).abs() < 1e-4,
                            "centre x drifted: want {cx}, got {}",
                            l + w / 2.0
                        );
                        assert!(
                            (t + h / 2.0 - cy).abs() < 1e-4,
                            "centre y drifted: want {cy}, got {}",
                            t + h / 2.0
                        );
                    }
                }
            }
        }
    }

    /// A drag moves the sticker's centre by the pointer delta, so the whole
    /// box has to end up exactly that far along -- all four edges, since
    /// a sticker slides rigidly along its travel. This is what makes the
    /// handle track the cursor. The drag goes through `at_centre` to get
    /// it: adding the delta straight onto `x` would have to divide by
    /// the sticker's own width to come out right, and did not.
    #[test]
    fn moving_by_a_known_offset_lands_where_it_was_asked_to() {
        use super::OverlayPlacement;

        let start = OverlayPlacement {
            scale: 0.34,
            x: 0.5,
            y: 0.5,
            aspect: 1.0,
            animation: None,
        };
        let (l, t, w, h) = start.frame_box(FRAME_ASPECT);
        let moved = OverlayPlacement::at_centre(
            l + w / 2.0 + 0.1,
            t + h / 2.0 + 0.1,
            0.34,
            1.0,
            FRAME_ASPECT,
        );
        let (l2, t2, w2, h2) = moved.frame_box(FRAME_ASPECT);

        for (name, was, now) in [
            ("left", l, l2),
            ("right", l + w, l2 + w2),
            ("top", t, t2),
            ("bottom", t + h, t2 + h2),
        ] {
            assert!(
                (now - was - 0.1).abs() < 1e-4,
                "{name} moved {} instead of 0.1",
                now - was
            );
        }
    }

    /// A sticker filling the frame has no travel, so the inverse has to
    /// stay finite. A NaN here would land in a saved project and then in
    /// the filtergraph.
    #[test]
    fn a_full_frame_sticker_does_not_produce_nan_placement() {
        use super::OverlayPlacement;

        let p = OverlayPlacement::at_centre(0.3, 0.3, 1.0, 1.0, FRAME_ASPECT);
        assert!(p.x.is_finite(), "x is {}", p.x);
        assert!(p.y.is_finite(), "y is {}", p.y);
        let (l, t, w, h) = p.frame_box(FRAME_ASPECT);
        assert!(l.is_finite() && t.is_finite() && w.is_finite() && h.is_finite());
    }
}
