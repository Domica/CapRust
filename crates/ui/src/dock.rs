//! egui_dock layout for the editor.
//!
//! Replaces the fixed SidePanel/CentralPanel layout with a dock tree
//! the user can rearrange: tabs can be dragged between zones, zones
//! can be split horizontally or vertically, and the arrangement
//! persists in AppSettings (Session 4).
//!
//! During the migration (Session 2) every tab renders a placeholder.
//! Session 3 moves the real panels into `AppTabViewer::ui`.

use eframe::egui;
use egui_dock::{DockState, NodeIndex, TabViewer};

use crate::i18n_helper::tr;

/// One panel that can live anywhere in the dock tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Tab {
    // Asset browser tabs (six of them, all starting in the left zone).
    AssetMedia,
    AssetTransitions,
    AssetEffects,
    AssetFilters,
    AssetText,
    AssetTemplates,
    // Standalone editor panels.
    Preview,
    Properties,
    Timeline,
}

impl Tab {
    /// FTL key for the tab title.
    pub fn title_key(&self) -> &'static str {
        match self {
            Self::AssetMedia => "asset-tab-media",
            Self::AssetTransitions => "asset-tab-transitions",
            Self::AssetEffects => "asset-tab-effects",
            Self::AssetFilters => "asset-tab-filters",
            Self::AssetText => "asset-tab-text",
            Self::AssetTemplates => "asset-tab-templates",
            Self::Preview => "dock-tab-preview",
            Self::Properties => "props-heading",
            Self::Timeline => "dock-tab-timeline",
        }
    }
}

/// Starting layout: assets left, preview / timeline centre, properties
/// right. The user can move anything anywhere.
///
///   ┌───────┬──────────────────┬────────┐
///   │       │     Preview      │        │
///   │Assets ├──────────────────┤ Props  │
///   │       │     Timeline     │        │
///   └───────┴──────────────────┴────────┘
pub fn default_dock_state() -> DockState<Tab> {
    let mut state = DockState::new(vec![Tab::Preview]);
    let surface = state.main_surface_mut();

    // Root: [Preview]. Split right: [Preview] | [Properties].
    let [center, _right] = surface.split_right(NodeIndex::root(), 0.78, vec![Tab::Properties]);

    // Center: split vertically into Preview (top) + Timeline (bottom).
    let [center, _bottom] = surface.split_below(center, 0.62, vec![Tab::Timeline]);

    // Center: split left so Assets take the left zone.
    let [_left, _center] = surface.split_left(
        center,
        0.24,
        vec![
            Tab::AssetMedia,
            Tab::AssetTransitions,
            Tab::AssetEffects,
            Tab::AssetFilters,
            Tab::AssetText,
            Tab::AssetTemplates,
        ],
    );

    state
}

/// Bridge between egui_dock and `CapRustApp`. Session 3 will fill in
/// the real per-tab `ui()` calls; for now every tab renders a
/// placeholder so the layout itself can be tested.
pub struct AppTabViewer<'a> {
    pub app: &'a mut crate::app::CapRustApp,
}

impl<'a> TabViewer for AppTabViewer<'a> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        tr(tab.title_key()).into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            Tab::Preview => {
                self.app.render_preview_panel(ui);
            }
            // Session 3 will migrate the remaining tabs one at a time.
            _ => {
                let label = format!("[{} — migration in progress]", tr(tab.title_key()));
                ui.centered_and_justified(|ui| {
                    ui.label(egui::RichText::new(label).italics().weak());
                });
            }
        }
    }
}
