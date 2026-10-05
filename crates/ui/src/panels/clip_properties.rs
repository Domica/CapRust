//! Clip properties panel — Video / Sound / Effects tabs.

use crate::i18n_helper::tr;
use crate::theme::tokens::{space, text};
use caprust_core::{Clip, ClipType, ProjectState};
use egui::Ui;
use egui_phosphor::regular as ph;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PropertiesTab {
    #[default]
    Video,
    Sound,
    Effects,
}

#[derive(Debug, Default)]
pub struct PropertiesState {
    pub tab: PropertiesTab,
    /// Pending edits to be committed via SetClipCommand.
    pub pending: Vec<PendingEdit>,
    /// Live text buffers for the caption-segment editors, keyed by
    /// (clip_id, segment_idx). Populated on first render, edited in
    /// place, and dropped when the edit commits (blur / Enter).
    pub caption_edit_buffers: std::collections::HashMap<(Uuid, usize), String>,
    /// The (clip_id, idx) currently holding keyboard focus in a caption
    /// editor. Used to detect blur → commit.
    pub caption_edit_focus: Option<(Uuid, usize)>,
    /// Text buffer for the name field while it has focus.
    pub name_buffer: Option<String>,
    /// True while the name field holds focus (used to detect blur).
    pub name_focus: bool,
    /// Text buffer for the TextOverlay content field while it has focus.
    pub content_buffer: Option<String>,
    /// True while the content field holds focus (used to detect blur).
    pub content_focus: bool,
    /// Set by a timeline double-click on a clip; the next render of
    /// `show` requests keyboard focus on that clip's TextOverlay
    /// content editor. Cleared the moment it is consumed, or on any
    /// frame where the shown clip is not the requested TextOverlay.
    pub focus_content_for: Option<Uuid>,
    /// Last playhead value the panel saw, so "+ Add at playhead" knows
    /// where to drop a new keyframe.
    pub last_playhead_ms: u64,
    /// Cached list of clips for the current frame, so dropdowns that
    /// need project-wide candidates (duck-against) do not have to
    /// re-borrow the project while state is mutably borrowed.
    pub last_project_clips: Vec<caprust_core::Clip>,
}

#[derive(Debug)]
pub enum PendingEdit {
    RemoveEffect(String),
    ClearTransitionIn,
    ClearTransitionOut,
    Speed(f32),
    Reverse(bool),
    FlipH(bool),
    FlipV(bool),
    VolumeDb(f32),
    TrimStart(u64),
    TrimDuration(u64),
    TrackIndex(usize),
    /// Edit the text of caption segment `idx` on the currently-selected
    /// Captions clip. Committed once per edit (blur or Enter), never
    /// per keystroke.
    CaptionSegmentText {
        idx: usize,
        text: String,
    },
    /// Set or clear the clip's display name. Empty string = clear.
    Name(String),
    /// Set the drawtext style id on a TextOverlay clip.
    TextStyle(String),
    /// Replace the text content of a TextOverlay clip. Committed on
    /// blur, never per keystroke.
    TextContent(String),
    /// Replace the visual style of a Captions clip.
    CaptionStyle(caprust_core::clip::CaptionStyle),
    /// Replace the motion transform on a TextOverlay clip.
    TextMotion(caprust_core::clip::TextMotion),
    /// Set or clear the procedural effect on a TextOverlay clip.
    /// Some(Some(e)) = set, Some(None) = clear, None is not used
    /// because this variant is only pushed with an action.
    TextEffect(Option<caprust_core::clip::TextEffect>),
    /// Replace the whole volume automation curve on a clip.
    VolumeKeyframes(Vec<caprust_core::clip::VolumeKeyframe>),
    /// Fade-in duration in seconds. UI works in seconds; converted
    /// to ms in the drain handler (SetClipCommand::fade_in_ms).
    FadeInSec(f32),
    /// Fade-out duration in seconds. Same conversion as FadeInSec.
    FadeOutSec(f32),
    /// Apply EBU R128 loudness normalization on this clip's audio.
    AudioNormalize(bool),
    /// Apply spectral denoise on this clip's audio.
    AudioDenoise(bool),
    /// Apply speech-tuned dynamic range compression on this clip.
    AudioVoiceBoost(bool),
    /// Set or clear the chroma-key spec on the selected clip.
    /// Some(spec) = enable keying, None = disable.
    ChromaKey(Option<caprust_core::clip::ChromaKeySpec>),
    /// Set or clear the auto-duck sidechain control clip.
    DuckAgainst(Option<Uuid>),
    /// Replace the auto-reframe keypoints on the clip. Empty Vec =
    /// clear the pan path and fall back to source fit.
    AutoReframe(Vec<caprust_core::clip::ReframeKeypoint>),
    /// Start an auto-reframe analysis for the currently-selected
    /// clip. The dispatcher resolves the clip id from the selection
    /// and hands it to start_reframe_job. No payload because the
    /// result arrives asynchronously through reframe_rx.
    StartReframe,
    /// Clear the background-removal mask path. The mask file itself
    /// stays in cache until the user clears the project cache.
    ClearBgRemoval,
    /// Start a background-removal job for the currently-selected clip.
    /// Dispatcher resolves the clip id from the selection and hands
    /// it to start_bg_removal_job.
    StartBgRemoval,
    /// Enable or disable the speed ramp end. Some(x) = ramp to x.
    SpeedEnd(Option<f32>),
    /// Set the easing curve of the speed ramp.
    SpeedEase(caprust_core::clip::EaseCurve),
    /// Set the range of the speed ramp (whole clip / first N / last N).
    SpeedRange(caprust_core::clip::SpeedRampRange),
    /// Set the in-transition easing curve. Only meaningful when
    /// `transition_in == Some("fade")`.
    TransitionInEasing(caprust_core::clip::EaseCurve),
    /// Set the out-transition easing curve.
    TransitionOutEasing(caprust_core::clip::EaseCurve),
    /// Set the shared transition duration in milliseconds.
    TransitionDuration(u64),
}

pub fn show(
    ui: &mut Ui,
    project: &ProjectState,
    selected: Option<Uuid>,
    state: &mut PropertiesState,
    playhead_ms: u64,
) {
    state.last_playhead_ms = playhead_ms;
    state.last_project_clips = project.clips.clone();
    let Some(id) = selected else {
        ui.label(
            egui::RichText::new(tr("props-empty"))
                .italics()
                .color(egui::Color32::from_gray(140)),
        );
        return;
    };
    let Some(clip) = project.clips.iter().find(|c| c.id == id) else {
        ui.label(
            egui::RichText::new(tr("props-empty"))
                .italics()
                .color(egui::Color32::from_gray(140)),
        );
        return;
    };

    // Clear a stale focus request when the shown clip is not the one
    // the timeline asked us to focus. A TextOverlay clip whose id
    // matches keeps the request alive until its content editor below
    // consumes it.
    if state.focus_content_for != Some(clip.id)
        || !matches!(clip.clip_type, ClipType::TextOverlay { .. })
    {
        state.focus_content_for = None;
    }

    // --- Header ---
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(clip_kind_label(clip))
                .strong()
                .size(13.0),
        );
        ui.separator();
        ui.label(
            egui::RichText::new(format_duration(clip.start_time_ms + clip.duration_ms))
                .monospace()
                .color(egui::Color32::from_gray(180)),
        );
    });

    // --- Name ---
    // Editable display name. Empty clears it, falling back to the file
    // name at render time.
    ui.horizontal(|ui| {
        ui.label(tr("props-name"));
        let current = clip.name.clone().unwrap_or_default();
        let mut buf = state.name_buffer.clone().unwrap_or_else(|| current.clone());
        let resp = ui.add(
            egui::TextEdit::singleline(&mut buf)
                .desired_width(f32::INFINITY)
                .hint_text("(file name)"),
        );
        let had_focus = state.name_focus;
        let has_focus = resp.has_focus();
        if has_focus {
            state.name_focus = true;
            state.name_buffer = Some(buf.clone());
        }
        if (had_focus && !has_focus) || (has_focus && ui.input(|i| i.key_pressed(egui::Key::Enter)))
        {
            if buf != current {
                state.pending.push(PendingEdit::Name(buf.clone()));
            }
            state.name_focus = false;
            state.name_buffer = None;
        }
    });
    ui.separator();

    // --- TextOverlay content editor ---
    // Multiline TextEdit that commits on blur (never per keystroke).
    // The clip's render uses `content` directly, so editing here updates
    // both preview and export on the next render pass.
    if let ClipType::TextOverlay { content, .. } = &clip.clip_type {
        ui.horizontal(|ui| {
            ui.label(tr("props-text-content"));
        });
        let mut buf = state
            .content_buffer
            .clone()
            .unwrap_or_else(|| content.clone());
        let resp = ui.add(
            egui::TextEdit::multiline(&mut buf)
                .id(egui::Id::new(("text_content_edit", clip.id)))
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
        if state.focus_content_for == Some(clip.id) {
            resp.request_focus();
            state.focus_content_for = None;
        }
        let had_focus = state.content_focus;
        let has_focus = resp.has_focus();
        if has_focus {
            state.content_focus = true;
            state.content_buffer = Some(buf.clone());
        }
        if had_focus && !has_focus && buf != *content {
            state.pending.push(PendingEdit::TextContent(buf));
            state.content_focus = false;
            state.content_buffer = None;
        } else if !has_focus {
            state.content_focus = false;
            state.content_buffer = None;
        }
        ui.separator();
    }

    // --- TextOverlay style picker ---
    if let ClipType::TextOverlay { style, .. } = &clip.clip_type {
        const STYLES: &[(&str, &str)] = &[
            ("default", "asset-text-default"),
            ("bold", "asset-text-bold"),
            ("subtitle", "asset-text-subtitle"),
            ("lower", "asset-text-lower"),
            ("quote", "asset-text-quote"),
            ("caption", "asset-text-caption"),
            ("glow", "asset-text-glow"),
            ("handwrite", "asset-text-handwrite"),
        ];
        let current = if style.is_empty() {
            "default"
        } else {
            style.as_str()
        };
        let current_label = STYLES
            .iter()
            .find(|(id, _)| *id == current)
            .map(|(_, key)| tr(key))
            .unwrap_or_else(|| current.to_string());
        ui.horizontal(|ui| {
            ui.label(tr("props-text-style"));
            egui::ComboBox::from_id_salt("text_style_combo")
                .selected_text(current_label)
                .width(180.0) // model / language combo; component-specific
                .show_ui(ui, |ui| {
                    for (id, key) in STYLES {
                        let selected = *id == current;
                        if ui.selectable_label(selected, tr(key)).clicked() && !selected {
                            state
                                .pending
                                .push(PendingEdit::TextStyle((*id).to_string()));
                        }
                    }
                });
        });
        ui.separator();
    }

    // --- TextOverlay: motion transform + procedural effect ---
    // Both are stored on the TextOverlay variant; the UI shows them
    // only for that kind. Rotation is stored for forward-compat but
    // NOT exposed here because drawtext has no rotate filter — a real
    // rotation needs a PNG-overlay refactor (see DIRECTIVES §30).
    if let ClipType::TextOverlay { motion, effect, .. } = &clip.clip_type {
        use caprust_core::clip::{TextEffect, TextEffectKind, TextMotion};

        // --- Motion ---
        let mut m: TextMotion = *motion;
        let mut motion_changed = false;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tr("props-text-motion-header")).strong());
            if ui.small_button(tr("props-text-motion-reset")).clicked() {
                m = TextMotion::default();
                motion_changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label(tr("props-text-motion-x"));
            motion_changed |= ui
                .add(
                    egui::DragValue::new(&mut m.x)
                        .speed(0.01)
                        .range(-1.0..=1.0)
                        .fixed_decimals(3),
                )
                .changed();
        });
        ui.horizontal(|ui| {
            ui.label(tr("props-text-motion-y"));
            motion_changed |= ui
                .add(
                    egui::DragValue::new(&mut m.y)
                        .speed(0.01)
                        .range(-1.0..=1.0)
                        .fixed_decimals(3),
                )
                .changed();
        });
        ui.horizontal(|ui| {
            ui.label(tr("props-text-motion-scale"));
            motion_changed |= ui
                .add(
                    egui::DragValue::new(&mut m.scale)
                        .speed(0.05)
                        .range(0.1..=5.0)
                        .fixed_decimals(2),
                )
                .changed();
        });
        if motion_changed {
            state.pending.push(PendingEdit::TextMotion(m));
        }

        ui.separator();

        // --- Effect ---
        let cur_effect: Option<TextEffect> = *effect;
        let current_label = match cur_effect {
            None => tr("props-text-effect-none"),
            Some(e) => match e.kind {
                TextEffectKind::Blink => tr("props-text-effect-blink"),
                TextEffectKind::Pulse => tr("props-text-effect-pulse"),
                TextEffectKind::ColorCycle => tr("props-text-effect-color-cycle"),
            },
        };
        let mut pending_effect: Option<Option<TextEffect>> = None;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tr("props-text-effect-header")).strong());
            egui::ComboBox::from_id_salt("text_effect_combo")
                .selected_text(current_label)
                .width(180.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(cur_effect.is_none(), tr("props-text-effect-none"))
                        .clicked()
                        && cur_effect.is_some()
                    {
                        pending_effect = Some(None);
                    }
                    for (k, key) in [
                        (TextEffectKind::Blink, "props-text-effect-blink"),
                        (TextEffectKind::Pulse, "props-text-effect-pulse"),
                        (TextEffectKind::ColorCycle, "props-text-effect-color-cycle"),
                    ] {
                        let selected = cur_effect.map(|e| e.kind) == Some(k);
                        if ui.selectable_label(selected, tr(key)).clicked() && !selected {
                            let mut base = cur_effect.unwrap_or(TextEffect {
                                kind: k,
                                period: 1.0,
                                amount: 1.0,
                            });
                            base.kind = k;
                            pending_effect = Some(Some(base));
                        }
                    }
                });
        });
        if let Some(v) = pending_effect {
            state.pending.push(PendingEdit::TextEffect(v));
        }

        // Period + amount only when an effect is selected.
        if let Some(cur_e) = cur_effect {
            let mut e = cur_e;
            let mut eff_changed = false;
            ui.horizontal(|ui| {
                ui.label(tr("props-text-effect-period"));
                eff_changed |= ui
                    .add(
                        egui::DragValue::new(&mut e.period)
                            .speed(0.05)
                            .range(0.05..=10.0)
                            .fixed_decimals(2),
                    )
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label(tr("props-text-effect-amount"));
                eff_changed |= ui
                    .add(
                        egui::DragValue::new(&mut e.amount)
                            .speed(0.02)
                            .range(0.0..=1.0)
                            .fixed_decimals(2),
                    )
                    .changed();
            });
            if eff_changed {
                state.pending.push(PendingEdit::TextEffect(Some(e)));
            }
        }

        ui.separator();
    }

    // --- Captions: read-only transcript view ---
    //
    // Captions clips have no editable geometry or effects; the useful
    // information is the transcript itself. Show model, language,
    // segment count, and the segment list with timestamps so the user
    // can verify a transcription without opening the .caprust JSON.
    if let ClipType::Captions {
        model_id,
        language,
        segments,
        style: caption_style,
        motion,
        effect,
    } = &clip.clip_type
    {
        ui.add_space(space::XS);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Model")
                    .small()
                    .color(egui::Color32::from_gray(140)),
            );
            ui.label(egui::RichText::new(model_id).monospace().size(text::S));
            ui.separator();
            ui.label(
                egui::RichText::new("Language")
                    .small()
                    .color(egui::Color32::from_gray(140)),
            );
            ui.label(egui::RichText::new(language).monospace().size(text::S));
        });

        let n = segments.len();
        ui.add_space(space::XXS);
        if n == 0 {
            ui.label(
                egui::RichText::new("No speech detected. (Empty segments — the clip can be deleted from the timeline.)")
                    .italics()
                    .color(egui::Color32::from_gray(150)),
            );
            return;
        }
        ui.label(
            egui::RichText::new(format!("{n} segment{}", if n == 1 { "" } else { "s" }))
                .color(egui::Color32::from_gray(200)),
        );
        ui.label(
            egui::RichText::new("Edit any text and press Enter (or click away) to save.")
                .small()
                .color(egui::Color32::from_gray(140)),
        );
        // --- Style editor ---
        ui.add_space(space::S);
        ui.label(egui::RichText::new(tr("props-captions-style-header")).strong());
        {
            use caprust_core::clip::{CaptionPosition, CaptionStyle};
            let mut cs: CaptionStyle = *caption_style;
            let mut changed = false;

            ui.horizontal(|ui| {
                ui.label(tr("props-captions-style-font-size"));
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut cs.font_size)
                            .speed(0.5)
                            .range(8.0..=200.0)
                            .fixed_decimals(0),
                    )
                    .changed();
            });

            ui.horizontal(|ui| {
                ui.label(tr("props-captions-style-position"));
                let label = match cs.position {
                    CaptionPosition::Top => tr("props-captions-style-position-top"),
                    CaptionPosition::Middle => tr("props-captions-style-position-middle"),
                    CaptionPosition::Bottom => tr("props-captions-style-position-bottom"),
                };
                egui::ComboBox::from_id_salt("cap_pos_combo")
                    .selected_text(label)
                    .width(140.0) // effect/param combo; component-specific
                    .show_ui(ui, |ui| {
                        for (p, key) in [
                            (CaptionPosition::Top, "props-captions-style-position-top"),
                            (
                                CaptionPosition::Middle,
                                "props-captions-style-position-middle",
                            ),
                            (
                                CaptionPosition::Bottom,
                                "props-captions-style-position-bottom",
                            ),
                        ] {
                            let sel = cs.position == p;
                            if ui.selectable_label(sel, tr(key)).clicked() && !sel {
                                cs.position = p;
                                changed = true;
                            }
                        }
                    });
            });

            ui.horizontal(|ui| {
                ui.label(tr("props-captions-style-color"));
                let mut c = [
                    cs.color[0] as f32 / 255.0,
                    cs.color[1] as f32 / 255.0,
                    cs.color[2] as f32 / 255.0,
                ];
                if ui.color_edit_button_rgb(&mut c).changed() {
                    cs.color = [
                        (c[0] * 255.0).round() as u8,
                        (c[1] * 255.0).round() as u8,
                        (c[2] * 255.0).round() as u8,
                    ];
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label(tr("props-captions-style-outline-color"));
                let mut c = [
                    cs.outline_color[0] as f32 / 255.0,
                    cs.outline_color[1] as f32 / 255.0,
                    cs.outline_color[2] as f32 / 255.0,
                ];
                if ui.color_edit_button_rgb(&mut c).changed() {
                    cs.outline_color = [
                        (c[0] * 255.0).round() as u8,
                        (c[1] * 255.0).round() as u8,
                        (c[2] * 255.0).round() as u8,
                    ];
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label(tr("props-captions-style-outline-width"));
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut cs.outline_width)
                            .speed(0.2)
                            .range(0.0..=10.0)
                            .fixed_decimals(1),
                    )
                    .changed();
            });

            ui.horizontal(|ui| {
                changed |= ui
                    .checkbox(&mut cs.bg_enabled, tr("props-captions-style-bg"))
                    .changed();
            });
            if cs.bg_enabled {
                ui.horizontal(|ui| {
                    ui.label(tr("props-captions-style-bg-opacity"));
                    changed |= ui
                        .add(egui::Slider::new(&mut cs.bg_opacity, 0.0..=1.0).fixed_decimals(2))
                        .changed();
                });
            }

            if changed {
                state.pending.push(PendingEdit::CaptionStyle(cs));
            }
        }

        // --- Motion transform + procedural effect ---
        // Same controls as a TextOverlay clip; both variants carry
        // the same fields now. Shared PendingEdit variants route
        // through SetClipCommand, which applies to either kind.
        ui.add_space(space::S);
        {
            use caprust_core::clip::{TextEffect, TextEffectKind, TextMotion};

            let mut m: TextMotion = *motion;
            let mut m_changed = false;
            ui.horizontal(|ui| {
                ui.label(tr("props-text-motion-x"));
                m_changed |= ui
                    .add(
                        egui::DragValue::new(&mut m.x)
                            .speed(0.005)
                            .range(-1.0..=1.0)
                            .fixed_decimals(3),
                    )
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label(tr("props-text-motion-y"));
                m_changed |= ui
                    .add(
                        egui::DragValue::new(&mut m.y)
                            .speed(0.005)
                            .range(-1.0..=1.0)
                            .fixed_decimals(3),
                    )
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label(tr("props-text-motion-scale"));
                m_changed |= ui
                    .add(
                        egui::DragValue::new(&mut m.scale)
                            .speed(0.01)
                            .range(0.3..=3.0)
                            .fixed_decimals(2),
                    )
                    .changed();
            });
            if m_changed {
                state.pending.push(PendingEdit::TextMotion(m));
            }

            // Effect
            ui.add_space(space::XS);
            ui.label(
                egui::RichText::new(tr("props-text-effect-header"))
                    .strong()
                    .size(12.0),
            );
            let mut cur_kind_label = match effect.as_ref().map(|e| e.kind) {
                None => tr("props-text-effect-none").to_string(),
                Some(TextEffectKind::Blink) => tr("props-text-effect-blink").to_string(),
                Some(TextEffectKind::Pulse) => tr("props-text-effect-pulse").to_string(),
                Some(TextEffectKind::ColorCycle) => tr("props-text-effect-color-cycle").to_string(),
            };
            let mut new_effect: Option<TextEffect> = *effect;
            ui.horizontal(|ui| {
                ui.label(tr("props-text-effect-kind"));
                egui::ComboBox::from_id_salt("cap_effect_combo")
                    .selected_text(cur_kind_label.clone())
                    .width(140.0)
                    .show_ui(ui, |ui| {
                        for (kind, key) in [
                            (None, "props-text-effect-none"),
                            (Some(TextEffectKind::Blink), "props-text-effect-blink"),
                            (Some(TextEffectKind::Pulse), "props-text-effect-pulse"),
                            (
                                Some(TextEffectKind::ColorCycle),
                                "props-text-effect-color-cycle",
                            ),
                        ] {
                            let sel = effect.as_ref().map(|e| e.kind) == kind;
                            if ui.selectable_label(sel, tr(key)).clicked() && !sel {
                                new_effect = kind.map(|k| TextEffect {
                                    kind: k,
                                    period: effect.as_ref().map(|e| e.period).unwrap_or(1.0),
                                    amount: effect.as_ref().map(|e| e.amount).unwrap_or(0.8),
                                });
                                cur_kind_label = tr(key).to_string();
                            }
                        }
                    });
            });
            if let Some(mut e) = new_effect {
                let mut e_changed = effect.as_ref().map(|old| old.kind) != Some(e.kind);
                ui.horizontal(|ui| {
                    ui.label(tr("props-text-effect-period"));
                    e_changed |= ui
                        .add(
                            egui::DragValue::new(&mut e.period)
                                .speed(0.05)
                                .range(0.1..=10.0)
                                .fixed_decimals(2),
                        )
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label(tr("props-text-effect-amount"));
                    e_changed |= ui
                        .add(egui::Slider::new(&mut e.amount, 0.0..=1.0).fixed_decimals(2))
                        .changed();
                });
                if e_changed {
                    state.pending.push(PendingEdit::TextEffect(Some(e)));
                }
            } else if effect.is_some() {
                state.pending.push(PendingEdit::TextEffect(None));
            }
        }

        ui.add_space(space::XS);
        ui.separator();
        ui.add_space(space::XS);

        let clip_id = clip.id;
        let mut local_edits: Vec<PendingEdit> = Vec::new();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (i, seg) in segments.iter().enumerate() {
                    let key = (clip_id, i);
                    let original = seg.text.clone();

                    // Initialize the buffer on first render of this
                    // segment in this session.
                    state
                        .caption_edit_buffers
                        .entry(key)
                        .or_insert_with(|| original.clone());

                    // Take the buffer out of the map so we can hand a
                    // `&mut String` to TextEdit without holding a
                    // mutable borrow on `state` for the whole closure.
                    let mut buf = state
                        .caption_edit_buffers
                        .remove(&key)
                        .unwrap_or_else(|| original.clone());

                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{:>2}", i + 1))
                                .monospace()
                                .color(egui::Color32::from_gray(130)),
                        );
                        ui.label(
                            egui::RichText::new(format!(
                                "{} → {}",
                                format_duration(seg.start_ms),
                                format_duration(seg.end_ms)
                            ))
                            .monospace()
                            .size(10.5)
                            .color(egui::Color32::from_gray(150)),
                        );
                    });

                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut buf)
                            .id(egui::Id::new(("cap_seg_edit", clip_id, i)))
                            .desired_width(f32::INFINITY),
                    );

                    let had_focus = state.caption_edit_focus == Some(key);
                    let has_focus = resp.has_focus();
                    let enter_pressed =
                        has_focus && ui.input(|inp| inp.key_pressed(egui::Key::Enter));

                    if has_focus {
                        state.caption_edit_focus = Some(key);
                    }

                    // Commit on blur (had focus last frame, lost it now)
                    // or on Enter while focused. Never on every keystroke.
                    let commit = (had_focus && !has_focus) || enter_pressed;
                    if commit {
                        if buf != original {
                            local_edits.push(PendingEdit::CaptionSegmentText {
                                idx: i,
                                text: buf.clone(),
                            });
                        }
                        state.caption_edit_focus = None;
                        // Buffer intentionally dropped: next frame
                        // re-reads from the (now updated) segment.
                    } else {
                        state.caption_edit_buffers.insert(key, buf);
                    }

                    ui.add_space(space::XS);
                    ui.separator();
                    ui.add_space(space::XS);
                }
            });

        state.pending.extend(local_edits);
        return;
    }

    // Audio and Narration clips have no video geometry. Hide the
    // Video tab button below, and route a stale `state.tab == Video`
    // from a previously selected clip into show_sound, so audio clips
    // never surface video-only controls. Do NOT force state.tab every
    // frame: the user must still be able to click Efekti.
    let clip_is_audio = matches!(
        clip.clip_type,
        ClipType::Audio { .. } | ClipType::Narration { .. }
    );

    // --- Tabs ---
    ui.horizontal(|ui| {
        if !clip_is_audio {
            ui.selectable_value(&mut state.tab, PropertiesTab::Video, tr("props-tab-video"));
        }
        ui.selectable_value(&mut state.tab, PropertiesTab::Sound, tr("props-tab-sound"));
        ui.selectable_value(
            &mut state.tab,
            PropertiesTab::Effects,
            tr("props-tab-effects"),
        );
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match state.tab {
            PropertiesTab::Video => {
                if clip_is_audio {
                    show_sound(ui, clip, state);
                } else {
                    show_video(ui, clip, state);
                }
            }
            PropertiesTab::Sound => show_sound(ui, clip, state),
            PropertiesTab::Effects => show_effects(ui, clip, state),
        });
}

fn ease_label(e: caprust_core::clip::EaseCurve) -> String {
    use caprust_core::clip::EaseCurve;
    let key = match e {
        EaseCurve::Linear => "props-ease-linear",
        EaseCurve::EaseIn => "props-ease-in",
        EaseCurve::EaseOut => "props-ease-out",
        EaseCurve::EaseInOut => "props-ease-in-out",
    };
    tr(key)
}

/// Extract the N (ms) from a SpeedRampRange, or the default 2000 for
/// WholeClip so the radio button switch keeps a sensible last value.
fn speed_range_n(r: &caprust_core::clip::SpeedRampRange) -> u64 {
    use caprust_core::clip::SpeedRampRange;
    match r {
        SpeedRampRange::FirstN(n) => *n,
        SpeedRampRange::LastN(n) => *n,
        SpeedRampRange::WholeClip => 2000,
    }
}

fn show_video(ui: &mut Ui, clip: &Clip, state: &mut PropertiesState) {
    // --- Main ---
    ui.label(egui::RichText::new(tr("props-video-main")).strong());
    ui.add_space(space::XS);
    // (Rotation / Width / Height fields used to live here. They wrote
    // into local f32 bindings that were dropped every frame, so they
    // never affected the render. Real rotation needs a PNG-overlay
    // refactor (drawtext has no rotate filter); size for TextOverlay
    // is already controlled by the motion.scale field in the Text
    // section. Removed to stop showing controls that lie.)

    ui.add_space(space::M_PLUS);
    ui.separator();

    // Auto-reframe and background removal are video/image features.
    // Hide the whole block on TextOverlay clips so the panel does not
    // offer controls that do not apply.
    let is_text_overlay = matches!(clip.clip_type, ClipType::TextOverlay { .. });

    if !is_text_overlay {
        // --- Auto-reframe ---
        ui.label(egui::RichText::new(tr("props-video-auto-reframe")).strong());
        ui.add_space(space::XS);
        ui.label(
            egui::RichText::new(tr("props-auto-reframe-hint"))
                .small()
                .color(egui::Color32::from_gray(150)),
        );
        ui.horizontal(|ui| {
            let can_run = matches!(
                clip.clip_type,
                ClipType::Video { .. } | ClipType::Image { .. }
            );
            let n = clip.auto_reframe.len();
            if n == 0 {
                ui.label(
                    egui::RichText::new(tr("props-auto-reframe-none"))
                        .small()
                        .color(egui::Color32::from_gray(140)),
                );
            } else {
                ui.label(
                    egui::RichText::new(format!("{} ({n})", tr("props-auto-reframe-done")))
                        .small()
                        .color(egui::Color32::from_gray(200)),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if n > 0
                    && ui
                        .button(tr("props-auto-reframe-clear"))
                        .on_hover_text(tr("props-auto-reframe-clear-hint"))
                        .clicked()
                {
                    state.pending.push(PendingEdit::AutoReframe(Vec::new()));
                }
                ui.add_enabled_ui(can_run, |ui| {
                    if ui
                        .button(tr("props-auto-reframe-run"))
                        .on_hover_text(tr("props-auto-reframe-run-hint"))
                        .clicked()
                    {
                        state.pending.push(PendingEdit::StartReframe);
                    }
                });
            });
        });

        ui.add_space(space::M_PLUS);
        ui.separator();

        // --- Background removal ---
        ui.label(egui::RichText::new(tr("props-video-bg-removal")).strong());
        ui.add_space(space::XS);
        ui.label(
            egui::RichText::new(tr("props-bg-removal-hint"))
                .small()
                .color(egui::Color32::from_gray(150)),
        );
        ui.horizontal(|ui| {
            let can_run = matches!(
                clip.clip_type,
                ClipType::Video { .. } | ClipType::Image { .. }
            );
            let has_mask = clip.bg_removal.is_some();
            if has_mask {
                ui.label(
                    egui::RichText::new(tr("props-bg-removal-done"))
                        .small()
                        .color(egui::Color32::from_gray(200)),
                );
            } else {
                ui.label(
                    egui::RichText::new(tr("props-bg-removal-none"))
                        .small()
                        .color(egui::Color32::from_gray(140)),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if has_mask
                    && ui
                        .button(tr("props-bg-removal-clear"))
                        .on_hover_text(tr("props-bg-removal-clear-hint"))
                        .clicked()
                {
                    state.pending.push(PendingEdit::ClearBgRemoval);
                }
                ui.add_enabled_ui(can_run, |ui| {
                    if ui
                        .button(tr("props-bg-removal-run"))
                        .on_hover_text(tr("props-bg-removal-run-hint"))
                        .clicked()
                    {
                        state.pending.push(PendingEdit::StartBgRemoval);
                    }
                });
            });
        });

        ui.add_space(space::M_PLUS);
        ui.separator();
    } // !is_text_overlay
    ui.add_space(space::M_PLUS);
    ui.separator();

    // --- Chroma key (green screen) ---
    ui.label(egui::RichText::new(tr("props-video-chroma-key")).strong());
    ui.add_space(space::XS);
    ui.label(
        egui::RichText::new(tr("props-chroma-hint"))
            .small()
            .color(egui::Color32::from_gray(150)),
    );
    let chroma_supported = matches!(
        clip.clip_type,
        ClipType::Video { .. } | ClipType::Image { .. }
    );
    if !chroma_supported {
        ui.label(
            egui::RichText::new(tr("props-chroma-not-video"))
                .small()
                .color(egui::Color32::from_gray(140)),
        );
    } else if let Some(ck) = clip.chroma_key {
        let mut next = ck;
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label(tr("props-chroma-color"));
            let mut rgb = [
                next.key_color[0] as f32 / 255.0,
                next.key_color[1] as f32 / 255.0,
                next.key_color[2] as f32 / 255.0,
            ];
            if ui.color_edit_button_rgb(&mut rgb).changed() {
                next.key_color = [
                    (rgb[0] * 255.0).round() as u8,
                    (rgb[1] * 255.0).round() as u8,
                    (rgb[2] * 255.0).round() as u8,
                ];
                changed = true;
            }
        });
        ui.label(tr("props-chroma-similarity"));
        if ui
            .add(egui::Slider::new(&mut next.similarity, 0.01..=0.5).fixed_decimals(3))
            .changed()
        {
            changed = true;
        }
        ui.label(tr("props-chroma-blend"));
        if ui
            .add(egui::Slider::new(&mut next.blend, 0.0..=0.3).fixed_decimals(3))
            .changed()
        {
            changed = true;
        }
        if changed {
            state.pending.push(PendingEdit::ChromaKey(Some(next)));
        }
        if ui.button(tr("props-chroma-clear")).clicked() {
            state.pending.push(PendingEdit::ChromaKey(None));
        }
    } else {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(tr("props-chroma-none"))
                    .small()
                    .color(egui::Color32::from_gray(140)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(tr("props-chroma-enable"))
                    .on_hover_text(tr("props-chroma-enable-hint"))
                    .clicked()
                {
                    state.pending.push(PendingEdit::ChromaKey(Some(
                        caprust_core::clip::ChromaKeySpec::default(),
                    )));
                }
            });
        });
    }

    // --- Speed ---
    ui.label(egui::RichText::new(tr("props-video-speed")).strong());
    ui.add_space(space::XS);
    egui::Grid::new("clip_speed_base_grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label(tr("props-field-speed"));
            let mut speed = clip.speed;
            let combo = egui::ComboBox::from_id_salt("clip_speed")
                .selected_text(format!("{speed:.2}×"))
                .width(120.0); // speed combo; component-specific
            combo.show_ui(ui, |ui| {
                for s in [0.1, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0] {
                    if ui
                        .selectable_value(&mut speed, s, format!("{s:.2}×"))
                        .changed()
                    {
                        state.pending.push(PendingEdit::Speed(speed));
                    }
                }
            });
            ui.end_row();

            ui.label(tr("props-field-custom"));
            let mut custom = speed;
            if ui
                .add(
                    egui::DragValue::new(&mut custom)
                        .range(0.1..=10.0)
                        .speed(0.01)
                        .suffix("×"),
                )
                .changed()
            {
                state.pending.push(PendingEdit::Speed(custom));
            }
            ui.end_row();

            ui.label(tr("props-field-reverse"));
            let mut rev = clip.reversed;
            if ui.checkbox(&mut rev, "").changed() {
                state.pending.push(PendingEdit::Reverse(rev));
            }
            ui.end_row();
        });

    ui.add_space(space::M_PLUS);
    ui.separator();

    // --- Mirror ---
    ui.label(egui::RichText::new(tr("props-video-mirror")).strong());
    ui.add_space(space::XS);
    egui::Grid::new("clip_mirror_grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label(tr("props-mirror-h"));
            let mut h = clip.flip_h;
            if ui.checkbox(&mut h, "").changed() {
                state.pending.push(PendingEdit::FlipH(h));
            }
            ui.end_row();

            ui.label(tr("props-mirror-v"));
            let mut v = clip.flip_v;
            if ui.checkbox(&mut v, "").changed() {
                state.pending.push(PendingEdit::FlipV(v));
            }
            ui.end_row();
        });

    ui.add_space(space::M_PLUS);
    ui.separator();

    // --- Trim (read-only info) ---
    ui.label(egui::RichText::new(tr("props-video-trim")).strong());
    ui.add_space(space::XS);
    ui.label(
        egui::RichText::new(tr("props-trim-hint"))
            .small()
            .color(egui::Color32::from_gray(150)),
    );
    egui::Grid::new("clip_trim_grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label(tr("props-field-start"));
            ui.label(format_duration(clip.start_time_ms));
            ui.end_row();
            ui.label(tr("props-field-duration"));
            ui.label(format_duration(clip.duration_ms));
            ui.end_row();
            ui.label(tr("props-field-source"));
            if clip.source_duration_ms == 0 {
                ui.label(tr("props-value-unlimited"));
            } else {
                ui.label(format_duration(clip.source_duration_ms));
            }
            ui.end_row();
        });
    ui.add_space(space::M_PLUS);
    ui.separator();

    // ---- Speed ramp ----
    ui.label(egui::RichText::new(tr("props-video-speed-ramp")).strong());
    ui.add_space(space::XS);
    egui::Grid::new("clip_speed_grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label(tr("props-field-speed-start"));
            let mut s_start = clip.speed;
            if ui
                .add(
                    egui::DragValue::new(&mut s_start)
                        .range(0.1..=8.0)
                        .speed(0.05)
                        .suffix(" x"),
                )
                .changed()
            {
                state.pending.push(PendingEdit::Speed(s_start));
            }
            ui.end_row();

            let mut ramp_enabled = clip.speed_end.is_some();
            ui.label(tr("props-field-speed-end"));
            ui.horizontal(|ui| {
                if ui.checkbox(&mut ramp_enabled, "").changed() {
                    if ramp_enabled {
                        // Default the ramp end to the current speed,
                        // then the user drags it in the direction they
                        // want. Zero-delta ramps are a no-op anyway.
                        state.pending.push(PendingEdit::SpeedEnd(Some(clip.speed)));
                    } else {
                        state.pending.push(PendingEdit::SpeedEnd(None));
                    }
                }
                let mut s_end = clip.speed_end.unwrap_or(clip.speed);
                if ui
                    .add_enabled(
                        ramp_enabled,
                        egui::DragValue::new(&mut s_end)
                            .range(0.1..=8.0)
                            .speed(0.05)
                            .suffix(" x"),
                    )
                    .changed()
                    && ramp_enabled
                {
                    state.pending.push(PendingEdit::SpeedEnd(Some(s_end)));
                }
            });
            ui.end_row();
        });

    // Ease + range (only meaningful when a ramp is enabled)
    let ramp_on = clip.speed_end.is_some();
    ui.add_enabled_ui(ramp_on, |ui| {
        egui::Grid::new("clip_speed_ease_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label(tr("props-field-speed-ease"));
                egui::ComboBox::from_id_salt("speed_ease_combo")
                    .selected_text(ease_label(clip.speed_ease))
                    .width(140.0)
                    .show_ui(ui, |ui| {
                        for e in [
                            caprust_core::clip::EaseCurve::Linear,
                            caprust_core::clip::EaseCurve::EaseIn,
                            caprust_core::clip::EaseCurve::EaseOut,
                            caprust_core::clip::EaseCurve::EaseInOut,
                        ] {
                            let selected = clip.speed_ease == e;
                            if ui.selectable_label(selected, ease_label(e)).clicked() && !selected {
                                state.pending.push(PendingEdit::SpeedEase(e));
                            }
                        }
                    });
                ui.end_row();

                ui.label(tr("props-field-speed-range"));
                ui.horizontal(|ui| {
                    let current_n = speed_range_n(&clip.speed_range);
                    let is_whole = matches!(
                        clip.speed_range,
                        caprust_core::clip::SpeedRampRange::WholeClip
                    );
                    let is_first = matches!(
                        clip.speed_range,
                        caprust_core::clip::SpeedRampRange::FirstN(_)
                    );
                    let is_last = matches!(
                        clip.speed_range,
                        caprust_core::clip::SpeedRampRange::LastN(_)
                    );

                    if ui
                        .selectable_label(is_whole, tr("props-speed-range-whole"))
                        .clicked()
                        && !is_whole
                    {
                        state.pending.push(PendingEdit::SpeedRange(
                            caprust_core::clip::SpeedRampRange::WholeClip,
                        ));
                    }
                    if ui
                        .selectable_label(is_first, tr("props-speed-range-first"))
                        .clicked()
                        && !is_first
                    {
                        state.pending.push(PendingEdit::SpeedRange(
                            caprust_core::clip::SpeedRampRange::FirstN(current_n),
                        ));
                    }
                    if ui
                        .selectable_label(is_last, tr("props-speed-range-last"))
                        .clicked()
                        && !is_last
                    {
                        state.pending.push(PendingEdit::SpeedRange(
                            caprust_core::clip::SpeedRampRange::LastN(current_n),
                        ));
                    }
                });
                ui.end_row();

                if !matches!(
                    clip.speed_range,
                    caprust_core::clip::SpeedRampRange::WholeClip
                ) {
                    ui.label(tr("props-field-speed-n"));
                    let mut n_secs = (speed_range_n(&clip.speed_range) as f32) / 1000.0;
                    if ui
                        .add(
                            egui::Slider::new(&mut n_secs, 0.5..=10.0)
                                .suffix(" s")
                                .show_value(true),
                        )
                        .changed()
                    {
                        let n_ms = (n_secs * 1000.0) as u64;
                        let new_range = match clip.speed_range {
                            caprust_core::clip::SpeedRampRange::FirstN(_) => {
                                caprust_core::clip::SpeedRampRange::FirstN(n_ms)
                            }
                            caprust_core::clip::SpeedRampRange::LastN(_) => {
                                caprust_core::clip::SpeedRampRange::LastN(n_ms)
                            }
                            caprust_core::clip::SpeedRampRange::WholeClip => {
                                caprust_core::clip::SpeedRampRange::WholeClip
                            }
                        };
                        state.pending.push(PendingEdit::SpeedRange(new_range));
                    }
                    ui.end_row();
                }
            });
    });

    // Presets
    ui.horizontal(|ui| {
        ui.label(tr("props-field-speed-preset"));
        use caprust_core::clip::{EaseCurve, SpeedRampRange};
        type Preset = (&'static str, f32, Option<f32>, EaseCurve, SpeedRampRange);
        let presets: &[Preset] = &[
            (
                "1x",
                1.0,
                None,
                EaseCurve::Linear,
                SpeedRampRange::WholeClip,
            ),
            (
                "0.5x",
                0.5,
                None,
                EaseCurve::Linear,
                SpeedRampRange::WholeClip,
            ),
            (
                "2x",
                2.0,
                None,
                EaseCurve::Linear,
                SpeedRampRange::WholeClip,
            ),
            (
                "4x",
                4.0,
                None,
                EaseCurve::Linear,
                SpeedRampRange::WholeClip,
            ),
            (
                "Slow-out",
                1.0,
                Some(0.5),
                EaseCurve::EaseOut,
                SpeedRampRange::LastN(2000),
            ),
            (
                "Fast-in",
                0.5,
                Some(1.0),
                EaseCurve::EaseIn,
                SpeedRampRange::FirstN(2000),
            ),
            (
                "Ramp-up",
                0.5,
                Some(2.0),
                EaseCurve::EaseIn,
                SpeedRampRange::WholeClip,
            ),
            (
                "Ramp-down",
                2.0,
                Some(0.5),
                EaseCurve::EaseOut,
                SpeedRampRange::WholeClip,
            ),
        ];
        for (label, start_v, end_v, e, r) in presets {
            let selected = (clip.speed - *start_v).abs() < 0.01
                && clip.speed_end == *end_v
                && clip.speed_ease == *e
                && clip.speed_range == *r;
            if ui.selectable_label(selected, *label).clicked() && !selected {
                state.pending.push(PendingEdit::Speed(*start_v));
                state.pending.push(PendingEdit::SpeedEnd(*end_v));
                state.pending.push(PendingEdit::SpeedEase(*e));
                state.pending.push(PendingEdit::SpeedRange(*r));
            }
        }
    });
}

fn show_sound(ui: &mut Ui, clip: &Clip, state: &mut PropertiesState) {
    ui.label(egui::RichText::new(tr("props-sound-volume")).strong());
    ui.add_space(space::XS);
    ui.horizontal(|ui| {
        let mut vol = clip.volume_db;
        if ui
            .add(
                egui::Slider::new(&mut vol, -30.0..=30.0)
                    .suffix(" dB")
                    .show_value(true),
            )
            .changed()
        {
            state.pending.push(PendingEdit::VolumeDb(vol));
        }
    });

    ui.add_space(space::M_PLUS);
    ui.separator();

    // ---- Auto-ducking ----
    // Only meaningful on audio-bearing clips. The list of candidates
    // is every other audio-bearing clip in the project; picking one
    // makes this clip drop in level whenever that clip's stream plays.
    ui.label(egui::RichText::new(tr("props-sound-ducking")).strong());
    ui.add_space(space::XS);
    // Collect candidates: any clip that carries audio and is not this
    // clip. We do not filter by track — a narration on A2 can duck a
    // music clip on A1 just as easily.
    let mut candidates: Vec<(uuid::Uuid, String)> = Vec::new();
    for c in &state.last_project_clips {
        if c.id == clip.id {
            continue;
        }
        let carries_audio = match &c.clip_type {
            caprust_core::ClipType::Audio { .. } | caprust_core::ClipType::Narration { .. } => true,
            caprust_core::ClipType::Video { .. } => !c.audio_detached,
            _ => false,
        };
        if !carries_audio {
            continue;
        }
        // Human-readable label: name if set, else "<kind> clip",
        // followed by "@ MM:SS (duration)". Avoids the UUID soup the
        // old version showed when clips had no explicit name.
        let kind_label = match &c.clip_type {
            caprust_core::ClipType::Audio { .. } => "Audio",
            caprust_core::ClipType::Narration { .. } => "Narration",
            caprust_core::ClipType::Video { .. } => "Video",
            _ => "Clip",
        };
        let name = c.name.clone().unwrap_or_else(|| kind_label.to_string());
        let start_s = c.start_time_ms / 1000;
        let dur_s = c.duration_ms / 1000;
        let label = format!(
            "{} @ {:02}:{:02} ({:.1}s)",
            name,
            start_s / 60,
            start_s % 60,
            dur_s as f64,
        );
        candidates.push((c.id, label));
    }

    let current_label = clip
        .duck_against
        .and_then(|u| {
            candidates
                .iter()
                .find(|(id, _)| *id == u)
                .map(|(_, l)| l.clone())
        })
        .unwrap_or_else(|| tr("props-sound-duck-none"));

    ui.horizontal(|ui| {
        ui.label(tr("props-sound-duck-against"));
        egui::ComboBox::from_id_salt("duck_against_combo")
            .selected_text(current_label)
            .width(200.0) // font-family combo; component-specific
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(clip.duck_against.is_none(), tr("props-sound-duck-none"))
                    .clicked()
                {
                    state.pending.push(PendingEdit::DuckAgainst(None));
                }
                for (id, label) in &candidates {
                    let selected = clip.duck_against == Some(*id);
                    if ui.selectable_label(selected, label).clicked() && !selected {
                        state.pending.push(PendingEdit::DuckAgainst(Some(*id)));
                    }
                }
            });
    });
    ui.label(
        egui::RichText::new(tr("props-sound-duck-hint"))
            .small()
            .color(egui::Color32::from_gray(150)),
    );

    ui.add_space(space::M_PLUS);
    ui.separator();

    // ---- Volume automation (keyframes) ----
    ui.label(egui::RichText::new(tr("props-sound-automation")).strong());
    ui.add_space(space::XS);

    let mut kfs = clip.volume_keyframes.clone();
    let mut changed = false;
    let max_kfs = 20usize;

    if kfs.is_empty() {
        ui.label(
            egui::RichText::new(tr("props-sound-no-kf"))
                .italics()
                .color(egui::Color32::from_gray(150)),
        );
        ui.add_space(space::XS);
    } else {
        // Local table header
        egui::Grid::new("kf_header")
            .num_columns(4)
            .spacing([space::S, space::XS])
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(tr("props-sound-kf-time"))
                        .small()
                        .color(egui::Color32::from_gray(140)),
                );
                ui.label(
                    egui::RichText::new(tr("props-sound-kf-gain"))
                        .small()
                        .color(egui::Color32::from_gray(140)),
                );
                ui.label("");
                ui.label("");
                ui.end_row();
            });
    }

    let clip_dur = clip.duration_ms;
    let mut remove_idx: Option<usize> = None;
    egui::ScrollArea::vertical()
        .id_salt("kf_list")
        .max_height(180.0) // keyframe list cap; component-specific
        .show(ui, |ui| {
            egui::Grid::new("kf_rows")
                .num_columns(4)
                .spacing([space::S, space::XS])
                .show(ui, |ui| {
                    for (i, kf) in kfs.iter_mut().enumerate() {
                        let mut t = kf.t_ms as f64;
                        if ui
                            .add(
                                egui::DragValue::new(&mut t)
                                    .range(0.0..=(clip_dur as f64))
                                    .speed(10.0)
                                    .suffix(" ms"),
                            )
                            .changed()
                        {
                            kf.t_ms = t.max(0.0) as u64;
                            changed = true;
                        }
                        let mut g = kf.gain_db;
                        if ui
                            .add(
                                egui::DragValue::new(&mut g)
                                    .range(-40.0..=40.0)
                                    .speed(0.5)
                                    .suffix(" dB"),
                            )
                            .changed()
                        {
                            kf.gain_db = g;
                            changed = true;
                        }
                        if ui.small_button("X").clicked() {
                            remove_idx = Some(i);
                        }
                        ui.label(
                            egui::RichText::new(format!("#{}", i + 1))
                                .small()
                                .color(egui::Color32::from_gray(120)),
                        );
                        ui.end_row();
                    }
                });
        });

    if let Some(i) = remove_idx {
        if i < kfs.len() {
            kfs.remove(i);
            changed = true;
        }
    }

    ui.add_space(space::XS);
    let can_add = kfs.len() < max_kfs;
    let add_resp = ui.add_enabled(can_add, egui::Button::new(tr("props-sound-add-kf")));
    if add_resp.clicked() {
        // Place at the playhead relative to the clip's start, clamped
        // inside [0, duration]. Default gain = static volume_db so the
        // new point does not change the sound until the user edits it.
        let t = state
            .last_playhead_ms
            .saturating_sub(clip.start_time_ms)
            .min(clip.duration_ms);
        kfs.push(caprust_core::clip::VolumeKeyframe {
            t_ms: t,
            gain_db: clip.volume_db,
        });
        changed = true;
    }
    if !can_add {
        add_resp.on_hover_text(tr("props-sound-kf-limit"));
    }

    if changed {
        kfs.sort_by_key(|k| k.t_ms);
        state.pending.push(PendingEdit::VolumeKeyframes(kfs));
    }

    ui.add_space(space::M_PLUS);
    ui.separator();

    ui.label(egui::RichText::new(tr("props-sound-fade")).strong());
    ui.add_space(space::XS);
    // Model stores milliseconds; the slider works in seconds. Max is
    // the smaller of (clip duration, 5 s) so a long clip cannot
    // accept a fade that extends beyond its own length.
    let max_sec = (clip.duration_ms as f32 / 1000.0).min(5.0);
    let mut fade_in_s = clip.fade_in_ms as f32 / 1000.0;
    let mut fade_out_s = clip.fade_out_ms as f32 / 1000.0;
    egui::Grid::new("clip_fade_grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label(tr("props-sound-fade-in"));
            if ui
                .add(
                    egui::Slider::new(&mut fade_in_s, 0.0..=max_sec)
                        .suffix(" s")
                        .fixed_decimals(2),
                )
                .changed()
            {
                state.pending.push(PendingEdit::FadeInSec(fade_in_s));
            }
            ui.end_row();
            ui.label(tr("props-sound-fade-out"));
            if ui
                .add(
                    egui::Slider::new(&mut fade_out_s, 0.0..=max_sec)
                        .suffix(" s")
                        .fixed_decimals(2),
                )
                .changed()
            {
                state.pending.push(PendingEdit::FadeOutSec(fade_out_s));
            }
            ui.end_row();
        });

    ui.add_space(space::M_PLUS);
    ui.separator();
    ui.label(egui::RichText::new(tr("props-sound-processing")).strong());
    ui.add_space(space::XS);
    // Order matters in the render graph: denoise runs first (cleans
    // the signal), then voice boost (raises quiet parts), then
    // loudnorm last (targets a final integrated loudness).
    let mut norm = clip.audio_normalize;
    if ui
        .checkbox(&mut norm, tr("props-sound-normalize"))
        .changed()
    {
        state.pending.push(PendingEdit::AudioNormalize(norm));
    }
    let mut denoise = clip.audio_denoise;
    if ui
        .checkbox(&mut denoise, tr("props-sound-denoise"))
        .changed()
    {
        state.pending.push(PendingEdit::AudioDenoise(denoise));
    }
    let mut voice_boost = clip.audio_voice_boost;
    if ui
        .checkbox(&mut voice_boost, tr("props-sound-boost"))
        .changed()
    {
        state
            .pending
            .push(PendingEdit::AudioVoiceBoost(voice_boost));
    }
}

fn show_effects(ui: &mut Ui, clip: &Clip, state: &mut PropertiesState) {
    ui.label(egui::RichText::new(tr("props-tab-effects")).strong());
    ui.add_space(space::S);

    // --- Transitions ---
    ui.label(egui::RichText::new("Transitions").strong());
    egui::Grid::new("clip_transitions")
        .num_columns(3)
        .spacing([space::S, space::XS])
        .show(ui, |ui| {
            ui.label("In");
            ui.label(clip.transition_in.clone().unwrap_or_else(|| "—".into()));
            if clip.transition_in.is_some() && ui.small_button(ph::X).clicked() {
                state.pending.push(PendingEdit::ClearTransitionIn);
            }
            ui.end_row();

            // Easing row for the in transition. Visible only when
            // there is a transition, and only enabled for the
            // transitions that support easing (currently `fade`).
            if clip.transition_in.is_some() {
                ui.label(tr("props-transition-easing"));
                let supports_easing = clip.transition_in.as_deref() == Some("fade");
                ui.add_enabled_ui(supports_easing, |ui| {
                    egui::ComboBox::from_id_salt("trans_in_ease_combo")
                        .selected_text(ease_label(clip.transition_in_easing))
                        .width(140.0)
                        .show_ui(ui, |ui| {
                            for e in [
                                caprust_core::clip::EaseCurve::Linear,
                                caprust_core::clip::EaseCurve::EaseIn,
                                caprust_core::clip::EaseCurve::EaseOut,
                                caprust_core::clip::EaseCurve::EaseInOut,
                            ] {
                                let selected = clip.transition_in_easing == e;
                                if ui.selectable_label(selected, ease_label(e)).clicked()
                                    && !selected
                                {
                                    state.pending.push(PendingEdit::TransitionInEasing(e));
                                }
                            }
                        });
                });
                ui.label("");
                ui.end_row();

                // Shared duration slider on the in row. Applies to
                // both edges; not duplicated on the out side.
                ui.label(tr("props-transition-duration"));
                let mut dur = clip.transition_duration_ms;
                let resp = ui.add(
                    egui::Slider::new(&mut dur, 100..=3000)
                        .suffix(" ms")
                        .step_by(10.0),
                );
                if resp.changed() && dur != clip.transition_duration_ms {
                    state.pending.push(PendingEdit::TransitionDuration(dur));
                }
                ui.label("");
                ui.end_row();
            }

            ui.label("Out");
            ui.label(clip.transition_out.clone().unwrap_or_else(|| "—".into()));
            if clip.transition_out.is_some() && ui.small_button(ph::X).clicked() {
                state.pending.push(PendingEdit::ClearTransitionOut);
            }
            ui.end_row();

            if clip.transition_out.is_some() {
                ui.label(tr("props-transition-easing"));
                let supports_easing = clip.transition_out.as_deref() == Some("fade");
                ui.add_enabled_ui(supports_easing, |ui| {
                    egui::ComboBox::from_id_salt("trans_out_ease_combo")
                        .selected_text(ease_label(clip.transition_out_easing))
                        .width(140.0)
                        .show_ui(ui, |ui| {
                            for e in [
                                caprust_core::clip::EaseCurve::Linear,
                                caprust_core::clip::EaseCurve::EaseIn,
                                caprust_core::clip::EaseCurve::EaseOut,
                                caprust_core::clip::EaseCurve::EaseInOut,
                            ] {
                                let selected = clip.transition_out_easing == e;
                                if ui.selectable_label(selected, ease_label(e)).clicked()
                                    && !selected
                                {
                                    state.pending.push(PendingEdit::TransitionOutEasing(e));
                                }
                            }
                        });
                });
                ui.label("");
                ui.end_row();
            }
        });

    ui.add_space(space::M_PLUS);
    ui.separator();

    // --- Applied effects ---
    ui.label(egui::RichText::new("Applied Effects").strong());
    ui.add_space(space::XS);

    if clip.effects.is_empty() {
        ui.label(
            egui::RichText::new(tr("props-effects-empty"))
                .italics()
                .color(egui::Color32::from_gray(140)),
        );
    } else {
        let mut to_remove: Option<String> = None;
        for fx in &clip.effects {
            ui.horizontal(|ui| {
                ui.label(format!("{} {}", ph::SPARKLE, fx.effect_id));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(ph::X).clicked() {
                        to_remove = Some(fx.effect_id.clone());
                    }
                });
            });
        }
        if let Some(id) = to_remove {
            state.pending.push(PendingEdit::RemoveEffect(id));
        }
    }

    ui.add_space(space::M);
    ui.label(
        egui::RichText::new(tr("props-effects-hint"))
            .small()
            .color(egui::Color32::from_gray(160)),
    );
}

fn clip_kind_label(clip: &Clip) -> &'static str {
    match &clip.clip_type {
        ClipType::Video { .. } => "Video Clip",
        ClipType::Audio { .. } => "Audio Clip",
        ClipType::Image { .. } => "Image Clip",
        ClipType::TextOverlay { .. } => "Text Overlay",
        ClipType::Captions { .. } => "Captions",
        ClipType::Narration { .. } => "Narration",
    }
}

fn format_duration(ms: u64) -> String {
    let s = ms / 1000;
    let m = s / 60;
    let sec = s % 60;
    let millis = ms % 1000;
    format!("{m:02}:{sec:02}.{millis:03}")
}
