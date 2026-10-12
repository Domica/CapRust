//! Add, remove, split and merge caption segments.
//!
//! Whisper is a black box that does not know what the editor needs:
//! it can return nothing at all for a clip (the panel then shows
//! "delete this clip" and offers no way to type anything), it can pack
//! three sentences into one segment, and it can put two words in a
//! one-second segment. These commands are how the user fixes that by
//! hand.
//!
//! Every command keeps `words` consistent with `text`, because the
//! renderer prefers word timings when a segment carries them: a text
//! edit that left `words` alone would silently stop reaching the
//! screen.

use crate::clip::{CaptionSegment, ClipType, WordTiming};
use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

/// Split `words` across two text runs on the nearest word boundary at
/// or before `at_ms`, returning `(left, right)`.
///
/// Returns `None` when the split cannot be made: no words at all, a
/// split before the first word, or a split after the last one. Splitting
/// exactly on a boundary takes the whole word to the right side.
fn split_words(words: &[WordTiming], at_ms: u64) -> Option<(Vec<WordTiming>, Vec<WordTiming>)> {
    if words.is_empty() {
        return None;
    }
    // A cut before the first word leaves an empty left side, and one
    // at or after the last word's start leaves an empty right side.
    // Both would produce a caption segment that shows nothing.
    if at_ms <= words[0].start_ms || at_ms >= words[words.len() - 1].start_ms {
        return None;
    }
    let idx = words.iter().rposition(|w| w.start_ms <= at_ms)? + 1;
    if idx == 0 || idx >= words.len() {
        return None;
    }
    let (left, right) = words.split_at(idx);
    // Word end times come from whisper; the renderer derives a word's
    // on-screen duration from the next word's start, so the two runs
    // simply meet at the boundary and need no adjustment.
    Some((left.to_vec(), right.to_vec()))
}

/// Insert a segment at `idx` (clamped to the end).
pub struct AddCaptionSegmentCommand {
    clip_id: Uuid,
    idx: usize,
    segment: CaptionSegment,
    /// Segment length used when the caller passes zero-width timing.
    default_len_ms: u64,
}

impl AddCaptionSegmentCommand {
    pub fn new(clip_id: Uuid, idx: usize, start_ms: u64, end_ms: u64, text: String) -> Self {
        Self {
            clip_id,
            idx,
            segment: CaptionSegment {
                start_ms,
                end_ms,
                text,
                words: Vec::new(),
            },
            default_len_ms: 2000,
        }
    }

    /// Segment length used for a new entry when the user has not set
    /// explicit timing.
    pub fn with_default_len(mut self, ms: u64) -> Self {
        self.default_len_ms = ms.max(100);
        self
    }
}

impl Command for AddCaptionSegmentCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) else {
            return Ok(());
        };
        let ClipType::Captions { segments, .. } = &mut clip.clip_type else {
            return Ok(());
        };
        let mut seg = self.segment.clone();
        if seg.text.trim().is_empty() {
            // An empty entry renders nothing and would just be a dead
            // row in the panel.
            return Ok(());
        }
        if seg.end_ms <= seg.start_ms {
            seg.end_ms = seg.start_ms.saturating_add(self.default_len_ms);
        }
        // Even spacing for hand-typed text, so the caption actually
        // shows up when the clip has no word data to fall back on.
        if seg.words.is_empty() {
            seg.words = even_words(&seg);
        }
        let at = self.idx.min(segments.len());
        segments.insert(at, seg);
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) else {
            return Ok(());
        };
        let ClipType::Captions { segments, .. } = &mut clip.clip_type else {
            return Ok(());
        };
        if self.idx < segments.len() {
            segments.remove(self.idx);
        }
        Ok(())
    }

    fn description(&self) -> String {
        "Add caption segment".into()
    }
}

/// Split evenly by character length so the caption is readable and
/// karaoke still animates. Word timings only matter when they came
/// from whisper; hand-typed text has none.
fn even_words(seg: &CaptionSegment) -> Vec<WordTiming> {
    let parts: Vec<&str> = seg.text.split_whitespace().collect();
    if parts.len() < 2 {
        return Vec::new();
    }
    let total: usize = parts.iter().map(|p| p.chars().count().max(1)).sum();
    let total = total as u64;
    let span = seg.end_ms.saturating_sub(seg.start_ms);
    let mut out = Vec::with_capacity(parts.len());
    let mut consumed = 0u64;
    for (i, part) in parts.iter().enumerate() {
        let chars = part.chars().count().max(1) as u64;
        let start = seg.start_ms + span.saturating_mul(consumed) / total;
        consumed += chars;
        let end = if i + 1 == parts.len() {
            seg.end_ms
        } else {
            seg.start_ms + span.saturating_mul(consumed) / total
        };
        out.push(WordTiming {
            start_ms: start,
            end_ms: end.max(start),
            text: (*part).to_string(),
        });
    }
    out
}

/// Remove the segment at `idx`.
pub struct RemoveCaptionSegmentCommand {
    clip_id: Uuid,
    idx: usize,
    removed: Option<CaptionSegment>,
}

impl RemoveCaptionSegmentCommand {
    pub fn new(clip_id: Uuid, idx: usize) -> Self {
        Self {
            clip_id,
            idx,
            removed: None,
        }
    }
}

impl Command for RemoveCaptionSegmentCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) else {
            return Ok(());
        };
        let ClipType::Captions { segments, .. } = &mut clip.clip_type else {
            return Ok(());
        };
        if self.idx < segments.len() {
            self.removed = Some(segments.remove(self.idx));
        }
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        let (Some(removed), Some(clip)) = (
            self.removed.clone(),
            state.clips.iter_mut().find(|c| c.id == self.clip_id),
        ) else {
            return Ok(());
        };
        let ClipType::Captions { segments, .. } = &mut clip.clip_type else {
            return Ok(());
        };
        let at = self.idx.min(segments.len());
        segments.insert(at, removed);
        Ok(())
    }

    fn description(&self) -> String {
        "Remove caption segment".into()
    }
}

/// Split one segment into two at `at_ms` (relative to the clip).
///
/// `at_ms` lands on the nearest word boundary at or before it, so the
/// caption never renders a half-revealed word. A segment with no word
/// timings is split by character proportion instead.
pub struct SplitCaptionSegmentCommand {
    clip_id: Uuid,
    idx: usize,
    at_ms: u64,
}

impl SplitCaptionSegmentCommand {
    pub fn new(clip_id: Uuid, idx: usize, at_ms: u64) -> Self {
        Self {
            clip_id,
            idx,
            at_ms,
        }
    }

    /// Whether this split would do anything.
    pub fn can_split(seg: &CaptionSegment, at_ms: u64) -> bool {
        if at_ms <= seg.start_ms || at_ms >= seg.end_ms {
            return false;
        }
        match split_words(&seg.words, at_ms.saturating_sub(seg.start_ms)) {
            Some(_) => true,
            // Without word data we can still split by proportion, as
            // long as there are words on both sides of the cut.
            None => {
                let len = seg.text.split_whitespace().count();
                let before = seg.text[..seg.text.len().saturating_sub(seg.text.len() / 2)]
                    .split_whitespace()
                    .count();
                len > 1 && before > 0 && before < len
            }
        }
    }
}

impl Command for SplitCaptionSegmentCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) else {
            return Ok(());
        };
        let ClipType::Captions { segments, .. } = &mut clip.clip_type else {
            return Ok(());
        };
        if self.idx >= segments.len() {
            return Ok(());
        }
        let seg = segments[self.idx].clone();
        if !Self::can_split(&seg, self.at_ms) {
            return Ok(());
        }
        let rel = self.at_ms.saturating_sub(seg.start_ms);
        let join = |v: &[WordTiming]| {
            v.iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        };

        let (left_text, right_text) = match split_words(&seg.words, rel) {
            Some((lw, rw)) => (join(&lw), join(&rw)),
            None => {
                // No usable word timings: cut proportionally so both
                // halves keep some text.
                let words: Vec<&str> = seg.text.split_whitespace().collect();
                let cut = (words.len() / 2).clamp(1, words.len().saturating_sub(1));
                (words[..cut].join(" "), words[cut..].join(" "))
            }
        };

        let (lw, rw) = match split_words(&seg.words, rel) {
            Some((l, r)) => (l, r),
            None => {
                let mut l = seg.clone();
                l.end_ms = self.at_ms;
                l.text = left_text.clone();
                l.words = even_words(&l);
                let mut r = seg.clone();
                r.start_ms = self.at_ms;
                r.text = right_text.clone();
                r.words = even_words(&r);
                (l.words, r.words)
            }
        };

        let left = CaptionSegment {
            start_ms: seg.start_ms,
            end_ms: self.at_ms,
            text: left_text,
            words: lw,
        };
        let right = CaptionSegment {
            start_ms: self.at_ms,
            end_ms: seg.end_ms,
            text: right_text,
            words: rw,
        };
        segments[self.idx] = left;
        segments.insert(self.idx + 1, right);
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) else {
            return Ok(());
        };
        let ClipType::Captions { segments, .. } = &mut clip.clip_type else {
            return Ok(());
        };
        if self.idx + 1 >= segments.len() {
            return Ok(());
        }
        let right = segments.remove(self.idx + 1);
        let left = &mut segments[self.idx];
        left.end_ms = right.end_ms.max(left.end_ms);
        left.text = format!("{} {}", left.text, right.text).trim().to_string();
        left.words.extend(right.words);
        Ok(())
    }

    fn description(&self) -> String {
        "Split caption segment".into()
    }
}

/// Merge segment `idx` with the one after it.
pub struct MergeCaptionSegmentsCommand {
    clip_id: Uuid,
    idx: usize,
}

impl MergeCaptionSegmentsCommand {
    pub fn new(clip_id: Uuid, idx: usize) -> Self {
        Self { clip_id, idx }
    }
}

impl Command for MergeCaptionSegmentsCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) else {
            return Ok(());
        };
        let ClipType::Captions { segments, .. } = &mut clip.clip_type else {
            return Ok(());
        };
        if self.idx + 1 >= segments.len() {
            return Ok(());
        }
        let right = segments.remove(self.idx + 1);
        let left = &mut segments[self.idx];
        left.end_ms = right.end_ms.max(left.end_ms);
        if !left.text.trim().is_empty() && !right.text.trim().is_empty() {
            left.text = format!("{} {}", left.text.trim(), right.text.trim());
        } else if left.text.trim().is_empty() {
            left.text = right.text.clone();
        }
        if left.words.is_empty() {
            left.words = right.words;
        } else if !right.words.is_empty() {
            left.words.extend(right.words);
        } else {
            // Nothing but text on the right: re-time so the reveal
            // still animates across the whole merged run.
            left.words = even_words(&CaptionSegment {
                start_ms: left.start_ms,
                end_ms: left.end_ms,
                text: left.text.clone(),
                words: Vec::new(),
            });
        }
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) else {
            return Ok(());
        };
        let ClipType::Captions { segments, .. } = &mut clip.clip_type else {
            return Ok(());
        };
        if self.idx >= segments.len() {
            return Ok(());
        }
        let left = segments.remove(self.idx);
        // Put the split back at the original boundary: where the left
        // segment's own words end.
        let at = left
            .words
            .iter()
            .filter(|w| w.end_ms < left.end_ms)
            .map(|w| w.end_ms)
            .max()
            .unwrap_or(left.end_ms);
        let left_text: Vec<&str> = left.words.iter().map(|w| w.text.as_str()).collect();
        let right_start = left
            .words
            .iter()
            .position(|w| w.start_ms >= at)
            .unwrap_or(left.words.len());
        let ltext = if left.words.is_empty() {
            left.text.split_whitespace().collect::<Vec<_>>().join(" ")
        } else {
            left_text[..right_start.min(left_text.len())].join(" ")
        };
        let rtext = if left.words.is_empty() {
            String::new()
        } else {
            left_text[right_start.min(left_text.len())..].join(" ")
        };
        let lw = left.words[..right_start.min(left.words.len())].to_vec();
        let rw = left.words[right_start.min(left.words.len())..].to_vec();

        let mut first = CaptionSegment {
            start_ms: left.start_ms,
            end_ms: if rtext.trim().is_empty() {
                left.end_ms
            } else {
                at
            },
            text: if ltext.trim().is_empty() {
                left.text
            } else {
                ltext
            },
            words: lw,
        };
        if first.words.is_empty() && first.text.split_whitespace().count() > 1 {
            first.words = even_words(&first);
        }
        segments.insert(self.idx, first);
        if !rtext.trim().is_empty() {
            let mut second = CaptionSegment {
                start_ms: at,
                end_ms: left.end_ms,
                text: rtext,
                words: rw,
            };
            if second.words.is_empty() && second.text.split_whitespace().count() > 1 {
                second.words = even_words(&second);
            }
            segments.insert(self.idx + 1, second);
        }
        Ok(())
    }

    fn description(&self) -> String {
        "Merge caption segments".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(text: &str, words: &[(&str, u64, u64)]) -> CaptionSegment {
        CaptionSegment {
            start_ms: 0,
            end_ms: 4000,
            text: text.into(),
            words: words
                .iter()
                .map(|(t, s, e)| WordTiming {
                    start_ms: *s,
                    end_ms: *e,
                    text: (*t).into(),
                })
                .collect(),
        }
    }

    /// A caption split must not leave two runs overlapping, or libass
    /// would show a partial word in both.
    #[test]
    fn split_divides_words_without_overlap() {
        let s = seg(
            "the quick brown fox",
            &[
                ("the", 0, 300),
                ("quick", 300, 800),
                ("brown", 800, 1500),
                ("fox", 1500, 2000),
            ],
        );
        let (l, r) = split_words(&s.words, 700).expect("split");
        assert_eq!(
            l.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(),
            ["the", "quick"]
        );
        assert_eq!(
            r.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(),
            ["brown", "fox"]
        );
        assert!(l.last().unwrap().end_ms <= r.first().unwrap().start_ms + 1);
    }

    /// Splitting inside the first or last word has no valid boundary.
    #[test]
    fn split_rejects_edges() {
        let s = seg("a b", &[("a", 0, 100), ("b", 100, 200)]);
        assert!(split_words(&s.words, 0).is_none());
        assert!(split_words(&s.words, 5000).is_none());
        assert!(split_words(&seg("x", &[]).words, 100).is_none());
    }

    /// Hand-typed segments have no whisper timings; the split still
    /// has to leave text on both sides.
    #[test]
    fn split_without_word_data_divides_text() {
        let s = seg("one two three four", &[]);
        assert!(SplitCaptionSegmentCommand::can_split(&s, 2000));
    }

    /// A one-word segment cannot be split.
    #[test]
    fn split_rejects_single_word() {
        let s = seg("solo", &[]);
        assert!(!SplitCaptionSegmentCommand::can_split(&s, 2000));
    }

    /// Even spacing has to cover the whole span without inverting.
    #[test]
    fn even_words_cover_the_span() {
        let mut s = seg("a bb ccc", &[]);
        s.end_ms = 3000;
        let w = even_words(&s);
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].start_ms, 0);
        assert_eq!(w[2].end_ms, 3000);
        for pair in w.windows(2) {
            assert!(pair[0].end_ms <= pair[1].start_ms + 1, "{:?}", w);
        }
    }

    /// A segment with a single word gets no word timings: there is
    /// nothing to reveal progressively.
    #[test]
    fn even_words_needs_two_words() {
        let s = seg("solo", &[]);
        assert!(even_words(&s).is_empty());
    }
}
