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
}

/// Audio counterpart.
#[derive(Debug, Clone)]
pub struct AudioClip {
    pub input_index: usize,
    pub timeline_start_sec: f64,
    pub duration_sec: f64,
    pub speed: f32,
    /// Speed ramp end. Some(x) => audio tempo uses the ease-weighted
    /// mean of `speed` and `x` (single-instance atempo can't ramp).
    pub speed_end: Option<f32>,
    /// Easing of the ramp. Only used to pick the weighted mean.
    pub speed_ease: caprust_core::clip::EaseCurve,
    /// Linear gain from clip.volume_db.
    pub gain_db: f32,
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

            fg.push_str(&format!(
            "{in_label}trim=duration={dur:.6},{setpts},scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={num}/{den}",
            dur = c.duration_sec,
            w = self.width,
            h = self.height,
            num = self.fps_num,
            den = self.fps_den,
        ));

            let effects_chain = build_effects_chain(&c.effects);
            fg.push_str(&effects_chain);

            fg.push_str(&format!("[{v_out}];"));
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
                let gap = (next.timeline_start_sec - prev_end).abs();
                let has_xfade = next
                    .transition_in
                    .as_deref()
                    .map(is_xfade_id)
                    .unwrap_or(false);
                if has_xfade && gap <= ADJACENCY_TOL_SEC {
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
        for (ri, run) in runs.iter().enumerate() {
            let out_label = format!("v_run{ri}");

            if run.members.len() == 1 {
                // Single clip: apply legacy fade from/to-black edges
                // when requested, then shift PTS to timeline position.
                let idx = run.members[0];
                let c = &self.video_clips[idx];
                let mut tail = String::new();
                if c.transition_in.as_deref() == Some("fade") {
                    tail.push_str(",fade=t=in:st=0:d=0.35");
                }
                if c.transition_out.as_deref() == Some("fade") {
                    let st = (c.duration_sec - 0.35).max(0.0);
                    tail.push_str(&format!(",fade=t=out:st={st:.3}:d=0.35"));
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
                    start = run.start_sec,
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
                    let d = XFADE_DUR_SEC
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
                fg.push_str(&format!(
                    "[{current_label}]setpts=PTS+{start:.6}/TB[{out}];",
                    start = run.start_sec,
                    out = out_label,
                ));
            }

            run_labels.push((run.z_order, run.start_sec, out_label));
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
            let escaped = t
                .content
                .replace('\\', "\\\\")
                .replace(':', "\\:")
                .replace('\'', "\\'");
            // Vertical position: above=true → top, else bottom.
            let y = if t.above {
                "h*0.08".to_string()
            } else {
                "h*0.82".to_string()
            };
            // Style parameters. Any combination not matched falls back to
            // plain white text.
            let style_opts: String = match t.style.as_str() {
                "bold" => ":borderw=2:bordercolor=black".into(),
                "subtitle" => ":box=1:boxcolor=black@0.5:boxborderw=8".into(),
                "lower" => ":box=1:boxcolor=black@0.7:boxborderw=6".into(),
                "quote" => ":italics=1:shadowcolor=black@0.6:shadowx=3:shadowy=3".into(),
                "caption" => ":fontcolor=yellow:borderw=2:bordercolor=black".into(),
                "glow" => {
                    ":shadowcolor=cyan@0.8:shadowx=4:shadowy=4:borderw=1:bordercolor=white".into()
                }
                "handwrite" => ":font='Comic Sans MS'".into(),
                _ => String::new(),
            };
            fg.push_str(&format!(
            "[{v_prev}]drawtext=text='{escaped}':fontcolor=white:fontsize={fs}:x=(w-text_w)/2:y={y}{style_opts}:enable='between(t,{start:.6},{end:.6})'[v_txt{t_i}];",
            v_prev = v_prev,
            escaped = escaped,
            fs = t.font_size.round() as i32,
            y = y,
            style_opts = style_opts,
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
                // Audio cannot vary tempo smoothly across a single
                // atempo instance, so a speed ramp is approximated by
                // the ease-weighted mean of the two endpoints. The
                // video side uses an N-segment piecewise chain and
                // therefore tracks the ease curve more closely than
                // this single constant does.
                //
                // Consequences:
                //   * Clip BOUNDARIES stay in sync: the mean chosen
                //     here is the integral of the same curve, so total
                //     clip duration matches the video side exactly.
                //   * INSIDE a long ramp there is momentary drift of
                //     up to ~0.15x around the mean. For short ramps
                //     (typical of social edits) this is not audible.
                //
                // A segmented atempo chain (one atempo per piecewise
                // step, joined with concat) would eliminate the
                // mid-ramp drift entirely. Tracked in DIRECTIVES
                // §28.2 as "Audio segmented ramp sync".
                let effective_speed = match c.speed_end {
                    Some(s_end) if (s_end - c.speed).abs() > 0.001 => {
                        let t = ease_avg_progress(c.speed_ease) as f32;
                        c.speed + (s_end - c.speed) * t
                    }
                    _ => c.speed,
                };
                let atempo_chain = atempo_chain(effective_speed);
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
                fg.push_str(&format!(
                "{in_label}atrim=duration={dur:.6},asetpts=PTS-STARTPTS{atempo}{gain}{fade_in}{fade_out}[{a_out}];",
                dur = c.duration_sec,
                atempo = atempo_chain,
                gain = gain,
                fade_in = fade_in,
                fade_out = fade_out,
            ));
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
                    // Chain amix, output last link as ctrl_bus.
                    let mut prev = format!("a_delayed{}", ctrl_indices[0]);
                    let last = ctrl_indices.len() - 1;
                    for (k, &idx) in ctrl_indices.iter().enumerate().skip(1) {
                        let out = if k == last {
                            ctrl_bus.to_string()
                        } else {
                            format!("a_ctrl_tmp{k}")
                        };
                        fg.push_str(&format!(
                            "[{prev}][a_delayed{idx}]amix=inputs=2:duration=longest:dropout_transition=0[{out}];",
                        ));
                        prev = out;
                    }
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
            let mut a_prev = String::from("a_base");
            for (i, _c) in self.audio_clips.iter().enumerate() {
                let a_next = format!("a_mix{i}");
                fg.push_str(&format!(
                    "[{a_prev}][{clip}]amix=inputs=2:duration=longest:dropout_transition=0[{a_next}];",
                    a_prev = a_prev,
                    clip = mixed_labels[i],
                    a_next = a_next,
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

/// Build a `setpts=PTS-STARTPTS,setpts='<expr>'` chain for a clip with a
/// speed ramp. Returns None when no ramp applies (i.e. speed_end is
/// absent, equals speed, or the clip has zero duration).
///
/// The ramp is approximated by a piecewise-constant speed curve sampled
/// at RAMP_SEGMENTS points along the OUTPUT timeline. Each segment
/// contributes one linear branch to a nested if() expression, which
/// ffmpeg's setpts evaluates per frame. This keeps the visual curve
/// close to the requested easing while staying a simple string.
///
/// Easing curves: Linear, EaseIn (t^2), EaseOut (1-(1-t)^2), EaseInOut
/// (symmetrised).
///
/// Range: WholeClip (ramp spans the whole clip), FirstN(n) (ramp covers
/// the first n output seconds, rest runs at speed_end), LastN(n) (ramp
/// covers the last n output seconds, earlier portion runs at speed).
fn build_speed_ramp_setpts(
    speed: f32,
    speed_end: f32,
    ease: caprust_core::clip::EaseCurve,
    range: caprust_core::clip::SpeedRampRange,
    clip_in_dur_s: f64,
) -> Option<String> {
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

    // Piecewise-constant segments: (t_in_start, t_in_end, t_out_start, s_seg).
    let dt_out = total_out_dur / (RAMP_SEGMENTS as f64);
    let mut segments: Vec<(f64, f64, f64, f64)> = Vec::with_capacity(RAMP_SEGMENTS);
    let mut t_in_acc = 0.0;
    for i in 0..RAMP_SEGMENTS {
        let t_out_start = i as f64 * dt_out;
        let t_out_mid = t_out_start + dt_out * 0.5;
        let s_mid = s_at(t_out_mid).max(0.01);
        let t_in_end = t_in_acc + s_mid * dt_out;
        segments.push((t_in_acc, t_in_end, t_out_start, s_mid));
        t_in_acc = t_in_end;
    }

    // Nested if() from the tail backwards.
    let build_branch = |t_in_s: f64, t_out_s: f64, s: f64| -> String {
        format!("({t_out_s:.6}+(T/TB-{t_in_s:.6})/{s:.6})",)
    };

    let (t_in_last, _e, t_out_last, s_last) = segments[RAMP_SEGMENTS - 1];
    let mut expr = build_branch(t_in_last, t_out_last, s_last);

    for i in (0..RAMP_SEGMENTS - 1).rev() {
        let (t_in_s, t_in_e, t_out_s, s) = segments[i];
        let seg = build_branch(t_in_s, t_out_s, s);
        expr = format!("if(lt(T/TB\\,{t_in_e:.6})\\,{seg}\\,{expr})",);
    }

    Some(format!("setpts='TB*({expr})'"))
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
                let gap_sec =
                    ((c.start_time_ms as i64 - prev_end as i64).unsigned_abs() as f64) / 1000.0;
                let has_xfade = c.transition_in.as_deref().map(is_xfade_id).unwrap_or(false);
                if has_xfade && gap_sec <= ADJACENCY_TOL_SEC {
                    let prev_dur = p.duration_ms as f64 / 1000.0;
                    let curr_dur = c.duration_ms as f64 / 1000.0;
                    let d = XFADE_DUR_SEC
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

// plan_from_project has one argument per render dimension the caller
// knows about. Bundling them into a struct would just move the same
// fields behind one more layer. The signature is stable; leave it.
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
    // Clips downstream of an xfade appear earlier in the render than
    // their original timeline position; shift their audio and their
    // burned-in captions to match so the mix and the on-screen text
    // stay in sync with the shortened video.
    let xfade_audio_shifts = compute_xfade_audio_shifts(project);

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
            match &c.clip_type {
                ClipType::Video { path, .. } => {
                    let dur_sec = c.duration_ms as f64 / 1000.0;
                    let idx = register_input(&mut inputs, path, 0.0, dur_sec);
                    video_clips.push(VideoClip {
                        input_index: idx,
                        timeline_start_sec: c.start_time_ms as f64 / 1000.0,
                        duration_sec: dur_sec,
                        speed: c.speed,
                        speed_end: c.speed_end,
                        speed_ease: c.speed_ease,
                        speed_range: c.speed_range,
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
                        speed_end: c.speed_end,
                        speed_ease: c.speed_ease,
                        speed_range: c.speed_range,
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
                    style,
                } => {
                    text_clips.push(TextClip {
                        content: content.clone(),
                        font_size: *font_size,
                        timeline_start_sec: c.start_time_ms as f64 / 1000.0,
                        duration_sec: c.duration_ms as f64 / 1000.0,
                        above: *above || track_kind == TrackKind::Overlay,
                        z_order: z,
                        style: style.clone(),
                    });
                }
                ClipType::Captions { segments, .. } => {
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
                    let shift_sec = xfade_audio_shifts.get(&c.id).copied().unwrap_or(0.0);
                    let clip_start_sec = (c.start_time_ms as f64 / 1000.0 - shift_sec).max(0.0);
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
                            // Burned-in captions use the "caption" preset
                            // (yellow, black border) regardless of the
                            // style selector on any unrelated Text clip.
                            style: "caption".to_string(),
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
        let dur_sec = c.duration_ms as f64 / 1000.0;
        let idx = register_input(&mut inputs, path, 0.0, dur_sec);
        let shift_sec = xfade_audio_shifts.get(&c.id).copied().unwrap_or(0.0);
        let start_sec = (c.start_time_ms as f64 / 1000.0 - shift_sec).max(0.0);

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
            gain_db: c.volume_db,
            fade_in_sec: fi,
            fade_out_sec: fo,
            volume_keyframes: kfs,
            clip_id: c.id,
            duck_against: c.duck_against,
        });
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
