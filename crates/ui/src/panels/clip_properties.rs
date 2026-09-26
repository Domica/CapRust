//! Clip properties panel — Video / Sound / Effects tabs.

use crate::i18n_helper::tr;
use caprust_core::{Clip, ClipType, ProjectState};
use egui::Ui;
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
    /// Last playhead value the panel saw, so "+ Add at playhead" knows
    /// where to drop a new keyframe.
    pub last_playhead_ms: u64,
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
    /// Replace the whole volume automation curve on a clip.
    VolumeKeyframes(Vec<caprust_core::clip::VolumeKeyframe>),
}

pub fn show(
    ui: &mut Ui,
    project: &ProjectState,
    selected: Option<Uuid>,
    state: &mut PropertiesState,
    playhead_ms: u64,
) {
    state.last_playhead_ms = playhead_ms;
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
                .width(180.0)
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
    } = &clip.clip_type
    {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Model")
                    .small()
                    .color(egui::Color32::from_gray(140)),
            );
            ui.label(egui::RichText::new(model_id).monospace().size(11.0));
            ui.separator();
            ui.label(
                egui::RichText::new("Language")
                    .small()
                    .color(egui::Color32::from_gray(140)),
            );
            ui.label(egui::RichText::new(language).monospace().size(11.0));
        });

        let n = segments.len();
        ui.add_space(2.0);
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
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);

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

                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);
                }
            });

        state.pending.extend(local_edits);
        return;
    }

    // --- Tabs ---
    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.tab, PropertiesTab::Video, tr("props-tab-video"));
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
            PropertiesTab::Video => show_video(ui, clip, state),
            PropertiesTab::Sound => show_sound(ui, clip, state),
            PropertiesTab::Effects => show_effects(ui, clip, state),
        });
}

fn show_video(ui: &mut Ui, clip: &Clip, state: &mut PropertiesState) {
    // --- Main ---
    ui.label(egui::RichText::new(tr("props-video-main")).strong());
    ui.add_space(4.0);
    egui::Grid::new("clip_main_grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label(tr("props-field-rotation"));
            let mut rot = 0.0_f32;
            ui.add(
                egui::DragValue::new(&mut rot)
                    .range(-180.0..=180.0)
                    .suffix("°"),
            );
            ui.end_row();

            ui.label(tr("props-field-width"));
            let mut w = 100.0_f32;
            ui.add(
                egui::DragValue::new(&mut w)
                    .range(10.0..=400.0)
                    .suffix(" %"),
            );
            ui.end_row();

            ui.label(tr("props-field-height"));
            let mut h = 100.0_f32;
            ui.add(
                egui::DragValue::new(&mut h)
                    .range(10.0..=400.0)
                    .suffix(" %"),
            );
            ui.end_row();
        });

    ui.add_space(10.0);
    ui.separator();

    // --- Speed ---
    ui.label(egui::RichText::new(tr("props-video-speed")).strong());
    ui.add_space(4.0);
    egui::Grid::new("clip_speed_grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label(tr("props-field-speed"));
            let mut speed = clip.speed;
            let combo = egui::ComboBox::from_id_salt("clip_speed")
                .selected_text(format!("{speed:.2}×"))
                .width(120.0);
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

    ui.add_space(10.0);
    ui.separator();

    // --- Mirror ---
    ui.label(egui::RichText::new(tr("props-video-mirror")).strong());
    ui.add_space(4.0);
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

    ui.add_space(10.0);
    ui.separator();

    // --- Trim (read-only info) ---
    ui.label(egui::RichText::new(tr("props-video-trim")).strong());
    ui.add_space(4.0);
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
}

fn show_sound(ui: &mut Ui, clip: &Clip, state: &mut PropertiesState) {
    ui.label(egui::RichText::new(tr("props-sound-volume")).strong());
    ui.add_space(4.0);
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

    ui.add_space(10.0);
    ui.separator();

    // ---- Volume automation (keyframes) ----
    ui.label(egui::RichText::new(tr("props-sound-automation")).strong());
    ui.add_space(4.0);

    let mut kfs = clip.volume_keyframes.clone();
    let mut changed = false;
    let max_kfs = 20usize;

    if kfs.is_empty() {
        ui.label(
            egui::RichText::new(tr("props-sound-no-kf"))
                .italics()
                .color(egui::Color32::from_gray(150)),
        );
        ui.add_space(4.0);
    } else {
        // Local table header
        egui::Grid::new("kf_header")
            .num_columns(4)
            .spacing([6.0, 4.0])
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
        .max_height(180.0)
        .show(ui, |ui| {
            egui::Grid::new("kf_rows")
                .num_columns(4)
                .spacing([6.0, 4.0])
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

    ui.add_space(4.0);
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

    ui.add_space(10.0);
    ui.separator();

    ui.label(egui::RichText::new(tr("props-sound-fade")).strong());
    ui.add_space(4.0);
    let mut fade_in = 0.0_f32;
    let mut fade_out = 0.0_f32;
    egui::Grid::new("clip_fade_grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label(tr("props-sound-fade-in"));
            ui.add(
                egui::Slider::new(&mut fade_in, 0.0..=50.0)
                    .suffix(" %")
                    .show_value(true),
            );
            ui.end_row();
            ui.label(tr("props-sound-fade-out"));
            ui.add(
                egui::Slider::new(&mut fade_out, 0.0..=50.0)
                    .suffix(" %")
                    .show_value(true),
            );
            ui.end_row();
        });

    ui.add_space(10.0);
    ui.separator();
    ui.label(egui::RichText::new(tr("props-sound-processing")).strong());
    ui.add_space(4.0);
    let mut norm = false;
    let mut denoise = false;
    let mut voice_boost = false;
    ui.checkbox(&mut norm, tr("props-sound-normalize"));
    ui.checkbox(&mut denoise, tr("props-sound-denoise"));
    ui.checkbox(&mut voice_boost, tr("props-sound-boost"));
    ui.label(
        egui::RichText::new(tr("props-sound-wip"))
            .small()
            .italics()
            .color(egui::Color32::from_gray(140)),
    );
}

fn show_effects(ui: &mut Ui, clip: &Clip, state: &mut PropertiesState) {
    ui.label(egui::RichText::new(tr("props-tab-effects")).strong());
    ui.add_space(6.0);

    // --- Transitions ---
    ui.label(egui::RichText::new("Transitions").strong());
    egui::Grid::new("clip_transitions")
        .num_columns(3)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            ui.label("In");
            ui.label(clip.transition_in.clone().unwrap_or_else(|| "—".into()));
            if clip.transition_in.is_some() && ui.small_button("✕").clicked() {
                state.pending.push(PendingEdit::ClearTransitionIn);
            }
            ui.end_row();

            ui.label("Out");
            ui.label(clip.transition_out.clone().unwrap_or_else(|| "—".into()));
            if clip.transition_out.is_some() && ui.small_button("✕").clicked() {
                state.pending.push(PendingEdit::ClearTransitionOut);
            }
            ui.end_row();
        });

    ui.add_space(10.0);
    ui.separator();

    // --- Applied effects ---
    ui.label(egui::RichText::new("Applied Effects").strong());
    ui.add_space(4.0);

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
                ui.label(format!("✨ {}", fx.effect_id));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").clicked() {
                        to_remove = Some(fx.effect_id.clone());
                    }
                });
            });
        }
        if let Some(id) = to_remove {
            state.pending.push(PendingEdit::RemoveEffect(id));
        }
    }

    ui.add_space(8.0);
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
