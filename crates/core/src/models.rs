//! AI model registry — captions (Whisper family) and narration (Piper voices).
//!
//! Models are NOT bundled with the app. User enables/downloads them on
//! first use, they live in `%APPDATA%/CapRust/models/` (or `~/.caprust/models/`).

use serde::{Deserialize, Serialize};

pub mod download;

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
    /// HTTPS URL of the model file to download. Empty for models whose
    /// download is not yet wired (currently all four Whisper variants
    /// and the Piper voices are served from Hugging Face).
    #[serde(default)]
    pub url: String,
    /// Optional expected SHA-256 of the downloaded file, lowercase hex.
    /// When present the downloader verifies it before renaming
    /// `.part` to the final name. Empty = skip verification.
    #[serde(default)]
    pub sha256: String,
}

impl ModelInfo {
    /// Basic entry without a download URL. Use `with_url` to attach one.
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
            url: String::new(),
            sha256: String::new(),
        }
    }

    /// Attach the download URL (and optional SHA-256) to this entry.
    pub fn with_url(mut self, url: &str) -> Self {
        self.url = url.into();
        self
    }

    pub fn with_sha256(mut self, sha: &str) -> Self {
        self.sha256 = sha.into();
        self
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
                )
                .with_url(
                    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
                ),
                ModelInfo::new(
                    "whisper-base",
                    "Whisper Base",
                    ModelKind::Caption,
                    "multi",
                    142,
                    "Balanced speed/accuracy. Recommended default.",
                )
                .with_url(
                    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
                ),
                ModelInfo::new(
                    "whisper-small",
                    "Whisper Small",
                    ModelKind::Caption,
                    "multi",
                    466,
                    "Better accuracy for accented speech and noisy audio.",
                )
                .with_url(
                    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
                ),
                ModelInfo::new(
                    "whisper-medium",
                    "Whisper Medium",
                    ModelKind::Caption,
                    "multi",
                    1500,
                    "High accuracy. Slower, needs a decent GPU.",
                )
                .with_url(
                    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin",
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
    /// Stub: pretend to start a download into the given folder.
    /// Real impl will spawn a task that fetches from a CDN.
    pub fn start_download_in(&mut self, id: &str, target_dir: &std::path::Path) {
        // Ensure the folder exists so the user sees it in Explorer.
        let _ = std::fs::create_dir_all(target_dir);
        let _ = std::fs::write(
            target_dir.join(format!(".{id}.placeholder")),
            b"placeholder",
        );
        if let Some(m) = self.models.iter_mut().find(|m| m.id == id) {
            if matches!(m.status, ModelStatus::NotDownloaded | ModelStatus::Error) {
                m.status = ModelStatus::Downloading;
                m.progress = 0.0;
            }
        }
    }

    /// Legacy stub — uses the default models dir.
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

// ---------------------------------------------------------------------------
// Local filesystem helpers
// ---------------------------------------------------------------------------

impl ModelRegistry {
    /// Where this model's weights live on disk. Whisper is a single .bin
    /// file; Piper uses two files (.onnx + .onnx.json) but we only track
    /// the ONNX path here — the sibling JSON is downloaded alongside and
    /// discovered by the Piper process at runtime.
    pub fn local_path(&self, models_dir: &std::path::Path, id: &str) -> std::path::PathBuf {
        Self::local_path_static(&self.models, models_dir, id)
    }

    /// Static variant, callable while iterating `self.models` mutably.
    pub fn local_path_static(
        models: &[ModelInfo],
        models_dir: &std::path::Path,
        id: &str,
    ) -> std::path::PathBuf {
        let kind = models.iter().find(|m| m.id == id).map(|m| m.kind);
        let filename = match kind {
            Some(ModelKind::Narration) => format!("{id}.onnx"),
            _ => format!("{id}.bin"),
        };
        models_dir.join(filename)
    }

    /// Verify a single model against the filesystem and update its status.
    /// Call on app startup so previously-downloaded models are marked
    /// Ready without hitting the network.
    pub fn scan_local(&mut self, models_dir: &std::path::Path) {
        // Compute all paths first (immutable borrow), then mutate.
        let paths: Vec<std::path::PathBuf> = self
            .models
            .iter()
            .map(|m| Self::local_path_static(&self.models, models_dir, &m.id))
            .collect();
        for (m, path) in self.models.iter_mut().zip(paths) {
            let exists = path.is_file() && path.metadata().map(|md| md.len() > 0).unwrap_or(false);
            m.status = if exists {
                ModelStatus::Ready
            } else {
                ModelStatus::NotDownloaded
            };
            m.progress = if exists { 1.0 } else { 0.0 };
        }
    }

    /// Mutable access by id, for updating progress / status during a
    /// background download.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut ModelInfo> {
        self.models.iter_mut().find(|m| m.id == id)
    }
}
