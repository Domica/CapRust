//! Media bin: dense grid, thumbnail cache, dynamic columns based on panel width.

use caprust_core::media::{guess_kind, AUDIO_EXTS, IMAGE_EXTS, VIDEO_EXTS};
use caprust_core::{MediaItem, MediaKind, ProjectState};
use egui::{Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Sort + size
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSort {
    Added,
    Name,
    Type,
}

impl MediaSort {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Added => "Added",
            Self::Name => "Name",
            Self::Type => "Type",
        }
    }
    pub fn all() -> [Self; 3] {
        [Self::Added, Self::Name, Self::Type]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewSize {
    Small,
    Medium,
    Large,
}

impl PreviewSize {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Small => "S",
            Self::Medium => "M",
            Self::Large => "L",
        }
    }
    /// Card width (thumbnail width).
    pub fn card_w(&self) -> f32 {
        match self {
            Self::Small => 84.0,
            Self::Medium => 118.0,
            Self::Large => 168.0,
        }
    }
    /// Thumbnail aspect height, fixed 4:3 for now.
    pub fn thumb_h(&self) -> f32 {
        self.card_w() * 3.0 / 4.0
    }
}

// ---------------------------------------------------------------------------
// Thumbnail cache
// ---------------------------------------------------------------------------

/// Stable color derived from the file path, so each clip gets a consistent
/// placeholder even across restarts. This is the placeholder layer that will
/// be replaced by real JPEG thumbnails read from the project cache dir
/// (see `caprust_core::cache`). For now, it also precomputes nothing heavy.
#[derive(Debug, Default)]
pub struct ThumbnailCache {
    colors: HashMap<Uuid, Color32>,
    // Later: HashMap<Uuid, egui::TextureHandle> for real JPEG thumbnails.
}

impl ThumbnailCache {
    pub fn color_for(&mut self, item: &MediaItem) -> Color32 {
        if let Some(c) = self.colors.get(&item.id) {
            return *c;
        }
        let c = color_from_path(&item.path);
        self.colors.insert(item.id, c);
        c
    }
    pub fn invalidate(&mut self, id: Uuid) {
        self.colors.remove(&id);
    }
}

fn color_from_path(path: &str) -> Color32 {
    // FNV-1a hash → hue
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in path.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let h = (hash % 360) as f32;
    let s = 0.42;
    let v = 0.55;
    hsl_to_rgb(h, s, v)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> Color32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hp as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    Color32::from_rgb(
        ((r1 + m) * 255.0) as u8,
        ((g1 + m) * 255.0) as u8,
        ((b1 + m) * 255.0) as u8,
    )
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct MediaBinState {
    pub sort: MediaSort,
    pub preview: PreviewSize,
    pub thumb_cache: ThumbnailCache,
}

impl Default for MediaBinState {
    fn default() -> Self {
        Self {
            sort: MediaSort::Added,
            preview: PreviewSize::Medium,
            thumb_cache: ThumbnailCache::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

pub fn show(ui: &mut Ui, project: &mut ProjectState, state: &mut MediaBinState) -> Option<Uuid> {
    // --- Import buttons ---
    ui.horizontal(|ui| {
        if ui.button("📥 Clips").clicked() {
            import_with(project, VIDEO_EXTS, "Video");
        }
        if ui.button("🎵 Music").clicked() {
            import_with(project, AUDIO_EXTS, "Audio");
        }
        if ui.button("🖼 Images").clicked() {
            import_with(project, IMAGE_EXTS, "Image");
        }
    });

    ui.separator();

    // --- Sort + size controls ---
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("media_sort")
            .selected_text(format!("Sort: {}", state.sort.label()))
            .width(110.0)
            .show_ui(ui, |ui| {
                for s in MediaSort::all() {
                    ui.selectable_value(&mut state.sort, s, s.label());
                }
            });
        ui.separator();
        ui.label("Size");
        for sz in [PreviewSize::Small, PreviewSize::Medium, PreviewSize::Large] {
            ui.selectable_value(&mut state.preview, sz, sz.label());
        }
    });

    ui.separator();

    // --- Sorted item list ---
    let mut sorted: Vec<MediaItem> = project.media.items.clone();
    match state.sort {
        MediaSort::Added => sorted.sort_by_key(|m| m.added_at),
        MediaSort::Name => sorted.sort_by_key(|m| a_lower(&m.name)),
        MediaSort::Type => sorted.sort_by(|a, b| {
            kind_rank(a.kind)
                .cmp(&kind_rank(b.kind))
                .then_with(|| a.name.cmp(&b.name))
        }),
    }

    if sorted.is_empty() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("No media imported yet.")
                    .italics()
                    .color(Color32::from_gray(120)),
            );
            ui.label(
                RichText::new("Click 📥 Clips / 🎵 Music / 🖼 Images above.")
                    .small()
                    .color(Color32::from_gray(90)),
            );
        });
        return None;
    }

    // --- Dynamic column layout ---
    let card_w = state.preview.card_w();
    let thumb_h = state.preview.thumb_h();
    let h_gap = 6.0;
    let v_gap = 6.0;
    let scrollbar_reserve = 16.0;
    let avail = (ui.available_width() - scrollbar_reserve).max(card_w);

    // number of columns that fit; at least 1
    let cols = (((avail + h_gap) / (card_w + h_gap)).floor() as usize).max(1);

    let mut dragging: Option<Uuid> = None;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(h_gap, v_gap);

            for row in sorted.chunks(cols) {
                ui.horizontal(|ui| {
                    for item in row {
                        let resp = draw_card(ui, item, card_w, thumb_h, &mut state.thumb_cache);
                        if resp.drag_started() {
                            dragging = Some(item.id);
                        }
                    }
                });
            }
        });

    dragging
}

fn a_lower(s: &str) -> String {
    s.to_lowercase()
}

fn kind_rank(k: MediaKind) -> u8 {
    match k {
        MediaKind::Video => 0,
        MediaKind::Audio => 1,
        MediaKind::Image => 2,
    }
}

// ---------------------------------------------------------------------------
// Card
// ---------------------------------------------------------------------------

fn draw_card(
    ui: &mut Ui,
    item: &MediaItem,
    card_w: f32,
    thumb_h: f32,
    cache: &mut ThumbnailCache,
) -> egui::Response {
    let id = egui::Id::new(item.id);
    let payload = item.id;

    let inner = ui.dnd_drag_source(id, payload, |ui| {
        // Reserve card rect (thumbnail + filename line)
        let line_h = 14.0;
        let total = Vec2::new(card_w, thumb_h + 4.0 + line_h);
        let (rect, resp) = ui.allocate_exact_size(total, Sense::click_and_drag());

        // Thumbnail rect
        let thumb_rect = Rect::from_min_size(rect.min, Vec2::new(card_w, thumb_h));

        // Base color (per-item placeholder)
        let base = cache.color_for(item);
        ui.painter().rect_filled(thumb_rect, 4.0, base);

        // Slight top-to-bottom darkening for depth
        let dark = Color32::from_black_alpha(40);
        let grad_rect = Rect::from_min_max(
            Pos2::new(thumb_rect.left(), thumb_rect.bottom() - thumb_h * 0.35),
            thumb_rect.max,
        );
        ui.painter().rect_filled(grad_rect, 4.0, dark);

        // Kind icon, centered, big
        let icon_size = (thumb_h * 0.42).clamp(16.0, 40.0);
        ui.painter().text(
            thumb_rect.center(),
            egui::Align2::CENTER_CENTER,
            item.kind.icon(),
            FontId::proportional(icon_size),
            Color32::from_white_alpha(230),
        );

        // Border
        let border = if resp.hovered() {
            Color32::from_white_alpha(120)
        } else {
            Color32::from_gray(60)
        };
        ui.painter().rect_stroke(
            thumb_rect,
            4.0,
            Stroke::new(1.0_f32, border),
            egui::StrokeKind::Inside,
        );

        // Duration badge (bottom-right) if known
        if item.duration_ms > 0 {
            let label = format_ms(item.duration_ms);
            let text_pos = thumb_rect.right_bottom() - Vec2::new(4.0, 3.0);
            let font = FontId::monospace(10.0);
            let galley = ui
                .painter()
                .layout_no_wrap(label.clone(), font.clone(), Color32::WHITE);
            let bg_rect = Rect::from_min_size(
                Pos2::new(
                    text_pos.x - galley.size().x - 6.0,
                    text_pos.y - galley.size().y - 3.0,
                ),
                Vec2::new(galley.size().x + 6.0, galley.size().y + 4.0),
            );
            ui.painter()
                .rect_filled(bg_rect, 3.0, Color32::from_black_alpha(150));
            ui.painter().galley(
                Pos2::new(bg_rect.left() + 3.0, bg_rect.top() + 2.0),
                galley,
                Color32::WHITE,
            );
        }

        // Filename line
        let name_pos = Pos2::new(thumb_rect.left(), thumb_rect.bottom() + 3.0);
        let max_chars = (card_w / 6.5) as usize;
        let name = truncate(&item.name, max_chars.max(6));
        ui.painter().text(
            name_pos,
            egui::Align2::LEFT_TOP,
            name,
            FontId::proportional(11.0),
            Color32::from_gray(210),
        );

        resp.on_hover_text(format!(
            "{}\n{} · {} ms\n{}",
            item.path,
            item.kind.label(),
            item.duration_ms,
            human_size(item.size_bytes),
        ))
    });

    inner.inner
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn format_ms(ms: u64) -> String {
    let s = ms / 1000;
    let m = s / 60;
    let sec = s % 60;
    format!("{:02}:{:02}", m, sec)
}

fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

// ---------------------------------------------------------------------------
// Import helpers
// ---------------------------------------------------------------------------

fn import_with(project: &mut ProjectState, exts: &[&str], label: &str) {
    if let Some(paths) = rfd::FileDialog::new()
        .add_filter(label, exts)
        .add_filter("All files", &["*"])
        .pick_files()
    {
        for p in paths {
            let s = p.to_string_lossy().to_string();
            if let Some(kind) = guess_kind(&s) {
                project.media.add(&s, kind);
            } else {
                // fallback: guess by extension family
                let kind = guess_kind(&s).unwrap_or(MediaKind::Video);
                project.media.add(&s, kind);
            }
        }
    }
}
