//! Build FFmpeg filtergraph from a timeline project.
//!
//! We build ONE ffmpeg command that:
//!   1. Reads every clip's source file.
//!   2. Trims each clip to (source_start, duration) using `-ss`/`-t`
//!      per input, so timestamp discontinuities in source files are
//!      handled by Rust, not by `atrim` (which closes inputs early —
//!      same lesson as the audio sample-counter PR).
//!   3. Concatenates video clips with padding (color=black) for gaps.
//!   4. Mixes audio clips with `amix` on top of an `anullsrc` bed.
//!   5. Scales to the target resolution.
//!   6. Encodes with the requested codec/CRF/preset.
//!
//! Output is a single ffmpeg invocation; no intermediate files.

use anyhow::Result;
use std::path::PathBuf;

use crate::export::RateMode;

/// One input file we'll hand to ffmpeg with `-ss` / `-t`.
#[derive(Debug, Clone)]
pub struct InputSpec {
    /// Index in ffmpeg's input list. We assign this in order.
    pub ffmpeg_index: usize,
    pub path: PathBuf,
    /// Start time in the source (seconds). Passed to `-ss` before `-i`.
    pub source_start_sec: f64,
    /// How long to read from `source_start_sec`. Passed as `-t` after `-i`.
    pub duration_sec: f64,
}

/// A single video clip on the timeline mapped to a slice of an input.
#[derive(Debug, Clone)]
pub struct VideoClip {
    pub input_index: usize,
    pub timeline_start_sec: f64,
    pub duration_sec: f64,
    pub speed: f32,
    /// Speed ramp end. Some(x) => ramp from `speed` to `x`. None =>
    /// static `speed`.
    pub speed_end: Option<f32>,
    /// Easing of the ramp. Ignored when `speed_end` is None.
    pub speed_ease: caprust_core::clip::EaseCurve,
    /// Where inside the clip the ramp lives.
    pub speed_range: caprust_core::clip::SpeedRampRange,
    /// Higher z renders on top. V1 = 0, V2 = 1, Overlay = last.
    pub z_order: u32,
    /// Image sources need `-loop 1` on their input.
    pub is_image: bool,
    /// Applied effect instances (see asset_browser Effects list).
    /// Carries `amount` and `enabled` so the filtergraph can scale and
    /// skip stages without reaching back into the project state.
    pub effects: Vec<caprust_core::clip::EffectInstance>,
    /// Transition preset id (fade, slide_l, ...) on the in edge.
    pub transition_in: Option<String>,
    /// Transition preset id on the out edge.
    pub transition_out: Option<String>,
    /// Easing of the in transition. Only meaningful for the single-
    /// clip `fade` arm, which maps it to ffmpeg's `fade=curve=`
    /// parameter. Other transitions (xfade chains, slide/wipe/zoom)
    /// still use ffmpeg's built-in linear ramp.
    pub transition_in_easing: caprust_core::clip::EaseCurve,
    /// Same as `transition_in_easing`, for the out edge.
    pub transition_out_easing: caprust_core::clip::EaseCurve,
    /// Duration of the video fade in/out in seconds. Shared by both
    /// edges. Clamped to a sane range at use time.
    pub transition_duration_sec: f64,
    /// Auto-reframe keypoints (Phase P2c). Empty = render as-is.
    pub auto_reframe: Vec<caprust_core::clip::ReframeKeypoint>,
    /// Absolute path to the per-clip alpha mask (Phase P3d). None =
    /// no background removal. Non-empty = the base chain gets a
    /// maskedmerge over a black base. Ignored when auto_reframe is
    /// also set (see resolve_bg_removal_path for the reason).
    pub bg_removal_path: Option<std::path::PathBuf>,
}

/// Text overlay clip (drawtext filter).
#[derive(Debug, Clone)]
pub struct TextClip {
    pub content: String,
    pub font_size: f32,
    pub timeline_start_sec: f64,
    pub duration_sec: f64,
    pub above: bool,
    pub z_order: u32,
    /// Preset id ("default", "bold", "subtitle", "lower", "quote",
    /// "caption", "glow", "handwrite"). Empty == "default".
    pub style: String,
    /// Motion transform (normalized x/y offset, scale, rotation stub).
    /// Default = identity, matches pre-feature layout.
    pub motion: caprust_core::clip::TextMotion,
    /// Procedural effect (Blink / Pulse / ColorCycle). None = static.
    pub effect: Option<caprust_core::clip::TextEffect>,
    /// When Some, this drawtext came from a Captions clip and its
    /// appearance is driven by the user-editable CaptionStyle instead
    /// of the preset `style` string. TextOverlay clips leave this None.
    pub caption_style: Option<caprust_core::clip::CaptionStyle>,
}

/// Effect option suffix for a drawtext body. Blink = hard on/off via
/// `alpha`; Pulse = sinusoid on `alpha`; ColorCycle = `fontcolor_expr`
/// using ffmpeg's text-expansion (`%{eif:EXPR:x:2}`) to rotate the RGB
/// components. Backslash-colon inside the expansion survives the
/// filter parser and lets the expansion see the inner colon.
/// Build a Captions drawtext body using the user-editable CaptionStyle.
/// Position maps to the same layout the legacy "caption" preset used,
/// so bottom stays the default and existing projects look unchanged.
fn build_caption_drawtext_body(
    t: &TextClip,
    escaped: &str,
    cs: &caprust_core::clip::CaptionStyle,
) -> String {
    use caprust_core::clip::CaptionPosition;
    let y_base: String = match cs.position {
        CaptionPosition::Top => "h*0.08".to_string(),
        CaptionPosition::Middle => "(h-text_h)/2".to_string(),
        CaptionPosition::Bottom => "h*0.82".to_string(),
    };
    let mx = t.motion.x;
    let my = t.motion.y;
    let x_expr = if mx.abs() < 1e-4 {
        "(w-text_w)/2".to_string()
    } else {
        format!("(w-text_w)/2+({mx:.4})*w")
    };
    let y_expr = if my.abs() < 1e-4 {
        y_base
    } else {
        format!("{y_base}+({my:.4})*h")
    };
    let fs = ((cs.font_size as f64) * (t.motion.scale as f64))
        .round()
        .max(1.0) as i32;
    let color = format!(
        "0x{:02X}{:02X}{:02X}",
        cs.color[0], cs.color[1], cs.color[2]
    );
    let outline = if cs.outline_width > 0.5 {
        format!(
            ":borderw={}:bordercolor=0x{:02X}{:02X}{:02X}",
            cs.outline_width.round() as i32,
            cs.outline_color[0],
            cs.outline_color[1],
            cs.outline_color[2]
        )
    } else {
        String::new()
    };
    let bg = if cs.bg_enabled {
        format!(
            ":box=1:boxcolor=black@{:.2}:boxborderw=8",
            cs.bg_opacity.clamp(0.0, 1.0)
        )
    } else {
        String::new()
    };
    let effect_opts = build_text_effect_opts(t.effect.as_ref());
    format!(
        "drawtext=text='{escaped}':fontcolor={color}:fontsize={fs}:x={x_expr}:y={y_expr}{outline}{bg}{effect_opts}:enable='between(t,{start:.6},{end:.6})'",
        escaped = escaped,
        color = color,
        fs = fs,
        x_expr = x_expr,
        y_expr = y_expr,
        outline = outline,
        bg = bg,
        effect_opts = effect_opts,
        start = t.timeline_start_sec,
        end = t.timeline_start_sec + t.duration_sec,
    )
}

fn build_text_effect_opts(effect: Option<&caprust_core::clip::TextEffect>) -> String {
    use caprust_core::clip::TextEffectKind;
    let Some(e) = effect else {
        return String::new();
    };
    let p = (e.period as f64).max(0.05);
    match e.kind {
        TextEffectKind::Blink => {
            let half = p / 2.0;
            format!(":alpha='if(lt(mod(t,{p:.3}),{half:.3}),1,0)'")
        }
        TextEffectKind::Pulse => {
            let a = (e.amount as f64).clamp(0.0, 1.0);
            let lo = 1.0 - a;
            format!(":alpha='{lo:.3}+{a:.3}*0.5*(1+sin(2*PI*t/{p:.3}))'")
        }
        TextEffectKind::ColorCycle => {
            let k = 2.0 * std::f64::consts::PI / p;
            let ph = 2.0 * std::f64::consts::PI / 3.0;
            format!(
                ":fontcolor_expr=0x%{{eif\\:128+127*sin({k:.6}*t)\\:x\\:2}}%{{eif\\:128+127*sin({k:.6}*t+{ph1:.6})\\:x\\:2}}%{{eif\\:128+127*sin({k:.6}*t+{ph2:.6})\\:x\\:2}}",
                k = k,
                ph1 = ph,
                ph2 = 2.0 * ph,
            )
        }
    }
}

/// Audio counterpart.
#[derive(Debug, Clone)]
pub struct AudioClip {
    pub input_index: usize,
    pub timeline_start_sec: f64,
    pub duration_sec: f64,
    pub speed: f32,
    /// Speed ramp end. Some(x) => audio is split into segments and
    /// atempo'd per segment (see build_speed_ramp_atempo_segments),
    /// matching the video piecewise setpts windows.
    pub speed_end: Option<f32>,
    /// Easing of the ramp. Must match the source clip's speed_ease so
    /// the audio segments land on the same boundaries as the video.
    pub speed_ease: caprust_core::clip::EaseCurve,
    /// Where inside the clip the ramp lives. Must match the source
    /// clip's speed_range for the same reason as speed_ease above.
    pub speed_range: caprust_core::clip::SpeedRampRange,
    /// Linear gain from clip.volume_db.
    pub gain_db: f32,
    /// Per-track volume in dB (Track.volume_db). Applied after
    /// gain_db so the clip automation curve drives the source, and
    /// the track fader scales the result. 0.0 = unity.
    pub track_gain_db: f32,
    /// Fade-in duration in seconds. 0 = no fade.
    pub fade_in_sec: f64,
    /// Fade-out duration in seconds. 0 = no fade.
    pub fade_out_sec: f64,
    /// Volume automation. Empty = use gain_db only. Non-empty =
    /// piecewise-linear in dB between sorted points, held flat before
    /// the first and after the last.
    pub volume_keyframes: Vec<caprust_core::clip::VolumeKeyframe>,
    /// Source clip id, used to resolve duck_against UUIDs to indices
    /// inside this same Vec.
    pub clip_id: uuid::Uuid,
    /// Auto-ducking: source clip id whose audio drives this clip's
    /// sidechain. None = no ducking.
    pub duck_against: Option<uuid::Uuid>,
    /// Apply spectral denoise (ffmpeg `afftdn`) before any gain.
    pub denoise: bool,
    /// Apply speech-tuned dynamic range compression (ffmpeg
    /// `dynaudnorm`). Runs after denoise, before loudnorm.
    pub voice_boost: bool,
    /// Apply EBU R128 loudness normalization (ffmpeg `loudnorm`,
    /// -16 LUFS / -1.5 dBTP / LRA 11). Runs after voice_boost,
    /// before the user's volume / fades so those remain final.
    pub normalize: bool,
}

/// A fully-described render request.
#[derive(Debug, Clone)]
pub struct RenderPlan {
    pub inputs: Vec<InputSpec>,
    pub video_clips: Vec<VideoClip>,
    pub audio_clips: Vec<AudioClip>,
    pub text_clips: Vec<TextClip>,
    pub total_duration_sec: f64,
    pub width: u32,
    pub height: u32,
    pub fps_num: i64,
    pub fps_den: i64,
    /// CRF value for libx264/libx265/libsvtav1.
    pub crf: u8,
    /// Rate-control mode. Vbr uses `crf`; Cbr uses `bitrate_kbps`
    /// and emits `-b:v` / `-maxrate` / `-bufsize` instead.
    pub rate_mode: RateMode,
    /// Target bitrate in kbps for CBR exports. Ignored when
    /// `rate_mode == RateMode::Vbr`.
    pub bitrate_kbps: u32,
    pub preset: String,
    pub has_audio: bool,
    /// Clips the planner had to skip because their source file was
    /// missing. Callers may show a warning toast; the plan is still
    /// valid for the remaining clips.
    pub skipped: PlanSkipped,
    /// Timeline position (ms) that this plan's output starts from.
    /// Video encoder to hand ffmpeg at export time. Preview and the
    /// audio PCM cache ignore it (they use raw output / audio only).
    pub encoder: caprust_core::project::VideoEncoder,
    /// 0 for export and for the audio PCM cache render; the actual
    /// playhead for preview seek-respawns.
    pub seek_ms: u64,
    /// True when the planner rewrote per-input `-ss` / `-t` and
    /// shifted every clip's `timeline_start_sec` to a
    /// seek-relative frame. Callers (preview) can then skip the
    /// output-side `-ss`. False means the caller must fall back to
    /// output-side `-ss`.
    pub seek_optimized: bool,
}

impl RenderPlan {
    /// Build the `-filter_complex` string + `-map` arguments.
    /// Returns (filter_graph, video_label, audio_label).
    pub fn build_filtergraph(&self) -> Result<(String, String, Option<String>)> {
        let mut fg = String::new();

        // -------- VIDEO pipeline --------
        //
        // Two-phase build:
        //
        //  Phase 1 — per clip: decode, trim, scale/pad, fps, effects.
        //             Output: [v_baseN] with local PTS starting at 0.
        //             NO PTS shift yet — the shift is applied at the run
        //             output so that xfade offsets are relative to the
        //             run's own timeline, not the global timeline.
        //
        //  Phase 2 — per z-order track: group adjacent clips into "runs"
        //             where each non-first clip carries a transition_in
        //             id and touches the previous clip within
        //             ADJACENCY_TOL_SEC. Within a run, chain xfade. A
        //             single-clip run is just the phase-1 output with a
        //             PTS shift and, optionally, a fade-from/to-black
        //             edge.
        //
        // Phase 3 — overlay all run outputs on v_base in z-order.

        // ---- Phase 1: base chain per clip ----
        let mut v_base_labels: Vec<String> = Vec::with_capacity(self.video_clips.len());
        for (i, c) in self.video_clips.iter().enumerate() {
            let in_label = format!("[{}:v]", c.input_index);
            let v_out = format!("v_b{i}");

            // setpts with speed ramp: a piecewise-constant
            // approximation of the requested ease curve, sampled at
            // RAMP_SEGMENTS points along the output timeline. See
            // build_speed_ramp_setpts.
            let setpts = match c.speed_end {
                Some(s_end) if (s_end - c.speed).abs() > 0.001 => build_speed_ramp_setpts(
                    c.speed,
                    s_end,
                    c.speed_ease,
                    c.speed_range,
                    c.duration_sec,
                )
                .map(|ramp| format!("setpts=PTS-STARTPTS,{ramp}"))
                .unwrap_or_else(|| {
                    if (c.speed - 1.0).abs() < 0.001 {
                        String::from("setpts=PTS-STARTPTS")
                    } else {
                        format!("setpts=(PTS-STARTPTS)/{:.6}", c.speed)
                    }
                }),
                _ => {
                    if (c.speed - 1.0).abs() < 0.001 {
                        String::from("setpts=PTS-STARTPTS")
                    } else {
                        format!("setpts=(PTS-STARTPTS)/{:.6}", c.speed)
                    }
                }
            };

            // Fit chain: with auto-reframe, crop to the target
            // aspect (with pan) and scale exactly to (w, h). Without
            // it, letterbox/pillarbox into (w, h) as before.
            let fit_chain = match build_auto_reframe_crop(
                &c.auto_reframe,
                self.width,
                self.height,
            ) {
                Some(crop) => format!(
                    "{crop},scale={w}:{h}:flags=bicubic,setsar=1",
                    w = self.width,
                    h = self.height,
                ),
                None => format!(
                    "scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1",
                    w = self.width,
                    h = self.height,
                ),
            };
            // Optional background-removal stage. When a mask is
            // present, the video chain stops at an intermediate label
            // and a maskedmerge over a black base finishes into v_out.
            // Otherwise the chain writes v_out directly.
            let mask_escaped = c.bg_removal_path.as_deref().and_then(escape_movie_path);
            let mask_active = mask_escaped.is_some();
            let video_out = if mask_active {
                format!("v_pre{i}")
            } else {
                v_out.clone()
            };

            // setpts BEFORE trim: when `-ss` is on the input and the
            // ffmpeg build does not reset PTS, frame PTS starts at the
            // seek offset. Resetting first makes `trim=duration` see a
            // 0-based window and actually select frames. Same result
            // for the no-seek case (first PTS is already 0).
            fg.push_str(&format!(
                "{in_label}{setpts},trim=duration={dur:.6},{fit_chain},fps={num}/{den}",
                dur = c.duration_sec,
                num = self.fps_num,
                den = self.fps_den,
            ));
            let effects_chain = build_effects_chain(&c.effects);
            fg.push_str(&effects_chain);
            // Effects like zoom_pulse and shake use
            // `crop=w=iw*0.9:h=ih*0.9,...,scale=iw*1.111111:ih*1.111111`
            // to crop a smaller window then restore it. f32 rounding
            // (576 * 1.111111 = 639.9999...) makes ffmpeg floor to an
            // even dimension (638, 268) instead of exactly 640x270.
            // xfade and maskedmerge both reject differing input sizes
            // ("First input link main parameters ... do not match").
            // Normalize back to the exact project size after every
            // effect chain so downstream stages always see identical
            // dimensions. Skipped when there are no effects, so a
            // clean clip's filtergraph is byte-identical to before.
            if !effects_chain.is_empty() {
                fg.push_str(&format!(
                    ",scale={w}:{h}:flags=bicubic,setsar=1",
                    w = self.width,
                    h = self.height,
                ));
            }
            fg.push_str(&format!("[{video_out}];"));

            if let Some(mask) = mask_escaped {
                // Mask chain: read the FFV1 gray sequence, reset PTS,
                // force the project fps so the merge samples 1:1, and
                // convert to a pixel format maskedmerge accepts.
                // loop=0 holds the last frame if the sequence is
                // shorter than the clip; the colour base below
                // defines the clip's true length.
                let mask_label = format!("m_b{i}");
                let bg_label = format!("bg_b{i}");
                // The mask comes from the extraction pass at a
                // different resolution than the render target, so it
                // must be scaled + letterboxed the same way the video
                // chain was, otherwise maskedmerge rejects the pair
                // with "input link size mismatch".
                fg.push_str(&format!(
                    "movie={mask}:loop=0,setpts=PTS-STARTPTS,fps={num}/{den},format=gray,scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2[{mask_label}];",
                    num = self.fps_num,
                    den = self.fps_den,
                    w = self.width,
                    h = self.height,
                ));
                // Black base at project resolution and fps, matching
                // the clip duration. This is the layer the merge
                // falls back to outside the mask.
                fg.push_str(&format!(
                    "color=c=black:s={w}x{h}:r={num}/{den}:d={dur:.6}[{bg_label}];",
                    w = self.width,
                    h = self.height,
                    num = self.fps_num,
                    den = self.fps_den,
                    dur = c.duration_sec,
                ));
                fg.push_str(&format!(
                    "[{bg_label}][{video_out}][{mask_label}]maskedmerge[{v_out}];",
                ));
            }

            v_base_labels.push(v_out);
        }

        // ---- Phase 2: runs per z-order ----
        //
        // A "run" is a maximal sequence of clips on the same z_order
        // where clip[k] has a valid xfade transition_in AND its start
        // time touches clip[k-1]'s end (within ADJACENCY_TOL_SEC).
        //
        // Clips that don't belong to a multi-clip run become single-
        // clip runs; those keep the legacy fade-from/to-black behaviour
        // when transition_in/out == Some("fade").

        #[derive(Debug)]
        struct Run {
            z_order: u32,
            start_sec: f64,
            /// Indices into self.video_clips.
            members: Vec<usize>,
        }

        let mut by_track: std::collections::BTreeMap<u32, Vec<usize>> =
            std::collections::BTreeMap::new();
        for (i, c) in self.video_clips.iter().enumerate() {
            by_track.entry(c.z_order).or_default().push(i);
        }

        let mut runs: Vec<Run> = Vec::new();
        for (z, mut members) in by_track {
            members.sort_by(|&a, &b| {
                self.video_clips[a]
                    .timeline_start_sec
                    .partial_cmp(&self.video_clips[b].timeline_start_sec)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let mut current: Vec<usize> = Vec::new();
            for &idx in &members {
                if current.is_empty() {
                    current.push(idx);
                    continue;
                }
                let prev = &self.video_clips[*current.last().unwrap()];
                let next = &self.video_clips[idx];
                let prev_end = prev.timeline_start_sec + prev.duration_sec;
                // Positive = overlap (the follower starts before the
                // predecessor ends). The model shifts the follower
                // left by the transition duration when an xfade is
                // attached, so the normal case is a positive
                // overlap. Accept anything from "-tolerance" upward:
                // no gap.
                let overlap = prev_end - next.timeline_start_sec;
                let has_xfade = next
                    .transition_in
                    .as_deref()
                    .map(is_xfade_id)
                    .unwrap_or(false);
                if has_xfade && overlap >= -ADJACENCY_TOL_SEC {
                    current.push(idx);
                } else {
                    runs.push(Run {
                        z_order: z,
                        start_sec: self.video_clips[current[0]].timeline_start_sec,
                        members: std::mem::take(&mut current),
                    });
                    current.push(idx);
                }
            }
            if !current.is_empty() {
                runs.push(Run {
                    z_order: z,
                    start_sec: self.video_clips[current[0]].timeline_start_sec,
                    members: current,
                });
            }
        }

        // ---- Phase 2b: render each run ----
        let mut run_labels: Vec<(u32, f64, String)> = Vec::with_capacity(runs.len());
        // The model already stores follower clips at their shifted
        // positions when an xfade is attached; no per-z bookkeeping
        // is needed here.

        for (ri, run) in runs.iter().enumerate() {
            let out_label = format!("v_run{ri}");
            let effective_start = run.start_sec;

            if run.members.len() == 1 {
                // Single clip: apply legacy fade from/to-black edges
                // when requested, then shift PTS to timeline position.
                let idx = run.members[0];
                let c = &self.video_clips[idx];
                let mut tail = String::new();
                // Clamp fade so it never occupies more than half the
                // clip. On a short clip, a 3 s user setting would
                // otherwise start fading almost at the beginning and
                // read as a bug. Half-clip is what Premiere, Resolve
                // and CapCut all do.
                let fade_d = c.transition_duration_sec.min(c.duration_sec * 0.5);
                // NOTE: the gyan.dev essentials build we test against
                // (2026-01-26) rejects `fade=curve=` with "Option not
                // found". The `curve` parameter was added to the video
                // fade filter only in newer ffmpeg builds. Until the
                // project pins a newer ffmpeg, the fade arms emit
                // plain linear fades; the EaseCurve stored on the
                // clip is preserved in the project file but ignored
                // at render time.
                if c.transition_in.as_deref() == Some("fade") {
                    tail.push_str(&format!(",fade=t=in:st=0:d={fade_d:.3}"));
                }
                if c.transition_out.as_deref() == Some("fade") {
                    let st = (c.duration_sec - fade_d).max(0.0);
                    tail.push_str(&format!(",fade=t=out:st={st:.3}:d={fade_d:.3}"));
                }
                // ffmpeg requires `[label]filter`, never `[label],filter`.
                // Strip the leading comma that all our fragments carry and
                // fall back to a pass-through `null` when no edge filter
                // applies, so the chain is always syntactically valid.
                let tail_clean = tail.trim_start_matches(',');
                let chain = if tail_clean.is_empty() {
                    "null".to_string()
                } else {
                    tail_clean.to_string()
                };
                fg.push_str(&format!(
                    "[{base}]{chain},setpts=PTS+{start:.6}/TB[{out}];",
                    base = v_base_labels[idx],
                    chain = chain,
                    start = effective_start,
                    out = out_label,
                ));
            } else {
                // Xfade chain. Each link uses the requested transition
                // from the SECOND clip of the pair, and a fixed duration
                // of XFADE_DUR_SEC (clamped to half of either clip).
                let mut current_label = v_base_labels[run.members[0]].clone();
                let mut current_dur = self.video_clips[run.members[0]].duration_sec;
                for (k, &idx) in run.members.iter().enumerate().skip(1) {
                    let c = &self.video_clips[idx];
                    let xfade_id = c
                        .transition_in
                        .as_deref()
                        .and_then(xfade_name)
                        .unwrap_or("fade");
                    let d = c
                        .transition_duration_sec
                        .max(XFADE_DUR_SEC)
                        .min(current_dur * 0.5)
                        .min(c.duration_sec * 0.5)
                        .max(0.05);
                    let offset = (current_dur - d).max(0.0);
                    let link_out = format!("v_xf{ri}_{k}");
                    fg.push_str(&format!(
                        "[{a}][{b}]xfade=transition={name}:duration={d:.3}:offset={offset:.6}[{link_out}];",
                        a = current_label,
                        b = v_base_labels[idx],
                        name = xfade_id,
                        d = d,
                        offset = offset,
                        link_out = link_out,
                    ));
                    current_label = link_out;
                    current_dur = current_dur + c.duration_sec - d;
                }
                // Fade-out on the LAST clip of the run. Earlier clips
                // have their end consumed by the next clip's xfade, so
                // only the tail can fade back to black. The user has
                // to have set transition_out on that clip for this to
                // apply.
                let last_idx = *run.members.last().unwrap();
                let last_clip = &self.video_clips[last_idx];
                let tail_out = if last_clip.transition_out.as_deref() == Some("fade") {
                    let fade_d = last_clip
                        .transition_duration_sec
                        .min(last_clip.duration_sec * 0.5);
                    let st = (current_dur - fade_d).max(0.0);
                    format!("fade=t=out:st={st:.3}:d={fade_d:.3}")
                } else {
                    "null".to_string()
                };
                fg.push_str(&format!(
                    "[{current_label}]{tail_out},setpts=PTS+{start:.6}/TB[{out}];",
                    current_label = current_label,
                    tail_out = tail_out,
                    start = effective_start,
                    out = out_label,
                ));
            }

            run_labels.push((run.z_order, effective_start, out_label));
        }

        // ---- Phase 3: black base + overlay in z-order ----
        fg.push_str(&format!(
            "color=c=black:s={w}x{h}:r={num}/{den}:d={dur:.6}[v_base];",
            w = self.width,
            h = self.height,
            num = self.fps_num,
            den = self.fps_den,
            dur = self.total_duration_sec,
        ));

        run_labels.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        });

        let mut v_prev = String::from("v_base");
        for (i, (_z, _start, label)) in run_labels.iter().enumerate() {
            let v_next = format!("v_ov{i}");
            fg.push_str(&format!(
                "[{v_prev}][{label}]overlay=shortest=0:eof_action=pass[{v_next}];",
            ));
            v_prev = v_next;
        }

        // -------- Text overlays (drawtext) --------
        // Each preset tweaks fontcolor, fontsize, box, border, shadow or
        // italic. The base layout (x/y/above/centre) stays the same so
        // switching styles never moves the text off the frame.
        for (t_i, t) in self.text_clips.iter().enumerate() {
            let v_next = format!("v_txt{t_i}");
            // Escape for ffmpeg's filtergraph parser. Single-quote
            // escaping uses the close-reopen idiom: a literal `'`
            // inside a `text='...'` value must be written as `'\\''`
            // (close quote, escaped quote, reopen quote). Writing
            // `\\'` instead makes ffmpeg see the backslash as literal
            // and terminate the string early, which leaks subsequent
            // filter text (enable windows, etc.) into the graph as
            // top-level tokens and produces "No such filter: '<number>'".
            // Filtergraph escaping. `\'` and `'\''` both fail on
            // the BtbN/gyan ffmpeg builds: the first leaks the rest
            // of the option list as a new filter name, the second
            // terminates the string early. The reliable fix is a
            // Unicode substitution: replace ASCII apostrophe with
            // U+2019 RIGHT SINGLE QUOTATION MARK, which is not a
            // filtergraph special char, renders typographically
            // better, and never needs escaping. Same for ASCII
            // double-quote -> curly quotes. Source text in the
            // project file is untouched; substitution is render-time
            // only.
            let escaped = t
                .content
                .replace('\\', "\\\\")
                .replace(':', "\\:")
                .replace('\'', "\u{2019}")
                .replace('"', "\u{201C}");
            // Captions path: user-editable CaptionStyle wins over the
            // preset string. TextOverlay path: classic preset layout.
            if let Some(cs) = t.caption_style.as_ref() {
                let body = build_caption_drawtext_body(t, &escaped, cs);
                fg.push_str(&format!(
                    "[{v_prev}]{body}[v_txt{t_i}];",
                    v_prev = v_prev,
                    body = body,
                    t_i = t_i,
                ));
            } else {
                let y_base = if t.above { "h*0.08" } else { "h*0.82" };
                let mx = t.motion.x;
                let my = t.motion.y;
                let x_expr = if mx.abs() < 1e-4 {
                    "(w-text_w)/2".to_string()
                } else {
                    format!("(w-text_w)/2+({mx:.4})*w")
                };
                let y = if my.abs() < 1e-4 {
                    y_base.to_string()
                } else {
                    format!("{y_base}+({my:.4})*h")
                };
                let fs = ((t.font_size as f64) * (t.motion.scale as f64))
                    .round()
                    .max(1.0) as i32;
                let effect_opts = build_text_effect_opts(t.effect.as_ref());
                // Style parameters. Any combination not matched falls
                // back to plain white text.
                let style_opts: String = match t.style.as_str() {
                    "bold" => ":borderw=2:bordercolor=black".into(),
                    "subtitle" => ":box=1:boxcolor=black@0.5:boxborderw=8".into(),
                    "lower" => ":box=1:boxcolor=black@0.7:boxborderw=6".into(),
                    "quote" => ":italics=1:shadowcolor=black@0.6:shadowx=3:shadowy=3".into(),
                    "caption" => ":fontcolor=yellow:borderw=2:bordercolor=black".into(),
                    "glow" => {
                        ":shadowcolor=cyan@0.8:shadowx=4:shadowy=4:borderw=1:bordercolor=white"
                            .into()
                    }
                    "handwrite" => ":font='Comic Sans MS'".into(),
                    _ => String::new(),
                };
                fg.push_str(&format!(
                "[{v_prev}]drawtext=text='{escaped}':fontcolor=white:fontsize={fs}:x={x_expr}:y={y}{style_opts}{effect_opts}:enable='between(t,{start:.6},{end:.6})'[v_txt{t_i}];",
                v_prev = v_prev,
                escaped = escaped,
                fs = fs,
                x_expr = x_expr,
                y = y,
                style_opts = style_opts,
                effect_opts = effect_opts,
                start = t.timeline_start_sec,
                end = t.timeline_start_sec + t.duration_sec,
                t_i = t_i,
            ));
            }
            v_prev = v_next;
        }
        let v_final = v_prev;

        // -------- AUDIO (unchanged) --------
        let a_final = self.build_audio_chain(&mut fg);

        if fg.ends_with(';') {
            fg.pop();
        }

        Ok((fg, v_final, a_final))
    }

    /// Append the audio pipeline to `fg`, returning the output label
    /// (`a_final`) or `None` when there is no audio to mix.
    ///
    /// Extracted from `build_filtergraph` so the PCM cache can render
    /// audio alone without pulling video inputs into the filtergraph.
    fn build_audio_chain(&self, fg: &mut String) -> Option<String> {
        if !self.has_audio || self.audio_clips.is_empty() {
            return None;
        }
        let mut a_labels: Vec<String> = Vec::with_capacity(self.audio_clips.len());
        for (i, c) in self.audio_clips.iter().enumerate() {
            let in_label = format!("[{}:a]", c.input_index);
            let a_out = format!("a{i}_trim");
            // Speed chain. If a ramp applies, split the source into
            // N segments that match the video setpts windows and
            // atempo each one, then concat. Otherwise use a single
            // atempo for the static speed. The segmented path makes
            // audio and video consume identical source windows per
            // segment, so they cannot drift inside a ramp.
            let ramped = c
                .speed_end
                .filter(|s_end| (s_end - c.speed).abs() > 0.001)
                .and_then(|s_end| {
                    compute_speed_ramp_segments(
                        c.speed,
                        s_end,
                        c.speed_ease,
                        c.speed_range,
                        c.duration_sec,
                    )
                });

            let (ramp_preamble, pre_gain_label) = match ramped {
                Some(segs) => {
                    let pre = format!("a{i}_pre");
                    let frag =
                        build_speed_ramp_atempo_segments(&in_label, &pre, &segs, &format!("a{i}r"));
                    (frag, format!("[{pre}]"))
                }
                None => {
                    let atempo = atempo_chain(c.speed);
                    (
                        format!(
                            "{in_label}asetpts=PTS-STARTPTS,atrim=duration={dur:.6}{atempo}[a{i}_pre];",
                            dur = c.duration_sec,
                            atempo = atempo,
                            i = i,
                        ),
                        format!("[a{i}_pre]"),
                    )
                }
            };
            // Volume: an automation curve overrides the static
            // gain_db. Piecewise-linear in dB between sorted
            // keyframes, held flat before the first and after the
            // last. ffmpeg's volume filter evaluates the expression
            // per frame with eval=frame.
            let gain = if !c.volume_keyframes.is_empty() {
                let kfs = &c.volume_keyframes;
                let expr = if kfs.len() == 1 {
                    format!("{:.4}", kfs[0].gain_db)
                } else {
                    let mut e = format!("{:.4}", kfs.last().unwrap().gain_db);
                    for i in (0..kfs.len() - 1).rev() {
                        let a = &kfs[i];
                        let b = &kfs[i + 1];
                        let ta = a.t_ms as f64 / 1000.0;
                        let tb = b.t_ms as f64 / 1000.0;
                        let dt = (tb - ta).max(0.0001);
                        let seg = format!(
                            "{:.4}+({:.4})*(t-{:.4})/{:.4}",
                            a.gain_db,
                            b.gain_db - a.gain_db,
                            ta,
                            dt
                        );
                        e = format!(
                            "if(lt(t\\,{ta:.4})\\,{ga:.4}\\,if(lt(t\\,{tb:.4})\\,{seg}\\,{e}))",
                            ta = ta,
                            ga = a.gain_db,
                            tb = tb,
                            seg = seg,
                            e = e,
                        );
                    }
                    e
                };
                format!(",volume={expr}dB:eval=frame")
            } else if c.gain_db.abs() < 0.001 {
                String::new()
            } else {
                format!(",volume={:.4}dB", c.gain_db)
            };
            // Per-track fader. Applied after the clip stage so the
            // clip's volume automation drives the source, and the
            // track scales the resulting signal. Unity = 0 dB =
            // skip entirely, so projects with no track fader set
            // produce the same filtergraph as before.
            let track_gain = if c.track_gain_db.abs() < 0.001 {
                String::new()
            } else {
                format!(",volume={:.4}dB", c.track_gain_db)
            };
            // afade at the end of the chain so the gain is applied
            // first (fade ramps the already-gained signal).
            let fade_in = if c.fade_in_sec > 0.001 {
                format!(",afade=t=in:st=0:d={:.6}", c.fade_in_sec)
            } else {
                String::new()
            };
            let fade_out = if c.fade_out_sec > 0.001 {
                let st = (c.duration_sec - c.fade_out_sec).max(0.0);
                format!(",afade=t=out:st={st:.6}:d={:.6}", c.fade_out_sec)
            } else {
                String::new()
            };
            // Audio processing stages. Order: denoise cleans the
            // source, voice_boost levels speech, then normalize
            // hits a target loudness. All three run BEFORE the
            // user's volume / track fader / fades so those remain
            // the final say on the clip's loudness envelope.
            let denoise = if c.denoise {
                ",afftdn=nf=-25".to_string()
            } else {
                String::new()
            };
            let voice_boost = if c.voice_boost {
                ",dynaudnorm=p=0.9:m=10".to_string()
            } else {
                String::new()
            };
            let normalize = if c.normalize {
                ",loudnorm=I=-16:TP=-1.5:LRA=11".to_string()
            } else {
                String::new()
            };
            fg.push_str(&ramp_preamble);
            // Post-chain: processing, gain, fades. If all are
            // empty, pass through with `anull` so the chain is
            // always valid.
            let tail =
                format!("{denoise}{voice_boost}{normalize}{gain}{track_gain}{fade_in}{fade_out}");
            let tail_clean = tail.trim_start_matches(',');
            let tail_chain = if tail_clean.is_empty() {
                "anull"
            } else {
                tail_clean
            };
            fg.push_str(&format!("{pre_gain_label}{tail_chain}[{a_out}];"));
            a_labels.push(a_out);
        }

        fg.push_str(&format!(
            "anullsrc=channel_layout=stereo:sample_rate=48000:d={dur:.6}[a_base];",
            dur = self.total_duration_sec,
        ));

        // ---- Delay every clip to its timeline position ----
        for (i, c) in self.audio_clips.iter().enumerate() {
            let delay_ms = (c.timeline_start_sec * 1000.0).round() as i64;
            fg.push_str(&format!(
                "[{clip}]adelay={delay}|{delay}[a_delayed{i}];",
                clip = a_labels[i],
                delay = delay_ms,
                i = i,
            ));
        }

        // ---- Auto-ducking (sidechaincompress) ----
        // Resolve duck_against UUIDs to indices inside audio_clips,
        // dropping self-references. A control clip is any clip that
        // is the target of at least one duck. Its delayed stream is
        // mixed into a single control bus, which is then asplit for
        // each consumer.
        let id_to_idx: std::collections::HashMap<uuid::Uuid, usize> = self
            .audio_clips
            .iter()
            .enumerate()
            .map(|(i, c)| (c.clip_id, i))
            .collect();

        // (ducked_idx -> ctrl_idx)
        let duck_map: std::collections::HashMap<usize, usize> = self
            .audio_clips
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                c.duck_against
                    .and_then(|u| id_to_idx.get(&u).copied())
                    .filter(|&x| x != i)
                    .map(|x| (i, x))
            })
            .collect();

        let mut mixed_labels: Vec<String> = (0..self.audio_clips.len())
            .map(|i| format!("a_delayed{i}"))
            .collect();

        if !duck_map.is_empty() {
            // Control bus inputs: all distinct ctrl_idx values.
            let mut ctrl_indices: Vec<usize> = duck_map.values().copied().collect();
            ctrl_indices.sort_unstable();
            ctrl_indices.dedup();

            let ctrl_bus = "a_ctrl_in";
            if ctrl_indices.len() == 1 {
                // Single control: alias via anull (pass-through).
                fg.push_str(&format!(
                    "[a_delayed{idx}]anull[{bus}];",
                    idx = ctrl_indices[0],
                    bus = ctrl_bus,
                ));
            } else {
                // Single amix over every control input. The
                // previous sequential 2-input chain kept the
                // default normalize=1, which attenuates the first
                // input by 1/2^N and the last by 1/2. normalize=0
                // keeps every contribution at its original level.
                let mut ctrl_inputs = String::new();
                for &idx in &ctrl_indices {
                    ctrl_inputs.push_str(&format!("[a_delayed{idx}]"));
                }
                let n_ctrl = ctrl_indices.len();
                fg.push_str(&format!(
                    "{}amix=inputs={}:duration=longest:dropout_transition=0:normalize=0[{}];",
                    ctrl_inputs, n_ctrl, ctrl_bus,
                ));
            }

            // asplit the control bus once per consumer.
            let n_consumers = duck_map.len();
            if n_consumers == 1 {
                let (ducked_idx, _) = duck_map.iter().next().map(|(a, b)| (*a, *b)).unwrap();
                let out = format!("a_ducked{ducked_idx}");
                fg.push_str(&format!(
                    "[{ctrl_bus}][a_delayed{ducked_idx}]sidechaincompress=threshold=0.05:ratio=8:attack=20:release=500[{out}];",
                ));
                mixed_labels[ducked_idx] = out;
            } else {
                // Split into N copies.
                let mut split = format!("[{ctrl_bus}]asplit={n_consumers}");
                for k in 0..n_consumers {
                    split.push_str(&format!("[a_ctrl_k{k}]"));
                }
                split.push(';');
                fg.push_str(&split);

                for (k, (ducked_idx, _)) in duck_map.iter().enumerate() {
                    let out = format!("a_ducked{ducked_idx}");
                    fg.push_str(&format!(
                        "[a_delayed{ducked_idx}][a_ctrl_k{k}]sidechaincompress=threshold=0.05:ratio=8:attack=20:release=500[{out}];",
                    ));
                    mixed_labels[*ducked_idx] = out;
                }
            }
        }

        // ---- Final mix ----
        // One amix with every input (base + all clips), normalize=0.
        // The previous sequential 2-input chain with the default
        // normalize=1 attenuated the first clip by 1/2^N and the
        // last by 1/2. On a 17-clip project that made the opening
        // clip effectively silent while the closing clip played at
        // -6 dB -- audible as a slow fade-in across the whole
        // timeline. A single amix also drops CPU: one mixer
        // instead of N, no per-stage buffering.
        let mut final_inputs = String::from("[a_base]");
        for lbl in &mixed_labels {
            final_inputs.push_str(&format!("[{lbl}]"));
        }
        let n_final = self.audio_clips.len() + 1;
        fg.push_str(&format!(
            "{}amix=inputs={}:duration=longest:dropout_transition=0:normalize=0[a_mix_joined];",
            final_inputs, n_final,
        ));
        fg.push_str(&format!(
            "[a_mix_joined]atrim=duration={dur:.6},asetpts=PTS-STARTPTS[a_final]",
            dur = self.total_duration_sec,
        ));
        Some(String::from("a_final"))
    }

    /// Build only the audio pipeline. No video inputs are referenced,
    /// so the caller must not `-map` a video stream. The single output
    /// label is `[a_final]`.
    ///
    /// Returns `Ok(None)` when the plan has no audio to mix.
    pub fn build_audio_only_filtergraph(&self) -> Result<Option<String>> {
        let mut fg = String::new();
        let a = self.build_audio_chain(&mut fg);
        if a.is_none() {
            return Ok(None);
        }
        if fg.ends_with(';') {
            fg.pop();
        }
        Ok(Some(fg))
    }

    /// Full ffmpeg argument list.
    pub fn build_command(
        &self,
        _ffmpeg: &std::path::Path,
        output: &std::path::Path,
    ) -> Vec<String> {
        let mut args: Vec<String> = vec![
            "-y".into(),
            "-hide_banner".into(),
            "-loglevel".into(),
            "info".into(),
            "-progress".into(),
            "pipe:2".into(),
        ];

        // Image inputs get `-loop 1` + `-framerate` so ffmpeg treats them as video.
        let image_indices: std::collections::HashSet<usize> = self
            .video_clips
            .iter()
            .filter(|c| c.is_image)
            .map(|c| c.input_index)
            .collect();

        // Inputs, in order. `-ss` before `-i` (fast seek), `-t` after (read window).
        for inp in &self.inputs {
            if image_indices.contains(&inp.ffmpeg_index) {
                args.push("-loop".into());
                args.push("1".into());
                args.push("-framerate".into());
                args.push(format!("{}/{}", self.fps_num, self.fps_den));
            }
            args.push("-ss".into());
            args.push(format!("{:.6}", inp.source_start_sec));
            args.push("-t".into());
            args.push(format!("{:.6}", inp.duration_sec));
            args.push("-protocol_whitelist".into());
            args.push("file".into());
            args.push("-i".into());
            args.push(inp.path.to_string_lossy().to_string());
        }

        let (fg, v_label, a_label) = match self.build_filtergraph() {
            Ok(x) => x,
            Err(_) => return args, // caller will surface the error
        };
        args.push("-filter_complex".into());
        args.push(fg);

        // Maps
        args.push("-map".into());
        args.push(format!("[{v_label}]"));
        if let Some(a) = &a_label {
            args.push("-map".into());
            args.push(format!("[{a}]"));
        }

        // Encoder. `crf` and `preset` are interpreted per family:
        //   - CPU x264/x265:   -preset <name> -crf <q>
        //   - CPU libsvtav1:   -preset <0-13> -crf <q>
        //   - NVENC:           -preset p<n> -tune hq -rc vbr -cq <q> -b:v 0
        //   - AMF:             -quality balanced -rc cqp -qp_i <q> -qp_p <q+2>
        // HEVC and AV1 get an mp4 tag so players recognise the stream.
        use caprust_core::project::VideoEncoder as Enc;
        args.push("-c:v".into());
        args.push(self.encoder.ffmpeg_id().into());
        match self.encoder {
            Enc::H264Cpu | Enc::H265Cpu => {
                args.push("-preset".into());
                args.push(self.preset.clone());
                if self.rate_mode == RateMode::Cbr {
                    push_cbr_args(&mut args, self.bitrate_kbps);
                } else {
                    args.push("-crf".into());
                    args.push(self.crf.to_string());
                }
            }
            Enc::Av1Cpu => {
                // libsvtav1 wants an integer preset; 8 is the default
                // (roughly medium). The string mapping does not apply.
                args.push("-preset".into());
                args.push("8".into());
                if self.rate_mode == RateMode::Cbr {
                    push_cbr_args(&mut args, self.bitrate_kbps);
                } else {
                    args.push("-crf".into());
                    args.push(self.crf.to_string());
                }
            }
            Enc::H264Nvenc | Enc::H265Nvenc | Enc::Av1Nvenc => {
                args.push("-preset".into());
                args.push(nvenc_preset(&self.preset).into());
                args.push("-tune".into());
                args.push("hq".into());
                if self.rate_mode == RateMode::Cbr {
                    args.push("-rc".into());
                    args.push("cbr".into());
                    args.push("-b:v".into());
                    args.push(format!("{}k", self.bitrate_kbps));
                } else {
                    args.push("-rc".into());
                    args.push("vbr".into());
                    args.push("-cq".into());
                    args.push(self.crf.to_string());
                    args.push("-b:v".into());
                    args.push("0".into());
                }
            }
            Enc::H264Amf | Enc::H265Amf | Enc::Av1Amf => {
                args.push("-quality".into());
                args.push("balanced".into());
                if self.rate_mode == RateMode::Cbr {
                    args.push("-rc".into());
                    args.push("cbr".into());
                    args.push("-b:v".into());
                    args.push(format!("{}k", self.bitrate_kbps));
                } else {
                    args.push("-rc".into());
                    args.push("cqp".into());
                    args.push("-qp_i".into());
                    args.push(self.crf.to_string());
                    args.push("-qp_p".into());
                    args.push((self.crf as u32 + 2).min(51).to_string());
                }
            }
        }
        if matches!(self.encoder, Enc::H265Cpu | Enc::H265Nvenc | Enc::H265Amf) {
            args.push("-tag:v".into());
            args.push("hvc1".into());
        }
        if matches!(self.encoder, Enc::Av1Cpu | Enc::Av1Nvenc | Enc::Av1Amf) {
            args.push("-tag:v".into());
            args.push("av01".into());
        }
        args.push("-pix_fmt".into());
        args.push("yuv420p".into());

        if a_label.is_some() {
            args.push("-c:a".into());
            args.push("aac".into());
            args.push("-b:a".into());
            args.push("192k".into());
        }

        // Faststart for MP4
        args.push("-movflags".into());
        args.push("+faststart".into());

        args.push(output.to_string_lossy().to_string());
        args
    }
}

/// Two clips are considered adjacent for xfade purposes when their
/// timeline gap is under this many seconds. Matches audio and video
/// seams that the user placed by dragging; leaves room for rounding.
pub const ADJACENCY_TOL_SEC: f64 = 0.05;

/// Fixed crossfade duration for MVP. Clamped down to half of the
/// shorter adjacent clip so very short clips still get a transition.
pub const XFADE_DUR_SEC: f64 = 0.5;

/// True if `id` is a transition we know how to hand to xfade.
pub fn is_xfade_id(id: &str) -> bool {
    xfade_name(id).is_some()
}

/// Map an EaseCurve variant to the closest ffmpeg `fade=curve=`
/// parameter value. Returns None for Linear because the default
/// curve (tri) is already correct and the older code path omits
/// the parameter entirely -- keeping the byte layout of the
/// filtergraph unchanged for projects that do not use easing.
/// Kept for the day we pin a newer ffmpeg that has `fade=curve=`.
/// The current gyan.dev essentials build (2026-01-26) does not.
#[allow(dead_code)]
fn ease_curve_name(e: caprust_core::clip::EaseCurve) -> Option<&'static str> {
    use caprust_core::clip::EaseCurve;
    match e {
        EaseCurve::Linear => None,
        // ffmpeg's fade filter names are abbreviated: cub (not cubic),
        // qsin (quarter of a sine wave) not sin, par is exactly par.
        // A wrong value here makes ffmpeg refuse the whole filtergraph
        // with "Error applying option 'curve' to filter 'fade'".
        EaseCurve::EaseIn => Some("cub"),
        EaseCurve::EaseOut => Some("par"),
        EaseCurve::EaseInOut => Some("qsin"),
    }
}

/// Map a preset id to the ffmpeg `xfade=transition=...` keyword.
pub fn xfade_name(id: &str) -> Option<&'static str> {
    Some(match id {
        "fade" => "fade",
        "slide_l" => "slideleft",
        "slide_r" => "slideright",
        "slide_u" => "slideup",
        "slide_d" => "slidedown",
        "wipe_l" => "wipeleft",
        "wipe_r" => "wiperight",
        "zoom_in" => "circleopen",
        "zoom_out" => "circleclose",
        "rotate" => "radial",
        "blur_t" => "fadeblack",
        _ => return None,
    })
}

/// Translate a list of effect instances into an ffmpeg filter chain.
/// Each fragment begins with `,` and appends to the previous stage.
///
/// `amount` semantics per effect:
/// - Static effects (blur, vignette, chroma shift, ...): scales the
///   magnitude of the primary parameter. Clamped to a sane range so a
///   stray amount=100 does not make ffmpeg reject the filtergraph.
/// - Animated effects (shake, zoom_pulse): controls amplitude AND
///   frequency, per the J1 spec — a larger amount makes the motion
///   both wider and slower, which reads as "more dramatic".
///
/// `enabled == false` skips the effect entirely; the UI can toggle a
/// stage without removing it from the stack.
fn build_effects_chain(effects: &[caprust_core::clip::EffectInstance]) -> String {
    let mut out = String::new();
    for inst in effects {
        if !inst.enabled {
            continue;
        }
        // Clamp amount to a well-behaved range; UI slider is 0..=2 but
        // serialized projects might carry values from older builds.
        let amount = inst.amount.clamp(0.0, 4.0);
        let frag = build_one_effect(&inst.effect_id, amount);
        if let Some(f) = frag {
            out.push_str(&f);
        }
    }
    out
}

/// Single-effect fragment. Kept separate from the loop so tests can
/// assert on individual chains without constructing a whole slice.
fn build_one_effect(id: &str, amount: f32) -> Option<String> {
    let frag: String = match id {
        // ---- Static effects: amount scales magnitude ----
        "blur" => format!(",boxblur={:.2}:2", 4.0 * amount),
        "vignette" => format!(",vignette=PI/{:.2}", (5.0 / amount.max(0.2)).max(0.5)),
        "sepia" => ",colorchannelmixer=.393:.769:.189:0:.349:.686:.168:0:.272:.534:.131".into(),
        "bw" => ",hue=s=0".into(),
        "glitch" => format!(
            ",chromashift=cbh={cb:.1}:crh=-{cr:.1},format=yuv420p",
            cb = 4.0 * amount,
            cr = 4.0 * amount,
        ),
        "rgb_split" => format!(
            ",chromashift=cbh={cb:.1}:crh=-{cr:.1},format=yuv420p",
            cb = 6.0 * amount,
            cr = 6.0 * amount,
        ),
        "flash" => format!(
            ",eq=brightness={:.3}:contrast={:.3}",
            0.15 * amount,
            1.0 + 0.1 * amount
        ),
        "mirror" => ",hflip".into(),
        "kaleido" => ",vflip,hflip".into(),
        "old_film" => ",curves=preset=vintage,noise=alls=15:allf=t".into(),
        "vintage" => format!(
            ",curves=preset=vintage,colorbalance=rm={rm:.3}:gm={gm:.3}:bm={bm:.3}",
            rm = 0.08 * amount,
            gm = 0.02 * amount,
            bm = -0.06 * amount,
        ),
        "vhs" => format!(
            ",chromashift=cbh=2:crh=-2,noise=alls={s:.0}:allf=t,eq=brightness=0.06:contrast=1.05,format=yuv420p",
            s = 8.0 * amount,
        ),
        "light_leak" => format!(
            ",colorbalance=rm={:.3}:gm={:.3}",
            0.15 * amount,
            0.05 * amount
        ),

        // ---- Warm bloom: lens_flare ----
        // A true optical flare needs a sprite or a split+blend pair,
        // which the single-chain contract here cannot express. We
        // approximate the look instead: push highlights toward amber,
        // lift saturation, and bleed highlights via gblur so bright
        // areas bloom outward. Reads as "sunlight entering the lens".
        "lens_flare" => format!(
            ",colorbalance=rm={rm:.3}:gm={gm:.3}:bm={bm:.3},curves=preset=lighter,eq=saturation={sat:.3},gblur=sigma={s:.2}",
            rm = 0.22 * amount,
            gm = 0.10 * amount,
            bm = -0.06 * amount,
            sat = 1.0 + 0.12 * amount,
            s = (1.5 + 4.0 * amount).clamp(1.0, 10.0),
        ),
        "warm" => format!(
            ",colorbalance=rm={:.3}:bm=-{:.3}",
            0.1 * amount,
            0.1 * amount
        ),
        "cool" => format!(
            ",colorbalance=bm={:.3}:rm=-{:.3}",
            0.1 * amount,
            0.1 * amount
        ),
        "cinematic" => ",curves=preset=strong_contrast,colorbalance=bm=0.1".into(),
        "vivid" => format!(
            ",eq=saturation={:.3}:contrast={:.3}",
            1.0 + 0.4 * amount,
            1.0 + 0.05 * amount
        ),
        "matte" => format!(
            ",eq=saturation={:.3}:brightness={:.3}:contrast={:.3}",
            (1.0 - 0.15 * amount).max(0.0),
            -0.02 * amount,
            1.0 + 0.05 * amount
        ),
        "noir" => ",hue=s=0,curves=preset=strong_contrast".into(),
        "sunset" => format!(
            ",colorbalance=rm={:.3}:gm={:.3}:bm=-{:.3}",
            0.2 * amount,
            0.05 * amount,
            0.15 * amount
        ),
        "ocean" => format!(
            ",colorbalance=bm={:.3}:gm={:.3}:rm=-{:.3}",
            0.2 * amount,
            0.05 * amount,
            0.15 * amount
        ),
        "fade" => format!(",fade=t=in:st=0:d={:.3}", 0.6 * amount.max(0.1)),
        "pastel" => format!(
            ",eq=saturation={:.3}:brightness={:.3}",
            (1.0 - 0.25 * amount).max(0.0),
            0.05 * amount
        ),
        "neon" => format!(
            ",eq=saturation={:.3}:contrast={:.3}",
            1.0 + 0.6 * amount,
            1.0 + 0.15 * amount
        ),
        "gold" => format!(
            ",colorbalance=rm={:.3}:gm={:.3}:bm=-{:.3}",
            0.15 * amount,
            0.1 * amount,
            0.1 * amount
        ),

        // ---- Temporal: ghost ----
        // tmix blends N consecutive frames into the current one,
        // producing motion trails. frames scales with amount but is
        // clamped: a large value holds many full frames in memory
        // (roughly w*h*4 bytes per frame), and 4K@60 with 30 frames
        // would be hundreds of MB.
        //
        // Uniform weights so the trail fades linearly; a non-uniform
        // ramp would darken the whole image more than the tail.
        // ---- Temporal: ghost ----
        // tmix blends N consecutive frames into the current one,
        // producing motion trails. frames scales with amount but is
        // clamped: a large value holds many full frames in memory
        // (roughly w*h*4 bytes per frame), and 4K@60 with 30 frames
        // would be hundreds of MB.
        //
        // Uniform weights so the trail fades linearly; a non-uniform
        // ramp would darken the whole image more than the tail.
        "ghost" => {
            let frames = ((2.0 + 6.0 * amount).round() as i32).clamp(2, 12);
            let weights = (0..frames).map(|_| "1").collect::<Vec<_>>().join(" ");
            format!(",tmix=frames={frames}:weights='{weights}'")
        }

        // ---- Temporal + spatial: particle ----
        // No sprite system in ffmpeg, so we approximate atmospheric
        // dust / snow with a temporal noise field softened by boxblur
        // (noise -> soft orbs instead of sharp specks) and given short
        // motion trails via tmix. Distinct from sparkle, which keeps
        // the noise sharp and bright and skips the blur/trail stages.
        "particle" => {
            let s = (15.0 + 35.0 * amount).clamp(12.0, 55.0);
            let b = (0.5 + 1.5 * amount).clamp(0.3, 3.0);
            let frames = ((2.0 + 3.0 * amount).round() as i32).clamp(2, 6);
            let weights = (0..frames).map(|_| "1").collect::<Vec<_>>().join(" ");
            format!(
                ",noise=alls={s:.0}:allf=t,boxblur={b:.2}:1,eq=brightness={br:.3}:contrast={c:.3},tmix=frames={frames}:weights='{weights}'",
                s = s,
                b = b,
                br = 0.05 * amount,
                c = 1.0 - 0.05 * amount,
                frames = frames,
                weights = weights,
            )
        }

        // ---- Temporal + spatial: sparkle ----
        // ffmpeg has no particle system, so this approximates glittering
        // as temporal noise over a slightly brightened base. It reads as
        // "grain with bright specks" rather than a real sprite system,
        // but it is cheap, deterministic, and asset-free.
        //
        // A sprite-based sparkle is on the Asset-based effects backlog
        // (DIRECTIVES §18, phase S).
        "sparkle" => format!(
            ",noise=c0s={s:.0}:allf=t,eq=brightness={b:.3}:contrast={c:.3}",
            s = (15.0 + 35.0 * amount).clamp(10.0, 60.0),
            b = 0.03 * amount,
            c = 1.0 + 0.05 * amount,
        ),

        // ---- Animated: shake ----
        // Smooth, deterministic motion via sin/cos of `t`. The old
        // `random()` version jittered per frame but was not time-based,
        // so it looked like noise rather than a shake.
        //
        // Amplitude: 4 px * amount (clamped). Frequency: fixed at 8 Hz
        // so it stays perceptible across the amount range; "larger
        // amount = slower" is reserved for zoom_pulse.
        "shake" => {
            let amp = (4.0 * amount).clamp(0.5, 16.0);
            // crop w/h = iw-2*amp, i h-2*amp, offset oscillates in [-amp, amp].
            format!(
                ",crop=iw-{w}:ih-{h}:{ax}+{amp:.2}*sin(8*t*PI):{ay}+{amp:.2}*cos(8*t*PI)",
                w = 2.0 * amp,
                h = 2.0 * amp,
                ax = amp,
                ay = amp,
                amp = amp,
            )
        }

        // ---- Animated: zoom_pulse ----
        // A slow breathing zoom. Implemented with crop+scale rather than
        // zoompan because zoompan changes the output frame count and we
        // are inside a per-clip chain that already has a fixed fps.
        //
        // Amount controls both amplitude (zoom depth) and frequency:
        //   amount 0.5  -> ~4px amplitude, ~1.5 Hz
        //   amount 1.0  -> ~8px amplitude, ~1.0 Hz
        //   amount 2.0  -> ~16px amplitude, ~0.7 Hz
        //
        // Crop is centered (offset -(A/2) from each side), so the visible
        // window does not drift.
        "zoom_pulse" => {
            // Breathing crop: a fixed inner window (90% of the source)
            // whose x/y drift on a slow sine/cosine. Reads as a soft
            // pulse / handheld breathe.
            //
            // Why not a true zoom? ffmpeg's `crop` only re-evaluates
            // x/y per frame; w/h are fixed at startup, so the old
            // w='iw-A*(1+sin(t))/2' attempt produced a degenerate
            // crop and froze the preview. `zoompan` would work but its
            // s= option requires a concrete pixel size, and
            // build_one_effect runs without dimensions. Until the
            // fragment API carries w/h, breathing crop is the honest
            // approximation.
            //
            // amp and freq still scale with amount: more amount =
            // wider drift + slower pulse.
            let amp = (6.0 * amount).clamp(1.5, 20.0);
            let freq = (1.0 / amount.max(0.25)).clamp(0.3, 3.0);
            format!(
                ",crop=w=iw*0.9:h=ih*0.9:x='(iw-ow)/2+{amp:.2}*sin(2*PI*t*{freq:.3})':y='(ih-oh)/2+{amp:.2}*cos(2*PI*t*{freq:.3})',scale=iw*1.111111:ih*1.111111:flags=bicubic"
            )
        }

        _ => return None,
    };
    Some(frag)
}

/// Normalised easing progress in [0, 1]. `t` is linear progress in
/// [0, 1]; returns the eased value.
fn ease_progress(t: f64, ease: caprust_core::clip::EaseCurve) -> f64 {
    use caprust_core::clip::EaseCurve;
    let t = t.clamp(0.0, 1.0);
    match ease {
        EaseCurve::Linear => t,
        EaseCurve::EaseIn => t * t,
        EaseCurve::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
        EaseCurve::EaseInOut => {
            if t < 0.5 {
                2.0 * t * t
            } else {
                1.0 - 2.0 * (1.0 - t) * (1.0 - t)
            }
        }
    }
}

/// Time-average of the easing progress over [0, 1]. Used to estimate
/// the effective average speed of a ramp.
fn ease_avg_progress(ease: caprust_core::clip::EaseCurve) -> f64 {
    use caprust_core::clip::EaseCurve;
    match ease {
        EaseCurve::Linear => 0.5,
        EaseCurve::EaseIn => 1.0 / 3.0,
        EaseCurve::EaseOut => 2.0 / 3.0,
        EaseCurve::EaseInOut => 0.5,
    }
}

/// One piecewise-constant segment of a speed ramp, expressed in both
/// INPUT and OUTPUT time (relative to clip start).
///
/// Both the video setpts builder and the audio atempo segmenter sample
/// the same segments, so the two sides consume identical source
/// windows at identical speeds. That is what removes the mid-ramp
/// A/V drift the previous ease-weighted-mean approximation had.
#[derive(Debug, Clone, Copy)]
struct RampSegment {
    /// Segment start in INPUT seconds, relative to clip start.
    t_in_start: f64,
    /// Segment end in INPUT seconds, relative to clip start.
    t_in_end: f64,
    /// Segment start in OUTPUT seconds, relative to clip start.
    t_out_start: f64,
    /// Constant speed used inside this segment.
    speed: f64,
}

/// Compute the piecewise segments of a speed ramp. Returns None when
/// no ramp applies (speed_end absent or equal to speed, or the clip
/// has zero duration).
///
/// The ramp is approximated by RAMP_SEGMENTS constant-speed intervals
/// sampled at their midpoints along the OUTPUT timeline. Each segment
/// consumes `speed * dt_out` seconds of INPUT and produces `dt_out`
/// seconds of OUTPUT, so input time tiles contiguously. Both the
/// video and audio chains read this same list.
fn compute_speed_ramp_segments(
    speed: f32,
    speed_end: f32,
    ease: caprust_core::clip::EaseCurve,
    range: caprust_core::clip::SpeedRampRange,
    clip_in_dur_s: f64,
) -> Option<Vec<RampSegment>> {
    use caprust_core::clip::SpeedRampRange;

    const RAMP_SEGMENTS: usize = 4;

    let s0 = speed as f64;
    let s1 = speed_end as f64;
    if (s1 - s0).abs() < 0.001 || clip_in_dur_s <= 0.0 {
        return None;
    }

    let s_ramp_avg = s0 + (s1 - s0) * ease_avg_progress(ease);

    // Ramp duration in OUTPUT seconds and total OUTPUT duration.
    let (ramp_out_dur, total_out_dur) = match range {
        SpeedRampRange::WholeClip => {
            let d = clip_in_dur_s / s_ramp_avg.max(0.01);
            (d, d)
        }
        SpeedRampRange::FirstN(n_ms) => {
            let n = (n_ms as f64 / 1000.0).max(0.0);
            let ramp_in = s_ramp_avg * n;
            let const_in = (clip_in_dur_s - ramp_in).max(0.0);
            let const_out = const_in / s1.max(0.01);
            (n, n + const_out)
        }
        SpeedRampRange::LastN(n_ms) => {
            let n = (n_ms as f64 / 1000.0).max(0.0);
            let ramp_in = s_ramp_avg * n;
            let const_in = (clip_in_dur_s - ramp_in).max(0.0);
            let const_out = const_in / s0.max(0.01);
            (n, n + const_out)
        }
    };

    // Speed at output time t.
    let s_at = |t_out: f64| -> f64 {
        match range {
            SpeedRampRange::WholeClip => {
                let u = (t_out / total_out_dur).clamp(0.0, 1.0);
                s0 + (s1 - s0) * ease_progress(u, ease)
            }
            SpeedRampRange::FirstN(_) => {
                if t_out < ramp_out_dur {
                    let u = (t_out / ramp_out_dur).clamp(0.0, 1.0);
                    s0 + (s1 - s0) * ease_progress(u, ease)
                } else {
                    s1
                }
            }
            SpeedRampRange::LastN(_) => {
                let ramp_start = (total_out_dur - ramp_out_dur).max(0.0);
                if t_out < ramp_start {
                    s0
                } else {
                    let u = ((t_out - ramp_start) / ramp_out_dur).clamp(0.0, 1.0);
                    s0 + (s1 - s0) * ease_progress(u, ease)
                }
            }
        }
    };

    let dt_out = total_out_dur / (RAMP_SEGMENTS as f64);
    let mut segments: Vec<RampSegment> = Vec::with_capacity(RAMP_SEGMENTS);
    let mut t_in_acc = 0.0;
    for i in 0..RAMP_SEGMENTS {
        let t_out_start = i as f64 * dt_out;
        let t_out_mid = t_out_start + dt_out * 0.5;
        let s_mid = s_at(t_out_mid).max(0.01);
        let t_in_end = t_in_acc + s_mid * dt_out;
        segments.push(RampSegment {
            t_in_start: t_in_acc,
            t_in_end,
            t_out_start,
            speed: s_mid,
        });
        t_in_acc = t_in_end;
    }

    Some(segments)
}

/// Build a `crop` filter that pans a target-aspect window over the
/// source frame, following the clip's auto-reframe keypoints
/// (Phase P2c). Returns None when fewer than two keypoints are
/// present -- a single point is a static crop and better handled by
/// a manual crop, which is not part of this phase.
///
/// The crop rectangle keeps the target aspect ratio and is sized to
/// cover the source: whichever dimension is "extra" gets trimmed. Its
/// centre is placed according to a piecewise-linear interpolation
/// between keypoints, keyed on `t` in seconds. `t` here is the
/// post-setpts clip-relative time, so the caller must run this filter
/// AFTER `setpts=PTS-STARTPTS`. This is why the base chain splits the
/// auto-reframe branch into its own arm rather than prepending to the
/// existing fit chain.
///
/// Coordinates are clamped to [0, 1] so a stray keypoint cannot walk
/// the crop out of the source frame.
fn build_auto_reframe_crop(
    keypoints: &[caprust_core::clip::ReframeKeypoint],
    target_w: u32,
    target_h: u32,
) -> Option<String> {
    if keypoints.len() < 2 || target_w == 0 || target_h == 0 {
        return None;
    }
    let aspect = target_w as f64 / target_h as f64;

    let build_expr = |sel: fn(&caprust_core::clip::ReframeKeypoint) -> f32| -> String {
        let n = keypoints.len();
        let mut expr = format!("{:.6}", sel(&keypoints[n - 1]));
        for i in (0..n - 1).rev() {
            let a = keypoints[i];
            let b = keypoints[i + 1];
            let ta = a.t_ms as f64 / 1000.0;
            let tb = b.t_ms as f64 / 1000.0;
            let dt = (tb - ta).max(1e-6);
            let va = sel(&a) as f64;
            let vb = sel(&b) as f64;
            let seg = format!("({va:.6}+({vb:.6}-{va:.6})*(t-{ta:.6})/{dt:.6})");
            expr = format!("if(lt(t\\,{tb:.6})\\,{seg}\\,{expr})");
        }
        format!("min(max({expr}\\,0)\\,1)")
    };

    let cx_expr = build_expr(|k| k.cx_norm);
    let cy_expr = build_expr(|k| k.cy_norm);

    Some(format!(
        "crop=w='min(iw\\,ih*{aspect:.6})':h='min(ih\\,iw/{aspect:.6})':x='({cx_expr})*(iw-ow)':y='({cy_expr})*(ih-oh)'"
    ))
}

/// Build a `setpts=PTS-STARTPTS,setpts=\'<expr>\'` chain for a clip with
/// a speed ramp. Returns None when no ramp applies.
///
/// The ramp is a nested if() over `T/TB` (output seconds): each
/// segment contributes a linear branch mapping output time to a new
/// PTS. Boundaries and speeds come from compute_speed_ramp_segments,
/// shared with the audio side.
fn build_speed_ramp_setpts(
    speed: f32,
    speed_end: f32,
    ease: caprust_core::clip::EaseCurve,
    range: caprust_core::clip::SpeedRampRange,
    clip_in_dur_s: f64,
) -> Option<String> {
    let segments = compute_speed_ramp_segments(speed, speed_end, ease, range, clip_in_dur_s)?;
    let n = segments.len();

    // Branch: for INPUT time t_in inside the segment, output PTS is
    //   t_out_start + (t_in - t_in_start) / speed.
    let branch = |seg: &RampSegment| -> String {
        format!(
            "({t_out:.6}+(T/TB-{t_in:.6})/{s:.6})",
            t_out = seg.t_out_start,
            t_in = seg.t_in_start,
            s = seg.speed,
        )
    };

    // Nested if() from the tail backwards.
    let mut expr = branch(&segments[n - 1]);
    for seg in segments[..n - 1].iter().rev() {
        let b = branch(seg);
        expr = format!(
            "if(lt(T/TB\\,{t_in_e:.6})\\,{b}\\,{expr})",
            t_in_e = seg.t_in_end,
            b = b,
            expr = expr,
        );
    }

    Some(format!("setpts=\'TB*({expr})\'"))
}

/// Build an `asplit, atrim xN, atempo xN, concat` block that applies a
/// piecewise-constant speed ramp to an audio stream. Emits one or more
/// full ffmpeg chain lines (each `;`-terminated), reading from
/// `in_label` and writing to `out_name`.
///
/// Uses the same RampSegment list as build_speed_ramp_setpts, so every
/// segment boundary and speed matches the video side exactly. That
/// removes the mid-ramp A/V drift the previous ease-weighted-mean
/// approximation introduced.
///
/// Argument conventions -- easy to get wrong; a mismatch produces a
/// double-bracket label and ffmpeg rejects the filtergraph:
///   * `in_label` is a COMPLETE ffmpeg label including the brackets,
///     e.g. `"[0:a]"`. Interpolated verbatim.
///   * `out_name` is a BARE name with no brackets, e.g. `"a0_pre"`.
///     This function wraps it in `[...]` on the way out.
///   * `prefix` is a BARE name used to build the internal asplit and
///     atrim labels: `"a0r"` yields `[a0r_sp0]` and `[a0r_sg0]`.
fn build_speed_ramp_atempo_segments(
    in_label: &str,
    out_name: &str,
    segments: &[RampSegment],
    prefix: &str,
) -> String {
    let n = segments.len();
    let mut out = String::new();

    // Split the source into N branches.
    let split_labels: Vec<String> = (0..n).map(|i| format!("{prefix}_sp{i}")).collect();
    out.push_str(&format!(
        "{in_label}asplit={n}{outs};",
        in_label = in_label,
        n = n,
        outs = split_labels
            .iter()
            .map(|l| format!("[{l}]"))
            .collect::<String>(),
    ));

    // One atrim + atempo per segment.
    let seg_labels: Vec<String> = (0..n).map(|i| format!("{prefix}_sg{i}")).collect();
    for (i, seg) in segments.iter().enumerate() {
        let atempo = atempo_chain(seg.speed as f32);
        out.push_str(&format!(
            "[{split}]atrim=start={st:.6}:end={en:.6},asetpts=PTS-STARTPTS{atempo}[{sg}];",
            split = split_labels[i],
            st = seg.t_in_start,
            en = seg.t_in_end,
            atempo = atempo,
            sg = seg_labels[i],
        ));
    }

    // Concatenate segments end-to-end.
    out.push_str(&format!(
        "{ins}concat=n={n}:v=0:a=1[{out}];",
        ins = seg_labels
            .iter()
            .map(|l| format!("[{l}]"))
            .collect::<String>(),
        n = n,
        out = out_name,
    ));

    out
}

/// Build an `,atempo=x` chain that supports 0.5..=2.0 per stage.
fn atempo_chain(speed: f32) -> String {
    let mut s = speed;
    let mut parts: Vec<String> = Vec::new();
    if (s - 1.0).abs() < 0.001 {
        return String::new();
    }
    // atempo supports 0.5..=100 in recent builds; older only 0.5..=2.
    // Chain to be safe.
    while s > 2.0 {
        parts.push(",atempo=2.0".to_string());
        s /= 2.0;
    }
    while s < 0.5 {
        parts.push(",atempo=0.5".to_string());
        s *= 2.0;
    }
    parts.push(format!(",atempo={s:.6}"));
    parts.join("")
}

/// Builder helper: given a `ProjectState`, collect inputs + clips.
/// Only V1 (first Video track) and A1 (first Audio track) are used for now.
// plan_from_project has grown to 8 parameters as the pipeline gained
// features (crf, preset, and now models_dir for narration caching).
// Every parameter is a distinct, load-bearing input to the render plan;
// bundling them into a struct just shuffles the same fields around.
#[allow(clippy::too_many_arguments)]
/// Cumulative audio shift per clip caused by xfade chains.
///
/// A clip that participates in an xfade chain on its track appears in
/// the render output earlier than its original timeline position, by
/// the sum of the xfade durations upstream of it in that chain (the
/// video pipeline compresses the run by that amount via `xfade`).
/// Without this shift the audio of that clip plays D seconds too late
/// per transition, which is heard as a growing drift after each
/// transition.
///
/// Returns clip_id -> shift_seconds for every clip inside a chain.
/// Clips on follower tracks (Audio, Captions, Text) inherit the shift
/// of the video clip they overlapped most in the ORIGINAL timeline.
fn compute_xfade_audio_shifts(
    project: &caprust_core::ProjectState,
) -> std::collections::HashMap<uuid::Uuid, f64> {
    use std::collections::HashMap;

    let mut shifts: HashMap<uuid::Uuid, f64> = HashMap::new();

    // Pass 1: per-track xfade chains.
    let mut by_track: std::collections::BTreeMap<usize, Vec<&caprust_core::Clip>> =
        std::collections::BTreeMap::new();
    for c in &project.clips {
        by_track.entry(c.track_index).or_default().push(c);
    }

    for (_t, mut clips) in by_track {
        clips.sort_by_key(|c| c.start_time_ms);
        let mut cumulative = 0.0_f64;
        let mut prev: Option<&caprust_core::Clip> = None;
        for c in clips {
            if let Some(p) = prev {
                let prev_end = p.start_time_ms + p.duration_ms;
                let overlap_sec = (prev_end as i64 - c.start_time_ms as i64) as f64 / 1000.0;
                let has_xfade = c.transition_in.as_deref().map(is_xfade_id).unwrap_or(false);
                if has_xfade && overlap_sec >= -ADJACENCY_TOL_SEC {
                    let prev_dur = p.duration_ms as f64 / 1000.0;
                    let curr_dur = c.duration_ms as f64 / 1000.0;
                    // Match build_filtergraph exactly: user-configured
                    // transition duration, floored at XFADE_DUR_SEC,
                    // then clamped to half of either clip. Without this
                    // the video xfade could compress the timeline for
                    // 1.0s while audio shifted only 0.5s -> cumulative
                    // drift after each transition (DIRECTIVES 10.4).
                    let requested = if c.transition_duration_ms > 0 {
                        c.transition_duration_ms as f64 / 1000.0
                    } else {
                        XFADE_DUR_SEC
                    };
                    let d = requested
                        .max(XFADE_DUR_SEC)
                        .min(prev_dur * 0.5)
                        .min(curr_dur * 0.5)
                        .max(0.05);
                    cumulative += d;
                } else {
                    cumulative = 0.0;
                }
            }
            if cumulative > 0.0 {
                shifts.insert(c.id, cumulative);
            }
            prev = Some(c);
        }
    }

    // Pass 2: follower tracks inherit their parent video's shift.
    // Snapshot video shifts for overlap lookup.
    let video_shifts: Vec<(u64, u64, f64)> = project
        .clips
        .iter()
        .filter(|c| shifts.contains_key(&c.id))
        .map(|c| {
            (
                c.start_time_ms,
                c.start_time_ms + c.duration_ms,
                *shifts.get(&c.id).unwrap(),
            )
        })
        .collect();

    if video_shifts.is_empty() {
        return shifts;
    }

    let followers: Vec<uuid::Uuid> = project
        .clips
        .iter()
        .filter(|c| !shifts.contains_key(&c.id))
        .filter(|c| {
            project
                .tracks
                .get(c.track_index)
                .map(|t| {
                    !matches!(
                        t.kind,
                        caprust_core::TrackKind::Video | caprust_core::TrackKind::Overlay
                    )
                })
                .unwrap_or(false)
        })
        .map(|c| c.id)
        .collect();

    for fid in followers {
        let Some(fc) = project.clips.iter().find(|c| c.id == fid) else {
            continue;
        };
        let fs = fc.start_time_ms;
        let fe = fc.start_time_ms + fc.duration_ms;
        let mut best: Option<(u64, f64)> = None;
        for (vs, ve, sh) in &video_shifts {
            let ov_start = fs.max(*vs);
            let ov_end = fe.min(*ve);
            if ov_start < ov_end {
                let ov = ov_end - ov_start;
                if best.is_none_or(|(bo, _)| ov > bo) {
                    best = Some((ov, *sh));
                }
            }
        }
        if let Some((_, sh)) = best {
            shifts.insert(fid, sh);
        }
    }

    shifts
}

/// Escape an absolute filesystem path for use as the value of
/// `movie=...` inside an ffmpeg filter_complex string.
///
/// Rules:
///   * Backslashes are replaced with forward slashes. Windows accepts
///     both, and forward slashes dodge ffmpeg's escape semantics.
///   * Colons are escaped as `\:`. The filter option parser splits
///     on `:` at a level below where single-quote quoting applies, so
///     a Windows drive letter like `F:` would otherwise be truncated
///     to just `F`. This is what caused
///     `Failed to avformat_open_input 'F'` on every Windows path.
///   * The value is wrapped in single quotes so the outer
///     filter-description parser does not treat commas or semicolons
///     inside the path as separators.
///   * A path containing a single quote cannot be represented this
///     way. `movie=` on Windows video files almost never hits this,
///     so we reject rather than trying to build a two-level escape
///     that is hard to reason about.
///
/// Returns None when the path contains a single quote.
fn escape_movie_path(path: &std::path::Path) -> Option<String> {
    let s = path.to_string_lossy();
    if s.contains('\'') {
        return None;
    }
    let normalized = s.replace('\\', "/");
    // Escape the colon so a Windows drive letter survives the filter
    // option parser. Single-quote quoting does not protect it.
    let colon_escaped = normalized.replace(':', "\\:");
    Some(format!("'{colon_escaped}'"))
}

/// Resolve the mask path for a clip. The stored path is relative to
/// the project directory (e.g. "cache/masks/<clip>.mkv"); we join it
/// with `project.project_path` to get an absolute path for ffmpeg.
///
/// Returns None when:
///   * the clip has no bg_removal set;
///   * the project has no project_path (unsaved);
///   * the clip also has auto-reframe keypoints (mask would not align
///     with the crop; see below);
///   * the mask file is missing on disk.
///
/// The auto-reframe case is a known P3d limitation: auto-reframe
/// crops the source per-frame, but the mask was generated against the
/// whole frame. Fixing that means re-projecting the crop onto the
/// mask, which is a follow-up. For now, auto-reframe wins and
/// bg-removal is skipped for that clip with a warning.
fn resolve_bg_removal_path(
    project: &caprust_core::ProjectState,
    clip: &caprust_core::Clip,
) -> Option<std::path::PathBuf> {
    let rel = clip.bg_removal.as_ref()?;
    if !clip.auto_reframe.is_empty() {
        tracing::warn!(
            "clip {} has both auto_reframe and bg_removal; skipping mask (unsupported combination)",
            clip.id
        );
        return None;
    }
    let proj = project.project_path.as_ref()?;

    // Reject absolute paths and traversal components before touching
    // the filesystem. A malicious .caprust could otherwise point the
    // mask at any file on disk, or at an ffmpeg filter source like
    // `http://attacker/`, which `movie=` would happily open (the
    // `-protocol_whitelist` input option does not apply to
    // filter-level sources). See issue #7.
    use std::path::Component;
    let rel_path = std::path::Path::new(rel);
    if rel_path.is_absolute()
        || rel_path.components().any(|c| {
            matches!(
                c,
                Component::Prefix(_) | Component::RootDir | Component::ParentDir
            )
        })
    {
        tracing::warn!(
            "clip {} bg_removal path is not a safe relative path: {rel}",
            clip.id
        );
        return None;
    }

    let masks_dir = std::path::Path::new(proj).join("cache").join("masks");
    let abs = std::path::Path::new(proj).join(rel_path);

    if !abs.is_file() {
        tracing::warn!(
            "clip {} bg_removal mask missing on disk: {}",
            clip.id,
            abs.display()
        );
        return None;
    }

    // Canonicalize and require the mask to live directly inside
    // <project>/cache/masks/. This defeats `..`, symlinks, and case
    // tricks that would otherwise let the path escape the cache dir.
    let canon = match abs.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("clip {} bg_removal canonicalize failed: {e}", clip.id);
            return None;
        }
    };
    let canon_masks = match masks_dir.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("clip {} masks dir canonicalize failed: {e}", clip.id);
            return None;
        }
    };
    if canon.parent() != Some(canon_masks.as_path()) {
        tracing::warn!(
            "clip {} bg_removal mask resolves outside <project>/cache/masks: {}",
            clip.id,
            canon.display()
        );
        return None;
    }

    // The file name must be exactly <clip_id>.mkv. Any other name is
    // either a stale reference or an attempt to point at an unrelated
    // file that happens to sit in the cache dir.
    let expected_stem = clip.id.to_string();
    let stem_ok = canon.file_stem().and_then(|s| s.to_str()) == Some(expected_stem.as_str());
    let ext_ok = canon
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("mkv"))
        .unwrap_or(false);
    if !stem_ok || !ext_ok {
        tracing::warn!(
            "clip {} bg_removal mask must be <clip_id>.mkv, got {}",
            clip.id,
            canon.display()
        );
        return None;
    }

    // Return the pre-canonical path so downstream escaping keeps the
    // user-visible `F:/...` form (canonical on Windows adds a `\\?\`
    // prefix that would change the ffmpeg argument).
    Some(abs)
}

// plan_from_project has one argument per render dimension the caller
// knows about. Bundling them into a struct would just move the same
// fields behind one more layer. The signature is stable; leave it.
/// Count of clips skipped by `plan_from_project` because their source
/// file was missing on disk. The caller can surface a warning to the
/// user; the render continues with the remaining clips.
#[derive(Debug, Default, Clone, Copy)]
pub struct PlanSkipped {
    pub missing_source: usize,
}

/// Extract the on-disk source path from a clip, if it has one. Video,
/// Image, and Audio all carry a `path`; Text, Captions, and Narration
/// do not reference an input file.
fn clip_source_path(ct: &caprust_core::ClipType) -> Option<&str> {
    use caprust_core::ClipType;
    match ct {
        ClipType::Video { path, .. }
        | ClipType::Image { path, .. }
        | ClipType::Audio { path, .. } => Some(path.as_str()),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn plan_from_project(
    project: &caprust_core::ProjectState,
    width: u32,
    height: u32,
    fps_num: i64,
    fps_den: i64,
    crf: u8,
    rate_mode: RateMode,
    bitrate_kbps: u32,
    preset: &str,
    models_dir: &std::path::Path,
    seek_ms: u64,
    encoder: caprust_core::project::VideoEncoder,
) -> Result<RenderPlan> {
    use caprust_core::{ClipType, TrackKind};

    let mut inputs: Vec<InputSpec> = Vec::new();
    let mut video_clips: Vec<VideoClip> = Vec::new();
    let mut audio_clips: Vec<AudioClip> = Vec::new();
    let mut skipped = PlanSkipped::default();
    // Clips downstream of an xfade appear earlier in the render than
    // their original timeline position; shift their audio and their
    // burned-in captions to match so the mix and the on-screen text
    // stay in sync with the shortened video.
    let xfade_audio_shifts = compute_xfade_audio_shifts(project);

    // ---- Seek optimization (preview only) ----
    // When the caller starts playback at a non-zero playhead,
    // rewrite every input to `-ss <offset>` so ffmpeg's demuxer
    // seeks instead of decoding from t=0. Clips fully before the
    // seek are dropped; clips that straddle it shrink and shift.
    //
    // Disabled when the project has any transition, because xfade
    // runs assume chain adjacency that per-clip seek would break.
    // Those projects fall back to the caller-side output `-ss`.
    let seek_sec = seek_ms as f64 / 1000.0;
    let has_any_transition = project
        .clips
        .iter()
        .any(|c| c.transition_in.is_some() || c.transition_out.is_some());
    let do_input_seek = seek_ms > 0 && !has_any_transition;
    if do_input_seek {
        tracing::info!("plan_from_project: seek-optimized plan for {}ms", seek_ms);
    } else if seek_ms > 0 {
        tracing::info!(
            "plan_from_project: seek {}ms with transitions -- falling back to output-side -ss",
            seek_ms
        );
    }

    let mut text_clips: Vec<TextClip> = Vec::new();

    let register_input =
        |inputs: &mut Vec<InputSpec>, path: &str, start_sec: f64, dur_sec: f64| -> usize {
            let idx = inputs.len();
            inputs.push(InputSpec {
                ffmpeg_index: idx,
                path: PathBuf::from(path),
                source_start_sec: start_sec,
                duration_sec: dur_sec,
            });
            idx
        };

    // Video tracks in display order (pinned first, then V1, V2...).
    // For export we render bottom-up: V1 first, V2 over it, Overlay on top.
    let mut video_track_order: Vec<usize> = Vec::new();
    // Non-pinned video tracks, in declared order
    for (i, t) in project.tracks.iter().enumerate() {
        if t.kind == TrackKind::Video {
            video_track_order.push(i);
        }
    }
    // Overlay tracks on top
    for (i, t) in project.tracks.iter().enumerate() {
        if t.kind == TrackKind::Overlay {
            video_track_order.push(i);
        }
    }
    // Text and Captions tracks also feed text_clips. Push them after
    // Overlay so burned-in captions and text land above every video
    // layer, matching the timeline's visual stacking. Without this,
    // Captions placed on a Captions track were never visited by the
    // clip loop and produced no drawtext at all.
    for (i, t) in project.tracks.iter().enumerate() {
        if t.kind == TrackKind::Text {
            video_track_order.push(i);
        }
    }
    for (i, t) in project.tracks.iter().enumerate() {
        if t.kind == TrackKind::Captions {
            video_track_order.push(i);
        }
    }

    // Z-order index assigned to each clip as we walk tracks bottom-up.
    for (z, &t_idx) in video_track_order.iter().enumerate() {
        // The eye chip on the track header toggles `visible`. Hidden
        // tracks are excluded from the render entirely — clips on them
        // do not contribute video, text overlays, or anything else.
        if !project.tracks[t_idx].visible {
            continue;
        }
        let z = z as u32;
        let track_kind = project.tracks[t_idx].kind;
        let mut clips: Vec<&caprust_core::Clip> = project
            .clips
            .iter()
            .filter(|c| c.track_index == t_idx)
            .collect();
        clips.sort_by_key(|c| c.start_time_ms);

        for c in clips {
            if let Some(path) = clip_source_path(&c.clip_type) {
                if !std::path::Path::new(path).is_file() {
                    tracing::warn!(
                        "plan_from_project: skipping clip {} (source missing: {})",
                        c.id,
                        path
                    );
                    skipped.missing_source += 1;
                    continue;
                }
            }
            match &c.clip_type {
                ClipType::Video { path, .. } => {
                    let dur_sec = c.duration_ms as f64 / 1000.0;
                    let clip_start = c.start_time_ms as f64 / 1000.0;
                    let clip_end = clip_start + dur_sec;
                    if do_input_seek && clip_end <= seek_sec {
                        continue;
                    }
                    let (input_ss_sec, visible_dur, new_start) = if do_input_seek {
                        if clip_start >= seek_sec {
                            (0.0, dur_sec, clip_start - seek_sec)
                        } else {
                            let offset = (seek_sec - clip_start) * c.speed as f64;
                            let visible = clip_end - seek_sec;
                            (offset, visible, 0.0)
                        }
                    } else {
                        (0.0, dur_sec, clip_start)
                    };
                    // Drop clips reduced to less than 100 ms by the
                    // seek. ffmpeg's trim can produce zero frames for
                    // a duration this small, and a zero-frame branch
                    // in the filtergraph stalls the whole pipeline.
                    if do_input_seek && visible_dur < 0.1 {
                        continue;
                    }
                    let idx = register_input(&mut inputs, path, input_ss_sec, visible_dur);
                    video_clips.push(VideoClip {
                        input_index: idx,
                        timeline_start_sec: new_start,
                        duration_sec: visible_dur,
                        speed: c.speed,
                        speed_end: c.speed_end,
                        speed_ease: c.speed_ease,
                        speed_range: c.speed_range,
                        z_order: z,
                        is_image: false,
                        effects: c.effects.clone(),
                        transition_in: c.transition_in.clone(),
                        transition_out: c.transition_out.clone(),
                        transition_in_easing: c.transition_in_easing,
                        transition_out_easing: c.transition_out_easing,
                        transition_duration_sec: (c.transition_duration_ms as f64 / 1000.0)
                            .clamp(0.1, 3.0),
                        auto_reframe: c.auto_reframe.clone(),
                        bg_removal_path: resolve_bg_removal_path(project, c),
                    });
                }
                ClipType::Image { path, .. } => {
                    let dur_sec = c.duration_ms as f64 / 1000.0;
                    let clip_start = c.start_time_ms as f64 / 1000.0;
                    let clip_end = clip_start + dur_sec;
                    if do_input_seek && clip_end <= seek_sec {
                        continue;
                    }
                    let (visible_dur, new_start) = if do_input_seek {
                        if clip_start >= seek_sec {
                            (dur_sec, clip_start - seek_sec)
                        } else {
                            (clip_end - seek_sec, 0.0)
                        }
                    } else {
                        (dur_sec, clip_start)
                    };
                    let idx = register_input(&mut inputs, path, 0.0, visible_dur);
                    video_clips.push(VideoClip {
                        input_index: idx,
                        timeline_start_sec: new_start,
                        duration_sec: visible_dur,
                        speed: c.speed,
                        speed_end: c.speed_end,
                        speed_ease: c.speed_ease,
                        speed_range: c.speed_range,
                        z_order: z,
                        is_image: true,
                        effects: c.effects.clone(),
                        transition_in: c.transition_in.clone(),
                        transition_out: c.transition_out.clone(),
                        transition_in_easing: c.transition_in_easing,
                        transition_out_easing: c.transition_out_easing,
                        transition_duration_sec: (c.transition_duration_ms as f64 / 1000.0)
                            .clamp(0.1, 3.0),
                        auto_reframe: c.auto_reframe.clone(),
                        bg_removal_path: resolve_bg_removal_path(project, c),
                    });
                }
                ClipType::TextOverlay {
                    content,
                    font_size,
                    above,
                    style,
                    motion,
                    effect,
                } => {
                    // Drop the overlay when it sits entirely before the
                    // seek. Without this the arm clamps timeline_start
                    // to 0 and the overlay appears in the first N
                    // seconds of the seek-shifted output even though it
                    // belongs elsewhere on the timeline.
                    {
                        // The model (SetTransitionCommand) already shifts
                        // clips on the xfade's own track by the effective
                        // transition duration. compute_xfade_audio_shifts
                        // returns the SAME amount for those clips, so
                        // subtracting it again would double-shift the
                        // audio. Only apply the audio shift to clips the
                        // model did NOT move (separated audio tracks and
                        // follower tracks inherit through Pass 2).
                        let shift_sec = xfade_audio_shifts.get(&c.id).copied().unwrap_or(0.0);
                        let effective_shift = if c.applied_xfade_shift_ms > 0 {
                            0.0
                        } else {
                            shift_sec
                        };
                        let raw_start =
                            (c.start_time_ms as f64 / 1000.0 - effective_shift).max(0.0);
                        let raw_end = raw_start + c.duration_ms as f64 / 1000.0;
                        if do_input_seek && raw_end <= seek_sec {
                            continue;
                        }
                    }
                    text_clips.push(TextClip {
                        content: content.clone(),
                        font_size: *font_size,
                        timeline_start_sec: {
                            // Same seek arithmetic as video/audio.
                            // Without this the enable='between(t,...)'
                            // windows stay on the original timeline and
                            // every overlay fires at once or at the
                            // wrong time when the preview is seeked.
                            let clip_start = c.start_time_ms as f64 / 1000.0;
                            if do_input_seek {
                                if clip_start >= seek_sec {
                                    clip_start - seek_sec
                                } else {
                                    0.0
                                }
                            } else {
                                clip_start
                            }
                        },
                        duration_sec: {
                            let clip_start = c.start_time_ms as f64 / 1000.0;
                            let clip_end = clip_start + c.duration_ms as f64 / 1000.0;
                            if do_input_seek {
                                if clip_start >= seek_sec {
                                    c.duration_ms as f64 / 1000.0
                                } else {
                                    (clip_end - seek_sec).max(0.0)
                                }
                            } else {
                                c.duration_ms as f64 / 1000.0
                            }
                        },
                        above: *above || track_kind == TrackKind::Overlay,
                        z_order: z,
                        style: style.clone(),
                        motion: *motion,
                        effect: *effect,
                        caption_style: None,
                    });
                }
                ClipType::Captions {
                    segments,
                    style: caption_style,
                    motion,
                    effect,
                    ..
                } => {
                    // Each caption segment becomes its own drawtext with an
                    // enable window. Coordinates are relative to the clip's
                    // start: whisper reports absolute source times, the clip
                    // holds them relative to its own timeline position.
                    //
                    // The clip's effective start is shifted left by the
                    // upstream xfade durations on its track (same shift
                    // applied to the audio mix) so the burned-in text
                    // lands on top of the same frame it was transcribed
                    // from, not D seconds late after a transition.
                    //
                    // Two render modes per segment:
                    //   * words.is_empty() -- one drawtext for the whole
                    //     segment. Pre-P1 behaviour, still used for any
                    //     segment where token grouping produced nothing.
                    //   * words not empty -- progressive reveal (P1):
                    //     one drawtext per word, each showing the text
                    //     accumulated up to that word. The caption
                    //     "types itself" as the speaker talks, at the
                    //     same centring the single-drawtext path uses,
                    //     so nothing else needs to change.
                    // Same rule as the block above: only apply the
                    // audio shift to clips the model did not move.
                    let shift_sec = xfade_audio_shifts.get(&c.id).copied().unwrap_or(0.0);
                    let shift_sec = if c.applied_xfade_shift_ms > 0 {
                        0.0
                    } else {
                        shift_sec
                    };
                    // Raw timeline position after the xfade shift. Used
                    // to decide drop / partial visibility, then remapped
                    // to the seek-relative output timeline below.
                    let clip_start_raw = (c.start_time_ms as f64 / 1000.0 - shift_sec).max(0.0);
                    let clip_end_raw = clip_start_raw + c.duration_ms as f64 / 1000.0;
                    if do_input_seek && clip_end_raw <= seek_sec {
                        // Caption sits entirely before the seek. Same
                        // drop as video and audio clips: otherwise its
                        // enable windows stay on the original timeline
                        // and every segment fires at once against a
                        // shortened output.
                        continue;
                    }
                    // Seek-relative clip start in the output timeline.
                    let clip_start_sec = if do_input_seek {
                        (clip_start_raw - seek_sec).max(0.0)
                    } else {
                        clip_start_raw
                    };
                    for seg in segments {
                        let seg_start_raw = clip_start_raw + seg.start_ms as f64 / 1000.0;
                        let seg_end_raw = clip_start_raw + seg.end_ms as f64 / 1000.0;
                        if do_input_seek && seg_end_raw <= seek_sec {
                            // Segment sits entirely before the seek.
                            continue;
                        }
                        // Map from original timeline to seek-relative
                        // output. For a straddling segment this pulls
                        // the enable window forward to t=0 of the
                        // shortened output.
                        let seg_start_sec = if do_input_seek {
                            (seg_start_raw - seek_sec).max(0.0)
                        } else {
                            seg_start_raw
                        };
                        let seg_end_sec = seg_start_sec + (seg_end_raw - seg_start_raw).max(0.0);

                        // Fallback to a single drawtext when either the
                        // word list is empty, or every word carries a
                        // zero-duration timing. Whisper tiny (and some
                        // other configs) persist the word list but leave
                        // all start_ms/end_ms at 0; the progressive-reveal
                        // path would then stack every word on the same
                        // frame with the same enable window, producing
                        // a legible-but-overlapping pile.
                        let words_have_timing = !seg.words.is_empty()
                            && seg.words.iter().any(|w| w.start_ms != 0 || w.end_ms != 0);
                        if !words_have_timing {
                            let seg_dur = (seg_end_sec - seg_start_sec).max(0.05);
                            text_clips.push(TextClip {
                                content: seg.text.clone(),
                                font_size: 32.0,
                                timeline_start_sec: seg_start_sec,
                                duration_sec: seg_dur,
                                above: false,
                                z_order: z,
                                style: "caption".to_string(),
                                motion: *motion,
                                effect: *effect,
                                caption_style: Some(*caption_style),
                            });
                        } else {
                            let mut acc = String::new();
                            for (wi, w) in seg.words.iter().enumerate() {
                                // Skip whisper control tokens ([_BEG_],
                                // [_EOT_], ...). They carry zero/negative
                                // duration and would otherwise leave a
                                // zero-width enable window that stacks on
                                // the following real word.
                                let word_text = w.text.trim();
                                if word_text.starts_with('[') && word_text.ends_with(']') {
                                    continue;
                                }
                                if !acc.is_empty() {
                                    acc.push(' ');
                                }
                                acc.push_str(word_text);
                                let word_start_sec = clip_start_sec + w.start_ms as f64 / 1000.0;
                                // (clip_start_sec is already seek-shifted;
                                // word.start_ms is source-relative to the
                                // caption clip, so no further adjustment.)
                                // Each drawtext is on screen until the
                                // next word starts; the last one stays
                                // until the segment end. Clamped so a
                                // stray zero-duration word cannot
                                // produce an invisible enable window.
                                // Next non-control word's start time, so
                                // the current word's enable window does
                                // not collapse onto a zero-duration token.
                                let next_real_start = seg.words[wi + 1..]
                                    .iter()
                                    .find(|x| {
                                        let t = x.text.trim();
                                        !(t.starts_with('[') && t.ends_with(']'))
                                    })
                                    .map(|x| x.start_ms);
                                let word_end_raw = match next_real_start {
                                    Some(ms) => clip_start_raw + ms as f64 / 1000.0,
                                    None => seg_end_raw,
                                };
                                let word_end_sec = if do_input_seek {
                                    (word_end_raw - seek_sec).max(word_start_sec)
                                } else {
                                    word_end_raw
                                };
                                let dur = (word_end_sec - word_start_sec).max(0.05);
                                text_clips.push(TextClip {
                                    content: acc.clone(),
                                    font_size: 32.0,
                                    timeline_start_sec: word_start_sec,
                                    duration_sec: dur,
                                    above: false,
                                    z_order: z,
                                    style: "caption".to_string(),
                                    motion: *motion,
                                    effect: *effect,
                                    caption_style: Some(*caption_style),
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Audio: prefer dedicated Audio tracks *that actually contain clips*.
    // An empty Audio track (e.g. auto-created default) should NOT disable
    // the fallback to embedded audio in Video clips — otherwise projects
    // with only video+audio-embedded playback would produce silence.
    // Count only clips on Audio tracks that can actually produce audio.
    // A Captions or TextOverlay clip that happens to sit on an Audio
    // track (the UI currently mis-assigns track_index when creating
    // Captions clips) must not disable the fallback to video-embedded
    // audio. Otherwise a video-only project with one stray Captions
    // clip on A1 renders silent: has_audio_track becomes true, the
    // fallback to video is skipped, and the Captions clip contributes
    // nothing.
    // Harvest audio from every clip in one flat pass. A clip contributes
    // to the mix unless:
    //   - its track is muted (track-header mute chip),
    //   - it is a Video clip whose audio was detached (SeparateAudio
    //     marks the video and moves the audio to a sibling Audio clip;
    //     the video must not double).
    // Images, TextOverlay and Captions never contribute audio.
    //
    // The previous version branched on "does any Audio track hold at
    // least one clip": if yes, ONLY Audio tracks were scanned, so a
    // video clip with embedded audio on V1 was silently dropped the
    // moment any Audio track existed. That was the 'detached audio
    // mutes the next clip' bug.
    let mut audio_from_video = false;
    let mut audio_track_clip_count: usize = 0;

    for c in project.clips.iter() {
        let Some(track) = project.tracks.get(c.track_index) else {
            continue;
        };
        if track.muted {
            continue;
        }
        let is_audio_track = track.kind == TrackKind::Audio;

        let narration_path_owned: Option<String> = match &c.clip_type {
            ClipType::Narration { voice_id, text, .. } => Some(
                caprust_core::cache::narration_path(models_dir, text, voice_id)
                    .to_string_lossy()
                    .into_owned(),
            ),
            _ => None,
        };

        let path_opt: Option<&String> = match &c.clip_type {
            ClipType::Audio { path, .. } => {
                if is_audio_track {
                    audio_track_clip_count += 1;
                }
                Some(path)
            }
            ClipType::Video { path, .. } if !c.audio_detached => {
                if is_audio_track {
                    audio_track_clip_count += 1;
                } else {
                    audio_from_video = true;
                }
                Some(path)
            }
            ClipType::Video { .. } => None,
            ClipType::Narration { .. } => {
                if is_audio_track {
                    audio_track_clip_count += 1;
                }
                narration_path_owned.as_ref()
            }
            _ => None,
        };

        let Some(path) = path_opt else { continue };

        if matches!(&c.clip_type, ClipType::Narration { .. })
            && !std::path::Path::new(path).is_file()
        {
            tracing::warn!(
                "narration clip {} has no cached WAV at {} — skipping",
                c.id,
                path
            );
            continue;
        }
        if !std::path::Path::new(path).is_file() {
            tracing::warn!(
                "plan_from_project: skipping audio clip {} (source missing: {})",
                c.id,
                path
            );
            skipped.missing_source += 1;
            continue;
        }
        let dur_sec = c.duration_ms as f64 / 1000.0;
        let clip_start = c.start_time_ms as f64 / 1000.0;
        let clip_end = clip_start + dur_sec;

        // Same seek arithmetic as the video arm. Without this, the
        // audio chain would emit an `adelay=<full timeline ms>` and
        // force amix to buffer tens of seconds of silence for every
        // clip -- gigabytes of RAM on a 3-minute project.
        if do_input_seek && clip_end <= seek_sec {
            continue;
        }
        let (input_ss_sec, visible_dur, new_start) = if do_input_seek {
            if clip_start >= seek_sec {
                (0.0, dur_sec, clip_start - seek_sec)
            } else {
                let offset = (seek_sec - clip_start) * c.speed as f64;
                let visible = clip_end - seek_sec;
                (offset, visible, 0.0)
            }
        } else {
            (0.0, dur_sec, clip_start)
        };
        if do_input_seek && visible_dur < 0.1 {
            continue;
        }
        let idx = register_input(&mut inputs, path, input_ss_sec, visible_dur);
        let shift_sec = xfade_audio_shifts.get(&c.id).copied().unwrap_or(0.0);
        let effective_shift = if c.applied_xfade_shift_ms > 0 {
            0.0
        } else {
            shift_sec
        };
        let start_sec = (new_start - effective_shift).max(0.0);

        // Fades: clamp so the two never overlap. If the sum exceeds the
        // clip's duration, scale both down proportionally. Skip
        // inaudibly tiny fades (<1 ms) so the filtergraph stays clean.
        let mut fi = c.fade_in_ms as f64 / 1000.0;
        let mut fo = c.fade_out_ms as f64 / 1000.0;
        if fi + fo > dur_sec && fi + fo > 0.0 {
            let scale = dur_sec / (fi + fo);
            fi *= scale;
            fo *= scale;
        }
        if fi < 0.001 {
            fi = 0.0;
        }
        if fo < 0.001 {
            fo = 0.0;
        }

        let mut kfs = c.volume_keyframes.clone();
        kfs.sort_by_key(|k| k.t_ms);

        audio_clips.push(AudioClip {
            input_index: idx,
            timeline_start_sec: start_sec,
            duration_sec: dur_sec,
            speed: c.speed,
            speed_end: c.speed_end,
            speed_ease: c.speed_ease,
            speed_range: c.speed_range,
            gain_db: c.volume_db,
            track_gain_db: track.volume_db,
            fade_in_sec: fi,
            fade_out_sec: fo,
            volume_keyframes: kfs,
            clip_id: c.id,
            duck_against: c.duck_against,
            denoise: c.audio_denoise,
            voice_boost: c.audio_voice_boost,
            normalize: c.audio_normalize,
        });
    }

    if video_clips.is_empty() && text_clips.is_empty() {
        anyhow::bail!("no video or text clips on any video track — nothing to export");
    }

    // Effective video duration, accounting for xfade shortening.
    // A naive "max(start + dur)" over every clip is wrong once a
    // transition is in play: an xfade between clip A and clip B
    // shortens that run by D seconds (sum of durations minus the
    // overlap). If the black base and audio bed are sized to the
    // unshortened sum, the tail of the render is N seconds of pure
    // black where the overlay has no frames left. This mirrors the
    // run grouping in build_filtergraph so the bed matches what the
    // stack actually produces.
    let video_duration_sec: f64 = {
        use std::collections::BTreeMap;
        let mut by_z: BTreeMap<u32, Vec<&VideoClip>> = BTreeMap::new();
        for vc in &video_clips {
            by_z.entry(vc.z_order).or_default().push(vc);
        }
        let mut overall_max: f64 = 0.0;
        for (_z, mut group) in by_z {
            group.sort_by(|a, b| {
                a.timeline_start_sec
                    .partial_cmp(&b.timeline_start_sec)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let mut run_start = 0.0_f64;
            let mut run_dur = 0.0_f64;
            let mut prev_end = f64::NEG_INFINITY;
            let mut first = true;
            for vc in group {
                if first {
                    run_start = vc.timeline_start_sec;
                    run_dur = vc.duration_sec;
                    prev_end = vc.timeline_start_sec + vc.duration_sec;
                    first = false;
                    continue;
                }
                let has_xfade = vc
                    .transition_in
                    .as_deref()
                    .map(is_xfade_id)
                    .unwrap_or(false);
                let overlap = prev_end - vc.timeline_start_sec;
                if has_xfade && overlap >= -ADJACENCY_TOL_SEC {
                    // Same clamp build_filtergraph uses for the
                    // xfade link.
                    let d = vc
                        .transition_duration_sec
                        .max(XFADE_DUR_SEC)
                        .min(run_dur * 0.5)
                        .min(vc.duration_sec * 0.5)
                        .max(0.05);
                    run_dur = run_dur + vc.duration_sec - d;
                    prev_end = vc.timeline_start_sec + vc.duration_sec;
                } else {
                    overall_max = overall_max.max(run_start + run_dur);
                    run_start = vc.timeline_start_sec;
                    run_dur = vc.duration_sec;
                    prev_end = vc.timeline_start_sec + vc.duration_sec;
                }
            }
            if !first {
                overall_max = overall_max.max(run_start + run_dur);
            }
        }
        overall_max
    };

    // The plan output must span video, text, AND audio. Audio-only
    // tails (music longer than the last video clip) otherwise get cut
    // off: ffmpeg writes PCM only up to total_duration_sec, the audio
    // player hits EOF, and the tail goes silent even though the
    // playhead keeps advancing on the wall clock.
    let audio_end_sec = audio_clips
        .iter()
        .map(|c| c.timeline_start_sec + c.duration_sec)
        .fold(0.0_f64, f64::max);
    let text_end_sec = text_clips
        .iter()
        .map(|c| c.timeline_start_sec + c.duration_sec)
        .fold(0.0_f64, f64::max);
    let total_duration_sec = video_duration_sec.max(audio_end_sec).max(text_end_sec);

    let has_audio = !audio_clips.is_empty();

    tracing::info!(
        "export plan: {} video, {} audio, {} text \
         (audio_track_clips={}, from_video={})",
        video_clips.len(),
        audio_clips.len(),
        text_clips.len(),
        audio_track_clip_count,
        audio_from_video,
    );

    Ok(RenderPlan {
        inputs,
        video_clips,
        audio_clips,
        skipped,
        text_clips,
        width: width.max(2) & !1,
        height: height.max(2) & !1,
        fps_num,
        fps_den,
        crf,
        rate_mode,
        bitrate_kbps,
        preset: preset.to_string(),
        has_audio,
        encoder,
        seek_ms,
        seek_optimized: do_input_seek,
        // `total_duration_sec` was derived from `video_clips` above,
        // whose timeline_start_sec / duration_sec were ALREADY shifted
        // by the seek when do_input_seek is true. Subtracting seek_sec
        // here would zero it out. Leave as-is.
        total_duration_sec,
    })
}

/// Map a CPU x264/x265 preset name to the equivalent NVENC preset.
/// NVENC exposes 7 levels (p1 fastest .. p7 slowest). Unknown names
/// fall back to p5 (medium).
/// Emit `-b:v` / `-maxrate` / `-bufsize` for CBR encoding.
fn push_cbr_args(args: &mut Vec<String>, bitrate_kbps: u32) {
    let b = format!("{}k", bitrate_kbps);
    let buf = format!("{}k", bitrate_kbps.saturating_mul(2));
    args.push("-b:v".into());
    args.push(b.clone());
    args.push("-maxrate".into());
    args.push(b);
    args.push("-bufsize".into());
    args.push(buf);
}

fn nvenc_preset(cpu: &str) -> &'static str {
    match cpu {
        "ultrafast" => "p1",
        "superfast" => "p2",
        "veryfast" => "p3",
        "faster" => "p4",
        "fast" => "p4",
        "medium" => "p5",
        "slow" => "p6",
        "slower" => "p7",
        "veryslow" => "p7",
        _ => "p5",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xfade_mapping_covers_all_ids() {
        for id in [
            "fade", "slide_l", "slide_r", "slide_u", "slide_d", "wipe_l", "wipe_r", "zoom_in",
            "zoom_out", "rotate", "blur_t",
        ] {
            assert!(is_xfade_id(id), "{id} should be a valid xfade id");
        }
        assert!(!is_xfade_id("none"));
        assert!(!is_xfade_id("glitch"));
    }

    #[test]
    fn ghost_and_sparkle_produce_chain() {
        let g = build_one_effect("ghost", 1.0).expect("ghost chain");
        assert!(g.contains("tmix="), "ghost should emit tmix");
        assert!(g.contains("frames="), "ghost should specify frame count");

        let s = build_one_effect("sparkle", 1.0).expect("sparkle chain");
        assert!(s.contains("noise="), "sparkle should emit noise");
        assert!(s.contains("eq="), "sparkle should brighten via eq");
    }

    #[test]
    fn ramp_segments_tile_input_time_and_increase_for_linear_ramp() {
        use caprust_core::clip::{EaseCurve, SpeedRampRange};
        let segs = compute_speed_ramp_segments(
            1.0,
            2.0,
            EaseCurve::Linear,
            SpeedRampRange::WholeClip,
            4.0,
        )
        .expect("ramp segments");
        assert_eq!(segs.len(), 4);
        assert!(segs[0].t_in_start.abs() < 1e-9);
        for w in segs.windows(2) {
            assert!(
                (w[0].t_in_end - w[1].t_in_start).abs() < 1e-9,
                "input windows must tile: {:?} vs {:?}",
                w[0],
                w[1]
            );
        }
        // Linear 1x -> 2x: speeds must increase across segments.
        for w in segs.windows(2) {
            assert!(w[1].speed > w[0].speed, "speeds must rise: {w:?}");
        }
    }

    #[test]
    fn segmented_atempo_chain_has_one_stage_per_segment() {
        use caprust_core::clip::{EaseCurve, SpeedRampRange};
        let segs = compute_speed_ramp_segments(
            1.0,
            2.0,
            EaseCurve::Linear,
            SpeedRampRange::WholeClip,
            4.0,
        )
        .unwrap();
        let frag = build_speed_ramp_atempo_segments("[0:a]", "out", &segs, "a0r");
        assert!(frag.contains("asplit=4"), "asplit with N=4: {frag}");
        assert_eq!(
            frag.matches("atempo=").count(),
            4,
            "one atempo per segment: {frag}"
        );
        assert!(frag.contains("concat=n=4:v=0:a=1"), "concat tail: {frag}");
        assert!(frag.contains("atrim=start="), "each segment trims: {frag}");
        assert!(frag.ends_with("[out];"), "writes out_label: {frag}");
    }

    #[test]
    fn captions_with_words_expand_to_progressive_drawtexts() {
        use caprust_core::clip::{CaptionSegment, WordTiming};
        use caprust_core::{Clip, ClipType};

        // Two segments: the first has word timings (P1 progressive
        // reveal), the second has none (fallback to a single drawtext).
        // Both must appear in the plan with the correct shape.
        let mut c = Clip::new_video("placeholder.mp4", 0, 0, 2000);
        c.clip_type = ClipType::Captions {
            style: Default::default(),
            language: "en".into(),
            model_id: "test".into(),
            motion: Default::default(),
            effect: None,
            segments: vec![
                CaptionSegment {
                    start_ms: 0,
                    end_ms: 1000,
                    text: "hello world".into(),
                    words: vec![
                        WordTiming {
                            start_ms: 0,
                            end_ms: 500,
                            text: "hello".into(),
                        },
                        WordTiming {
                            start_ms: 500,
                            end_ms: 1000,
                            text: "world".into(),
                        },
                    ],
                },
                CaptionSegment {
                    start_ms: 1000,
                    end_ms: 2000,
                    text: "plain segment".into(),
                    words: vec![],
                },
            ],
        };

        let plan = RenderPlan {
            inputs: vec![InputSpec {
                ffmpeg_index: 0,
                path: PathBuf::from("placeholder.mp4"),
                source_start_sec: 0.0,
                duration_sec: 2.0,
            }],
            video_clips: vec![VideoClip {
                input_index: 0,
                timeline_start_sec: 0.0,
                duration_sec: 2.0,
                speed: 1.0,
                speed_end: None,
                speed_ease: caprust_core::clip::EaseCurve::Linear,
                speed_range: caprust_core::clip::SpeedRampRange::WholeClip,
                z_order: 0,
                is_image: false,
                effects: Vec::<caprust_core::clip::EffectInstance>::new(),
                transition_in: None,
                transition_out: None,
                transition_in_easing: caprust_core::clip::EaseCurve::Linear,
                transition_out_easing: caprust_core::clip::EaseCurve::Linear,
                transition_duration_sec: 0.35,
                auto_reframe: Vec::new(),
                bg_removal_path: None,
            }],
            audio_clips: vec![],
            text_clips: vec![],
            total_duration_sec: 2.0,
            width: 320,
            height: 240,
            fps_num: 30,
            fps_den: 1,
            has_audio: false,

            skipped: PlanSkipped::default(),

            encoder: caprust_core::project::VideoEncoder::H264Cpu,
            crf: 23,
            rate_mode: RateMode::Vbr,
            bitrate_kbps: 8000,
            preset: "veryfast".to_string(),
            seek_ms: 0,
            seek_optimized: false,
        };
        // Overwrite the video clip's type via the plan-construction
        // path: rather than reimplement plan_from_project here, we
        // simply run plan_from_project on a hand-built project.
        drop(plan);
        drop(c);

        let mut project = caprust_core::ProjectState::default();
        let mut clip = Clip::new_video("placeholder.mp4", 0, 0, 2000);
        clip.clip_type = ClipType::Captions {
            style: Default::default(),
            language: "en".into(),
            model_id: "test".into(),
            motion: Default::default(),
            effect: None,
            segments: vec![
                CaptionSegment {
                    start_ms: 0,
                    end_ms: 1000,
                    text: "hello world".into(),
                    words: vec![
                        WordTiming {
                            start_ms: 0,
                            end_ms: 500,
                            text: "hello".into(),
                        },
                        WordTiming {
                            start_ms: 500,
                            end_ms: 1000,
                            text: "world".into(),
                        },
                    ],
                },
                CaptionSegment {
                    start_ms: 1000,
                    end_ms: 2000,
                    text: "plain segment".into(),
                    words: vec![],
                },
            ],
        };
        // Captions clips live on the Captions track. plan_from_project
        // still requires at least one clip on a Video track, so add a
        // short placeholder video alongside.
        let captions_track = project
            .tracks
            .iter()
            .position(|t| t.kind == caprust_core::track::TrackKind::Captions)
            .expect("captions track exists in default_tracks()");
        clip.track_index = captions_track;
        project.add_clip(clip);

        let video_track = project
            .tracks
            .iter()
            .position(|t| t.kind == caprust_core::track::TrackKind::Video)
            .expect("video track exists in default_tracks()");
        let mut placeholder = Clip::new_video("placeholder.mp4", video_track, 0, 2000);
        placeholder.clip_type = ClipType::Video {
            path: "placeholder.mp4".into(),
            duration_ms: 2000,
        };
        project.add_clip(placeholder);

        let plan = plan_from_project(
            &project,
            320,
            240,
            30,
            1,
            23,
            RateMode::Vbr,
            8000,
            "veryfast",
            std::path::Path::new("."),
            0,
            caprust_core::project::VideoEncoder::H264Cpu,
        )
        .expect("plan built");

        // Both segments must contribute text clips. Progressive reveal
        // contributes one per word (2), fallback contributes one (1).
        assert_eq!(
            plan.text_clips.len(),
            3,
            "expected 2 word drawtexts + 1 fallback drawtext: {:#?}",
            plan.text_clips
        );

        // Progressive reveal: accumulated text.
        let contents: Vec<&str> = plan.text_clips.iter().map(|t| t.content.as_str()).collect();
        assert!(
            contents.contains(&"hello"),
            "first word drawtext must show 'hello': {contents:?}"
        );
        assert!(
            contents.contains(&"hello world"),
            "second word drawtext must show accumulated 'hello world': {contents:?}"
        );
        assert!(
            contents.contains(&"plain segment"),
            "fallback drawtext must show the whole segment text: {contents:?}"
        );
    }

    #[test]
    fn escape_movie_path_unix_plain() {
        let p = std::path::Path::new("/tmp/mask.mkv");
        assert_eq!(escape_movie_path(p).as_deref(), Some("'/tmp/mask.mkv'"));
    }

    #[test]
    fn escape_movie_path_windows_backslashes() {
        let p = std::path::Path::new("C:\\proj\\cache\\masks\\abc.mkv");
        let escaped = escape_movie_path(p).expect("no single quote");
        // Drive letter colon must be escaped or ffmpeg's option parser
        // truncates the value at `C`.
        assert_eq!(escaped, "'C\\:/proj/cache/masks/abc.mkv'");
    }

    #[test]
    fn escape_movie_path_rejects_single_quote() {
        let p = std::path::Path::new("/tmp/o'brien.mkv");
        assert!(escape_movie_path(p).is_none());
    }

    fn single_video_plan_with_mask(mask: Option<&str>) -> RenderPlan {
        RenderPlan {
            inputs: vec![InputSpec {
                ffmpeg_index: 0,
                path: PathBuf::from("clip.mp4"),
                source_start_sec: 0.0,
                duration_sec: 1.0,
            }],
            video_clips: vec![VideoClip {
                input_index: 0,
                timeline_start_sec: 0.0,
                duration_sec: 1.0,
                speed: 1.0,
                speed_end: None,
                speed_ease: caprust_core::clip::EaseCurve::Linear,
                speed_range: caprust_core::clip::SpeedRampRange::WholeClip,
                z_order: 0,
                is_image: false,
                effects: Vec::<caprust_core::clip::EffectInstance>::new(),
                transition_in: None,
                transition_out: None,
                transition_in_easing: caprust_core::clip::EaseCurve::Linear,
                transition_out_easing: caprust_core::clip::EaseCurve::Linear,
                transition_duration_sec: 0.35,
                auto_reframe: Vec::new(),
                bg_removal_path: mask.map(std::path::PathBuf::from),
            }],
            audio_clips: vec![],
            text_clips: vec![],
            total_duration_sec: 1.0,
            width: 320,
            height: 240,
            fps_num: 30,
            fps_den: 1,
            has_audio: false,

            skipped: PlanSkipped::default(),

            encoder: caprust_core::project::VideoEncoder::H264Cpu,
            crf: 23,
            rate_mode: RateMode::Vbr,
            bitrate_kbps: 8000,
            preset: "veryfast".to_string(),
            seek_ms: 0,
            seek_optimized: false,
        }
    }

    #[test]
    fn bg_removal_emits_maskedmerge() {
        let plan = single_video_plan_with_mask(Some("/tmp/mask.mkv"));
        let (fg, _v, _a) = plan.build_filtergraph().expect("filtergraph");
        assert!(
            fg.contains("maskedmerge"),
            "expected maskedmerge stage: {fg}"
        );
        assert!(
            fg.contains("movie='"),
            "expected movie= source for the mask: {fg}"
        );
        assert!(
            fg.contains("/tmp/mask.mkv"),
            "mask path missing from filtergraph: {fg}"
        );
        // The intermediate pre-merge label must be present so the
        // video chain stops before the merge, and only the merged
        // output feeds downstream.
        assert!(
            fg.contains("v_pre0"),
            "expected intermediate v_pre0 label: {fg}"
        );
        assert!(
            fg.contains("color=c=black"),
            "expected black base for the merge: {fg}"
        );
    }

    #[test]
    fn bg_removal_absent_has_no_maskedmerge() {
        let plan = single_video_plan_with_mask(None);
        let (fg, _v, _a) = plan.build_filtergraph().expect("filtergraph");
        assert!(
            !fg.contains("maskedmerge"),
            "plain plan must not emit maskedmerge: {fg}"
        );
        assert!(
            !fg.contains("v_pre0"),
            "plain plan must not emit an intermediate label: {fg}"
        );
    }

    #[test]
    fn bg_removal_windows_path_normalized() {
        // Backslashes become forward slashes, and the drive letter
        // colon is escaped (\:) so ffmpeg's option parser does not
        // truncate the value at 'C'.
        let plan = single_video_plan_with_mask(Some("C:\\proj\\cache\\masks\\abc.mkv"));
        let (fg, _v, _a) = plan.build_filtergraph().expect("filtergraph");
        assert!(
            fg.contains("C\\:/proj/cache/masks/abc.mkv"),
            "expected normalized path in filtergraph: {fg}"
        );
    }

    #[test]
    fn auto_reframe_crop_needs_two_keypoints() {
        use caprust_core::clip::ReframeKeypoint;
        let one = vec![ReframeKeypoint {
            t_ms: 0,
            cx_norm: 0.5,
            cy_norm: 0.5,
        }];
        assert!(
            build_auto_reframe_crop(&one, 1920, 1080).is_none(),
            "one keypoint must not emit a crop filter"
        );
        assert!(
            build_auto_reframe_crop(&[], 1920, 1080).is_none(),
            "empty keypoints must not emit a crop filter"
        );
    }

    #[test]
    fn auto_reframe_crop_emits_interpolated_expression() {
        use caprust_core::clip::ReframeKeypoint;
        let kps = vec![
            ReframeKeypoint {
                t_ms: 0,
                cx_norm: 0.2,
                cy_norm: 0.5,
            },
            ReframeKeypoint {
                t_ms: 1000,
                cx_norm: 0.8,
                cy_norm: 0.5,
            },
        ];
        let frag = build_auto_reframe_crop(&kps, 1920, 1080).expect("two keypoints produce a crop");
        // aspect = 1920 / 1080 = 1.777...
        assert!(frag.starts_with("crop="), "must start with crop: {frag}");
        assert!(
            frag.contains("iw"),
            "crop uses input width in expression: {frag}"
        );
        assert!(
            frag.contains("ih"),
            "crop uses input height in expression: {frag}"
        );
        // Both keypoint values must appear in the piecewise expression.
        assert!(frag.contains("0.200000"), "first cx_norm missing: {frag}");
        assert!(frag.contains("0.800000"), "second cx_norm missing: {frag}");
        // Clamping guards the crop from leaving the source frame.
        assert!(frag.contains("min(max("), "must clamp coordinates: {frag}");
    }

    #[test]
    fn particle_produces_atmospheric_chain() {
        let p = build_one_effect("particle", 1.0).expect("particle chain");
        assert!(p.contains("noise="), "particle should emit noise");
        assert!(p.contains("boxblur="), "particle should soften via boxblur");
        assert!(p.contains("tmix="), "particle should trail via tmix");
    }

    #[test]
    fn lens_flare_produces_warm_bloom_chain() {
        let lf = build_one_effect("lens_flare", 1.0).expect("lens_flare chain");
        assert!(
            lf.contains("colorbalance="),
            "lens_flare should warm via colorbalance"
        );
        assert!(lf.contains("curves="), "lens_flare should lift highlights");
        assert!(lf.contains("gblur="), "lens_flare should bloom via gblur");
    }

    #[test]
    fn ghost_frame_count_clamps_between_2_and_12() {
        let g = build_one_effect("ghost", 4.0).unwrap();
        let n: i32 = g
            .split("frames=")
            .nth(1)
            .and_then(|s| s.split(':').next())
            .and_then(|s| s.parse().ok())
            .expect("frame count");
        assert!((2..=12).contains(&n), "frames out of range: {n}");
    }

    #[test]
    fn ease_curve_name_linear_is_none() {
        assert!(ease_curve_name(caprust_core::clip::EaseCurve::Linear).is_none());
    }

    #[test]
    fn ease_curve_name_maps_each_variant() {
        use caprust_core::clip::EaseCurve;
        assert_eq!(ease_curve_name(EaseCurve::EaseIn), Some("cub"));
        assert_eq!(ease_curve_name(EaseCurve::EaseOut), Some("par"));
        assert_eq!(ease_curve_name(EaseCurve::EaseInOut), Some("qsin"));
    }

    #[test]
    fn single_clip_fade_linear_keeps_old_shape() {
        // Byte-identical to the pre-easing filtergraph: no `curve=`
        // parameter when the easing is Linear.
        let plan = single_fade_plan(caprust_core::clip::EaseCurve::Linear);
        let (fg, _, _) = plan.build_filtergraph().expect("fg");
        assert!(
            fg.contains("fade=t=in:st=0:d=0.350,") || fg.contains("fade=t=in:st=0:d=0.350["),
            "expected legacy fade-in without curve: {fg}"
        );
        assert!(
            !fg.contains("curve="),
            "linear easing must not emit a curve parameter: {fg}"
        );
    }

    #[test]
    fn single_clip_fade_ignores_easing_on_builds_without_curve() {
        // The gyan.dev essentials build (2026-01-26) rejects
        // `fade=curve=` with "Option not found". The fade arm
        // therefore always emits a plain linear fade; the EaseCurve
        // stored on the clip is preserved for a future build, but not
        // emitted here.
        use caprust_core::clip::EaseCurve;
        let plan = single_fade_plan(EaseCurve::EaseInOut);
        let (fg, _, _) = plan.build_filtergraph().expect("fg");
        assert!(
            fg.contains("fade=t=in:st=0:d=0.350") && !fg.contains("curve="),
            "fade must not emit curve on a build without support: {fg}"
        );
    }

    #[test]
    fn xfade_names_match_ffmpeg_keywords() {
        assert_eq!(xfade_name("fade"), Some("fade"));
        assert_eq!(xfade_name("slide_l"), Some("slideleft"));
        assert_eq!(xfade_name("zoom_in"), Some("circleopen"));
        assert_eq!(xfade_name("blur_t"), Some("fadeblack"));
    }

    #[test]
    fn atempo_chain_handles_extremes() {
        assert_eq!(atempo_chain(1.0), "");
        assert_eq!(atempo_chain(2.0), ",atempo=2.000000");
        assert!(atempo_chain(4.0).contains("atempo=2.0"));
        assert!(atempo_chain(4.0).ends_with("atempo=2.000000"));
        assert!(atempo_chain(0.25).contains("atempo=0.5"));
    }

    fn single_fade_plan(easing: caprust_core::clip::EaseCurve) -> RenderPlan {
        RenderPlan {
            inputs: vec![InputSpec {
                ffmpeg_index: 0,
                path: PathBuf::from("a.mp4"),
                source_start_sec: 0.0,
                duration_sec: 2.0,
            }],
            video_clips: vec![VideoClip {
                input_index: 0,
                timeline_start_sec: 0.0,
                duration_sec: 2.0,
                speed: 1.0,
                speed_end: None,
                speed_ease: caprust_core::clip::EaseCurve::Linear,
                speed_range: caprust_core::clip::SpeedRampRange::WholeClip,
                z_order: 0,
                is_image: false,
                effects: Vec::<caprust_core::clip::EffectInstance>::new(),
                transition_in: Some("fade".into()),
                transition_out: None,
                transition_in_easing: easing,
                transition_out_easing: caprust_core::clip::EaseCurve::Linear,
                transition_duration_sec: 0.35,
                auto_reframe: Vec::new(),
                bg_removal_path: None,
            }],
            audio_clips: vec![],
            text_clips: vec![],
            total_duration_sec: 2.0,
            width: 320,
            height: 240,
            fps_num: 30,
            fps_den: 1,
            crf: 23,
            rate_mode: RateMode::Vbr,
            bitrate_kbps: 8000,
            preset: "veryfast".into(),
            has_audio: false,
            skipped: PlanSkipped::default(),
            encoder: caprust_core::project::VideoEncoder::H264Cpu,
            seek_ms: 0,
            seek_optimized: false,
        }
    }

    fn enc_cmd_for(
        enc: caprust_core::project::VideoEncoder,
        rate_mode: RateMode,
        bitrate_kbps: u32,
    ) -> Vec<String> {
        let plan = RenderPlan {
            inputs: vec![InputSpec {
                ffmpeg_index: 0,
                path: PathBuf::from("a.mp4"),
                source_start_sec: 0.0,
                duration_sec: 2.0,
            }],
            video_clips: vec![VideoClip {
                input_index: 0,
                timeline_start_sec: 0.0,
                duration_sec: 2.0,
                speed: 1.0,
                speed_end: None,
                speed_ease: caprust_core::clip::EaseCurve::Linear,
                speed_range: caprust_core::clip::SpeedRampRange::WholeClip,
                z_order: 0,
                is_image: false,
                effects: Vec::<caprust_core::clip::EffectInstance>::new(),
                transition_in: None,
                transition_out: None,
                transition_in_easing: caprust_core::clip::EaseCurve::Linear,
                transition_out_easing: caprust_core::clip::EaseCurve::Linear,
                transition_duration_sec: 0.35,
                auto_reframe: Vec::new(),
                bg_removal_path: None,
            }],
            audio_clips: vec![],
            text_clips: vec![],
            total_duration_sec: 2.0,
            width: 1920,
            height: 1080,
            fps_num: 30,
            fps_den: 1,
            crf: 23,
            rate_mode,
            bitrate_kbps,
            preset: "veryfast".into(),
            has_audio: false,
            skipped: PlanSkipped::default(),
            encoder: enc,
            seek_ms: 0,
            seek_optimized: false,
        };
        plan.build_command(
            std::path::Path::new("ffmpeg"),
            std::path::Path::new("out.mp4"),
        )
    }

    fn bg_test_project() -> (caprust_core::ProjectState, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("caprust-bg-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("cache").join("masks")).expect("create temp masks dir");
        let project = caprust_core::ProjectState {
            project_path: Some(root.to_string_lossy().into_owned()),
            ..Default::default()
        };
        (project, root)
    }

    fn bg_test_clip(project: &caprust_core::ProjectState) -> caprust_core::Clip {
        use caprust_core::TrackKind;
        let idx = project
            .tracks
            .iter()
            .position(|t| t.kind == TrackKind::Video)
            .expect("default project has a video track");
        caprust_core::Clip::new_video("source.mp4", idx, 0, 1000)
    }

    #[test]
    fn bg_removal_accepts_valid_mask() {
        let (project, root) = bg_test_project();
        let mut clip = bg_test_clip(&project);
        let mask = root
            .join("cache")
            .join("masks")
            .join(format!("{}.mkv", clip.id));
        std::fs::write(&mask, b"fake-mask").expect("write mask");
        clip.bg_removal = Some(format!("cache/masks/{}.mkv", clip.id));
        let r = resolve_bg_removal_path(&project, &clip);
        assert!(r.is_some(), "valid mask should resolve: {r:?}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bg_removal_rejects_absolute_path() {
        let (project, root) = bg_test_project();
        let mut clip = bg_test_clip(&project);
        clip.bg_removal = Some("C:\\Windows\\System32\\config\\SAM".into());
        assert!(resolve_bg_removal_path(&project, &clip).is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bg_removal_rejects_parent_dir_traversal() {
        let (project, root) = bg_test_project();
        let mut clip = bg_test_clip(&project);
        clip.bg_removal = Some("cache/masks/../../etc/passwd".into());
        assert!(resolve_bg_removal_path(&project, &clip).is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bg_removal_rejects_protocol_url() {
        let (project, root) = bg_test_project();
        let mut clip = bg_test_clip(&project);
        clip.bg_removal = Some("http://attacker/mask.mkv".into());
        assert!(resolve_bg_removal_path(&project, &clip).is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bg_removal_rejects_wrong_filename() {
        let (project, root) = bg_test_project();
        let mut clip = bg_test_clip(&project);
        let other = root.join("cache").join("masks").join("not-the-clip-id.mkv");
        std::fs::write(&other, b"fake").expect("write mask");
        clip.bg_removal = Some("cache/masks/not-the-clip-id.mkv".into());
        assert!(resolve_bg_removal_path(&project, &clip).is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bg_removal_rejects_wrong_extension() {
        let (project, root) = bg_test_project();
        let mut clip = bg_test_clip(&project);
        let png = root
            .join("cache")
            .join("masks")
            .join(format!("{}.png", clip.id));
        std::fs::write(&png, b"fake").expect("write png");
        clip.bg_removal = Some(format!("cache/masks/{}.png", clip.id));
        assert!(resolve_bg_removal_path(&project, &clip).is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bg_removal_rejects_mask_outside_masks_dir() {
        let (project, root) = bg_test_project();
        let mut clip = bg_test_clip(&project);
        // File exists at <proj>/sneaky.mkv, not in cache/masks/.
        let sneaky = root.join("sneaky.mkv");
        std::fs::write(&sneaky, b"fake").expect("write sneaky");
        clip.bg_removal = Some("sneaky.mkv".into());
        assert!(resolve_bg_removal_path(&project, &clip).is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    fn arg_after(args: &[String], key: &str) -> Option<String> {
        let pos = args.iter().position(|a| a == key)?;
        args.get(pos + 1).cloned()
    }

    #[test]
    fn build_command_cpu_h264_uses_libx264_and_crf() {
        use caprust_core::project::VideoEncoder as Enc;
        let a = enc_cmd_for(Enc::H264Cpu, RateMode::Vbr, 8000);
        assert_eq!(arg_after(&a, "-c:v").as_deref(), Some("libx264"));
        assert_eq!(arg_after(&a, "-crf").as_deref(), Some("23"));
        assert_eq!(arg_after(&a, "-preset").as_deref(), Some("veryfast"));
        assert!(!a.iter().any(|x| x == "-cq"), "cpu must not emit -cq");
        assert!(!a.iter().any(|x| x == "-tag:v"), "h264 must not tag");
    }

    #[test]
    fn build_command_cpu_h264_cbr_emits_bitrate_flags() {
        use caprust_core::project::VideoEncoder as Enc;
        let a = enc_cmd_for(Enc::H264Cpu, RateMode::Cbr, 6000);
        assert_eq!(arg_after(&a, "-b:v").as_deref(), Some("6000k"));
        assert_eq!(arg_after(&a, "-maxrate").as_deref(), Some("6000k"));
        assert_eq!(arg_after(&a, "-bufsize").as_deref(), Some("12000k"));
        assert!(arg_after(&a, "-crf").is_none(), "CBR must not emit -crf");
    }

    #[test]
    fn build_command_nvenc_h264_cbr_uses_bitrate() {
        use caprust_core::project::VideoEncoder as Enc;
        let a = enc_cmd_for(Enc::H264Nvenc, RateMode::Cbr, 8000);
        assert_eq!(arg_after(&a, "-b:v").as_deref(), Some("8000k"));
        assert_eq!(arg_after(&a, "-rc").as_deref(), Some("cbr"));
        assert!(arg_after(&a, "-cq").is_none(), "CBR must not emit -cq");
    }

    #[test]
    fn build_command_nvenc_h264_uses_cq_and_no_crf() {
        use caprust_core::project::VideoEncoder as Enc;
        let a = enc_cmd_for(Enc::H264Nvenc, RateMode::Vbr, 8000);
        assert_eq!(arg_after(&a, "-c:v").as_deref(), Some("h264_nvenc"));
        assert_eq!(arg_after(&a, "-cq").as_deref(), Some("23"));
        assert_eq!(arg_after(&a, "-b:v").as_deref(), Some("0"));
        assert_eq!(arg_after(&a, "-preset").as_deref(), Some("p3"));
        assert_eq!(arg_after(&a, "-tune").as_deref(), Some("hq"));
        assert!(!a.iter().any(|x| x == "-crf"), "nvenc must not emit -crf");
    }

    #[test]
    fn build_command_amf_h264_uses_cqp() {
        use caprust_core::project::VideoEncoder as Enc;
        let a = enc_cmd_for(Enc::H264Amf, RateMode::Vbr, 8000);
        assert_eq!(arg_after(&a, "-c:v").as_deref(), Some("h264_amf"));
        assert_eq!(arg_after(&a, "-rc").as_deref(), Some("cqp"));
        assert_eq!(arg_after(&a, "-qp_i").as_deref(), Some("23"));
        assert_eq!(arg_after(&a, "-qp_p").as_deref(), Some("25"));
        assert!(!a.iter().any(|x| x == "-crf"), "amf must not emit -crf");
    }

    #[test]
    fn build_command_hevc_nvenc_tags_hvc1() {
        use caprust_core::project::VideoEncoder as Enc;
        let a = enc_cmd_for(Enc::H265Nvenc, RateMode::Vbr, 8000);
        assert_eq!(arg_after(&a, "-c:v").as_deref(), Some("hevc_nvenc"));
        assert_eq!(arg_after(&a, "-tag:v").as_deref(), Some("hvc1"));
    }

    #[test]
    fn filtergraph_contains_scale() {
        let plan = RenderPlan {
            inputs: vec![InputSpec {
                ffmpeg_index: 0,
                path: PathBuf::from("a.mp4"),
                source_start_sec: 0.0,
                duration_sec: 2.0,
            }],
            video_clips: vec![VideoClip {
                input_index: 0,
                timeline_start_sec: 0.0,
                duration_sec: 2.0,
                speed: 1.0,
                speed_end: None,
                speed_ease: caprust_core::clip::EaseCurve::Linear,
                speed_range: caprust_core::clip::SpeedRampRange::WholeClip,
                z_order: 0,
                is_image: false,
                effects: Vec::<caprust_core::clip::EffectInstance>::new(),
                transition_in: None,
                transition_out: None,
                transition_in_easing: caprust_core::clip::EaseCurve::Linear,
                transition_out_easing: caprust_core::clip::EaseCurve::Linear,
                transition_duration_sec: 0.35,
                auto_reframe: Vec::new(),
                bg_removal_path: None,
            }],
            audio_clips: vec![],
            text_clips: vec![],
            total_duration_sec: 2.0,
            width: 1920,
            height: 1080,
            fps_num: 30,
            fps_den: 1,
            crf: 23,
            rate_mode: RateMode::Vbr,
            bitrate_kbps: 8000,
            preset: "veryfast".into(),
            has_audio: false,

            skipped: PlanSkipped::default(),
            encoder: caprust_core::project::VideoEncoder::H264Cpu,
            seek_ms: 0,
            seek_optimized: false,
        };
        let (fg, _, _) = plan.build_filtergraph().unwrap();
        assert!(fg.contains("scale=1920:1080"));
        assert!(fg.contains("fps=30/1"));
    }
}

#[cfg(test)]
mod text_motion_render_tests {
    use super::*;
    use caprust_core::clip::{TextEffect, TextEffectKind, TextMotion};

    fn base() -> TextClip {
        TextClip {
            content: "hello".into(),
            font_size: 32.0,
            timeline_start_sec: 0.0,
            duration_sec: 1.0,
            above: false,
            z_order: 0,
            style: "default".into(),
            motion: TextMotion::default(),
            effect: None,
            caption_style: None,
        }
    }

    #[test]
    fn no_effect_no_opts() {
        assert!(build_text_effect_opts(None).is_empty());
        let t = base();
        assert!(build_text_effect_opts(t.effect.as_ref()).is_empty());
    }

    #[test]
    fn motion_offsets_build_expected_exprs() {
        let mut t = base();
        t.motion = TextMotion {
            x: 0.25,
            y: -0.1,
            rotation: 0.0,
            scale: 1.0,
        };
        let mx = t.motion.x;
        let my = t.motion.y;
        let x_expr = format!("(w-text_w)/2+({mx:.4})*w");
        let y_base = if t.above { "h*0.08" } else { "h*0.82" };
        let y_expr = format!("{y_base}+({my:.4})*h");
        assert_eq!(x_expr, "(w-text_w)/2+(0.2500)*w");
        assert_eq!(y_expr, "h*0.82+(-0.1000)*h");
    }

    #[test]
    fn motion_scale_multiplies_font_size() {
        let mut t = base();
        t.motion.scale = 1.5;
        let fs = ((t.font_size as f64) * (t.motion.scale as f64))
            .round()
            .max(1.0) as i32;
        assert_eq!(fs, 48);
    }

    #[test]
    fn blink_emits_alpha_mod_expr() {
        let e = TextEffect {
            kind: TextEffectKind::Blink,
            period: 1.0,
            amount: 1.0,
        };
        let opts = build_text_effect_opts(Some(&e));
        assert!(opts.contains(":alpha="), "alpha: {opts}");
        assert!(opts.contains("mod(t,1.000)"), "period: {opts}");
    }

    #[test]
    fn pulse_emits_alpha_sine_expr() {
        let e = TextEffect {
            kind: TextEffectKind::Pulse,
            period: 0.5,
            amount: 0.4,
        };
        let opts = build_text_effect_opts(Some(&e));
        assert!(opts.contains(":alpha="), "alpha: {opts}");
        assert!(opts.contains("sin(2*PI*t/0.500)"), "sine: {opts}");
    }

    #[test]
    fn color_cycle_emits_fontcolor_expr() {
        let e = TextEffect {
            kind: TextEffectKind::ColorCycle,
            period: 1.0,
            amount: 1.0,
        };
        let opts = build_text_effect_opts(Some(&e));
        assert!(opts.contains(":fontcolor_expr=0x"), "fce: {opts}");
        assert!(opts.contains("%{eif"), "eif: {opts}");
    }

    #[test]
    fn zero_period_is_clamped() {
        let e = TextEffect {
            kind: TextEffectKind::Pulse,
            period: 0.0,
            amount: 1.0,
        };
        let opts = build_text_effect_opts(Some(&e));
        assert!(opts.contains("/0.050"), "clamped period: {opts}");
    }
}
