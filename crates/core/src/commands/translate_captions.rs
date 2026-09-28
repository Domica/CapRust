//! Translate a Captions clip's segments into another language, on a
//! new Captions track. The source clip is left untouched.
//!
//! DIRECTIVES §7.1 used to declare Captions a singleton kind. As of
//! the translation feature (§32) it is multi-instance: each translation
//! lives on its own track so the user can show/hide languages
//! independently through the track-header chip.
//!
//! Inputs to the command are pre-translated: the HTTP call runs on a
//! background thread (see `core::translate`), and the command is
//! committed once the full batch returns.

use crate::clip::{CaptionSegment, Clip, ClipType};
use crate::commands::Command;
use crate::project::ProjectState;
use crate::track::{Track, TrackKind};
use anyhow::{bail, Result};
use uuid::Uuid;

pub struct TranslateCaptionsCommand {
    source_clip_id: Uuid,
    target_language: String,
    /// One translation per source segment, same order.
    translations: Vec<String>,
    /// Filled on execute.
    new_track_index: Option<usize>,
    new_clip_id: Option<Uuid>,
}

impl TranslateCaptionsCommand {
    pub fn new(
        source_clip_id: Uuid,
        target_language: impl Into<String>,
        translations: Vec<String>,
    ) -> Self {
        Self {
            source_clip_id,
            target_language: target_language.into(),
            translations,
            new_track_index: None,
            new_clip_id: None,
        }
    }
}

/// Find a Captions track name that does not collide, e.g.
/// "Captions (hr)", "Captions (hr) 2".
fn next_captions_name(state: &ProjectState, lang: &str) -> String {
    let base = format!("Captions ({lang})");
    if !state.tracks.iter().any(|t| t.name == base) {
        return base;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base} {n}");
        if !state.tracks.iter().any(|t| t.name == candidate) {
            return candidate;
        }
        n += 1;
    }
}

impl Command for TranslateCaptionsCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        // 1. Validate source clip.
        let Some(src) = state.clips.iter().find(|c| c.id == self.source_clip_id) else {
            return Ok(());
        };
        let ClipType::Captions {
            model_id,
            segments,
            style,
            ..
        } = &src.clip_type
        else {
            return Ok(());
        };
        if segments.len() != self.translations.len() {
            bail!(
                "translate command: {} translations for {} segments",
                self.translations.len(),
                segments.len()
            );
        }
        let model_id = model_id.clone();
        let style = *style;
        let start_ms = src.start_time_ms;
        let dur_ms = src.duration_ms;

        // 2. Build translated segments: same windows, no per-word data.
        let translated_segments: Vec<CaptionSegment> = segments
            .iter()
            .zip(self.translations.iter())
            .map(|(src_seg, text)| CaptionSegment {
                start_ms: src_seg.start_ms,
                end_ms: src_seg.end_ms,
                text: text.clone(),
                words: Vec::new(),
            })
            .collect();

        // 3. Append a new Captions track.
        let name = next_captions_name(state, &self.target_language);
        let new_track_index = state.tracks.len();
        state.tracks.push(Track::new(&name, TrackKind::Captions));
        self.new_track_index = Some(new_track_index);

        // 4. Create the clip.
        let mut clip = Clip::new_captions(
            new_track_index,
            start_ms,
            dur_ms,
            &model_id,
            &self.target_language,
        );
        if let ClipType::Captions {
            segments, style: s, ..
        } = &mut clip.clip_type
        {
            *segments = translated_segments;
            *s = style;
        }
        self.new_clip_id = Some(clip.id);
        state.clips.push(clip);
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(id) = self.new_clip_id.take() {
            state.clips.retain(|c| c.id != id);
        }
        if let Some(idx) = self.new_track_index.take() {
            // Track was appended last, so removing it does not shift
            // any other clip's track_index.
            if idx < state.tracks.len() {
                state.tracks.remove(idx);
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Translate captions to {}", self.target_language)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{CaptionSegment, Clip, ClipType};
    use crate::commands::UndoStack;
    use crate::project::ProjectState;

    fn project_with_captions() -> (ProjectState, Uuid) {
        let mut p = ProjectState::default();
        // find or create a Captions track (default_tracks may include one)
        let captions_idx = p
            .tracks
            .iter()
            .position(|t| t.kind == TrackKind::Captions)
            .unwrap_or_else(|| {
                let i = p.tracks.len();
                p.tracks.push(Track::new("Captions", TrackKind::Captions));
                i
            });
        let mut c = Clip::new_captions(captions_idx, 0, 5000, "whisper-tiny", "en");
        if let ClipType::Captions { segments, .. } = &mut c.clip_type {
            *segments = vec![
                CaptionSegment {
                    start_ms: 0,
                    end_ms: 2500,
                    text: "Hello".into(),
                    words: vec![],
                },
                CaptionSegment {
                    start_ms: 2500,
                    end_ms: 5000,
                    text: "World".into(),
                    words: vec![],
                },
            ];
        }
        let id = c.id;
        p.clips.push(c);
        (p, id)
    }

    #[test]
    fn adds_new_captions_clip_on_new_track() {
        let (mut p, src_id) = project_with_captions();
        let tracks_before = p.tracks.len();
        let clips_before = p.clips.len();
        let mut stack = UndoStack::default();

        let cmd = TranslateCaptionsCommand::new(src_id, "hr", vec!["Bok".into(), "Svijete".into()]);
        stack.execute(Box::new(cmd), &mut p).unwrap();

        assert_eq!(p.tracks.len(), tracks_before + 1);
        assert_eq!(p.clips.len(), clips_before + 1);
        let new_clip = p.clips.last().unwrap();
        match &new_clip.clip_type {
            ClipType::Captions {
                language, segments, ..
            } => {
                assert_eq!(language, "hr");
                assert_eq!(segments.len(), 2);
                assert_eq!(segments[0].text, "Bok");
                assert_eq!(segments[1].text, "Svijete");
                assert!(segments[0].words.is_empty());
            }
            _ => panic!("new clip is not Captions"),
        }
    }

    #[test]
    fn undo_removes_clip_and_track() {
        let (mut p, src_id) = project_with_captions();
        let tracks_before = p.tracks.len();
        let clips_before = p.clips.len();
        let mut stack = UndoStack::default();

        stack
            .execute(
                Box::new(TranslateCaptionsCommand::new(
                    src_id,
                    "hr",
                    vec!["Bok".into(), "Svijete".into()],
                )),
                &mut p,
            )
            .unwrap();
        stack.undo(&mut p).unwrap();

        assert_eq!(p.tracks.len(), tracks_before);
        assert_eq!(p.clips.len(), clips_before);
    }

    #[test]
    fn noop_when_source_missing() {
        let mut p = ProjectState::default();
        let mut stack = UndoStack::default();
        let clips_before = p.clips.len();
        stack
            .execute(
                Box::new(TranslateCaptionsCommand::new(
                    Uuid::new_v4(),
                    "hr",
                    vec!["x".into()],
                )),
                &mut p,
            )
            .unwrap();
        assert_eq!(p.clips.len(), clips_before);
    }

    #[test]
    fn errors_on_count_mismatch() {
        let (mut p, src_id) = project_with_captions();
        let mut stack = UndoStack::default();
        let err = stack
            .execute(
                Box::new(TranslateCaptionsCommand::new(
                    src_id,
                    "hr",
                    vec!["only one".into()], // 2 segments expected
                )),
                &mut p,
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("translations for"), "{err}");
    }
}
