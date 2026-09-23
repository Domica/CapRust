//! AI model registry — captions (Whisper family) and narration (Piper voices).
//!
//! Models are NOT bundled with the app. User enables/downloads them on
//! first use, they live in `%APPDATA%/CapRust/models/` (or `~/.caprust/models/`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelKind {
    Caption,
    Narration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelStatus {
    NotDownloaded,
    Downloading,
    Ready,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub kind: ModelKind,
    /// Language hint (ISO code, or "multi" for multilingual).
    pub language: String,
    /// Approximate size on disk, MB.
    pub size_mb: u32,
    /// Short description shown in Settings.
    pub description: String,
    pub status: ModelStatus,
    /// Progress 0.0..=1.0 when Downloading.
    pub progress: f32,
    /// Enabled = user has switched it on.
    pub enabled: bool,
}

impl ModelInfo {
    pub fn new(
        id: &str,
        name: &str,
        kind: ModelKind,
        language: &str,
        size_mb: u32,
        description: &str,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind,
            language: language.into(),
            size_mb,
            description: description.into(),
            status: ModelStatus::NotDownloaded,
            progress: 0.0,
            enabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRegistry {
    pub models: Vec<ModelInfo>,
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self {
            models: vec![
                // --- Captions (Whisper family) ---
                ModelInfo::new(
                    "whisper-tiny",
                    "Whisper Tiny",
                    ModelKind::Caption,
                    "multi",
                    75,
                    "Fastest. Good for quick drafts and low-end hardware.",
                ),
                ModelInfo::new(
                    "whisper-base",
                    "Whisper Base",
                    ModelKind::Caption,
                    "multi",
                    142,
                    "Balanced speed/accuracy. Recommended default.",
                ),
                ModelInfo::new(
                    "whisper-small",
                    "Whisper Small",
                    ModelKind::Caption,
                    "multi",
                    466,
                    "Better accuracy for accented speech and noisy audio.",
                ),
                ModelInfo::new(
                    "whisper-medium",
                    "Whisper Medium",
                    ModelKind::Caption,
                    "multi",
                    1500,
                    "High accuracy. Slower, needs a decent GPU.",
                ),
                // --- Narration (Piper TTS voices) ---
                ModelInfo::new(
                    "piper-en-lessac",
                    "Piper — en_US (Lessac)",
                    ModelKind::Narration,
                    "en",
                    63,
                    "Clear American English voice. Natural pacing.",
                ),
                ModelInfo::new(
                    "piper-en-amy",
                    "Piper — en_US (Amy)",
                    ModelKind::Narration,
                    "en",
                    63,
                    "Warmer female voice for narration.",
                ),
                ModelInfo::new(
                    "piper-hr-ivan",
                    "Piper — hr_HR (Ivan)",
                    ModelKind::Narration,
                    "hr",
                    61,
                    "Hrvatski muški glas za naraciju.",
                ),
                ModelInfo::new(
                    "piper-de-thorsten",
                    "Piper — de_DE (Thorsten)",
                    ModelKind::Narration,
                    "de",
                    63,
                    "German male voice.",
                ),
            ],
        }
    }
}

impl ModelRegistry {
    /// Stub: pretend to start a download. Real impl spawns a task that
    /// fetches from the project CDN and writes into the model cache dir.
    pub fn start_download(&mut self, id: &str) {
        if let Some(m) = self.models.iter_mut().find(|m| m.id == id) {
            if matches!(m.status, ModelStatus::NotDownloaded | ModelStatus::Error) {
                m.status = ModelStatus::Downloading;
                m.progress = 0.0;
            }
        }
    }

    pub fn tick_downloads(&mut self, dt: f32) {
        for m in &mut self.models {
            if m.status == ModelStatus::Downloading {
                m.progress = (m.progress + dt * 0.15).min(1.0);
                if m.progress >= 1.0 {
                    m.status = ModelStatus::Ready;
                    m.enabled = true;
                }
            }
        }
    }

    pub fn ready_captions(&self) -> Vec<&ModelInfo> {
        self.models
            .iter()
            .filter(|m| m.kind == ModelKind::Caption && m.status == ModelStatus::Ready)
            .collect()
    }

    pub fn ready_narration(&self) -> Vec<&ModelInfo> {
        self.models
            .iter()
            .filter(|m| m.kind == ModelKind::Narration && m.status == ModelStatus::Ready)
            .collect()
    }
}

/// Where models live on disk.
pub fn models_dir() -> std::path::PathBuf {
    let base = std::env::var("APPDATA")
        .or_else(|_| std::env::var("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    base.join("CapRust").join("models")
}
