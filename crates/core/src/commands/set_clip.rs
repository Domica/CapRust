//! Generic command to edit any subset of clip fields.

use crate::clip::Clip;
use crate::commands::Command;
use crate::project::ProjectState;
use anyhow::Result;
use uuid::Uuid;

pub struct SetClipCommand {
    pub clip_id: Uuid,
    pub start_time_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub speed: Option<f32>,
    pub reversed: Option<bool>,
    pub flip_h: Option<bool>,
    pub flip_v: Option<bool>,
    pub volume_db: Option<f32>,
    pub track_index: Option<usize>,
    pub name: Option<Option<String>>,
    /// Only meaningful on TextOverlay clips: sets the drawtext style id.
    /// No-op on other clip kinds.
    pub text_style: Option<String>,
    /// Only meaningful on TextOverlay clips: replaces the text content.
    /// No-op on other clip kinds.
    pub text_content: Option<String>,
    /// Only meaningful on Captions clips: replaces the visual style.
    /// No-op on other clip kinds.
    pub caption_style: Option<crate::clip::CaptionStyle>,
    /// Only meaningful on TextOverlay clips: replaces the motion
    /// transform. No-op on other clip kinds.
    pub text_motion: Option<crate::clip::TextMotion>,
    /// Only meaningful on TextOverlay clips: Some(Some(x)) sets the
    /// effect, Some(None) clears it, None leaves untouched.
    pub text_effect: Option<Option<crate::clip::TextEffect>>,
    pub fade_in_ms: Option<u64>,
    pub fade_out_ms: Option<u64>,
    pub volume_keyframes: Option<Vec<crate::clip::VolumeKeyframe>>,
    pub duck_against: Option<Option<Uuid>>,
    /// Some(Some(x)) = ramp from `speed` to `x`.
    /// Some(None) = clear ramp, use static `speed`.
    pub speed_end: Option<Option<f32>>,
    pub speed_ease: Option<crate::clip::EaseCurve>,
    pub speed_range: Option<crate::clip::SpeedRampRange>,
    /// Some(v) = replace the auto-reframe keypoints with `v` (empty
    /// Vec clears them). None = leave untouched.
    pub auto_reframe: Option<Vec<crate::clip::ReframeKeypoint>>,
    /// Some(Some(p)) = set the mask path. Some(None) = clear it.
    /// None = leave untouched.
    pub bg_removal: Option<Option<String>>,
    /// Some(v) = replace the in-transition easing. None = leave
    /// untouched. Easing is only meaningful when `transition_in` is
    /// `Some("fade")`; on other transitions the field is stored but
    /// ignored by the render arms.
    pub transition_in_easing: Option<crate::clip::EaseCurve>,
    /// Same as `transition_in_easing`, for the out transition.
    pub transition_out_easing: Option<crate::clip::EaseCurve>,
    /// Some(v) = replace the shared transition duration in ms.
    pub transition_duration_ms: Option<u64>,
    before: Option<Clip>,
}

impl SetClipCommand {
    pub fn new(clip_id: Uuid) -> Self {
        Self {
            clip_id,
            start_time_ms: None,
            duration_ms: None,
            speed: None,
            reversed: None,
            flip_h: None,
            flip_v: None,
            volume_db: None,
            track_index: None,
            name: None,
            text_style: None,
            text_content: None,
            caption_style: None,
            text_motion: None,
            text_effect: None,
            fade_in_ms: None,
            fade_out_ms: None,
            volume_keyframes: None,
            duck_against: None,
            speed_end: None,
            speed_ease: None,
            speed_range: None,
            auto_reframe: None,
            bg_removal: None,
            transition_in_easing: None,
            transition_out_easing: None,
            transition_duration_ms: None,
            before: None,
        }
    }

    pub fn speed(mut self, v: f32) -> Self {
        self.speed = Some(v);
        self
    }
    pub fn reversed(mut self, v: bool) -> Self {
        self.reversed = Some(v);
        self
    }
    pub fn flip_h(mut self, v: bool) -> Self {
        self.flip_h = Some(v);
        self
    }
    pub fn flip_v(mut self, v: bool) -> Self {
        self.flip_v = Some(v);
        self
    }
    pub fn volume_db(mut self, v: f32) -> Self {
        self.volume_db = Some(v);
        self
    }
    pub fn start_time_ms(mut self, v: u64) -> Self {
        self.start_time_ms = Some(v);
        self
    }
    pub fn duration_ms(mut self, v: u64) -> Self {
        self.duration_ms = Some(v);
        self
    }
    pub fn track_index(mut self, v: usize) -> Self {
        self.track_index = Some(v);
        self
    }
    /// Set the clip name. Pass `None` to clear it back to file-derived.
    pub fn name(mut self, v: Option<String>) -> Self {
        self.name = Some(v);
        self
    }
    /// Set the TextOverlay drawtext style. No-op on other clip kinds.
    pub fn text_style(mut self, v: impl Into<String>) -> Self {
        self.text_style = Some(v.into());
        self
    }
    /// Replace the Captions visual style. No-op on other kinds.
    pub fn caption_style(mut self, v: crate::clip::CaptionStyle) -> Self {
        self.caption_style = Some(v);
        self
    }
    /// Replace the TextOverlay text content. No-op on other kinds.
    pub fn text_content(mut self, v: impl Into<String>) -> Self {
        self.text_content = Some(v.into());
        self
    }
    /// Replace the TextOverlay OR Captions motion transform. No-op on other kinds.
    pub fn text_motion(mut self, v: crate::clip::TextMotion) -> Self {
        self.text_motion = Some(v);
        self
    }
    /// Set or clear the TextOverlay OR Captions procedural effect.
    /// No-op on other kinds. Pass None to remove an effect.
    pub fn text_effect(mut self, v: Option<crate::clip::TextEffect>) -> Self {
        self.text_effect = Some(v);
        self
    }
    /// Replace the in-transition easing. Ignored by the render arms
    /// unless `transition_in` is `Some("fade")`.
    pub fn transition_in_easing(mut self, v: crate::clip::EaseCurve) -> Self {
        self.transition_in_easing = Some(v);
        self
    }
    /// Replace the out-transition easing. Ignored unless
    /// `transition_out` is `Some("fade")`.
    pub fn transition_out_easing(mut self, v: crate::clip::EaseCurve) -> Self {
        self.transition_out_easing = Some(v);
        self
    }
    /// Replace the shared transition duration (ms).
    pub fn transition_duration_ms(mut self, v: u64) -> Self {
        self.transition_duration_ms = Some(v);
        self
    }
    pub fn fade_in_ms(mut self, v: u64) -> Self {
        self.fade_in_ms = Some(v);
        self
    }
    pub fn fade_out_ms(mut self, v: u64) -> Self {
        self.fade_out_ms = Some(v);
        self
    }
    /// Replace the whole automation curve. Pass an empty Vec to fall
    /// back to the static `volume_db`.
    pub fn volume_keyframes(mut self, v: Vec<crate::clip::VolumeKeyframe>) -> Self {
        self.volume_keyframes = Some(v);
        self
    }
    /// Set or clear the sidechain control clip. `None` disables
    /// ducking on this clip.
    pub fn duck_against(mut self, v: Option<Uuid>) -> Self {
        self.duck_against = Some(v);
        self
    }
    /// Enable or disable the speed ramp end. Pass `Some(x)` for a
    /// ramp, `None` for static speed.
    pub fn speed_end(mut self, v: Option<f32>) -> Self {
        self.speed_end = Some(v);
        self
    }
    pub fn speed_ease(mut self, v: crate::clip::EaseCurve) -> Self {
        self.speed_ease = Some(v);
        self
    }

    /// Replace the auto-reframe keypoints. Empty Vec = clear.
    pub fn auto_reframe(mut self, v: Vec<crate::clip::ReframeKeypoint>) -> Self {
        self.auto_reframe = Some(v);
        self
    }

    /// Set or clear the background-removal mask path.
    pub fn bg_removal(mut self, v: Option<String>) -> Self {
        self.bg_removal = Some(v);
        self
    }
    pub fn speed_range(mut self, v: crate::clip::SpeedRampRange) -> Self {
        self.speed_range = Some(v);
        self
    }
}

impl Command for SetClipCommand {
    fn execute(&mut self, state: &mut ProjectState) -> Result<()> {
        let Some(c) = state.clips.iter_mut().find(|c| c.id == self.clip_id) else {
            return Ok(());
        };
        self.before = Some(c.clone());
        if let Some(v) = self.start_time_ms {
            c.start_time_ms = v;
        }
        if let Some(v) = self.duration_ms {
            c.duration_ms = v;
        }
        if let Some(v) = self.speed {
            c.speed = v;
        }
        if let Some(v) = self.reversed {
            c.reversed = v;
        }
        if let Some(v) = self.flip_h {
            c.flip_h = v;
        }
        if let Some(v) = self.flip_v {
            c.flip_v = v;
        }
        if let Some(v) = self.volume_db {
            c.volume_db = v;
        }
        if let Some(v) = self.track_index {
            c.track_index = v;
        }
        if let Some(v) = self.name.clone() {
            c.name = v;
        }
        if let Some(v) = self.text_style.clone() {
            if let crate::clip::ClipType::TextOverlay { style, .. } = &mut c.clip_type {
                *style = v;
            }
        }
        if let Some(v) = self.caption_style {
            if let crate::clip::ClipType::Captions { style, .. } = &mut c.clip_type {
                *style = v;
            }
        }
        if let Some(v) = self.text_content.clone() {
            if let crate::clip::ClipType::TextOverlay { content, .. } = &mut c.clip_type {
                *content = v;
            }
        }
        if let Some(v) = self.text_motion {
            match &mut c.clip_type {
                crate::clip::ClipType::TextOverlay { motion, .. } => *motion = v,
                crate::clip::ClipType::Captions { motion, .. } => *motion = v,
                _ => {}
            }
        }
        if let Some(v) = self.text_effect {
            match &mut c.clip_type {
                crate::clip::ClipType::TextOverlay { effect, .. } => *effect = v,
                crate::clip::ClipType::Captions { effect, .. } => *effect = v,
                _ => {}
            }
        }
        if let Some(v) = self.fade_in_ms {
            c.fade_in_ms = v;
        }
        if let Some(v) = self.fade_out_ms {
            c.fade_out_ms = v;
        }
        if let Some(v) = self.volume_keyframes.clone() {
            c.volume_keyframes = v;
        }
        if let Some(v) = self.duck_against {
            c.duck_against = v;
        }
        if let Some(v) = self.speed_end {
            c.speed_end = v;
        }
        if let Some(v) = self.speed_ease {
            c.speed_ease = v;
        }
        if let Some(v) = self.speed_range {
            c.speed_range = v;
        }
        if let Some(v) = self.auto_reframe.clone() {
            c.auto_reframe = v;
        }
        if let Some(v) = self.bg_removal.clone() {
            c.bg_removal = v;
        }
        if let Some(v) = self.transition_in_easing {
            c.transition_in_easing = v;
        }
        if let Some(v) = self.transition_out_easing {
            c.transition_out_easing = v;
        }
        if let Some(v) = self.transition_duration_ms {
            c.transition_duration_ms = v;
        }
        Ok(())
    }

    fn undo(&mut self, state: &mut ProjectState) -> Result<()> {
        if let Some(orig) = self.before.take() {
            if let Some(c) = state.clips.iter_mut().find(|c| c.id == self.clip_id) {
                *c = orig;
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Edit clip {}", self.clip_id)
    }
}

#[cfg(test)]
mod text_motion_cmd_tests {
    use super::*;
    use crate::clip::{Clip, ClipType, TextEffect, TextEffectKind, TextMotion};
    use crate::commands::UndoStack;
    use crate::project::ProjectState;

    fn project_with_text() -> (ProjectState, Uuid) {
        let mut p = ProjectState::default();
        let c = Clip::new_text("hi", 0, 0, 1000, false);
        let id = c.id;
        p.clips.push(c);
        (p, id)
    }

    fn motion_of(p: &ProjectState, id: Uuid) -> TextMotion {
        let c = p.clips.iter().find(|c| c.id == id).unwrap();
        match &c.clip_type {
            ClipType::TextOverlay { motion, .. } => *motion,
            _ => panic!("not a TextOverlay"),
        }
    }

    fn effect_of(p: &ProjectState, id: Uuid) -> Option<TextEffect> {
        let c = p.clips.iter().find(|c| c.id == id).unwrap();
        match &c.clip_type {
            ClipType::TextOverlay { effect, .. } => *effect,
            _ => panic!("not a TextOverlay"),
        }
    }

    #[test]
    fn transition_easing_set_and_undo() {
        use crate::clip::{Clip, EaseCurve};
        let mut p = ProjectState::default();
        let clip = Clip::new_video("a.mp4", 0, 0, 1000);
        let id = clip.id;
        p.add_clip(clip);

        // Before: Linear (default).
        let before = p.clips.iter().find(|c| c.id == id).unwrap().clone();
        assert_eq!(before.transition_in_easing, EaseCurve::Linear);
        assert_eq!(before.transition_out_easing, EaseCurve::Linear);

        let mut stack = crate::commands::UndoStack::default();
        let cmd = SetClipCommand::new(id)
            .transition_in_easing(EaseCurve::EaseIn)
            .transition_out_easing(EaseCurve::EaseInOut);
        stack.execute(Box::new(cmd), &mut p).unwrap();

        let after = p.clips.iter().find(|c| c.id == id).unwrap();
        assert_eq!(after.transition_in_easing, EaseCurve::EaseIn);
        assert_eq!(after.transition_out_easing, EaseCurve::EaseInOut);

        stack.undo(&mut p).unwrap();
        let back = p.clips.iter().find(|c| c.id == id).unwrap();
        assert_eq!(back.transition_in_easing, EaseCurve::Linear);
        assert_eq!(back.transition_out_easing, EaseCurve::Linear);
    }

    #[test]
    fn transition_duration_set_and_undo() {
        use crate::clip::Clip;
        let mut p = ProjectState::default();
        let clip = Clip::new_video("a.mp4", 0, 0, 1000);
        let id = clip.id;
        p.add_clip(clip);

        let before = p
            .clips
            .iter()
            .find(|c| c.id == id)
            .unwrap()
            .transition_duration_ms;
        assert_eq!(before, 500, "default is 500 ms");

        let mut stack = crate::commands::UndoStack::default();
        stack
            .execute(
                Box::new(SetClipCommand::new(id).transition_duration_ms(1500)),
                &mut p,
            )
            .unwrap();
        assert_eq!(
            p.clips
                .iter()
                .find(|c| c.id == id)
                .unwrap()
                .transition_duration_ms,
            1500
        );

        stack.undo(&mut p).unwrap();
        assert_eq!(
            p.clips
                .iter()
                .find(|c| c.id == id)
                .unwrap()
                .transition_duration_ms,
            500
        );
    }

    #[test]
    fn transition_easing_builders_are_noop_when_unset() {
        use crate::clip::{Clip, EaseCurve};
        let mut p = ProjectState::default();
        let mut clip = Clip::new_video("a.mp4", 0, 0, 1000);
        clip.transition_in_easing = EaseCurve::EaseIn;
        clip.transition_out_easing = EaseCurve::EaseOut;
        let id = clip.id;
        p.add_clip(clip);

        // Command with no builders set touches nothing.
        let mut stack = crate::commands::UndoStack::default();
        stack
            .execute(Box::new(SetClipCommand::new(id)), &mut p)
            .unwrap();

        let after = p.clips.iter().find(|c| c.id == id).unwrap();
        assert_eq!(after.transition_in_easing, EaseCurve::EaseIn);
        assert_eq!(after.transition_out_easing, EaseCurve::EaseOut);
    }

    #[test]
    fn text_motion_set_and_undo() {
        let (mut p, id) = project_with_text();
        let mut stack = UndoStack::default();
        let before = motion_of(&p, id);

        let new_motion = TextMotion {
            x: 0.3,
            y: -0.2,
            rotation: 0.0,
            scale: 1.5,
        };
        let cmd = SetClipCommand::new(id).text_motion(new_motion);
        stack.execute(Box::new(cmd), &mut p).unwrap();
        assert_eq!(motion_of(&p, id), new_motion);

        stack.undo(&mut p).unwrap();
        assert_eq!(motion_of(&p, id), before);
    }

    #[test]
    fn text_effect_set_and_clear_roundtrip() {
        let (mut p, id) = project_with_text();
        let mut stack = UndoStack::default();

        let e = TextEffect {
            kind: TextEffectKind::Pulse,
            period: 0.8,
            amount: 0.5,
        };
        stack
            .execute(
                Box::new(SetClipCommand::new(id).text_effect(Some(e))),
                &mut p,
            )
            .unwrap();
        assert_eq!(effect_of(&p, id), Some(e));

        stack
            .execute(Box::new(SetClipCommand::new(id).text_effect(None)), &mut p)
            .unwrap();
        assert_eq!(effect_of(&p, id), None);

        stack.undo(&mut p).unwrap();
        assert_eq!(effect_of(&p, id), Some(e));
    }

    #[test]
    fn text_motion_noop_on_video_clip() {
        let mut p = ProjectState::default();
        let c = Clip::new_video("x.mp4", 0, 0, 1000);
        let id = c.id;
        p.clips.push(c);
        let mut stack = UndoStack::default();

        let before = p.clips[0].clone();
        let cmd = SetClipCommand::new(id).text_motion(TextMotion {
            x: 0.5,
            y: 0.5,
            rotation: 0.0,
            scale: 2.0,
        });
        stack.execute(Box::new(cmd), &mut p).unwrap();
        assert_eq!(p.clips[0].clip_type, before.clip_type);
    }
}
