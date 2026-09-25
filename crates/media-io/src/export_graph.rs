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
}

/// Audio counterpart.
#[derive(Debug, Clone)]
pub struct AudioClip {
    pub input_index: usize,
    pub timeline_start_sec: f64,
    pub duration_sec: f64,
    pub speed: f32,
    /// Linear gain from clip.volume_db.
    pub gain_db: f32,
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
    pub preset: String,
    pub has_audio: bool,
}

impl RenderPlan {
    /// Build the `-filter_complex` string + `-map` arguments.
    /// Returns (filter_graph, video_label, audio_label).
    pub fn build_filtergraph(&self) -> Result<(String, String, Option<String>)> {
        let mut fg = String::new();

        // -------- VIDEO: per-clip chains --------
        let mut v_labels: Vec<String> = Vec::with_capacity(self.video_clips.len());
        for (i, c) in self.video_clips.iter().enumerate() {
            let in_label = if c.is_image {
                // Images need `-loop 1` on the input; the input label still works.
                format!("[{}:v]", c.input_index)
            } else {
                format!("[{}:v]", c.input_index)
            };
            let v_out = format!("v{i}_pre");

            let setpts = if (c.speed - 1.0).abs() < 0.001 {
                String::from("setpts=PTS-STARTPTS")
            } else {
                format!("setpts=(PTS-STARTPTS)/{:.6}", c.speed)
            };

            // Base chain: trim, reset, scale, fps.
            fg.push_str(&format!(
            "{in_label}trim=duration={dur:.6},{setpts},scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={num}/{den}",
            dur = c.duration_sec,
            w = self.width,
            h = self.height,
            num = self.fps_num,
            den = self.fps_den,
        ));

            // Effect chain
            let effects_chain = build_effects_chain(&c.effects);
            fg.push_str(&effects_chain);

            // Edge transitions (fade in/out)
            let fade_in = if c.transition_in.as_deref() == Some("fade") {
                ",fade=t=in:st=0:d=0.35".to_string()
            } else {
                String::new()
            };
            let fade_out = if c.transition_out.as_deref() == Some("fade") {
                let st = (c.duration_sec - 0.35).max(0.0);
                format!(",fade=t=out:st={st:.3}:d=0.35")
            } else {
                String::new()
            };
            fg.push_str(&fade_in);
            fg.push_str(&fade_out);

            // Shift this clip's PTS to its position on the timeline.
            // Without this, `overlay` matches by PTS and everything past
            // the first clip is shown at the wrong moment (usually black).
            fg.push_str(&format!(
                ",setpts=PTS+{start:.6}/TB[{v_out}];",
                start = c.timeline_start_sec,
                v_out = v_out,
            ));
            v_labels.push(v_out);
        }

        // -------- Black base --------
        fg.push_str(&format!(
            "color=c=black:s={w}x{h}:r={num}/{den}:d={dur:.6}[v_base];",
            w = self.width,
            h = self.height,
            num = self.fps_num,
            den = self.fps_den,
            dur = self.total_duration_sec,
        ));

        // -------- Composite clips in z-order --------
        // Sort a copy of indices by (z_order, timeline_start) so higher z is later.
        let mut order: Vec<usize> = (0..self.video_clips.len()).collect();
        order.sort_by(|&a, &b| {
            let ca = &self.video_clips[a];
            let cb = &self.video_clips[b];
            ca.z_order.cmp(&cb.z_order).then(
                ca.timeline_start_sec
                    .partial_cmp(&cb.timeline_start_sec)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        });

        let mut v_prev = String::from("v_base");
        for (overlay_i, &clip_i) in order.iter().enumerate() {
            let v_next = format!("v_ov{overlay_i}");
            fg.push_str(&format!(
                "[{v_prev}][{clip}]overlay=shortest=0:eof_action=pass[v_ov{overlay_i}];",
                v_prev = v_prev,
                clip = v_labels[clip_i],
                overlay_i = overlay_i,
            ));
            v_prev = v_next;
        }

        // -------- Text overlays (drawtext) --------
        // Two passes: below video? Not supported yet; all go on top for now.
        for (t_i, t) in self.text_clips.iter().enumerate() {
            let v_next = format!("v_txt{t_i}");
            let escaped = t
                .content
                .replace('\\', "\\\\")
                .replace(':', "\\:")
                .replace('\'', "\\'");
            // Approximate vertical position: above=true → top, else bottom.
            let y = if t.above {
                "h*0.08".to_string()
            } else {
                "h*0.82".to_string()
            };
            fg.push_str(&format!(
            "[{v_prev}]drawtext=text='{escaped}':fontcolor=white:fontsize={fs}:x=(w-text_w)/2:y={y}:enable='between(t,{start:.6},{end:.6})'[v_txt{t_i}];",
            v_prev = v_prev,
            escaped = escaped,
            fs = t.font_size.round() as i32,
            y = y,
            start = t.timeline_start_sec,
            end = t.timeline_start_sec + t.duration_sec,
            t_i = t_i,
        ));
            v_prev = v_next;
        }
        let v_final = v_prev;

        // -------- AUDIO (unchanged) --------
        let a_final = if self.has_audio && !self.audio_clips.is_empty() {
            let mut a_labels: Vec<String> = Vec::with_capacity(self.audio_clips.len());
            for (i, c) in self.audio_clips.iter().enumerate() {
                let in_label = format!("[{}:a]", c.input_index);
                let a_out = format!("a{i}_trim");
                let atempo_chain = atempo_chain(c.speed);
                let gain = if c.gain_db.abs() < 0.001 {
                    String::new()
                } else {
                    format!(",volume={:.4}dB", c.gain_db)
                };
                fg.push_str(&format!(
                "{in_label}atrim=duration={dur:.6},asetpts=PTS-STARTPTS{atempo}{gain}[{a_out}];",
                dur = c.duration_sec,
                atempo = atempo_chain,
                gain = gain,
            ));
                a_labels.push(a_out);
            }

            fg.push_str(&format!(
                "anullsrc=channel_layout=stereo:sample_rate=48000:d={dur:.6}[a_base];",
                dur = self.total_duration_sec,
            ));

            let mut a_prev = String::from("a_base");
            for (i, c) in self.audio_clips.iter().enumerate() {
                let a_next = format!("a_mix{i}");
                let delay_ms = (c.timeline_start_sec * 1000.0).round() as i64;
                fg.push_str(&format!(
                    "[{clip}]adelay={delay}|{delay}[a_delayed{i}];",
                    clip = a_labels[i],
                    delay = delay_ms,
                    i = i,
                ));
                fg.push_str(&format!(
                "[{a_prev}][a_delayed{i}]amix=inputs=2:duration=longest:dropout_transition=0[{a_next}];"
            ));
                a_prev = a_next;
            }
            fg.push_str(&format!(
                "[{a_prev}]atrim=duration={dur:.6},asetpts=PTS-STARTPTS[a_final]",
                a_prev = a_prev,
                dur = self.total_duration_sec,
            ));
            Some(String::from("a_final"))
        } else {
            None
        };

        if fg.ends_with(';') {
            fg.pop();
        }

        Ok((fg, v_final, a_final))
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

        // Encoder
        args.push("-c:v".into());
        args.push("libx264".into());
        args.push("-preset".into());
        args.push(self.preset.clone());
        args.push("-crf".into());
        args.push(self.crf.to_string());
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
            ",chromashift=cbh={:.1}:crh=-{:.1}",
            4.0 * amount,
            4.0 * amount
        ),
        "rgb_split" => format!(
            ",chromashift=cbh={:.1}:crh=-{:.1}",
            6.0 * amount,
            6.0 * amount
        ),
        "flash" => format!(
            ",eq=brightness={:.3}:contrast={:.3}",
            0.15 * amount,
            1.0 + 0.1 * amount
        ),
        "mirror" => ",hflip".into(),
        "kaleido" => ",vflip,hflip".into(),
        "old_film" => ",curves=preset=vintage,noise=alls=15:allf=t".into(),
        "vhs" => format!(
            ",chromashift=cbh=2:crh=-2,noise=alls={:.0}:allf=t",
            8.0 * amount
        ),
        "light_leak" => format!(
            ",colorbalance=rm={:.3}:gm={:.3}",
            0.15 * amount,
            0.05 * amount
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
            let amp = (8.0 * amount).clamp(2.0, 40.0);
            let freq = (1.0 / amount.max(0.25)).clamp(0.3, 3.0);
            // iw-2*amp .. iw (zoom 1x..1+2amp/iw). We oscillate the crop
            // size between (iw-amp) and iw, using sin mapped to [0,1].
            // Crop x/y stay centered: (iw-ow)/2.
            //
            // The `crop` filter accepts expressions in `t` and `ow`/`oh`
            // which refer to the output size. Use `sin(2*PI*t*freq)`.
            format!(
                ",crop=w='iw-{amp:.2}*(1+sin(2*PI*t*{freq:.3}))/2':                 h='ih-{amp:.2}*(1+sin(2*PI*t*{freq:.3}))/2':                 x='(iw-ow)/2':y='(ih-oh)/2',                 scale=iw:ih:flags=bicubic"
            )
        }

        _ => return None,
    };
    Some(frag)
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
pub fn plan_from_project(
    project: &caprust_core::ProjectState,
    width: u32,
    height: u32,
    fps_num: i64,
    fps_den: i64,
    crf: u8,
    preset: &str,
    models_dir: &std::path::Path,
) -> Result<RenderPlan> {
    use caprust_core::{ClipType, TrackKind};

    let mut inputs: Vec<InputSpec> = Vec::new();
    let mut video_clips: Vec<VideoClip> = Vec::new();
    let mut audio_clips: Vec<AudioClip> = Vec::new();
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

    // Z-order index assigned to each clip as we walk tracks bottom-up.
    for (z, &t_idx) in video_track_order.iter().enumerate() {
        let z = z as u32;
        let track_kind = project.tracks[t_idx].kind;
        let mut clips: Vec<&caprust_core::Clip> = project
            .clips
            .iter()
            .filter(|c| c.track_index == t_idx)
            .collect();
        clips.sort_by_key(|c| c.start_time_ms);

        for c in clips {
            match &c.clip_type {
                ClipType::Video { path, .. } => {
                    let dur_sec = c.duration_ms as f64 / 1000.0;
                    let idx = register_input(&mut inputs, path, 0.0, dur_sec);
                    video_clips.push(VideoClip {
                        input_index: idx,
                        timeline_start_sec: c.start_time_ms as f64 / 1000.0,
                        duration_sec: dur_sec,
                        speed: c.speed,
                        z_order: z,
                        is_image: false,
                        effects: c.effects.clone(),
                        transition_in: c.transition_in.clone(),
                        transition_out: c.transition_out.clone(),
                    });
                }
                ClipType::Image { path, .. } => {
                    let dur_sec = c.duration_ms as f64 / 1000.0;
                    let idx = register_input(&mut inputs, path, 0.0, dur_sec);
                    video_clips.push(VideoClip {
                        input_index: idx,
                        timeline_start_sec: c.start_time_ms as f64 / 1000.0,
                        duration_sec: dur_sec,
                        speed: c.speed,
                        z_order: z,
                        is_image: true,
                        effects: c.effects.clone(),
                        transition_in: c.transition_in.clone(),
                        transition_out: c.transition_out.clone(),
                    });
                }
                ClipType::TextOverlay {
                    content,
                    font_size,
                    above,
                } => {
                    text_clips.push(TextClip {
                        content: content.clone(),
                        font_size: *font_size,
                        timeline_start_sec: c.start_time_ms as f64 / 1000.0,
                        duration_sec: c.duration_ms as f64 / 1000.0,
                        above: *above || track_kind == TrackKind::Overlay,
                        z_order: z,
                    });
                }
                ClipType::Captions { segments, .. } => {
                    // Each caption segment becomes its own drawtext with an
                    // enable window. Coordinates are relative to the clip's
                    // start: whisper reports absolute source times, the clip
                    // holds them relative to its own timeline position.
                    let clip_start_sec = c.start_time_ms as f64 / 1000.0;
                    for seg in segments {
                        let seg_start_sec = clip_start_sec + seg.start_ms as f64 / 1000.0;
                        let seg_end_sec = clip_start_sec + seg.end_ms as f64 / 1000.0;
                        let seg_dur = (seg_end_sec - seg_start_sec).max(0.05);
                        text_clips.push(TextClip {
                            content: seg.text.clone(),
                            font_size: 32.0,
                            timeline_start_sec: seg_start_sec,
                            duration_sec: seg_dur,
                            // Captions always render above everything except
                            // pinned overlay; force above = true.
                            above: true,
                            z_order: z,
                        });
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
    let audio_track_clip_count: usize = project
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.kind == TrackKind::Audio)
        .map(|(i, _)| project.clips.iter().filter(|c| c.track_index == i).count())
        .sum();

    let has_audio_track = audio_track_clip_count > 0;

    let audio_source_tracks: Vec<usize> = if has_audio_track {
        project
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.kind == TrackKind::Audio)
            .map(|(i, _)| i)
            .collect()
    } else {
        // Use the exact same track order that produced video_clips so
        // embedded audio is harvested from EVERY video-bearing track —
        // including Overlay, which the plain Video filter missed.
        video_track_order.clone()
    };

    let audio_from_video = !has_audio_track;

    for t_idx in audio_source_tracks {
        let mut clips: Vec<&caprust_core::Clip> = project
            .clips
            .iter()
            .filter(|c| c.track_index == t_idx)
            .collect();
        clips.sort_by_key(|c| c.start_time_ms);
        for c in clips {
            // Narration clips resolve to a cached WAV on disk. The path
            // is deterministic from (text, voice_id) via
            // core::cache::narration_path, so the same project can be
            // opened on another machine and re-synthesized.
            let narration_path_owned: Option<String> = match &c.clip_type {
                ClipType::Narration { voice_id, text, .. } => Some(
                    caprust_core::cache::narration_path(models_dir, text, voice_id)
                        .to_string_lossy()
                        .into_owned(),
                ),
                _ => None,
            };

            let path_opt: Option<&String> = match &c.clip_type {
                ClipType::Audio { path, .. } => Some(path),
                ClipType::Video { path, .. } if audio_from_video => Some(path),
                ClipType::Narration { .. } => narration_path_owned.as_ref(),
                _ => None,
            };
            if let Some(path) = path_opt {
                // Narration WAVs are cached on demand. If the file is not
                // there yet (project shared, cache cleared, first render),
                // skip the audio with a warning rather than letting
                // ffmpeg fail with an unhelpful error. F5 wires the UI
                // synthesis flow that populates this cache.
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
                let dur_sec = c.duration_ms as f64 / 1000.0;
                let idx = register_input(&mut inputs, path, 0.0, dur_sec);
                audio_clips.push(AudioClip {
                    input_index: idx,
                    timeline_start_sec: c.start_time_ms as f64 / 1000.0,
                    duration_sec: dur_sec,
                    speed: c.speed,
                    gain_db: c.volume_db,
                });
            }
        }
    }

    if video_clips.is_empty() && text_clips.is_empty() {
        anyhow::bail!("no video or text clips on any video track — nothing to export");
    }

    let total_duration_sec = video_clips
        .iter()
        .map(|c| c.timeline_start_sec + c.duration_sec)
        .chain(
            text_clips
                .iter()
                .map(|c| c.timeline_start_sec + c.duration_sec),
        )
        .fold(0.0_f64, f64::max);

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
        text_clips,
        total_duration_sec,
        width: width.max(2) & !1,
        height: height.max(2) & !1,
        fps_num,
        fps_den,
        crf,
        preset: preset.to_string(),
        has_audio,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atempo_chain_handles_extremes() {
        assert_eq!(atempo_chain(1.0), "");
        assert_eq!(atempo_chain(2.0), ",atempo=2.000000");
        assert!(atempo_chain(4.0).contains("atempo=2.0"));
        assert!(atempo_chain(4.0).ends_with("atempo=2.000000"));
        assert!(atempo_chain(0.25).contains("atempo=0.5"));
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
                z_order: 0,
                is_image: false,
                effects: Vec::<caprust_core::clip::EffectInstance>::new(),
                transition_in: None,
                transition_out: None,
            }],
            audio_clips: vec![],
            text_clips: vec![],
            total_duration_sec: 2.0,
            width: 1920,
            height: 1080,
            fps_num: 30,
            fps_den: 1,
            crf: 23,
            preset: "veryfast".into(),
            has_audio: false,
        };
        let (fg, _, _) = plan.build_filtergraph().unwrap();
        assert!(fg.contains("scale=1920:1080"));
        assert!(fg.contains("fps=30/1"));
    }
}
