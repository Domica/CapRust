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

// ---------------------------------------------------------------------------
// Narration cache (global, in models_dir)
// ---------------------------------------------------------------------------
//
// TTS output is deterministic for a given (text, voice_id) pair, so we
// cache WAVs by hash rather than per-clip. This keeps the project file
// small and sync-friendly: a regenerable audio asset never needs to be
// committed alongside the .caprust file.
//
// Layout: <models_dir>/narration/<blake3(text|voice_id)>.wav

/// Hash used as the narration cache key. Deterministic across runs.
pub fn narration_hash(text: &str, voice_id: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(voice_id.as_bytes());
    hasher.update(b"|");
    hasher.update(text.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Path where the TTS WAV for this (text, voice_id) lives.
pub fn narration_path(
    models_dir: &std::path::Path,
    text: &str,
    voice_id: &str,
) -> std::path::PathBuf {
    models_dir
        .join("narration")
        .join(format!("{}.wav", narration_hash(text, voice_id)))
}

/// Best-effort: has this narration already been synthesized?
pub fn narration_exists(models_dir: &std::path::Path, text: &str, voice_id: &str) -> bool {
    narration_path(models_dir, text, voice_id).is_file()
}
