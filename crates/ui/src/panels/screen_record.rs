//! Screen recording modal (Faza Q). Windows only.

use crate::i18n_helper::tr;
use caprust_screen_record::MonitorInfo;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Instant;

#[derive(Default)]
pub struct ScreenRecordState {
    pub monitors: Vec<MonitorInfo>,
    pub monitors_loaded: bool,
    pub selected_monitor: usize,
    pub duration_sec: u32,
    pub fps: u32,
    pub in_progress: bool,
    pub started_at: Option<Instant>,
    pub stop_flag: Option<Arc<AtomicBool>>,
    pub result_rx: Option<Receiver<Result<PathBuf, String>>>,
    pub error: Option<String>,
}

impl ScreenRecordState {
    pub fn new_defaults() -> Self {
        Self {
            duration_sec: 5,
            fps: 30,
            ..Default::default()
        }
    }
}

pub enum ScreenRecordAction {
    None,
    Start {
        monitor: usize,
        duration_sec: u32,
        fps: u32,
    },
    Stop,
    Close,
}

pub fn show_modal(
    ctx: &egui::Context,
    state: &mut ScreenRecordState,
    open: &mut bool,
) -> ScreenRecordAction {
    let mut action = ScreenRecordAction::None;
    let mut local_open = *open;
    egui::Window::new(tr("screen-record-title"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut local_open)
        .show(ctx, |ui| {
            ui.set_min_width(360.0);
            if state.in_progress {
                show_in_progress(ui, state, &mut action);
                return;
            }
            if !state.monitors_loaded {
                ui.label(tr("screen-record-loading"));
                return;
            }
            if state.monitors.is_empty() {
                ui.label(tr("screen-record-no-monitors"));
                if ui.button(tr("screen-record-close")).clicked() {
                    action = ScreenRecordAction::Close;
                }
                return;
            }

            // Monitor picker
            ui.horizontal(|ui| {
                ui.label(tr("screen-record-monitor"));
                let current = state
                    .monitors
                    .get(state.selected_monitor)
                    .map(|m| format!("{} ({}x{})", m.name, m.width, m.height))
                    .unwrap_or_default();
                egui::ComboBox::from_id_salt("screen_record_monitor")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for m in &state.monitors {
                            let label = format!("{} ({}x{})", m.name, m.width, m.height);
                            ui.selectable_value(&mut state.selected_monitor, m.id, label);
                        }
                    });
            });

            ui.horizontal(|ui| {
                ui.label(tr("screen-record-duration"));
                ui.add(
                    egui::DragValue::new(&mut state.duration_sec)
                        .range(1..=600)
                        .speed(1.0),
                );
            });

            ui.horizontal(|ui| {
                ui.label(tr("screen-record-fps"));
                ui.add(
                    egui::DragValue::new(&mut state.fps)
                        .range(15..=60)
                        .speed(1.0),
                );
            });

            ui.add_space(8.0);
            ui.separator();

            if let Some(err) = &state.error {
                ui.colored_label(egui::Color32::from_rgb(220, 80, 80), err);
                ui.add_space(4.0);
            }

            ui.horizontal(|ui| {
                if ui.button(tr("screen-record-start")).clicked() {
                    action = ScreenRecordAction::Start {
                        monitor: state.selected_monitor,
                        duration_sec: state.duration_sec,
                        fps: state.fps,
                    };
                }
                if ui.button(tr("screen-record-close")).clicked() {
                    action = ScreenRecordAction::Close;
                }
            });
        });
    *open = local_open;
    action
}

fn show_in_progress(ui: &mut egui::Ui, state: &ScreenRecordState, action: &mut ScreenRecordAction) {
    let elapsed = state
        .started_at
        .map(|t| t.elapsed().as_secs_f32())
        .unwrap_or(0.0);
    let total = state.duration_sec as f32;
    let frac = (elapsed / total).clamp(0.0, 1.0);
    ui.label(tr("screen-record-recording"));
    ui.add(egui::ProgressBar::new(frac).show_percentage());
    ui.label(format!("{:.1} / {} s", elapsed, state.duration_sec));
    ui.add_space(8.0);
    if ui.button(tr("screen-record-stop")).clicked() {
        *action = ScreenRecordAction::Stop;
    }
}
