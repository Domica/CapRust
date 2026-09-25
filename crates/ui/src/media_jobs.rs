//! Background jobs: ffprobe + thumbnail extraction after import.

use caprust_core::media::{MediaItem, MediaKind};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

#[derive(Debug)]
pub enum Job {
    /// Probe metadata for a media item.
    Probe {
        media_id: uuid::Uuid,
        path: PathBuf,
        kind: MediaKind,
    },
}

#[derive(Debug)]
pub enum JobResult {
    ProbeDone {
        media_id: uuid::Uuid,
        duration_ms: u64,
    },
    ThumbDone {
        media_id: uuid::Uuid,
    },
    Failed {
        media_id: uuid::Uuid,
        error: String,
    },
}

pub struct JobRunner {
    pub tx: Sender<Job>,
    pub rx: Receiver<JobResult>,
}

impl JobRunner {
    pub fn new(ffmpeg: Option<PathBuf>, ffprobe: Option<PathBuf>) -> Self {
        let (tx, rx_job) = channel::<Job>();
        let (tx_result, rx) = channel::<JobResult>();

        std::thread::spawn(move || {
            while let Ok(job) = rx_job.recv() {
                match job {
                    Job::Probe {
                        media_id,
                        path,
                        kind,
                    } => {
                        tracing::info!("job: Probe {} ({:?})", path.display(), kind);
                        // 1) probe
                        let mut duration_ms = 0u64;
                        if let Some(ffprobe) = &ffprobe {
                            match caprust_media_io::ffprobe::probe(ffprobe, &path) {
                                Ok(p) => {
                                    duration_ms = p.duration_ms;
                                    tracing::info!("job: probe ok — {}ms", duration_ms);
                                    let _ = tx_result.send(JobResult::ProbeDone {
                                        media_id,
                                        duration_ms,
                                    });
                                }
                                Err(e) => {
                                    tracing::warn!("job: probe failed — {e}");
                                    let _ = tx_result.send(JobResult::Failed {
                                        media_id,
                                        error: e.to_string(),
                                    });
                                }
                            }
                        }

                        // 2) thumbnail (only for video/image; audio gets a waveform later)
                        if matches!(kind, MediaKind::Video | MediaKind::Image) {
                            if let Some(ffmpeg) = &ffmpeg {
                                // Store alongside a temp path here; the caller
                                // moves it into the project cache dir once
                                // project_path is known. For now, use a shared
                                // temp dir keyed by media id.
                                let tmp_dir = std::env::temp_dir().join("caprust-thumbs");
                                let tmp = tmp_dir.join(format!("{media_id}.jpg"));
                                let at = caprust_media_io::thumbnail::best_frame_time(duration_ms);
                                match caprust_media_io::thumbnail::extract_jpeg(
                                    ffmpeg, &path, &tmp, at, 320,
                                ) {
                                    Ok(()) => {
                                        tracing::info!(
                                            "job: thumbnail extracted to {}",
                                            tmp.display()
                                        );
                                        let _ = tx_result.send(JobResult::ThumbDone { media_id });
                                    }
                                    Err(e) => {
                                        tracing::warn!("job: thumbnail failed — {e}");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        Self { tx, rx }
    }

    /// Kick off a probe+thumbnail job for a media item.
    pub fn enqueue(&self, item: &MediaItem) {
        let _ = self.tx.send(Job::Probe {
            media_id: item.id,
            path: PathBuf::from(&item.path),
            kind: item.kind,
        });
    }

    /// Drain pending results, applying them to the media library.
    pub fn drain(&self, project: &mut caprust_core::ProjectState) -> Vec<uuid::Uuid> {
        let mut thumbs_ready = Vec::new();
        while let Ok(res) = self.rx.try_recv() {
            match res {
                JobResult::ProbeDone {
                    media_id,
                    duration_ms,
                } => {
                    if let Some(m) = project.media.items.iter_mut().find(|m| m.id == media_id) {
                        m.duration_ms = duration_ms;
                        m.probe_done = true;
                    }
                }
                JobResult::ThumbDone { media_id } => {
                    tracing::info!("job: ThumbDone for {media_id}");
                    if let Some(m) = project.media.items.iter_mut().find(|m| m.id == media_id) {
                        m.thumb_done = true;
                    }
                    thumbs_ready.push(media_id);

                    // Copy temp jpg into project cache. rename() fails across
                    // drives on Windows (temp on C:, project may be on D:/F:).
                    if let Some(proj_path) = project.project_path.clone() {
                        let src = std::env::temp_dir()
                            .join("caprust-thumbs")
                            .join(format!("{media_id}.jpg"));
                        let dst = caprust_core::cache::thumbnail_path(
                            std::path::Path::new(&proj_path),
                            media_id,
                        );
                        if let Some(parent) = dst.parent() {
                            if let Err(e) = std::fs::create_dir_all(parent) {
                                tracing::error!("create_dir_all {}: {e}", parent.display());
                                continue;
                            }
                        }
                        match std::fs::copy(&src, &dst) {
                            Ok(_) => {
                                tracing::info!(
                                    "thumbnail copied: {} → {}",
                                    src.display(),
                                    dst.display()
                                );
                                let _ = std::fs::remove_file(&src);
                            }
                            Err(e) => {
                                tracing::error!(
                                    "thumbnail copy failed: {} → {}: {e}",
                                    src.display(),
                                    dst.display()
                                );
                            }
                        }
                    } else {
                        tracing::warn!("ThumbDone but project_path is None — left in temp");
                    }
                }
                JobResult::Failed { media_id, error } => {
                    tracing::warn!("job failed for {media_id}: {error}");
                }
            }
        }
        thumbs_ready
    }
}

// ---------------------------------------------------------------------------
// Caption jobs (Whisper transcription)
// ---------------------------------------------------------------------------

/// Request for a caption transcription job.
#[derive(Debug)]
pub struct CaptionRequest {
    pub model_id: String,
    pub model_path: std::path::PathBuf,
    pub language: String,
    /// First audio source to transcribe: absolute file path plus the
    /// source-side window inside that file.
    pub source_path: std::path::PathBuf,
    pub source_start_ms: u64,
    pub duration_ms: u64,
}

/// Result of a transcription job.
#[derive(Debug)]
pub struct CaptionResult {
    pub model_id: String,
    pub language: String,
    pub segments: Vec<caprust_core::CaptionSegment>,
    /// Timeline position where the resulting Captions clip should be
    /// inserted (the playhead at request time, or the source clip start).
    pub insert_at_ms: u64,
    pub duration_ms: u64,
}

/// Spawn a background thread that extracts mono 16 kHz f32 audio, runs
/// Whisper, and sends the resulting segments back through the returned
/// `Receiver`.
///
/// Returns immediately. `ffmpeg` must be Some — caller is responsible
/// for verifying availability before calling.
pub fn spawn_caption_job(
    ffmpeg: std::path::PathBuf,
    req: CaptionRequest,
) -> std::sync::mpsc::Receiver<Result<CaptionResult, String>> {
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::Builder::new()
        .name("caprust-caption".into())
        .spawn(move || {
            let result = run_caption_job(&ffmpeg, &req);
            let _ = tx.send(result);
        })
        .expect("spawn caption thread");

    rx
}

fn run_caption_job(
    ffmpeg: &std::path::Path,
    req: &CaptionRequest,
) -> Result<CaptionResult, String> {
    tracing::info!(
        "caption: extracting PCM from {} ({}ms @ {})",
        req.source_path.display(),
        req.duration_ms,
        req.source_start_ms
    );

    // 1) ffmpeg → mono f32 16 kHz
    let samples = caprust_media_io::whisper::extract_16khz_mono_f32(
        ffmpeg,
        &req.source_path,
        req.source_start_ms,
        req.duration_ms,
    )
    .map_err(|e| format!("PCM extract: {e}"))?;

    if samples.is_empty() {
        return Err("no audio samples produced (source may be silent)".into());
    }

    // 2) load model + transcribe
    let engine = caprust_media_io::whisper::WhisperEngine::load(&req.model_path)
        .map_err(|e| format!("whisper load: {e}"))?;

    let lang = if req.language.is_empty() || req.language == "multi" {
        None
    } else {
        Some(req.language.as_str())
    };
    let segments = engine
        .transcribe(&samples, lang)
        .map_err(|e| format!("whisper transcribe: {e}"))?;

    tracing::info!(
        "caption: transcribed {} segments for {}",
        segments.len(),
        req.source_path.display()
    );

    Ok(CaptionResult {
        model_id: req.model_id.clone(),
        language: req.language.clone(),
        segments,
        insert_at_ms: req.source_start_ms,
        duration_ms: req.duration_ms,
    })
}

// ---------------------------------------------------------------------------
// Narration jobs (Piper TTS)
// ---------------------------------------------------------------------------

/// Request for a narration synthesis job.
#[derive(Debug)]
pub struct NarrationRequest {
    pub voice_id: String,
    /// Full path to the voice ONNX model (sibling .onnx.json must exist).
    pub voice_onnx_path: std::path::PathBuf,
    pub models_dir: std::path::PathBuf,
    /// Text to speak. Typically short (a few hundred chars).
    pub text: String,
}

/// Result of a narration synthesis job.
#[derive(Debug)]
pub struct NarrationResult {
    pub voice_id: String,
    pub text: String,
    /// Where the WAV was written (deterministic cache path).
    pub wav_path: std::path::PathBuf,
    /// Duration of the generated WAV, milliseconds. Measured with ffprobe.
    pub duration_ms: u64,
}

/// Spawn a background thread that ensures the Piper binary is present,
/// synthesizes `text`, and measures the resulting WAV duration.
/// Returns a channel the UI polls in `drain_narration_job`.
pub fn spawn_narration_job(
    ffprobe: std::path::PathBuf,
    req: NarrationRequest,
) -> std::sync::mpsc::Receiver<Result<NarrationResult, String>> {
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::Builder::new()
        .name("caprust-narration".into())
        .spawn(move || {
            let result = run_narration_job(&ffprobe, &req);
            let _ = tx.send(result);
        })
        .expect("spawn narration thread");

    rx
}

fn run_narration_job(
    ffprobe: &std::path::Path,
    req: &NarrationRequest,
) -> Result<NarrationResult, String> {
    // 1) Ensure Piper binary is on disk (downloads on first use).
    let piper_bin = caprust_media_io::piper::ensure_binary(&req.models_dir)
        .map_err(|e| format!("piper binary: {e}"))?;

    // 2) Compute cache path and skip synthesis if it already exists.
    let wav_path = caprust_core::cache::narration_path(&req.models_dir, &req.text, &req.voice_id);

    if !wav_path.is_file() {
        tracing::info!(
            "narration: synthesizing {} chars with voice {}",
            req.text.len(),
            req.voice_id
        );
        caprust_media_io::piper::synthesize(&piper_bin, &req.voice_onnx_path, &req.text, &wav_path)
            .map_err(|e| format!("piper synthesize: {e}"))?;
    } else {
        tracing::info!("narration: cache hit at {}", wav_path.display());
    }

    // 3) Measure duration with ffprobe (Piper does not tell us).
    let duration_ms = match caprust_media_io::ffprobe::probe(ffprobe, &wav_path) {
        Ok(p) => p.duration_ms,
        Err(e) => {
            tracing::warn!("narration: ffprobe failed ({e}), defaulting to 3000ms");
            3_000
        }
    };

    Ok(NarrationResult {
        voice_id: req.voice_id.clone(),
        text: req.text.clone(),
        wav_path,
        duration_ms,
    })
}
