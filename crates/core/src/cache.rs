//! Clear project cache — regenerable artwork and waveforms.

use anyhow::Result;
use std::path::Path;

/// Path: <project>/cache/thumbnails/<id>.jpg
pub fn thumbnail_path(project_path: &Path, media_id: uuid::Uuid) -> std::path::PathBuf {
    project_path
        .join("cache")
        .join("thumbnails")
        .join(format!("{media_id}.jpg"))
}

/// Best-effort: does the thumbnail already exist?
pub fn thumbnail_exists(project_path: &Path, media_id: uuid::Uuid) -> bool {
    thumbnail_path(project_path, media_id).is_file()
}

pub fn clear_cache(project_path: &Path) -> Result<usize> {
    let cache_dir = project_path.join("cache");
    if !cache_dir.exists() {
        return Ok(0);
    }
    let count = count_files(&cache_dir);
    std::fs::remove_dir_all(&cache_dir)?;
    Ok(count)
}

fn count_files(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| {
            let p = e.path();
            if p.is_dir() {
                count_files(&p)
            } else {
                1
            }
        })
        .sum()
}
