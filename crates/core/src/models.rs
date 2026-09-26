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
                )
                .with_url(
                    "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx",
                ),
                ModelInfo::new(
                    "piper-en-amy",
                    "Piper — en_US (Amy)",
                    ModelKind::Narration,
                    "en",
                    63,
                    "Warmer female voice for narration.",
                )
                .with_url(
                    "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/amy/medium/en_US-amy-medium.onnx",
                ),
                ModelInfo::new(
                    "piper-hr-ivan",
                    "Piper — hr_HR (Ivan)",
                    ModelKind::Narration,
                    "hr",
                    61,
                    "Hrvatski muški glas za naraciju.",
                ),
                // Note: hr_HR / de_DE voices are not (yet) in the
                // rhasspy/piper-voices repo. Left without a URL so the
                // downloader reports "no URL configured" instead of
                // returning a 404. Adding them later is a one-line
                // change once the voice files exist.
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
    /// Legacy no-op kept so existing call sites compile. Actual
    /// downloads go through `spawn_model_download` (see below).
    pub fn start_download_in(&mut self, _id: &str, _target_dir: &std::path::Path) {}

    /// Legacy no-op.
    pub fn start_download(&mut self, _id: &str) {}

    /// Legacy no-op. Real progress is driven by DownloadEvent messages
    /// arriving from the download thread; the caller updates
    /// `ModelInfo::progress` itself. Kept so existing per-frame tick
    /// calls do not need to be removed in one go.
    pub fn tick_downloads(&mut self, _dt: f32) {}

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

    /// Backfill `url` and `sha256` from the built-in defaults when the
    /// loaded project's copy is missing them. Older projects carry a
    /// frozen registry snapshot from the moment they were saved, so a
    /// URL added later (e.g. for Piper voices in F1b) would never
    /// reach them. Called on project load.
    ///
    /// Also adds any models present in the defaults but missing from
    /// the loaded registry (new model added in a later release).
    pub fn merge_missing_defaults(&mut self) {
        let defaults = ModelRegistry::default();
        for d in &defaults.models {
            match self.models.iter_mut().find(|m| m.id == d.id) {
                Some(m) => {
                    if m.url.is_empty() && !d.url.is_empty() {
                        m.url = d.url.clone();
                    }
                    if m.sha256.is_empty() && !d.sha256.is_empty() {
                        m.sha256 = d.sha256.clone();
                    }
                }
                None => {
                    // Brand new model in a later build — surface it.
                    self.models.push(d.clone());
                }
            }
        }
    }
}

/// Progress events emitted by a background model download.
#[derive(Debug)]
pub enum DownloadEvent {
    /// Emitted once when the HTTP response is received. `total_bytes`
    /// is None when the server did not send Content-Length.
    Started { total_bytes: Option<u64> },
    /// Emitted roughly every 100 ms while bytes arrive.
    Progress { downloaded: u64, total: Option<u64> },
    /// Emitted before the SHA-256 verification pass.
    Verifying,
    /// Terminal success. The `.part` file has been renamed to its
    /// final name.
    Done,
    /// Terminal failure. Message is user-facing.
    Failed(String),
}

/// Spawn a background thread that downloads `url` into `target_path`.
///
/// Behavior:
///   * Creates parent directories as needed.
///   * Writes to `<target>.part` while downloading so a partial file
///     never looks complete to the rest of the app.
///   * Streams progress through the returned channel at ~10 Hz.
///   * Verifies SHA-256 if `expected_sha256` is non-empty (lowercase
///     hex). Mismatch deletes the .part file and reports Failed.
///   * Renames .part -> target on success.
pub fn spawn_model_download(
    model_id: String,
    url: String,
    target_path: std::path::PathBuf,
    expected_sha256: String,
) -> std::sync::mpsc::Receiver<DownloadEvent> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name(format!("caprust-model-dl-{model_id}"))
        .spawn(move || {
            let result = download_impl(&url, &target_path, &expected_sha256, &tx);
            let _ = tx.send(match result {
                Ok(()) => DownloadEvent::Done,
                Err(e) => DownloadEvent::Failed(format!("{e}")),
            });
        })
        .expect("spawn model download thread");
    rx
}

fn download_impl(
    url: &str,
    target: &std::path::Path,
    expected_sha256: &str,
    tx: &std::sync::mpsc::Sender<DownloadEvent>,
) -> anyhow::Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let part_path = target.with_extension("part");
    // Wipe any stale .part from a previous attempt.
    let _ = std::fs::remove_file(&part_path);

    tracing::info!("model download: GET {url}");
    let resp = ureq::get(url)
        .call()
        .map_err(|e| anyhow::anyhow!("HTTP request failed: {e}"))?;
    let total: Option<u64> = resp
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok());
    let _ = tx.send(DownloadEvent::Started { total_bytes: total });

    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(&part_path)?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut downloaded: u64 = 0;
    let mut last_emit = std::time::Instant::now();
    loop {
        let n = std::io::Read::read(&mut reader, &mut buf)?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buf[..n])?;
        downloaded += n as u64;
        if last_emit.elapsed().as_millis() >= 100 {
            let _ = tx.send(DownloadEvent::Progress { downloaded, total });
            last_emit = std::time::Instant::now();
        }
    }
    drop(file);
    let _ = tx.send(DownloadEvent::Progress { downloaded, total });

    if !expected_sha256.is_empty() {
        let _ = tx.send(DownloadEvent::Verifying);
        use sha2::{Digest, Sha256};
        let bytes = std::fs::read(&part_path)?;
        let hash = Sha256::digest(&bytes);
        let got: String = hash.iter().map(|b| format!("{b:02x}")).collect();
        if !got.eq_ignore_ascii_case(expected_sha256) {
            let _ = std::fs::remove_file(&part_path);
            anyhow::bail!("SHA-256 mismatch: got {got}, expected {expected_sha256}");
        }
        tracing::info!("model download: SHA-256 ok");
    }

    std::fs::rename(&part_path, target)?;
    tracing::info!("model download: wrote {}", target.display());
    Ok(())
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
