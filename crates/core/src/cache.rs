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

/// Path: <project>/cache/masks/<clip_id>.mkv
///
/// One alpha-mask sequence per clip, encoded as FFV1 in Matroska so
/// the mask is lossless, alpha-capable, and a single seekable file.
/// Lossless was chosen over PNG sequences because 900 PNGs at 1080p
/// is ~1.8 GB, whereas the same sequence as FFV1 gray is closer to
/// 30-100 MB. Keyed by clip id, not media id: the mask reflects the
/// clip's own trim window, so two clips sharing a source still get
/// their own mask files.
pub fn mask_path(project_path: &Path, clip_id: uuid::Uuid) -> std::path::PathBuf {
    project_path
        .join("cache")
        .join("masks")
        .join(format!("{clip_id}.mkv"))
}

/// Best-effort: does the mask sequence already exist?
pub fn mask_exists(project_path: &Path, clip_id: uuid::Uuid) -> bool {
    let p = mask_path(project_path, clip_id);
    p.is_file() && p.metadata().map(|m| m.len() > 0).unwrap_or(false)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_path_is_under_cache_masks() {
        let p = mask_path(std::path::Path::new("/proj"), uuid::Uuid::nil());
        let s = p.to_string_lossy();
        assert!(s.contains("cache"), "path missing cache/: {s}");
        assert!(s.contains("masks"), "path missing masks/: {s}");
        assert!(s.ends_with(".mkv"), "path should end in .mkv: {s}");
    }

    #[test]
    fn mask_path_keys_on_clip_id() {
        let a = uuid::Uuid::new_v4();
        let b = uuid::Uuid::new_v4();
        let pa = mask_path(std::path::Path::new("/p"), a);
        let pb = mask_path(std::path::Path::new("/p"), b);
        assert_ne!(pa, pb);
        assert!(pa.to_string_lossy().contains(&a.to_string()));
    }
}
