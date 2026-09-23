//! Save / load `.caprust` project files (JSON).

use crate::project::ProjectState;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub const PROJECT_EXT: &str = "caprust";

/// Write a project file. Creates parent dirs if needed.
pub fn save_project(state: &ProjectState, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dir {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(state).context("serialize project")?;
    std::fs::write(path, json).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Read a project file.
pub fn load_project(path: &Path) -> Result<ProjectState> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let state: ProjectState = serde_json::from_str(&text).context("parse project JSON")?;
    Ok(state)
}

/// Given a project folder + name, returns `<folder>/<name>.caprust`.
pub fn project_file_for(folder: &Path, name: &str) -> PathBuf {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    folder.join(format!("{safe}.{PROJECT_EXT}"))
}
