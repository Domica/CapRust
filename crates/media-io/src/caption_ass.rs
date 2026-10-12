//! Caption rendering through libass instead of chained `drawtext`.
//!
//! Progressive-reveal captions used to emit one `drawtext` per word,
//! each drawing the accumulated text so far, with its own enable
//! window. That is a full-frame CPU pass per word: a 58 s narration
//! turned into ~150 sequential `drawtext` filters sitting in front of
//! the encoder, which starved NVENC and made the export behave like
//! software encoding. The per-word windows also overlapped whenever
//! whisper's 10 ms timings were closer together than the minimum
//! window, and the centred line shifted sideways on every step as the
//! accumulated text grew.
//!
//! libass does all of that in one filter. We emit an ASS subtitle file
//! per Captions clip, with `\k` karaoke tags carrying the word
//! timings, and burn it in with a single `subtitles=` filter. Word
//! count stops affecting filter count, so exports return to their
//! previous speed.
//!
//! Requires an ffmpeg built `--enable-libass`; that covers the BtbN and
//! gyan builds the app ships or downloads.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use caprust_core::clip::{CaptionPosition, CaptionSegment, CaptionStyle};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temporary ASS file backing one `subtitles=` filter. Deleted on
/// drop so a crashed export does not litter the temp dir.
///
/// The path is shared through an `Arc` and the file is only removed
/// when the last guard goes away. `TextClip` is `Clone` (plans get
/// cloned when a preview respawns), and a naive `Drop` would delete the
/// file out from under the copy an in-flight ffmpeg is still reading.
#[derive(Debug, Clone)]
pub struct AssScript {
    path: Arc<PathBuf>,
}

impl AssScript {
    pub fn write(body: &str) -> Result<Self> {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "caprust-ass-{}-{n}-{nanos}.ass",
            std::process::id()
        ));
        std::fs::write(&path, body)
            .with_context(|| format!("write ASS script {}", path.display()))?;
        Ok(Self {
            path: Arc::new(path),
        })
    }

    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    /// The `subtitles=` filter value, escaped for the filtergraph
    /// parser.
    ///
    /// `subtitles` runs its value through two unescape passes, so a
    /// Windows path needs the drive colon and backslashes escaped, and
    /// single quotes avoided entirely (a literal `'` would terminate
    /// the value early). We fall back to the close-reopen idiom
    /// `'\''` only if the path somehow contains one, which temp paths
    /// generated from our own names never do.
    pub fn filter_value(&self) -> String {
        let raw = self.path.to_string_lossy().replace('\\', "/");
        let mut escaped = raw.replace(':', "\\:");
        if escaped.contains('\'') {
            escaped = escaped.replace('\'', "'\\''");
        }
        format!("filename='{escaped}'")
    }
}

impl Drop for AssScript {
    fn drop(&mut self) {
        // Only the last owner removes the file; a cloned guard may
        // still be feeding it to ffmpeg.
        if Arc::strong_count(&self.path) == 1 {
            let _ = std::fs::remove_file(self.path.as_path());
        }
    }
}

/// ASS timestamp `H:MM:SS.cc` (centiseconds, rounded to nearest).
/// Negative input clamps to zero so a mis-timed segment does not emit
/// a file libass refuses to parse.
pub fn ass_time(sec: f64) -> String {
    let cs = (sec.max(0.0) * 100.0).round() as u64;
    let (h, m, s, cc) = (cs / 360_000, cs / 6_000 % 60, cs / 100 % 60, cs % 100);
    format!("{h}:{m:02}:{s:02}.{cc:02}")
}

/// ASS colours are `&HAABBGGRR`: alpha inverted (00 = opaque, FF =
/// transparent) and channels reversed relative to RGB.
fn ass_color(rgb: [u8; 3], alpha: f32) -> String {
    let a = (1.0 - alpha.clamp(0.0, 1.0)) * 255.0;
    format!(
        "&H{:02X}{:02X}{:02X}{:02X}",
        a.round() as u8,
        rgb[2],
        rgb[1],
        rgb[0]
    )
}

/// Escape caption text for an ASS dialogue line.
///
/// `{` and `}` open ASS override blocks and would swallow the karaoke
/// tags that follow, so they are escaped. Newlines become `\N`. A
/// literal backslash is doubled.
fn escape_ass_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\n' => out.push_str("\\N"),
            '\r' => {}
            '\\' => out.push_str("\\\\"),
            _ => out.push(ch),
        }
    }
    out
}

/// numpad alignment for the three caption positions. ASS aligns the
/// text block within the margins, which is what the old centred
/// `x=(w-text_w)/2` drawtext did.
fn alignment(pos: CaptionPosition) -> u8 {
    match pos {
        CaptionPosition::Top => 8,    // top centre
        CaptionPosition::Middle => 5, // middle centre
        CaptionPosition::Bottom => 2, // bottom centre
    }
}

/// Karaoke tag for a run of words, or an empty string when the segment
/// carries no usable word timing.
///
/// SecondaryColour is set to fully transparent in the style, so a
/// `\k` run reveals the text progressively instead of recolouring it:
/// that reproduces the old "types itself as the speaker talks" look
/// with a single filter.
fn karaoke(segment: &CaptionSegment, time_offset_sec: f64) -> String {
    let words: Vec<_> = segment
        .words
        .iter()
        .filter(|w| !w.text.trim().is_empty())
        .collect();
    if words.len() < 2 {
        return String::new();
    }

    let mut parts = String::new();
    // Integer milliseconds throughout: doing this in f64 loses ~1e-17
    // per subtraction, which turned a 55 ms word into a 5 cs karaoke
    // run instead of 6, and would accumulate down a long segment.
    let mut prev_end_ms: Option<u64> = None;
    for (i, w) in words.iter().enumerate() {
        let mut start_ms = w.start_ms;
        let mut end_ms = w.end_ms;
        // Whisper occasionally repeats a timestamp across tokens; a
        // zero-length karaoke run makes libass stutter, so floor it at
        // one centisecond (10 ms).
        if let Some(prev) = prev_end_ms {
            if start_ms < prev {
                start_ms = prev;
            }
            if end_ms < start_ms + 10 {
                end_ms = start_ms + 10;
            }
        } else if end_ms < start_ms + 10 {
            end_ms = start_ms + 10;
        }
        // Round half up to whole centiseconds.
        let dur_cs = ((end_ms - start_ms) + 5) / 10;
        if i > 0 {
            parts.push(' ');
        }
        parts.push_str(&format!("\\k{dur_cs}"));
        parts.push_str(&escape_ass_text(w.text.trim()));
        prev_end_ms = Some(end_ms);
    }
    let _ = time_offset_sec;
    parts
}

/// Build the ASS document for one Captions clip.
///
/// `time_offset_sec` is subtracted from every timestamp, which is what
/// makes the file line up with a seeked export where output PTS start
/// at zero.
pub fn build(
    segments: &[CaptionSegment],
    style: &CaptionStyle,
    font_family: &str,
    width: u32,
    height: u32,
    time_offset_sec: f64,
) -> String {
    let border_style = if style.bg_enabled { 3 } else { 1 };
    // BorderStyle 3 paints the box in OutlineColour, so the "background"
    // colour moves there; with no box it stays the outline colour.
    let outline = if style.bg_enabled {
        ass_color([0, 0, 0], 1.0 - style.bg_opacity)
    } else {
        ass_color(style.outline_color, 0.0)
    };
    let mut s = String::with_capacity(1024);
    s.push_str("[Script Info]\n");
    s.push_str("ScriptType: v4.00+\n");
    s.push_str(&format!("PlayResX: {width}\n"));
    s.push_str(&format!("PlayResY: {height}\n"));
    // 2 = no automatic line wrapping; the caption arrives already
    // broken into segments and must not reflow differently from the
    // preview.
    s.push_str("WrapStyle: 2\n");
    s.push_str("ScaledBorderAndShadow: yes\n");
    s.push('\n');
    s.push_str("[V4+ Styles]\n");
    s.push_str(
        "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, \
         BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, \
         BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n",
    );
    s.push_str(&format!(
        "Style: CapRust,{font},{size},{primary},&H000000FF,{outline},&H80000000,0,0,0,0,100,100,0,0,{border_style},{outline_w},0,{align},10,10,{margin_v},1\n",
        font = if font_family.trim().is_empty() { "sans-serif" } else { font_family },
        size = style.font_size.max(4.0).round(),
        // SecondaryColour transparent => `\k` reveals instead of
        // recolouring, matching the previous progressive-reveal look.
        primary = ass_color(style.color, 0.0),
        outline = outline,
        border_style = border_style,
        // Outline is the text outline when there is no box, and the box
        // padding when there is. The old drawtext used boxborderw=8, so
        // keep the same padding to avoid a visual jump for existing
        // projects that turn the background on.
        outline_w = if style.bg_enabled {
            8
        } else {
            style.outline_width.max(0.0).round() as i64
        },
        align = alignment(style.position),
        margin_v = (height as f64 * 0.08).round(),
    ));
    s.push('\n');
    s.push_str("[Events]\n");
    s.push_str("Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n");

    for seg in segments {
        let start = (seg.start_ms as f64) / 1000.0 - time_offset_sec;
        let end = (seg.end_ms as f64) / 1000.0 - time_offset_sec;
        if end <= 0.0 {
            continue;
        }
        let text = if seg.words.is_empty() {
            escape_ass_text(seg.text.trim())
        } else {
            karaoke(seg, time_offset_sec)
        };
        if text.is_empty() {
            continue;
        }
        s.push_str(&format!(
            "Dialogue: 0,{},{},CapRust,,0,0,0,,{text}\n",
            ass_time(start),
            ass_time(end)
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use caprust_core::clip::WordTiming;

    fn seg_with_words() -> CaptionSegment {
        CaptionSegment {
            start_ms: 1000,
            end_ms: 3000,
            text: "hello brave world".into(),
            words: vec![
                WordTiming {
                    start_ms: 0,
                    end_ms: 300,
                    text: "hello".into(),
                },
                WordTiming {
                    start_ms: 300,
                    end_ms: 700,
                    text: "brave".into(),
                },
                WordTiming {
                    start_ms: 700,
                    end_ms: 1000,
                    text: "world".into(),
                },
            ],
        }
    }

    /// The whole point of the change: any number of words must still
    /// produce one Dialogue line, not one per word.
    #[test]
    fn word_count_does_not_change_dialogue_count() {
        let style = CaptionStyle::default();
        let body = build(&[seg_with_words()], &style, "Arial", 1920, 1080, 0.0);
        let lines = body.lines().filter(|l| l.starts_with("Dialogue:")).count();
        assert_eq!(lines, 1, "one segment must be one dialogue line: {body}");
        assert!(
            body.contains("\\k30"),
            "word 0 (300ms) should carry \\k30: {body}"
        );
    }

    /// A long segment is the export-speed case; it must not explode.
    #[test]
    fn long_segment_stays_single_line() {
        let style = CaptionStyle::default();
        let words: Vec<WordTiming> = (0..120)
            .map(|i| WordTiming {
                start_ms: i * 400,
                end_ms: i * 400 + 380,
                text: format!("w{i}"),
            })
            .collect();
        let seg = CaptionSegment {
            start_ms: 0,
            end_ms: 48_000,
            text: "long".into(),
            words,
        };
        let body = build(&[seg], &style, "Arial", 1920, 1080, 0.0);
        assert_eq!(
            body.lines().filter(|l| l.starts_with("Dialogue:")).count(),
            1
        );
    }

    /// Repeated timestamps must not produce a zero-length karaoke run;
    /// libass stutters on those.
    #[test]
    fn repeated_word_times_are_floored() {
        let style = CaptionStyle::default();
        let seg = CaptionSegment {
            start_ms: 0,
            end_ms: 1000,
            text: "a b c".into(),
            words: vec![
                WordTiming {
                    start_ms: 0,
                    end_ms: 100,
                    text: "a".into(),
                },
                WordTiming {
                    start_ms: 100,
                    end_ms: 100,
                    text: "b".into(),
                },
                WordTiming {
                    start_ms: 100,
                    end_ms: 200,
                    text: "c".into(),
                },
            ],
        };
        let body = build(&[seg], &style, "Arial", 1920, 1080, 0.0);
        assert!(
            !body.contains("\\k0 "),
            "no zero-length karaoke run: {body}"
        );
    }

    /// A segment without word data is plain text and must not get tags.
    #[test]
    fn segment_without_words_is_plain() {
        let style = CaptionStyle::default();
        let seg = CaptionSegment {
            start_ms: 0,
            end_ms: 2000,
            text: "no timing".into(),
            words: vec![],
        };
        let body = build(&[seg], &style, "Arial", 1920, 1080, 0.0);
        assert!(body.contains("no timing"));
        assert!(
            !body.contains("\\k"),
            "plain segment must not be karaoke: {body}"
        );
    }

    /// The background box stays user-switchable in Clip Properties.
    #[test]
    fn background_toggles_border_style() {
        let mut style = CaptionStyle::default();
        assert!(
            build(&[seg_with_words()], &style, "Arial", 1920, 1080, 0.0).contains("BorderStyle")
        );
        style.bg_enabled = true;
        let body = build(&[seg_with_words()], &style, "Arial", 1920, 1080, 0.0);
        let style_line = body.lines().find(|l| l.starts_with("Style:")).unwrap();
        assert!(
            style_line.contains(",3,"),
            "box on => BorderStyle 3: {style_line}"
        );
    }

    /// Braces in caption text would otherwise open an override block
    /// and eat the karaoke tags after them.
    #[test]
    fn braces_and_newlines_are_escaped() {
        assert_eq!(escape_ass_text("a{b}"), "a\\{b\\}");
        assert_eq!(escape_ass_text("a\nb"), "a\\Nb");
        assert_eq!(escape_ass_text("c:\\path"), "c:\\\\path");
    }

    /// Timestamps shift with the export seek so output PTS line up.
    #[test]
    fn seek_offset_shifts_times() {
        let style = CaptionStyle::default();
        let body = build(&[seg_with_words()], &style, "Arial", 1920, 1080, 0.5);
        assert!(
            body.contains("Dialogue: 0,0:00:00.50,"),
            "start shifted: {body}"
        );
        assert!(body.contains("0:00:02.50,CapRust"), "end shifted: {body}");
    }

    #[test]
    fn ass_time_formats_and_clamps() {
        assert_eq!(ass_time(0.0), "0:00:00.00");
        assert_eq!(ass_time(3661.5), "1:01:01.50");
        assert_eq!(ass_time(-5.0), "0:00:00.00");
    }

    /// The temp file must outlive the filtergraph build but not the
    /// export.
    #[test]
    fn script_is_removed_on_drop() {
        let path = {
            let s = AssScript::write("[Script Info]\n").unwrap();
            let p = s.path().to_path_buf();
            assert!(p.exists());
            assert!(s.filter_value().contains("filename='"));
            p
        };
        assert!(!path.exists(), "ASS file must be deleted on drop");
    }
}
