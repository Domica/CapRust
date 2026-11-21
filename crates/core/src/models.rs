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
    /// Background remover for the P3 feature (u2netp). Single ONNX
    /// file, no sibling config. Produces a per-frame alpha mask.
    BackgroundRemover,
    /// Face detector for auto-reframe (Phase P2). Single small ONNX
    /// file, no sibling config. Downloaded on demand like the others.
    FaceDetector,
    /// SCRFD-500M face detector (P2c alternative). 640x640 RGB input,
    /// distance-format decode. Verified tract-onnx compatible, used as
    /// the primary auto-reframe detector when present; YuNet stays as
    /// a fallback.
    ScrfdDetector,
    /// AI Enhance (Real-ESRGAN) for super-resolution upscaling.
    /// Single ONNX file. Runs during export/render (too slow for real-time preview).
    Enhancer,
    /// Object segmentation for video inpainting (MobileSAM).
    /// Single ONNX file. Generates mask from point/box prompt.
    Segmenter,
    /// AI Inpainting for object removal (LaMa).
    /// Single ONNX file. Runs during export/render to fill masked regions.
    Inpainter,
}

impl ModelKind {
    /// Historical filename rule: Whisper GGML weights are `.bin`,
    /// every other model is ONNX. Used only when a `ModelInfo` has no
    /// explicit filename (older saved registries, or a freshly built
    /// `ModelInfo::new`).
    pub fn default_filename(self, id: &str) -> String {
        match self {
            ModelKind::Caption => format!("{id}.bin"),
            _ => format!("{id}.onnx"),
        }
    }
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
    /// Expected on-disk filename. Empty (the default for older saved
    /// registries) derives it from `kind` via `ModelKind::default_filename`.
    /// Set explicitly for models whose filename is not just `<id>.onnx`.
    #[serde(default)]
    pub filename: String,
}

impl ModelInfo {
    /// Filename this model is stored under. An explicit `filename`
    /// wins; otherwise the legacy kind-based rule applies (1.9).
    pub fn filename_for(&self) -> String {
        if self.filename.is_empty() {
            self.kind.default_filename(&self.id)
        } else {
            self.filename.clone()
        }
    }

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
            filename: String::new(),
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

    pub fn with_filename(mut self, filename: &str) -> Self {
        self.filename = filename.into();
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
                )
                .with_sha256(
                    "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
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
                )
                .with_sha256(
                    "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
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
                )
                .with_sha256(
                    "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
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
                )
                .with_sha256(
                    "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
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
                )
                .with_sha256(
                    "5efe09e69902187827af646e1a6e9d269dee769f9877d17b16b1b46eeaaf019f",
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
                )
                .with_sha256(
                    "b3a6e47b57b8c7fbe6a0ce2518161a50f59a9cdd8a50835c02cb02bdd6206c18",
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
                // --- Face detection (auto-reframe, Phase P2) ---
                // YuNet, ~230 KB, Apache 2.0, fixed 320x320 input.
                // SHA-256 is pinned below and equals the Git LFS oid in
                // opencv_zoo's own pointer file for this exact file.
                ModelInfo::new(
                    "yunet-face",
                    "YuNet face detector",
                    ModelKind::FaceDetector,
                    "multi",
                    1,
                    "Small CPU face detector used by Auto-reframe.",
                )
                .with_url(
                    // media.githubusercontent.com serves the LFS
                    // binary (raw.* returns the pointer).
                    //
                    // 2022mar, not 2023mar: the 2023 export has
                    // symbolic Resize nodes tract-onnx 0.23 cannot
                    // reconcile ("Clashing resolution for expression.
                    // 640=640 != 320"). 2022mar is the fixed-input
                    // version and parses cleanly.
                    //
                    // The opencv_zoo main branch no longer has 2022mar, so
                    // this URL pins the last commit that did
                    // (c97242c); the official repo is the source.
                    "https://media.githubusercontent.com/media/opencv/opencv_zoo/c97242ce7f2a554e288b50eabd9f5df957e78801/models/face_detection_yunet/face_detection_yunet_2022mar.onnx",
                )
                .with_sha256(
                    "50ef07f702a31741ca46a4c0d947773b64143b9362780237bf0d427d6c79bab7",
                ),
                // --- SCRFD-500M (P2c alternative face detector) ---
                // InsightFace's deployed detector. 640x640 RGB input,
                // normalised (x - 127.5) / 128.0. 9 outputs: 3 strides
                // x 3 heads (cls, reg, kps). Verified tract-onnx
                // compatible; used as the primary auto-reframe backend
                // when present. Falls back to YuNet if missing.
                ModelInfo::new(
                    "scrfd-face",
                    "SCRFD-500M face detector",
                    ModelKind::ScrfdDetector,
                    "multi",
                    2,
                    "Fast face detector with 5-point landmarks. Primary auto-reframe backend.",
                )
                .with_url(
                    // yakhyo hosts a direct-download mirror of
                    // InsightFace's det_500m.onnx (buffalo_s variant
                    // with 5-point keypoints). No LFS pointer issue,
                    // no zip. If this mirror disappears, look for
                    // det_500m.onnx in other InsightFace mirrors; the
                    // decoder expects the 9-output layout.
                    "https://github.com/yakhyo/face-reidentification/releases/download/v0.0.1/det_500m.onnx",
                )
                .with_sha256(
                    "5e4447f50245bbd7966bd6c0fa52938c61474a04ec7def48753668a9d8b4ea3a",
                ),
                // --- Background removal (Phase P3) ---
                // u2netp, the lightweight variant of U^2-Net. ~4.7 MB,
                // Apache 2.0. Served from the rembg project releases.
                // Input 320x320 RGB, output 320x320 single-channel
                // alpha. SHA-256 pinned below.
                ModelInfo::new(
                    "u2netp-bg",
                    "u2netp background remover",
                    ModelKind::BackgroundRemover,
                    "multi",
                    5,
                    "Small segmentation model used to remove video backgrounds.",
                )
                .with_url(
                    "https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx",
                )
                .with_sha256(
                    "309c8469258dda742793dce0ebea8e6dd393174f89934733ecc8b14c76f4ddd8",
                ),
                // --- AI Enhance (Real-ESRGAN) ---
                // realesrgan-x4plus, Apache 2.0 / BSD-3-Clause. Input variable RGB, output 4x upscaled.
                // ONNX model from Hugging Face mirror (original: xinntao/Real-ESRGAN, converted by jonathanst29/tinier-upscale-models)
                // Runs during export for AI super-resolution.
                ModelInfo::new(
                    "realesrgan-x4plus",
                    "Real-ESRGAN x4+",
                    ModelKind::Enhancer,
                    "multi",
                    67,
                    "AI super-resolution upscaler (4x). Runs during export — too slow for real-time preview.",
                )
                .with_url(
                    "https://huggingface.co/jonathanst29/tinier-upscale-models/resolve/main/realesrgan-x4plus.onnx",
                )
                .with_sha256(
                    "4ed6a45a8185bde6c8d7c7790a6e80e3c5b9f608946f634021068dd0fd0473c8",
                ),
                // --- Face Restore (GFPGAN v1.4) ---
                // GFPGAN v1.4, Apache 2.0. Blind face restoration: takes a
                // low-quality / distorted face and restores fine detail
                // while preserving identity, expression and mouth motion.
                // Runs during export (too slow for real-time preview).
                // Ideal for AI-generated clips where the face model
                // produced garbled features (e.g. distant talking heads).
                ModelInfo::new(
                    "gfpgan-v1.4",
                    "GFPGAN v1.4 Face Restore",
                    ModelKind::Enhancer,
                    "multi",
                    350,
                    "Real GFPGAN v1.4 face restoration. Pre-pass with SCRFD face detection; restored video is cached and used in preview and export. Requires SCRFD face model.",
                )
                .with_url(
                    "https://huggingface.co/neurobytemind/GFPGANv1.4.onnx/resolve/main/GFPGANv1.4.onnx",
                )
                .with_sha256(
                    "cd7311b8d9e13cdb1e208b12363182da58c7bf45e26d1aa67bbeac4751aae92e",
                )
                .with_filename("GFPGANv1.4.onnx"),
                // --- Object Segmentation (MobileSAM) ---
                // MobileSAM, Apache 2.0. Input 1024x1024 RGB + point/box prompt,
                // output 256x256 mask logits. Lightweight SAM variant (~40 MB).
                // Runs during export/render for object removal inpainting.
                // MobileSAM has TWO files: encoder and decoder.
                //
                // IMPORTANT for tract-onnx 0.23 compatibility:
                // The ONNX export must use FIXED input dimensions (no symbolic
                // image_width/image_height in Pad/Transpose nodes). Export with:
                //   torch.onnx.export(model, dummy_input, "model.onnx",
                //     input_names=["images"], output_names=["..."],
                //     dynamic_axes=None, opset_version=17)
                // The Acly/MobileSAM export uses dynamic symbolic dims which
                // tract-onnx 0.23 cannot resolve ("Undetermined symbol in
                // expression: 1024+-1*image_height"). If you hit this, re-export
                // the model with fixed dimensions or use an alternative export.
                ModelInfo::new(
                    "mobile-sam-encoder",
                    "MobileSAM Encoder",
                    ModelKind::Segmenter,
                    "multi",
                    28,
                    "MobileSAM image encoder (1024x1024). Part of MobileSAM segmentation for object removal.",
                )
                .with_url(
                    "https://huggingface.co/Acly/MobileSAM/resolve/main/mobile_sam_image_encoder.onnx",
                )
                .with_sha256(
                    "580f5fb648ea1062c0aabc26217aed56921985f03f0cbbd852bba81d760cc749",
                )
                // Explicit: both this and the decoder are ModelKind::Segmenter.
                // Keeping the filename on the entry means a future second
                // segmenter cannot silently collide on the kind-based rule.
                .with_filename("mobile-sam-encoder.onnx"),
                // Alternative encoder URL (uncomment and update sha256 if using a
                // different export with fixed input dimensions):
                // ModelInfo::new(
                //     "mobile-sam-encoder-alt",
                //     "MobileSAM Encoder (alt)",
                //     ModelKind::Segmenter,
                //     "multi",
                //     28,
                //     "Alternative MobileSAM image encoder export with fixed dims.",
                // )
                // .with_url("https://huggingface.co/YOUR_REPO/resolve/main/encoder.onnx")
                // .with_sha256("..."),
                ModelInfo::new(
                    "mobile-sam-decoder",
                    "MobileSAM Decoder",
                    ModelKind::Segmenter,
                    "multi",
                    17,
                    "MobileSAM mask decoder (single mask). Part of MobileSAM segmentation for object removal.",
                )
                .with_url(
                    "https://huggingface.co/Acly/MobileSAM/resolve/main/sam_mask_decoder_single.onnx",
                )
                .with_sha256(
                    "93915fc7c993ab9d59ab8c9ccd3bce37f7509c81ab4150a74abd4d2abbd8570d",
                )
                .with_filename("mobile-sam-decoder.onnx"),
                // Alternative decoder URL (uncomment and update sha256 if using a
                // different export with fixed input dimensions):
                // ModelInfo::new(
                //     "mobile-sam-decoder-alt",
                //     "MobileSAM Decoder (alt)",
                //     ModelKind::Segmenter,
                //     "multi",
                //     17,
                //     "Alternative MobileSAM mask decoder export with fixed dims.",
                // )
                // .with_url("https://huggingface.co/YOUR_REPO/resolve/main/decoder.onnx")
                // .with_sha256("..."),
                // --- AI Inpainting (LaMa) ---
                // LaMa (Large Mask Inpainting), Apache 2.0. Input 512x512 RGB + mask,
                // output inpainted RGB. Excellent for object removal.
                // Runs during export/render (too slow for real-time preview).
                // Recommended: lama_fp32.onnx (exported with torch.onnx.export, opset 17)
                ModelInfo::new(
                    "lama-inpainter",
                    "LaMa Inpainter",
                    ModelKind::Inpainter,
                    "multi",
                    208,
                    "AI inpainting for object removal. Fill masked regions naturally. Runs during export.",
                )
                .with_url(
                    "https://huggingface.co/Carve/LaMa-ONNX/resolve/main/lama_fp32.onnx",
                )
                .with_sha256(
                    "1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6",
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

    pub fn ready_segmenter(&self) -> Vec<&ModelInfo> {
        self.models
            .iter()
            .filter(|m| m.kind == ModelKind::Segmenter && m.status == ModelStatus::Ready)
            .collect()
    }

    pub fn ready_inpainter(&self) -> Vec<&ModelInfo> {
        self.models
            .iter()
            .filter(|m| m.kind == ModelKind::Inpainter && m.status == ModelStatus::Ready)
            .collect()
    }

    /// Reconcile a loaded registry with the built-in defaults. Called on
    /// project load and every frame.
    ///
    /// The project file is untrusted input, so the registry it carries is
    /// not trusted either:
    ///   * entries whose id is not a built-in model are dropped (their
    ///     id feeds a filesystem path and their URL a network fetch);
    ///   * `kind`, `url` and `sha256` are always taken from the defaults,
    ///     including when the default is empty;
    ///   * only user state (`enabled`, `status`, `progress`) survives.
    ///
    /// Models present in the defaults but missing from the loaded
    /// registry (added in a later release) are appended.
    pub fn merge_missing_defaults(&mut self) {
        let defaults = ModelRegistry::default();
        self.models
            .retain(|m| defaults.models.iter().any(|d| d.id == m.id));
        for d in &defaults.models {
            match self.models.iter_mut().find(|m| m.id == d.id) {
                Some(m) => {
                    // Preserve user-enabled state; only update metadata from defaults
                    m.kind = d.kind;
                    m.url = d.url.clone();
                    m.sha256 = d.sha256.clone();
                    // m.enabled is intentionally preserved
                }
                None => self.models.push(d.clone()),
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
    spawn_model_download_with_aux(model_id, url, target_path, expected_sha256, None)
}

/// Like `spawn_model_download`, but optionally downloads a second file
/// (aux) after the main one succeeds. Used for Piper voices: the ONNX
/// model needs its sibling `.onnx.json` config to run, and that config
/// lives at `<main_url>.json` on Hugging Face.
///
/// The aux file is written next to the main file with `.json` appended
/// to the main target's filename (so `piper-en-lessac.onnx` produces
/// `piper-en-lessac.onnx.json`, matching what `piper.rs` expects).
///
/// Aux file is NOT SHA-verified (the registry does not carry its hash);
/// only size is checked by the caller indirectly through read success.
pub fn spawn_model_download_with_aux(
    model_id: String,
    url: String,
    target_path: std::path::PathBuf,
    expected_sha256: String,
    aux_url: Option<String>,
) -> std::sync::mpsc::Receiver<DownloadEvent> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name(format!("caprust-model-dl-{model_id}"))
        .spawn(move || {
            let result = download_impl(&url, &target_path, &expected_sha256, &tx);
            match result {
                Ok(()) => {
                    if let Some(aux) = aux_url {
                        let aux_target = std::path::PathBuf::from(format!(
                            "{}.json",
                            target_path.to_string_lossy()
                        ));
                        tracing::info!("model download: aux -> {}", aux_target.display());
                        if let Err(e) = download_impl(&aux, &aux_target, "", &tx) {
                            let _ =
                                tx.send(DownloadEvent::Failed(format!("aux download failed: {e}")));
                            return;
                        }
                    }
                    let _ = tx.send(DownloadEvent::Done);
                }
                Err(e) => {
                    let _ = tx.send(DownloadEvent::Failed(format!("{e}")));
                }
            }
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
        let got = download::sha256_file(&part_path)?;
        if !got.eq_ignore_ascii_case(expected_sha256) {
            // NE brišite .part datoteku na SHA mismatch — ostavite je za
            // inspekaciju tako korisnik vidi što je zapravo preuzeto. Ovo
            // sprječava "dođe do kraja i onda nestane file" scenarij gdje
            // preuzimanje stigne do 100 %, SHA ne uspije, .part se briše,
            // a .onnx nikada nije kreiran.
            let part_display = part_path.display();
            anyhow::bail!(
                "SHA-256 mismatch: got {got}, expected {expected_sha256}. \
                 Partial file kept at {part_display}"
            );
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
        let filename = models
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.filename_for())
            // Unknown ids keep the legacy .bin name.
            .unwrap_or_else(|| ModelKind::Caption.default_filename(id));
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
            if m.status == ModelStatus::Downloading {
                // Don't clobber an in-flight download: the .part file exists
                // while downloading but the final target doesn't yet, so a
                // naive exists-check would reset progress to 0 every frame.
                // However, if neither the final file nor the .part file
                // exists, the download thread died or was aborted and the
                // status is stranded — reset so the Download button
                // reappears instead of being stuck at Downloading forever.
                let part_path = path.with_extension("part");
                let part_exists = part_path.is_file();
                if !part_exists && !exists {
                    m.status = ModelStatus::NotDownloaded;
                    m.progress = 0.0;
                }
                continue;
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn explicit_filename_wins_over_kind_rule() {
        let mut m = ModelInfo::new(
            "custom-segmenter",
            "Custom",
            ModelKind::Segmenter,
            "multi",
            1,
            "test",
        );
        assert_eq!(m.filename_for(), "custom-segmenter.onnx");
        m.filename = "custom-name.bin".into();
        assert_eq!(m.filename_for(), "custom-name.bin");
    }

    #[test]
    fn both_mobile_sam_entries_resolve_to_distinct_paths() {
        let reg = ModelRegistry::default();
        let dir = Path::new("models");
        let enc = ModelRegistry::local_path_static(&reg.models, dir, "mobile-sam-encoder");
        let dec = ModelRegistry::local_path_static(&reg.models, dir, "mobile-sam-decoder");
        assert_eq!(enc, dir.join("mobile-sam-encoder.onnx"));
        assert_eq!(dec, dir.join("mobile-sam-decoder.onnx"));
        assert_ne!(enc, dec, "the two Segmenter entries must not collide");
    }

    #[test]
    fn legacy_registry_without_filename_still_resolves() {
        let mut m = ModelInfo::new("whisper", "Whisper", ModelKind::Caption, "en", 1, "test");
        m.filename.clear();
        assert_eq!(m.filename_for(), "whisper.bin");

        let mut n = ModelInfo::new(
            "scrfd",
            "SCRFD",
            ModelKind::ScrfdDetector,
            "multi",
            1,
            "test",
        );
        n.filename.clear();
        assert_eq!(n.filename_for(), "scrfd.onnx");
    }
}
