//! First-run FFmpeg prompt. Shown when no ffmpeg/ffprobe is found on
//! the system and the user has not dismissed the prompt before.
//!
//! Three actions:
//!   1. Download the BtbN GPL build into a managed folder (user picks
//!      where; defaults to `%APPDATA%/CapRust/ffmpeg`).
//!   2. Point CapRust at an existing ffmpeg.exe (opens a file picker).
//!   3. Cancel — suppresses the modal for this session and, if the
//!      user does not want to see it again, sets
//!      `AppSettings.ffmpeg_prompt_dismissed`.

use std::sync::mpsc::Receiver;

use caprust_core::ffmpeg::FfmpegDownloadEvent;
use caprust_core::AppSettings;
use egui::Ui;

use crate::i18n_helper::tr;
use crate::theme::tokens::space;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptPhase {
    /// Waiting on the user's choice.
    Choose,
    /// Download in flight; `done`/`total` bytes so far.
    Downloading { done: u64, total: Option<u64> },
    /// Archive downloaded, extraction running.
    Extracting,
    /// Extraction done; caller is responsible for re-running detection.
    Done,
    /// Human-readable error. User can retry or cancel.
    Failed,
}

pub struct FfmpegPromptState {
    /// Where the managed build will land. Editable in Choose phase.
    pub install_dir: String,
    pub phase: PromptPhase,
    /// Set by the app when a download is spawned; cleared on Done/Failed.
    pub receiver: Option<Receiver<FfmpegDownloadEvent>>,
    /// Set by the user. Caller persists it to AppSettings on close.
    pub dont_ask_again: bool,
}

impl Default for FfmpegPromptState {
    fn default() -> Self {
        Self {
            install_dir: String::new(),
            phase: PromptPhase::Choose,
            receiver: None,
            dont_ask_again: false,
        }
    }
}

/// Actions the caller (app.rs) must perform because it owns the
/// project, the settings, and the ability to spawn threads.
#[derive(Debug, Default)]
pub struct FfmpegPromptEvents {
    pub close: bool,
    /// Spawn a download into `state.install_dir`.
    pub start_download: bool,
    /// Open a file picker and set `settings.ffmpeg_path` /
    /// `settings.ffprobe_path` from the chosen directory.
    pub browse_existing: bool,
}

/// Poll the receiver the app stored in `state.receiver`. Returns
/// `Some(done_or_failed)` when the download has terminated.
pub fn poll(state: &mut FfmpegPromptState) -> Option<PromptPhase> {
    let rx = state.receiver.as_ref()?;
    let mut terminal: Option<PromptPhase> = None;
    loop {
        match rx.try_recv() {
            Ok(ev) => match ev {
                FfmpegDownloadEvent::Progress { done, total } => {
                    state.phase = PromptPhase::Downloading { done, total };
                }
                FfmpegDownloadEvent::Extracting => {
                    state.phase = PromptPhase::Extracting;
                }
                FfmpegDownloadEvent::Done => {
                    terminal = Some(PromptPhase::Done);
                    break;
                }
                FfmpegDownloadEvent::Failed(e) => {
                    terminal = Some(PromptPhase::Failed);
                    // Stash the message on the receiver side of state.
                    state.phase = PromptPhase::Failed;
                    tracing::warn!("ffmpeg download failed: {e}");
                    // Also log the string so the user can see it in the log.
                    // We do not store it in `phase` to keep `Copy`; the
                    // modal shows a generic message and the log has details.
                }
            },
            Err(std::sync::mpsc::TryRecvError::Empty) => break,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // Channel dropped without a terminal event.
                if !matches!(state.phase, PromptPhase::Done | PromptPhase::Failed) {
                    terminal = Some(PromptPhase::Failed);
                }
                break;
            }
        }
    }
    if terminal.is_some() {
        state.receiver = None;
    }
    terminal.inspect(|&t| {
        state.phase = t;
    })
}

/// Render the modal body. `ui` should already be inside an
/// `egui::Window` positioned by the caller.
pub fn show(
    ui: &mut Ui,
    state: &mut FfmpegPromptState,
    settings: &mut AppSettings,
) -> FfmpegPromptEvents {
    let mut ev = FfmpegPromptEvents::default();

    match state.phase {
        PromptPhase::Choose => {
            ui.label(egui::RichText::new(tr("ffmpeg-prompt-body")).size(12.5));
            ui.add_space(space::S);

            ui.label(egui::RichText::new(tr("ffmpeg-prompt-install-dir")).strong());
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut state.install_dir)
                        .desired_width(340.0)
                        .hint_text("%APPDATA%\\CapRust\\ffmpeg"),
                );
                if ui.button(tr("new-button-browse")).clicked() {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        state.install_dir = dir.to_string_lossy().to_string();
                    }
                }
            });
            ui.label(
                egui::RichText::new(tr("ffmpeg-prompt-install-dir-hint"))
                    .small()
                    .color(egui::Color32::from_gray(150)),
            );

            ui.add_space(space::M_PLUS);
            ui.horizontal(|ui| {
                if ui
                    .button(tr("ffmpeg-prompt-download"))
                    .on_hover_text(tr("ffmpeg-prompt-download-hint"))
                    .clicked()
                {
                    ev.start_download = true;
                }
                if ui
                    .button(tr("ffmpeg-prompt-browse"))
                    .on_hover_text(tr("ffmpeg-prompt-browse-hint"))
                    .clicked()
                {
                    ev.browse_existing = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tr("ffmpeg-prompt-cancel")).clicked() {
                        ev.close = true;
                    }
                });
            });

            ui.add_space(space::XS);
            ui.checkbox(&mut state.dont_ask_again, tr("ffmpeg-prompt-dont-ask"));
        }
        PromptPhase::Downloading { done, total } => {
            ui.label(egui::RichText::new(tr("ffmpeg-prompt-downloading")).strong());
            ui.add_space(space::XS);
            let frac = total
                .map(|t| (done as f32 / t.max(1) as f32).clamp(0.0, 1.0))
                .unwrap_or(0.0);
            ui.add(
                egui::ProgressBar::new(frac)
                    .show_percentage()
                    .desired_width(380.0),
            );
            let label = match total {
                Some(t) => format!("{} / {} MiB", done / 1_048_576, t / 1_048_576),
                None => format!("{} MiB", done / 1_048_576),
            };
            ui.label(
                egui::RichText::new(label)
                    .small()
                    .color(egui::Color32::from_gray(150)),
            );
        }
        PromptPhase::Extracting => {
            ui.label(egui::RichText::new(tr("ffmpeg-prompt-extracting")).strong());
            ui.add_space(space::XS);
            ui.add(
                egui::ProgressBar::new(0.0)
                    .animate(true)
                    .desired_width(380.0),
            );
        }
        PromptPhase::Done => {
            ui.label(
                egui::RichText::new(tr("ffmpeg-prompt-done"))
                    .strong()
                    .color(egui::Color32::from_rgb(120, 200, 120)),
            );
            ui.add_space(space::S);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tr("ffmpeg-prompt-close")).clicked() {
                        ev.close = true;
                    }
                });
            });
        }
        PromptPhase::Failed => {
            ui.label(
                egui::RichText::new(tr("ffmpeg-prompt-failed"))
                    .strong()
                    .color(egui::Color32::from_rgb(220, 120, 120)),
            );
            ui.label(
                egui::RichText::new(tr("ffmpeg-prompt-failed-hint"))
                    .small()
                    .color(egui::Color32::from_gray(150)),
            );
            ui.add_space(space::S);
            ui.horizontal(|ui| {
                if ui.button(tr("ffmpeg-prompt-retry")).clicked() {
                    state.phase = PromptPhase::Choose;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tr("ffmpeg-prompt-cancel")).clicked() {
                        ev.close = true;
                    }
                });
            });
        }
    }

    // Suppress unused warnings when settings is not read in this
    // version of the modal — the caller uses it after Done.
    let _ = settings;

    ev
}
