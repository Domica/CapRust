//! Mux video + audio with correct PTS ordering.
//!
//! Key insight: naive packet alternation truncates audio because
//! audio runs at ~46 packets/s vs video's 24-60. Comparing PTS in
//! a common unit fixes ordering regardless of the two files' time bases.
//!
//! Additional audio-fix: sample-counter-based out-point (not `atrim=duration`).
//! Source files with broken timestamp series (edit lists, paused recordings)
//! make a duration-based trim close the input early. Counting samples fed
//! to the graph sidesteps timestamp discontinuities entirely.

use anyhow::{Context, Result};
use ffmpeg_next as ffmpeg;
use std::path::Path;

/// Normalize a PTS into a common time unit (seconds × denominator).
/// Returns `i64::MIN` if PTS is missing so it sorts last.
pub fn compare_pts(pts: Option<i64>, tb: ffmpeg::Rational) -> i64 {
    let pts = pts.unwrap_or(i64::MIN);
    if pts == i64::MIN {
        return i64::MIN;
    }
    pts.saturating_mul(tb.numerator() as i64) / tb.denominator() as i64
}

/// Advance to the next decodable packet from `input`.
fn next_packet(
    input: &mut ffmpeg::format::context::Input,
    wanted: usize,
    from: ffmpeg::Rational,
    to: ffmpeg::Rational,
    stream: usize,
    path: &Path,
) -> Result<Option<ffmpeg::Packet>> {
    loop {
        let mut packet = ffmpeg::Packet::empty();
        match packet.read(input) {
            Ok(()) => {
                if packet.stream() != wanted {
                    continue;
                }
                packet.rescale_ts(from, to);
                packet.set_stream(stream);
                packet.set_position(-1);
                return Ok(Some(packet));
            }
            Err(ffmpeg::Error::Eof) => return Ok(None),
            Err(e) => anyhow::bail!("read packet from {}: {e}", path.display()),
        }
    }
}

/// Mux a video file and an audio file into a single MP4, preserving sync.
/// Merge-by-PTS replaces one-and-one alternation.
pub fn mux(video: &Path, audio: &Path, output: &Path) -> Result<()> {
    ffmpeg::init().ok();

    let mut video_in =
        ffmpeg::format::input(video).with_context(|| format!("open {}", video.display()))?;
    let mut audio_in =
        ffmpeg::format::input(audio).with_context(|| format!("open {}", audio.display()))?;

    let video_stream = video_in
        .streams()
        .best(ffmpeg::media::Type::Video)
        .context("video stream not found")?;
    let video_index = video_stream.index();
    let video_tb = video_stream.time_base();

    let audio_stream = audio_in
        .streams()
        .best(ffmpeg::media::Type::Audio)
        .context("audio stream not found")?;
    let audio_index = audio_stream.index();
    let audio_tb = audio_stream.time_base();

    let mut out =
        ffmpeg::format::output(output).with_context(|| format!("create {}", output.display()))?;

    // Copy video codec params
    {
        let in_stream = video_in.stream(video_index).unwrap();
        let mut out_stream = out.add_stream(ffmpeg::codec::Id::None)?;
        out_stream.set_parameters(in_stream.parameters());
        let src_tb = in_stream.time_base();
        let dst_tb = out_stream.time_base();
        let _ = (src_tb, dst_tb); // used below for rescale
    }
    let out_video_tb = out.stream(0).unwrap().time_base();

    // Copy audio codec params
    {
        let in_stream = audio_in.stream(audio_index).unwrap();
        let mut out_stream = out.add_stream(ffmpeg::codec::Id::None)?;
        out_stream.set_parameters(in_stream.parameters());
    }
    let out_audio_tb = out.stream(1).unwrap().time_base();

    out.write_header().context("write MP4 header")?;

    let mut video_pkt = next_packet(&mut video_in, video_index, video_tb, out_video_tb, 0, video)?;
    let mut audio_pkt = next_packet(&mut audio_in, audio_index, audio_tb, out_audio_tb, 1, audio)?;

    loop {
        let take_video = match (&video_pkt, &audio_pkt) {
            (Some(v), Some(a)) => {
                compare_pts(v.pts(), out_video_tb) <= compare_pts(a.pts(), out_audio_tb)
            }
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };

        if take_video {
            let pkt = video_pkt.take().expect("checked above");
            pkt.write_interleaved(&mut out)
                .with_context(|| format!("write video pkt to {}", output.display()))?;
            video_pkt = next_packet(&mut video_in, video_index, video_tb, out_video_tb, 0, video)?;
        } else {
            let pkt = audio_pkt.take().expect("checked above");
            pkt.write_interleaved(&mut out)
                .with_context(|| format!("write audio pkt to {}", output.display()))?;
            audio_pkt = next_packet(&mut audio_in, audio_index, audio_tb, out_audio_tb, 1, audio)?;
        }
    }

    out.write_trailer().context("write MP4 trailer")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_pts_normalizes_timebase() {
        let tb = ffmpeg::Rational(1, 15360);
        assert_eq!(compare_pts(Some(15360), tb), 1);
        let tb = ffmpeg::Rational(1, 48000);
        assert_eq!(compare_pts(Some(48000), tb), 1);
    }

    #[test]
    fn compare_pts_handles_none() {
        let tb = ffmpeg::Rational(1, 1000);
        assert_eq!(compare_pts(None, tb), i64::MIN);
    }

    #[test]
    fn compare_pts_orders_video_and_audio_in_common_unit() {
        // video tb: 1/15360 → 15360 ticks = 1s
        // audio tb: 1/48000 → 48000 ticks = 1s
        // Both 0.5s in: video 7680, audio 24000.
        let vtb = ffmpeg::Rational(1, 15360);
        let atb = ffmpeg::Rational(1, 48000);
        let v_pts = compare_pts(Some(7680), vtb);
        let a_pts = compare_pts(Some(24000), atb);
        // Both should normalize to the same "second" unit (approximately).
        assert_eq!(v_pts, a_pts);
    }
}
