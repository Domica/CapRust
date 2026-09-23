//! Media bin: sort, preview size, import, thumbnail grid, drag source.

use caprust_core::media::{guess_kind, AUDIO_EXTS, IMAGE_EXTS, VIDEO_EXTS};
use caprust_core::{MediaKind, ProjectState};
use egui::{Color32, Sense, Ui, Vec2};

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
    pub fn thumb(&self) -> Vec2 {
        match self {
            Self::Small => Vec2::new(64.0, 48.0),
            Self::Medium => Vec2::new(96.0, 72.0),
            Self::Large => Vec2::new(140.0, 105.0),
        }
    }
}

pub struct MediaBinState {
    pub sort: MediaSort,
    pub preview: PreviewSize,
}

impl Default for MediaBinState {
    fn default() -> Self {
        Self {
            sort: MediaSort::Added,
            preview: PreviewSize::Medium,
        }
    }
}

/// Returns Some(id) of media item being dragged (from previous frame).
pub fn show(
    ui: &mut Ui,
    project: &mut ProjectState,
    state: &mut MediaBinState,
) -> Option<uuid::Uuid> {
    // --- Header: Import buttons ---
    ui.horizontal(|ui| {
        if ui.button("📥 Import Clips").clicked() {
            import_clips(project);
        }
        if ui.button("🎵 Import Music").clicked() {
            import_music(project);
        }
        if ui.button("🖼 Import Image").clicked() {
            import_images(project);
        }
    });

    ui.separator();

    // --- Toolbar: sort + preview size ---
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("media_sort")
            .selected_text(format!("Sort: {}", state.sort.label()))
            .show_ui(ui, |ui| {
                for s in MediaSort::all() {
                    ui.selectable_value(&mut state.sort, s, s.label());
                }
            });

        ui.separator();

        ui.label("Size:");
        for sz in [PreviewSize::Small, PreviewSize::Medium, PreviewSize::Large] {
            ui.selectable_value(&mut state.preview, sz, sz.label());
        }
    });

    ui.separator();

    // --- Item count ---
    ui.label(format!("{} item(s) in library", project.media.items.len()));
    ui.add_space(4.0);

    // --- Sorted list ---
    let mut sorted: Vec<_> = project.media.items.iter().collect();
    match state.sort {
        MediaSort::Added => sorted.sort_by_key(|m| m.added_at),
        MediaSort::Name => sorted.sort_by_key(|a| a.name.to_lowercase()),
        MediaSort::Type => sorted.sort_by(|a, b| {
            (a.kind as u8)
                .cmp(&(b.kind as u8))
                .then_with(|| a.name.cmp(&b.name))
        }),
    }

    let mut dragging: Option<uuid::Uuid> = None;
    let thumb = state.preview.thumb();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for item in sorted {
                let resp = ui
                    .dnd_drag_source(egui::Id::new(item.id), item.id, |ui| {
                        thumbnail_card(ui, item, thumb);
                    })
                    .response;

                if resp.drag_started() {
                    dragging = Some(item.id);
                }
                resp.on_hover_text(format!(
                    "{}\n{} · {} ms",
                    item.path,
                    item.kind.label(),
                    item.duration_ms
                ));
            }
        });

    dragging
}

fn thumbnail_card(ui: &mut Ui, item: &caprust_core::MediaItem, size: Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let color = match item.kind {
        MediaKind::Video => Color32::from_rgb(40, 60, 90),
        MediaKind::Audio => Color32::from_rgb(60, 40, 80),
        MediaKind::Image => Color32::from_rgb(40, 80, 60),
    };
    ui.painter().rect_filled(rect, 4.0, color);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        item.kind.icon(),
        egui::FontId::proportional(size.y * 0.4),
        Color32::WHITE,
    );
    // filename under
    let text_pos = rect.left_bottom() + egui::vec2(4.0, 2.0);
    ui.painter().text(
        text_pos,
        egui::Align2::LEFT_TOP,
        truncate(&item.name, 16),
        egui::FontId::proportional(11.0),
        Color32::from_gray(200),
    );
    ui.add_space(size.y + 16.0);
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

fn import_clips(project: &mut ProjectState) {
    if let Some(paths) = rfd::FileDialog::new()
        .add_filter("Video", VIDEO_EXTS)
        .add_filter("All files", &["*"])
        .pick_files()
    {
        for p in paths {
            if let Some(kind) = guess_kind(&p.to_string_lossy()) {
                project.media.add(&p.to_string_lossy(), kind);
            }
        }
    }
}

fn import_music(project: &mut ProjectState) {
    if let Some(paths) = rfd::FileDialog::new()
        .add_filter("Audio", AUDIO_EXTS)
        .add_filter("All files", &["*"])
        .pick_files()
    {
        for p in paths {
            if let Some(kind) = guess_kind(&p.to_string_lossy()) {
                project.media.add(&p.to_string_lossy(), kind);
            }
        }
    }
}

fn import_images(project: &mut ProjectState) {
    if let Some(paths) = rfd::FileDialog::new()
        .add_filter("Image", IMAGE_EXTS)
        .add_filter("All files", &["*"])
        .pick_files()
    {
        for p in paths {
            if let Some(kind) = guess_kind(&p.to_string_lossy()) {
                project.media.add(&p.to_string_lossy(), kind);
            }
        }
    }
}
