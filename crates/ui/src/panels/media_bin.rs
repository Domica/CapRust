//! Media bin: dense grid, thumbnail cache, dynamic columns based on panel width.

use crate::i18n_helper::tr;
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
    pub fn key(&self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Name => "name",
            Self::Type => "type",
        }
    }
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
pub enum SortDirection {
    Ascending,
    Descending,
}

impl SortDirection {
    pub fn flipped(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Self::Ascending => egui_phosphor::regular::SORT_ASCENDING,
            Self::Descending => egui_phosphor::regular::SORT_DESCENDING,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Ascending => "asc",
            Self::Descending => "desc",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaFilter {
    All,
    Video,
    Audio,
    Image,
}

impl MediaFilter {
    pub fn key(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Image => "image",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Image => "Image",
        }
    }
    pub fn all() -> [Self; 4] {
        [Self::All, Self::Video, Self::Audio, Self::Image]
    }
    pub fn matches(&self, k: MediaKind) -> bool {
        match self {
            Self::All => true,
            Self::Video => k == MediaKind::Video,
            Self::Audio => k == MediaKind::Audio,
            Self::Image => k == MediaKind::Image,
        }
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
#[derive(Default)]
pub struct ThumbnailCache {
    colors: HashMap<Uuid, Color32>,
    textures: HashMap<Uuid, egui::TextureHandle>,
}

impl std::fmt::Debug for ThumbnailCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThumbnailCache")
            .field("colors", &self.colors.len())
            .field("textures", &self.textures.len())
            .finish()
    }
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

    /// Returns the thumbnail texture if a JPEG exists on disk, else None.
    pub fn texture_for(
        &mut self,
        ctx: &egui::Context,
        project_path: Option<&str>,
        item: &MediaItem,
    ) -> Option<egui::TextureHandle> {
        if let Some(t) = self.textures.get(&item.id) {
            return Some(t.clone());
        }

        let project_path = project_path?;
        let jpg = caprust_core::cache::thumbnail_path(std::path::Path::new(project_path), item.id);
        if !jpg.is_file() {
            return None;
        }

        let bytes = std::fs::read(&jpg).ok()?;
        let img = image::load_from_memory(&bytes).ok()?;
        let rgba = img.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let color_img = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
        let handle = ctx.load_texture(
            format!("thumb-{}", item.id),
            color_img,
            egui::TextureOptions::LINEAR,
        );
        self.textures.insert(item.id, handle.clone());
        Some(handle)
    }

    pub fn invalidate(&mut self, id: Uuid) {
        self.colors.remove(&id);
        self.textures.remove(&id);
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
    pub sort_dir: SortDirection,
    pub filter: MediaFilter,
    pub preview: PreviewSize,
    pub thumb_cache: ThumbnailCache,
}

impl Default for MediaBinState {
    fn default() -> Self {
        Self {
            sort: MediaSort::Added,
            sort_dir: SortDirection::Ascending,
            filter: MediaFilter::All,
            preview: PreviewSize::Medium,
            thumb_cache: ThumbnailCache::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

/// Result of one frame of the media-bin panel.
#[derive(Default)]
pub struct MediaBinOutput {
    pub dragging: Option<Uuid>,
    pub newly_imported: Vec<Uuid>,
    pub remove_requested: Vec<Uuid>,
}

pub fn show(ui: &mut Ui, project: &mut ProjectState, state: &mut MediaBinState) -> MediaBinOutput {
    // --- Import buttons ---
    let mut newly_imported: Vec<Uuid> = Vec::new();
    let mut clear_all = false;
    ui.horizontal(|ui| {
        if ui.button(tr("media-import-clips")).clicked() {
            newly_imported.extend(import_with(project, VIDEO_EXTS, "Video"));
        }
        if ui.button(tr("media-import-music")).clicked() {
            newly_imported.extend(import_with(project, AUDIO_EXTS, "Audio"));
        }
        if ui.button(tr("media-import-images")).clicked() {
            newly_imported.extend(import_with(project, IMAGE_EXTS, "Image"));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button(tr("media-clear-all"))
                .on_hover_text(tr("media-clear-all-tooltip"))
                .clicked()
            {
                clear_all = true;
            }
        });
    });
    if clear_all {
        project.media.items.clear();
        tracing::info!("media library cleared");
    }

    ui.separator();

    // --- Sort + size controls ---
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("media_sort")
            .selected_text(format!(
                "{} {}",
                tr("media-sort-label"),
                tr(&format!("media-sort-{}", state.sort.key()))
            ))
            .width(110.0)
            .show_ui(ui, |ui| {
                for s in MediaSort::all() {
                    ui.selectable_value(&mut state.sort, s, tr(&format!("media-sort-{}", s.key())));
                }
            });
        // Sort direction toggle
        let dir_icon = state.sort_dir.icon();
        if ui
            .button(dir_icon)
            .on_hover_text(tr(&format!("media-sort-dir-{}", state.sort_dir.key())))
            .clicked()
        {
            state.sort_dir = state.sort_dir.flipped();
        }
        ui.separator();
        ui.label(tr("media-filter-label"));
        egui::ComboBox::from_id_salt("media_filter")
            .selected_text(state.filter.label())
            .width(90.0)
            .show_ui(ui, |ui| {
                for f in MediaFilter::all() {
                    ui.selectable_value(
                        &mut state.filter,
                        f,
                        tr(&format!("media-filter-{}", f.key())),
                    );
                }
            });
        ui.separator();
        ui.label(tr("media-size-label"));
        for sz in [PreviewSize::Small, PreviewSize::Medium, PreviewSize::Large] {
            ui.selectable_value(&mut state.preview, sz, sz.label());
        }
    });

    ui.separator();

    // --- Sorted item list ---
    let mut sorted: Vec<MediaItem> = project
        .media
        .items
        .iter()
        .filter(|m| state.filter.matches(m.kind))
        .cloned()
        .collect();
    match state.sort {
        MediaSort::Added => sorted.sort_by_key(|m| m.added_at),
        MediaSort::Name => sorted.sort_by_key(|m| a_lower(&m.name)),
        MediaSort::Type => sorted.sort_by(|a, b| {
            kind_rank(a.kind)
                .cmp(&kind_rank(b.kind))
                .then_with(|| a.name.cmp(&b.name))
        }),
    }

    if state.sort_dir == SortDirection::Descending {
        sorted.reverse();
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
        return MediaBinOutput {
            dragging: None,
            newly_imported,
            remove_requested: Vec::new(),
        };
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
    let mut remove_requested: Vec<Uuid> = Vec::new();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(h_gap, v_gap);

            let project_path_opt: Option<&str> = project.project_path.as_deref();
            for row in sorted.chunks(cols) {
                ui.horizontal(|ui| {
                    for item in row {
                        let (resp, should_remove) = draw_card(
                            ui,
                            item,
                            card_w,
                            thumb_h,
                            &mut state.thumb_cache,
                            project_path_opt,
                        );
                        if should_remove {
                            remove_requested.push(item.id);
                        }
                        if resp.drag_started() {
                            dragging = Some(item.id);
                        }
                    }
                });
            }
        });

    MediaBinOutput {
        dragging,
        newly_imported,
        remove_requested,
    }
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
    project_path: Option<&str>,
) -> (egui::Response, bool) {
    let id = egui::Id::new(item.id);
    let payload = item.id;
    let mut remove_requested = false;

    let inner = ui.dnd_drag_source(id, payload, |ui| {
        // Reserve card rect (thumbnail + filename line)
        let line_h = 14.0;
        let total = Vec2::new(card_w, thumb_h + 4.0 + line_h);
        let (rect, resp) = ui.allocate_exact_size(total, Sense::click_and_drag());

        // Thumbnail rect
        let thumb_rect = Rect::from_min_size(rect.min, Vec2::new(card_w, thumb_h));

        // Try real thumbnail first; fall back to placeholder.
        let texture = cache.texture_for(ui.ctx(), project_path, item);

        if let Some(tex) = texture {
            let tex_size = tex.size_vec2();
            let scale = (thumb_rect.width() / tex_size.x).max(thumb_rect.height() / tex_size.y);
            let draw_size = tex_size * scale;
            let frac_x = (thumb_rect.width() / draw_size.x).min(1.0);
            let frac_y = (thumb_rect.height() / draw_size.y).min(1.0);
            let uv_min = Pos2::new((1.0 - frac_x) / 2.0, (1.0 - frac_y) / 2.0);
            let uv_max = Pos2::new(1.0 - uv_min.x, 1.0 - uv_min.y);
            ui.painter().image(
                tex.id(),
                thumb_rect,
                Rect::from_min_max(uv_min, uv_max),
                Color32::WHITE,
            );
        } else {
            // Placeholder: hashed color + icon
            let base = cache.color_for(item);
            ui.painter().rect_filled(thumb_rect, 4.0, base);

            let dark = Color32::from_black_alpha(40);
            let grad_rect = Rect::from_min_max(
                Pos2::new(thumb_rect.left(), thumb_rect.bottom() - thumb_h * 0.35),
                thumb_rect.max,
            );
            ui.painter().rect_filled(grad_rect, 4.0, dark);

            let icon_size = (thumb_h * 0.42).clamp(16.0, 40.0);
            ui.painter().text(
                thumb_rect.center(),
                egui::Align2::CENTER_CENTER,
                item.kind.icon(),
                FontId::proportional(icon_size),
                Color32::from_white_alpha(230),
            );
        }

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

        // X button (top-right of thumbnail) — visible on hover
        if resp.hovered() {
            let btn = 18.0;
            let x_rect = Rect::from_min_size(
                Pos2::new(thumb_rect.right() - btn - 3.0, thumb_rect.top() + 3.0),
                Vec2::splat(btn),
            );
            let x_resp = ui.interact(x_rect, egui::Id::new(("rm_media", item.id)), Sense::click());
            let bg = if x_resp.hovered() {
                Color32::from_rgb(200, 60, 60)
            } else {
                Color32::from_black_alpha(140)
            };
            ui.painter().rect_filled(x_rect, 3.0, bg);
            ui.painter().text(
                x_rect.center(),
                egui::Align2::CENTER_CENTER,
                "✕",
                FontId::proportional(12.0),
                Color32::WHITE,
            );
            if x_resp.clicked() {
                remove_requested = true;
            }
        }

        // Right-click menu
        resp.context_menu(|ui| {
            if ui.button(tr("media-remove-one")).clicked() {
                remove_requested = true;
                ui.close_menu();
            }
        });

        resp.on_hover_text(format!(
            "{}\n{} · {} ms\n{}",
            item.path,
            item.kind.label(),
            item.duration_ms,
            human_size(item.size_bytes),
        ))
    });

    (inner.inner, remove_requested)
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

fn import_with(project: &mut ProjectState, exts: &[&str], label: &str) -> Vec<Uuid> {
    let mut new_ids = Vec::new();
    if let Some(paths) = rfd::FileDialog::new()
        .add_filter(label, exts)
        .add_filter("All files", &["*"])
        .pick_files()
    {
        for p in paths {
            let s = p.to_string_lossy().to_string();
            let kind = guess_kind(&s).unwrap_or(MediaKind::Video);
            let id = project.media.add(&s, kind);
            new_ids.push(id);
        }
    }
    new_ids
}

#[cfg(test)]
mod sort_filter_tests {
    use super::*;
    use caprust_core::media::{MediaItem, MediaKind};

    fn make(name: &str, kind: MediaKind, added_at: u64) -> MediaItem {
        MediaItem {
            id: uuid::Uuid::new_v4(),
            name: name.to_string(),
            path: format!("/tmp/{name}"),
            kind,
            duration_ms: 1000,
            size_bytes: 100,
            added_at,
            probe_done: true,
            thumb_done: true,
        }
    }

    #[test]
    fn filter_video_only() {
        let items = vec![
            make("a.mp4", MediaKind::Video, 1),
            make("b.mp3", MediaKind::Audio, 2),
            make("c.png", MediaKind::Image, 3),
        ];
        let filtered: Vec<_> = items
            .iter()
            .filter(|m| MediaFilter::Video.matches(m.kind))
            .collect();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "a.mp4");
    }

    #[test]
    fn sort_dir_descending_reverses_ascending() {
        use super::SortDirection;
        // Ascending ostaje kak je
        let asc = SortDirection::Ascending;
        assert_eq!(asc.flipped(), SortDirection::Descending);
        assert_eq!(
            SortDirection::Descending.flipped(),
            SortDirection::Ascending
        );
    }

    #[test]
    fn sort_dir_icon_and_key() {
        use super::SortDirection;
        assert_eq!(SortDirection::Ascending.key(), "asc");
        assert_eq!(SortDirection::Descending.key(), "desc");
        assert_eq!(SortDirection::Ascending.icon(), "\u{2193}");
        assert_eq!(SortDirection::Descending.icon(), "\u{2191}");
    }

    #[test]
    fn sort_by_name() {
        let mut items = vec![
            make("z.mp4", MediaKind::Video, 1),
            make("a.mp4", MediaKind::Video, 2),
            make("m.mp4", MediaKind::Video, 3),
        ];
        items.sort_by_key(|m| m.name.to_lowercase());
        assert_eq!(items[0].name, "a.mp4");
        assert_eq!(items[2].name, "z.mp4");
    }

    #[test]
    fn sort_by_added() {
        let mut items = vec![
            make("z.mp4", MediaKind::Video, 3),
            make("a.mp4", MediaKind::Video, 1),
            make("m.mp4", MediaKind::Video, 2),
        ];
        items.sort_by_key(|m| m.added_at);
        assert_eq!(items[0].added_at, 1);
        assert_eq!(items[2].added_at, 3);
    }
}
