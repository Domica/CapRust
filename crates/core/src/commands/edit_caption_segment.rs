//! Edit the text of a single caption segment in place.
//!
//! A segment carries both a flat `text` and a parallel `words` array
//! used by the progressive-reveal render path. The renderer prefers
//! `words` whenever they carry timing (see `export_graph`), so an edit
//! that only touched `text` would leave the two out of sync and the
//! user's edit would never reach the screen. Every edit therefore
//! re-times `words` to match the new text.

use crate::clip::ClipType;
use crate::clip::{CaptionSegment, WordTiming};
use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct EditCaptionSegmentCommand {
    clip_id: Uuid,
    idx: usize,
    new_text: String,
    /// Captured on `execute`, restored on `undo`.
    before_text: Option<String>,
    before_words: Option<Vec<WordTiming>>,
}

impl EditCaptionSegmentCommand {
    pub fn new(clip_id: Uuid, idx: usize, new_text: impl Into<String>) -> Self {
        Self {
            clip_id,
            idx,
            new_text: new_text.into(),
            before_text: None,
            before_words: None,
        }
    }
}

/// Re-time `old_words` so they line up with `new_text`.
///
/// Two cases, chosen to keep the karaoke working after a typo fix:
///   * Same word count: map one-to-one, keeping every timestamp. A
///     single-word correction costs nothing.
///   * Different word count: distribute the segment's span across the
///     new words in proportion to their length, so the reveal stays
///     paced like the speech.
///
/// An empty `old_words` stays empty; the caller then falls back to the
/// single-drawtext render, which is correct for a segment that never
/// had word data.
fn retime_words(seg: &CaptionSegment, new_text: &str) -> Vec<WordTiming> {
    if seg.words.is_empty() {
        return Vec::new();
    }
    let new_parts: Vec<&str> = new_text.split_whitespace().collect();
    if new_parts.is_empty() {
        // Empty text renders nothing; stale words would still draw the
        // old caption.
        return Vec::new();
    }

    if new_parts.len() == seg.words.len() {
        return new_parts
            .iter()
            .zip(seg.words.iter())
            .map(|(t, w)| WordTiming {
                start_ms: w.start_ms,
                end_ms: w.end_ms,
                text: (*t).to_string(),
            })
            .collect();
    }

    // Spread the new words across the same window the old ones used,
    // weighted by character length so long words get more time.
    let first = seg
        .words
        .first()
        .map(|w| w.start_ms)
        .unwrap_or(seg.start_ms);
    let last = seg
        .words
        .last()
        .map(|w| w.end_ms.max(w.start_ms))
        .unwrap_or(seg.end_ms.max(seg.start_ms));
    let span = last.saturating_sub(first);
    let total_chars: usize = new_parts.iter().map(|p| p.chars().count().max(1)).sum();
    let total_chars = u64::try_from(total_chars).unwrap_or(1);

    let mut out = Vec::with_capacity(new_parts.len());
    let mut consumed_chars = 0u64;
    for (i, part) in new_parts.iter().enumerate() {
        let chars = part.chars().count().max(1) as u64;
        let start = first + span.saturating_mul(consumed_chars) / total_chars;
        consumed_chars += chars;
        let end = if i + 1 == new_parts.len() {
            last
        } else {
            first + span.saturating_mul(consumed_chars) / total_chars
        };
        out.push(WordTiming {
            start_ms: start,
            end_ms: end.max(start),
            text: (*part).to_string(),
        });
    }
    out
}

impl Command for EditCaptionSegmentCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) else {
            return Ok(());
        };
        let ClipType::Captions { segments, .. } = &mut clip.clip_type else {
            return Ok(());
        };
        let Some(seg) = segments.get_mut(self.idx) else {
            return Ok(());
        };
        self.before_text = Some(seg.text.clone());
        self.before_words = Some(seg.words.clone());
        seg.words = retime_words(seg, &self.new_text);
        seg.text = self.new_text.clone();
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(before) = self.before_text.clone() else {
            return Ok(());
        };
        let Some(clip) = state.clips.iter_mut().find(|c| c.id == self.clip_id) else {
            return Ok(());
        };
        let ClipType::Captions { segments, .. } = &mut clip.clip_type else {
            return Ok(());
        };
        let Some(seg) = segments.get_mut(self.idx) else {
            return Ok(());
        };
        seg.text = before;
        if let Some(words) = self.before_words.clone() {
            seg.words = words;
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Edit caption segment {}", self.idx + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg_with_words() -> CaptionSegment {
        CaptionSegment {
            start_ms: 0,
            end_ms: 4000,
            text: "the quick brown fox".into(),
            words: vec![
                WordTiming {
                    start_ms: 0,
                    end_ms: 500,
                    text: "the".into(),
                },
                WordTiming {
                    start_ms: 500,
                    end_ms: 1200,
                    text: "quick".into(),
                },
                WordTiming {
                    start_ms: 1200,
                    end_ms: 2400,
                    text: "brown".into(),
                },
                WordTiming {
                    start_ms: 2400,
                    end_ms: 4000,
                    text: "fox".into(),
                },
            ],
        }
    }

    /// Typo fixes are the common case; they must not destroy the
    /// word timings the progressive reveal depends on.
    #[test]
    fn same_word_count_keeps_every_timestamp() {
        let seg = seg_with_words();
        let out = retime_words(&seg, "the quik brown fox");
        assert_eq!(out.len(), 4);
        assert_eq!(out[1].text, "quik");
        assert_eq!(out[1].start_ms, 500);
        assert_eq!(out[1].end_ms, 1200);
        assert_eq!(out[3].start_ms, 2400);
    }

    /// Rewording changes the word count; the reveal still has to span
    /// the segment instead of collapsing or running past it.
    #[test]
    fn changed_word_count_respreads_over_the_segment() {
        let seg = seg_with_words();
        let out = retime_words(&seg, "one two");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].start_ms, 0);
        assert_eq!(out[1].end_ms, 4000);
        assert!(out[0].end_ms <= out[1].start_ms);
        for w in &out {
            assert!(w.end_ms >= w.start_ms, "word window must not invert");
        }
    }

    /// A segment with no word data must stay that way, so the render
    /// keeps using the single-drawtext fallback.
    #[test]
    fn no_words_stays_empty() {
        let seg = CaptionSegment {
            start_ms: 0,
            end_ms: 2000,
            text: "hello".into(),
            words: Vec::new(),
        };
        assert!(retime_words(&seg, "hello there").is_empty());
    }

    /// Clearing the text has to clear the words too, otherwise the old
    /// caption keeps rendering.
    #[test]
    fn clearing_text_clears_words() {
        let seg = seg_with_words();
        assert!(retime_words(&seg, "   ").is_empty());
    }
}
