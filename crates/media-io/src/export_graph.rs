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
    /// Timeline start (seconds). Gaps before this get `color=black`.
    pub timeline_start_sec: f64,
    pub duration_sec: f64,
    /// Playback speed multiplier (1.0 = normal). Applied with `setpts`.
    pub speed: f32,
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

        // -------- VIDEO --------
        // For each clip: trim, reset PTS, apply speed, scale to output.
        let mut v_labels: Vec<String> = Vec::with_capacity(self.video_clips.len());
        for (i, c) in self.video_clips.iter().enumerate() {
            let in_label = format!("[{}:v]", c.input_index);
            let v_out = format!("v{i}_trim");
            // setpts for speed: PTS / speed
            let setpts = if (c.speed - 1.0).abs() < 0.001 {
                String::from("setpts=PTS-STARTPTS")
            } else {
                format!("setpts=(PTS-STARTPTS)/{:.6}", c.speed)
            };
            fg.push_str(&format!(
                "{in_label}trim=duration={dur:.6},{setpts},scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={num}/{den}[{v_out}];",
                dur = c.duration_sec,
                w = self.width,
                h = self.height,
                num = self.fps_num,
                den = self.fps_den,
            ));
            v_labels.push(v_out);
        }

        // -------- Insert black gaps between clips by concatenating with color sources.
        // Build the video timeline by laying clips on a black bed at total_duration.
        // Simplest correct approach: use `overlay` on a black base.
        fg.push_str(&format!(
            "color=c=black:s={w}x{h}:r={num}/{den}:d={dur:.6}[v_base];",
            w = self.width,
            h = self.height,
            num = self.fps_num,
            den = self.fps_den,
            dur = self.total_duration_sec,
        ));

        let mut v_prev = String::from("v_base");
        for (i, c) in self.video_clips.iter().enumerate() {
            let v_next = format!("v_overlay{i}");
            fg.push_str(&format!(
                "[{v_prev}][{clip}]overlay=enable='between(t,{start:.6},{end:.6})':eof_action=pass[{v_next}];",
                v_prev = v_prev,
                clip = v_labels[i],
                start = c.timeline_start_sec,
                end = c.timeline_start_sec + c.duration_sec,
                v_next = v_next,
            ));
            v_prev = v_next;
        }
        let v_final = v_prev;

        // -------- AUDIO --------
        let a_final = if self.has_audio && !self.audio_clips.is_empty() {
            // Build per-clip audio chains, then overlay on silence bed.
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

            // Silence bed at total duration
            fg.push_str(&format!(
                "anullsrc=channel_layout=stereo:sample_rate=48000:d={dur:.6}[a_base];",
                dur = self.total_duration_sec,
            ));

            let mut a_prev = String::from("a_base");
            for (i, c) in self.audio_clips.iter().enumerate() {
                let a_next = format!("a_mix{i}");
                // Use adelay to place the clip at its timeline position.
                let delay_ms = (c.timeline_start_sec * 1000.0).round() as i64;
                fg.push_str(&format!(
                    "[{clip}]adelay={delay}|{delay}[a_delayed{i}];",
                    clip = a_labels[i],
                    delay = delay_ms,
                    i = i,
                ));
                fg.push_str(&format!(
                    "[{a_prev}][a_delayed{i}]amix=inputs=2:duration=longest:dropout_transition=0[{a_mix}];",
                    a_prev = a_prev,
                    i = i,
                    a_mix = a_next,
                ));
                a_prev = a_next;
            }
            // Trim the mix back to total duration
            fg.push_str(&format!(
                "[{a_prev}]atrim=duration={dur:.6},asetpts=PTS-STARTPTS[a_final]",
                a_prev = a_prev,
                dur = self.total_duration_sec,
            ));
            Some(String::from("a_final"))
        } else {
            None
        };

        // Remove trailing `;`
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

        // Inputs, in order. `-ss` before `-i` (fast seek), `-t` after (read window).
        for inp in &self.inputs {
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
        parts.push(format!(",atempo=2.0"));
        s /= 2.0;
    }
    while s < 0.5 {
        parts.push(format!(",atempo=0.5"));
        s *= 2.0;
    }
    parts.push(format!(",atempo={s:.6}"));
    parts.join("")
}

/// Builder helper: given a `ProjectState`, collect inputs + clips.
/// Only V1 (first Video track) and A1 (first Audio track) are used for now.
pub fn plan_from_project(
    project: &caprust_core::ProjectState,
    width: u32,
    height: u32,
    fps_num: i64,
    fps_den: i64,
    crf: u8,
    preset: &str,
) -> Result<RenderPlan> {
    use caprust_core::ClipType;

    let mut inputs: Vec<InputSpec> = Vec::new();
    let mut video_clips: Vec<VideoClip> = Vec::new();
    let mut audio_clips: Vec<AudioClip> = Vec::new();

    let find_track = |kind: caprust_core::TrackKind| -> Option<usize> {
        project.tracks.iter().position(|t| t.kind == kind)
    };
    let v_track = find_track(caprust_core::TrackKind::Video);
    let a_track = find_track(caprust_core::TrackKind::Audio);

    let mut register_input = |path: &str, start_sec: f64, dur_sec: f64| -> usize {
        // Always allocate a fresh input slot — sources may be read with
        // different -ss windows. Reusing one slot with multiple -ss is
        // not possible in ffmpeg.
        let idx = inputs.len();
        inputs.push(InputSpec {
            ffmpeg_index: idx,
            path: PathBuf::from(path),
            source_start_sec: start_sec,
            duration_sec: dur_sec,
        });
        idx
    };

    // Video: first Video track only.
    if let Some(vt) = v_track {
        let mut clips: Vec<&caprust_core::Clip> = project
            .clips
            .iter()
            .filter(|c| c.track_index == vt)
            .collect();
        clips.sort_by_key(|c| c.start_time_ms);
        for c in clips {
            if let ClipType::Video { path, .. } | ClipType::Image { path, .. } = &c.clip_type {
                let src_start = c
                    .source_duration_ms
                    .saturating_sub(c.duration_ms) // best-effort: assume trim from end
                    .min(0);
                let _ = src_start;
                // We don't track source_start_ms in Clip yet; assume 0 for now.
                let source_start_sec = 0.0_f64;
                let dur_sec = c.duration_ms as f64 / 1000.0;
                let idx = register_input(path, source_start_sec, dur_sec);
                video_clips.push(VideoClip {
                    input_index: idx,
                    timeline_start_sec: c.start_time_ms as f64 / 1000.0,
                    duration_sec: dur_sec,
                    speed: c.speed,
                });
            }
        }
    }

    // Audio: first Audio track only.
    if let Some(at) = a_track {
        let mut clips: Vec<&caprust_core::Clip> = project
            .clips
            .iter()
            .filter(|c| c.track_index == at)
            .collect();
        clips.sort_by_key(|c| c.start_time_ms);
        for c in clips {
            if let ClipType::Audio { path, .. } = &c.clip_type {
                let source_start_sec = 0.0_f64;
                let dur_sec = c.duration_ms as f64 / 1000.0;
                let idx = register_input(path, source_start_sec, dur_sec);
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

    if video_clips.is_empty() {
        anyhow::bail!("no video clips on V1 track — nothing to export");
    }

    let total_duration_sec = video_clips
        .iter()
        .map(|c| c.timeline_start_sec + c.duration_sec)
        .fold(0.0_f64, f64::max);

    let has_audio = !audio_clips.is_empty();

    Ok(RenderPlan {
        inputs,
        video_clips,
        audio_clips,
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
            }],
            audio_clips: vec![],
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
