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

/// Alternative layouts available from View → Layout. The classic
/// one is `default_dock_state`; the others place Timeline at the
/// root level so it always spans the full window width and cannot
/// be dragged into a mid-column by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutPreset {
    Classic,
    WideTimeline,
    TimelineFocus,
    PreviewFocus,
}

impl LayoutPreset {
    pub fn all() -> [Self; 4] {
        [
            Self::Classic,
            Self::WideTimeline,
            Self::TimelineFocus,
            Self::PreviewFocus,
        ]
    }
    pub fn label_key(&self) -> &'static str {
        match self {
            Self::Classic => "menu-view-layout-classic",
            Self::WideTimeline => "menu-view-layout-wide-timeline",
            Self::TimelineFocus => "menu-view-layout-timeline-focus",
            Self::PreviewFocus => "menu-view-layout-preview-focus",
        }
    }
}

/// Build one of the alternative editor layouts.
///
/// All non-classic presets share this shape:
///
///   ┌─────────────────────────┬────────┐
///   │  Assets | Preview       │ Props  │
///   ├─────────────────────────┴────────┤
///   │  Timeline (full width)           │
///   └──────────────────────────────────┘
///
/// so Timeline is a root-level split and always spans the window.
pub fn preset_dock_state(preset: LayoutPreset) -> DockState<Tab> {
    if matches!(preset, LayoutPreset::Classic) {
        return default_dock_state();
    }

    let timeline_fraction = match preset {
        LayoutPreset::WideTimeline => 0.35,
        LayoutPreset::TimelineFocus => 0.50,
        LayoutPreset::PreviewFocus => 0.20,
        LayoutPreset::Classic => unreachable!(),
    };

    // Root: [top-row] over [Timeline]
    let mut state = DockState::new(vec![Tab::Preview]);
    let surface = state.main_surface_mut();
    let [top, _timeline] =
        surface.split_below(NodeIndex::root(), timeline_fraction, vec![Tab::Timeline]);

    // top-row: [assets | preview] | [props]
    let [center, _props] = surface.split_right(top, 0.78, vec![Tab::Properties]);

    // center: assets | preview
    let [_assets, _preview] = surface.split_left(
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
            Tab::Timeline => {
                self.app.render_timeline_panel(ui);
            }
            Tab::Properties => {
                self.app.render_properties_panel(ui);
            }
            Tab::AssetMedia => {
                self.app
                    .render_assets_panel(ui, crate::panels::asset_browser::AssetTab::Media);
            }
            Tab::AssetTransitions => {
                self.app
                    .render_assets_panel(ui, crate::panels::asset_browser::AssetTab::Transitions);
            }
            Tab::AssetEffects => {
                self.app
                    .render_assets_panel(ui, crate::panels::asset_browser::AssetTab::Effects);
            }
            Tab::AssetFilters => {
                self.app
                    .render_assets_panel(ui, crate::panels::asset_browser::AssetTab::Filters);
            }
            Tab::AssetText => {
                self.app
                    .render_assets_panel(ui, crate::panels::asset_browser::AssetTab::Text);
            }
            Tab::AssetTemplates => {
                self.app
                    .render_assets_panel(ui, crate::panels::asset_browser::AssetTab::Templates);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Material-style egui_dock theme
// ---------------------------------------------------------------------------

/// Build an egui_dock [`Style`] that matches the app's Material-ish
/// design language: no full-width tab bar fill, accent-tinted active
/// tab, 1px hairline under the active label, transparent hover with
/// a subtle accent wash. Colors come from [`Theme`], so switching to
/// light mode or changing the accent updates the dock too.
pub fn material_style(ctx: &egui::Context, theme: &crate::theme::Theme) -> egui_dock::Style {
    let mut style = egui_dock::Style::from_egui(ctx.style().as_ref());
    let visuals = ctx.style().visuals.clone();
    let accent = theme.accent_color();
    let panel_fill = visuals.panel_fill;
    let hairline = egui::Color32::from_gray(58);
    let inactive_text = egui::Color32::from_gray(160);
    let hover_text = egui::Color32::from_gray(225);

    // Outer surface: slight rounding, 1px hairline border.
    style.main_surface_border_rounding = egui::CornerRadius::same(6);
    style.main_surface_border_stroke = egui::Stroke::new(1.0_f32, hairline);
    style.dock_area_padding = None;

    // Tab bar: matches the panel background, no fill, 1px bottom
    // hairline colour to separate tab strip from body.
    style.tab_bar.bg_fill = panel_fill;
    style.tab_bar.height = 30.0;
    style.tab_bar.corner_radius = egui::CornerRadius::ZERO;
    style.tab_bar.hline_color = hairline;
    style.tab_bar.fill_tab_bar = false;
    style.tab_bar.show_scroll_bar_on_overflow = false;

    // Active tab: accent wash background, accent text.
    style.tab.active.bg_fill = accent.gamma_multiply(0.22);
    style.tab.active.text_color = accent;
    style.tab.active.outline_color = egui::Color32::TRANSPARENT;
    style.tab.active_with_kb_focus = style.tab.active.clone();

    // Inactive tab: transparent background, muted text.
    style.tab.inactive.bg_fill = egui::Color32::TRANSPARENT;
    style.tab.inactive.text_color = inactive_text;
    style.tab.inactive.outline_color = egui::Color32::TRANSPARENT;

    // Hover: light accent wash, brighter text.
    style.tab.hovered.bg_fill = accent.gamma_multiply(0.10);
    style.tab.hovered.text_color = hover_text;
    style.tab.hovered.outline_color = egui::Color32::TRANSPARENT;

    // Focused (but not active): same as inactive for the moment,
    // keyboard focus will not draw a distracting ring on the tab.
    let inactive_snapshot = style.tab.inactive.clone();
    style.tab.focused = inactive_snapshot.clone();
    style.tab.focused_with_kb_focus = inactive_snapshot.clone();
    style.tab.inactive_with_kb_focus = inactive_snapshot;

    // Tab body: same panel background as the rest of the editor.
    style.tab.tab_body.bg_fill = panel_fill;
    style.tab.tab_body.stroke = egui::Stroke::NONE;

    // Close button: subtle grey, brightens on hover of the tab.
    style.buttons.close_tab_color = egui::Color32::from_gray(150);
    style.buttons.close_tab_active_color = egui::Color32::from_gray(230);
    style.buttons.close_tab_bg_fill = egui::Color32::TRANSPARENT;

    style
}
