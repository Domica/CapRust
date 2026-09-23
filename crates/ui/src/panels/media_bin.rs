//! Media bin. Sorting scaffold from jub0t/Concat#63.

use caprust_core::ProjectState;
use egui::Ui;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MediaSort {
    Added,
    Name,
    Type,
}

pub fn show(ui: &mut Ui, project: &ProjectState) {
    ui.label("📁 Imports");
    ui.label("✨ Filters");
    ui.label("🔤 Text Templates");
    ui.separator();
    ui.label(format!("{} clips in project", project.clips.len()));
}
