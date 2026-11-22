//! Background jobs: ffprobe + thumbnail extraction after import.

use caprust_core::media::{MediaItem, MediaKind};
use caprust_media_io::inference_backend::BackendType;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

/// Cooperative cancellation flag shared between the UI thread (which
/// calls [`cancel`](CancellationToken::cancel)) and a worker loop (which
/// calls [`is_cancelled`](CancellationToken::is_cancelled) before each
/// frame). Cheap `Relaxed` atomics are sufficient: the flag is a hint,
/// never a synchronization point for other data.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Run `body` once per frame index `0..total`, stopping early (and
/// returning the number of frames actually processed) when `cancel` is
/// set. A `body` error aborts immediately and is propagated. Pulling
/// the loop into one helper makes the mid-loop cancellation behaviour
/// directly unit-testable without a real model or ffmpeg process.
#[allow(dead_code)]
fn run_cancellable_frames<E, F>(
    total: usize,
    cancel: &CancellationToken,
    mut body: F,
) -> Result<usize, E>
where
    F: FnMut(usize) -> Result<(), E>,
{
    for i in 0..total {
        if cancel.is_cancelled() {
            return Ok(i);
        }
        body(i)?;
    }
    Ok(total)
}

// --- 1.6: process-wide ONNX model cache ---------------------------------
//
// Parsing + optimizing an ONNX graph takes far longer than a single
// inference, and AI jobs used to reload the same model from disk on
// every invocation. tract's `SimplePlan::run(self: &Arc<Self>)` spawns a
// fresh per-call state, so one loaded model can safely serve concurrent
// inference. Cache entries are keyed by path and invalidated when the
// file's mtime changes (e.g. the user re-downloads a model).

type ModelSlot<T> = OnceLock<Mutex<HashMap<String, (Option<SystemTime>, Arc<T>)>>>;

/// Last-modified time of `path`, or `None` if unreadable. Used as the
/// cache invalidation token.
fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

/// Return the cached model for `key` when its recorded mtime still
/// matches `mtime`, otherwise call `load` once and store the result.
/// Generic over the model type so each loader gets its own static slot.
fn model_get_or_load<T>(
    slot: &ModelSlot<T>,
    key: String,
    mtime: Option<SystemTime>,
    load: impl FnOnce() -> Result<T, String>,
) -> Result<Arc<T>, String> {
    let cache = slot.get_or_init(|| Mutex::new(HashMap::new()));
    match cache.lock() {
        Ok(guard) => {
            if let Some((cached_mtime, model)) = guard.get(&key) {
                if *cached_mtime == mtime {
                    return Ok(Arc::clone(model));
                }
            }
        }
        Err(poisoned) => {
            // A loader panic must not permanently break every future
            // job; use the inner map and overwrite the stale entry.
            tracing::warn!("model cache mutex poisoned; recovering");
            let guard = poisoned.into_inner();
            if let Some((cached_mtime, model)) = guard.get(&key) {
                if *cached_mtime == mtime {
                    return Ok(Arc::clone(model));
                }
            }
        }
    }
    let fresh = Arc::new(load()?);
    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, (mtime, Arc::clone(&fresh)));
    }
    Ok(fresh)
}

// One slot per model type. `OnceLock` keeps them lazily initialized and
// `Mutex` guards the map against concurrent job threads.
static SCRFD_CACHE: ModelSlot<caprust_media_io::scrfd::ScrfdDetector> = OnceLock::new();
static YUNET_CACHE: ModelSlot<caprust_media_io::face_detect::FaceDetector> = OnceLock::new();
static BG_CACHE: ModelSlot<caprust_media_io::background_removal::BackgroundRemover> =
    OnceLock::new();
static SAM_CACHE: ModelSlot<caprust_media_io::mobile_sam::MobileSam> = OnceLock::new();
static LAMA_CACHE: ModelSlot<caprust_media_io::lama::Lama> = OnceLock::new();

#[derive(Debug)]
pub enum Job {
    /// Probe metadata for a media item.
    Probe {
        media_id: uuid::Uuid,
        path: PathBuf,
        kind: MediaKind,
        ffmpeg: Option<PathBuf>,
        ffprobe: Option<PathBuf>,
    },
    /// Extract a waveform peak cache for a media item that carries
    /// audio. Writes to a temp .bin; the drain handler copies it
    /// into the project cache once project_path is known.
    Waveform {
        media_id: uuid::Uuid,
        path: PathBuf,
        ffmpeg: Option<PathBuf>,
    },
}

#[derive(Debug)]
pub enum JobResult {
    ProbeDone {
        media_id: uuid::Uuid,
        duration_ms: u64,
        has_audio: bool,
    },
    ThumbDone {
        media_id: uuid::Uuid,
    },
    WaveformDone {
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
    pub fn new() -> Self {
        let (tx, rx_job) = channel::<Job>();
        let (tx_result, rx) = channel::<JobResult>();

        std::thread::spawn(move || {
            while let Ok(job) = rx_job.recv() {
                match job {
                    Job::Probe {
                        media_id,
                        path,
                        kind,
                        ffmpeg,
                        ffprobe,
                    } => {
                        tracing::info!("job: Probe {} ({:?})", path.display(), kind);
                        // 1) probe
                        let mut duration_ms = 0u64;
                        if let Some(ffprobe) = &ffprobe {
                            match caprust_media_io::ffprobe::probe(ffprobe, &path) {
                                Ok(p) => {
                                    duration_ms = p.duration_ms;
                                    let has_audio = p.has_audio;
                                    tracing::info!("job: probe ok — {}ms", duration_ms);
                                    let _ = tx_result.send(JobResult::ProbeDone {
                                        media_id,
                                        duration_ms,
                                        has_audio,
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
                                // seek_for returns 0.0 for images: a still
                                // has one frame at t=0, so any other point
                                // makes ffmpeg write nothing.
                                let at = caprust_media_io::thumbnail::seek_for(kind, duration_ms);
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
                    Job::Waveform {
                        media_id,
                        path,
                        ffmpeg,
                    } => {
                        let Some(ffmpeg) = &ffmpeg else {
                            continue;
                        };
                        tracing::info!("job: Waveform {}", path.display());
                        // Bucket count: 2048 gives ~2.5 px per bucket at
                        // a 5k-px clip strip, plenty of resolution for
                        // a visual waveform. The cache file is ~8 KB
                        // per media item regardless of clip length.
                        const WAVEFORM_BUCKETS: usize = 2048;
                        match caprust_media_io::waveform::extract_peaks(
                            ffmpeg,
                            &path,
                            WAVEFORM_BUCKETS,
                        ) {
                            Ok(peaks) => {
                                let tmp_dir = std::env::temp_dir().join("caprust-waveforms");
                                if let Err(e) = std::fs::create_dir_all(&tmp_dir) {
                                    tracing::warn!("job: waveform tmp dir failed: {e}");
                                    continue;
                                }
                                let tmp = tmp_dir.join(format!("{media_id}.bin"));
                                if let Err(e) = write_waveform_bin(&tmp, &peaks) {
                                    tracing::warn!("job: waveform write {}: {e}", tmp.display());
                                    continue;
                                }
                                tracing::info!(
                                    "job: waveform ok \u{2014} {} buckets -> {}",
                                    peaks.len(),
                                    tmp.display()
                                );
                                let _ = tx_result.send(JobResult::WaveformDone { media_id });
                            }
                            Err(e) => {
                                tracing::warn!("job: waveform failed \u{2014} {e}");
                            }
                        }
                    }
                }
            }
        });

        Self { tx, rx }
    }
}

impl Default for JobRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl JobRunner {
    /// Kick off a probe+thumbnail+waveform job for a media item.
    pub fn enqueue(&self, item: &MediaItem, ffmpeg: Option<PathBuf>, ffprobe: Option<PathBuf>) {
        let _ = self.tx.send(Job::Probe {
            media_id: item.id,
            path: PathBuf::from(&item.path),
            kind: item.kind,
            ffmpeg: ffmpeg.clone(),
            ffprobe,
        });
        // Waveform for anything that carries audio: Audio files and
        // Video (which may have an embedded audio stream). Images do
        // not need one. The extraction is cheap (8 kHz mono decode)
        // and the result is cached for every clip that uses the item.
        if matches!(item.kind, MediaKind::Audio | MediaKind::Video) {
            let _ = self.tx.send(Job::Waveform {
                media_id: item.id,
                path: PathBuf::from(&item.path),
                ffmpeg,
            });
        }
    }

    /// Enqueue only a waveform job (no probe, no thumbnail). Used
    /// for backfilling older projects whose media items were
    /// imported before the waveform pipeline existed.
    pub fn enqueue_waveform(&self, item: &MediaItem, ffmpeg: Option<PathBuf>) {
        if !matches!(item.kind, MediaKind::Audio | MediaKind::Video) {
            return;
        }
        let _ = self.tx.send(Job::Waveform {
            media_id: item.id,
            path: PathBuf::from(&item.path),
            ffmpeg,
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
                    has_audio,
                } => {
                    if let Some(m) = project
                        .media_mut()
                        .items
                        .iter_mut()
                        .find(|m| m.id == media_id)
                    {
                        m.duration_ms = duration_ms;
                        m.has_audio = has_audio;
                        m.probe_done = true;
                    }
                    // Tighten the growth cap on clips that were dropped
                    // before the probe finished (source_duration_ms = 0)
                    // or that still carry a stale cap from an earlier
                    // probe. Only touches clips that link to this media.
                    //
                    // Also extend `duration_ms` for clips that were
                    // placed with the 3 s placeholder: source_duration_ms
                    // is still 0 in that state, so we can detect them
                    // safely. Once a clip has any non-zero source cap
                    // (either this probe set it, or the user trimmed),
                    // we leave duration_ms alone.
                    if duration_ms > 0 {
                        let target_ids: Vec<uuid::Uuid> = project
                            .clips()
                            .iter()
                            .filter(|c| c.media_id == Some(media_id))
                            .map(|c| c.id)
                            .collect();
                        for id in target_ids {
                            project.modify_clip_by_id(id, |c| {
                                let was_placeholder = c.source_duration_ms == 0;
                                if c.source_duration_ms < duration_ms {
                                    c.source_duration_ms = duration_ms;
                                }
                                if was_placeholder && c.duration_ms < duration_ms {
                                    c.duration_ms = duration_ms;
                                }
                            });
                        }
                    }
                }
                JobResult::WaveformDone { media_id } => {
                    tracing::info!("job: WaveformDone for {media_id}");
                    if let Some(m) = project
                        .media_mut()
                        .items
                        .iter_mut()
                        .find(|m| m.id == media_id)
                    {
                        m.waveform_done = true;
                    }
                    // Move the temp .bin into the project cache.
                    if let Some(proj_path) = project.project_path.clone() {
                        let src = std::env::temp_dir()
                            .join("caprust-waveforms")
                            .join(format!("{media_id}.bin"));
                        let dst = caprust_core::cache::waveform_path(
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
                                let _ = std::fs::remove_file(&src);
                                tracing::info!(
                                    "waveform copied: {} \u{2192} {}",
                                    src.display(),
                                    dst.display()
                                );
                            }
                            Err(e) => {
                                tracing::error!(
                                    "waveform copy {} -> {}: {e}",
                                    src.display(),
                                    dst.display()
                                );
                            }
                        }
                    } else {
                        tracing::warn!(
                            "WaveformDone but project_path is None \u{2014} left in temp"
                        );
                    }
                }
                JobResult::ThumbDone { media_id } => {
                    tracing::info!("job: ThumbDone for {media_id}");
                    if let Some(m) = project
                        .media_mut()
                        .items
                        .iter_mut()
                        .find(|m| m.id == media_id)
                    {
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

/// Write a waveform cache file: u32 LE bucket count, then that
/// many f32 LE values.
fn write_waveform_bin(path: &std::path::Path, peaks: &[f32]) -> std::io::Result<()> {
    use std::io::Write;
    let mut buf = Vec::with_capacity(4 + peaks.len() * 4);
    buf.extend_from_slice(&(peaks.len() as u32).to_le_bytes());
    for &v in peaks {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    let mut f = std::fs::File::create(path)?;
    f.write_all(&buf)?;
    Ok(())
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
    /// Offset inside the source file where the clip's audio begins.
    /// Used only for ffmpeg extraction (`-ss` before decoding).
    pub source_start_ms: u64,
    /// Length of the source window to transcribe.
    pub duration_ms: u64,
    /// Timeline position where the resulting Captions clip should be
    /// inserted. This is the source clip's `start_time_ms`, so the
    /// Captions clip lands directly under its video/audio source.
    pub insert_at_ms: u64,
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
        // Whisper.cpp and tract are stack-hungry; the
        // Rust default 2 MB thread stack overflows on
        // Windows during inference. 32 MB is comfortable.
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            // catch_unwind turns a Rust panic in the whisper/ffmpeg
            // worker into a reported error instead of killing the
            // thread silently. Native (C++) segfaults still terminate
            // the process and cannot be caught from Rust.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_caption_job(&ffmpeg, &req)
            }))
            .map_err(|_| "caption job thread panicked".to_string())
            .and_then(|inner| inner);
            let _ = tx.send(result);
        })
        .expect("spawn caption thread");

    rx
}

/// Demo-mode caption job. Activated by a zero-byte `DEMO` file in the
/// models directory. Returns a handful of evenly spaced fake segments
/// so the rest of the caption pipeline (insert, track routing, project
/// save) can be exercised without a real Whisper model on disk.
/// Delete the DEMO file to go back to real transcription.
pub fn spawn_demo_caption_job(
    req: CaptionRequest,
) -> std::sync::mpsc::Receiver<Result<CaptionResult, String>> {
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::Builder::new()
        .name("caprust-caption-demo".into())
        // Whisper.cpp and tract are stack-hungry; the
        // Rust default 2 MB thread stack overflows on
        // Windows during inference. 32 MB is comfortable.
        .stack_size(32 * 1024 * 1024)
        // Whisper.cpp and tract are stack-hungry; the
        // Rust default 2 MB thread stack overflows on
        // Windows during inference. 32 MB is comfortable.
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            // Small sleep so the UI toast/progress states are visible.
            std::thread::sleep(std::time::Duration::from_millis(400));

            let total = req.duration_ms.max(4000);
            let per = total / 4;
            // P1: DEMO segments carry fake per-word timings so the
            // progressive-reveal render path can be exercised end to end
            // without a real Whisper model. Words are laid out evenly
            // inside each segment; real whisper data replaces this when
            // the DEMO marker is absent.
            let seg = |i: u64, text: &str| {
                let start = i * per;
                let end = ((i + 1) * per).saturating_sub(80);
                caprust_core::CaptionSegment {
                    start_ms: start,
                    end_ms: end,
                    text: text.to_string(),
                    words: fake_word_timings(text, start, end),
                }
            };
            let segments = vec![
                seg(0, "DEMO caption — first line"),
                seg(1, "DEMO caption — second line"),
                seg(2, "DEMO caption — third line"),
                seg(3, "DEMO caption — fourth line"),
            ];

            let _ = tx.send(Ok(CaptionResult {
                model_id: req.model_id,
                language: req.language,
                segments,
                insert_at_ms: req.insert_at_ms,
                duration_ms: total,
            }));
        })
        .expect("spawn caption demo thread");

    rx
}

/// Split `text` into evenly-timed fake word timings inside
/// [start_ms, end_ms]. Used only by the DEMO caption path.
fn fake_word_timings(
    text: &str,
    start_ms: u64,
    end_ms: u64,
) -> Vec<caprust_core::clip::WordTiming> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let span = end_ms.saturating_sub(start_ms).max(1);
    let step = span / words.len() as u64;
    words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let ws = start_ms + i as u64 * step;
            let we = if i + 1 == words.len() {
                end_ms
            } else {
                start_ms + (i as u64 + 1) * step
            };
            caprust_core::clip::WordTiming {
                start_ms: ws,
                end_ms: we,
                text: (*w).to_string(),
            }
        })
        .collect()
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
        insert_at_ms: req.insert_at_ms,
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
        // Whisper.cpp and tract are stack-hungry; the
        // Rust default 2 MB thread stack overflows on
        // Windows during inference. 32 MB is comfortable.
        .stack_size(32 * 1024 * 1024)
        // Whisper.cpp and tract are stack-hungry; the
        // Rust default 2 MB thread stack overflows on
        // Windows during inference. 32 MB is comfortable.
        .stack_size(32 * 1024 * 1024)
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

// ---------------------------------------------------------------------------
// Auto-reframe (Phase P2c-3b)
// ---------------------------------------------------------------------------

/// Request for an auto-reframe analysis job. The job probes the source
/// to learn its dimensions, extracts a low-rate frame sequence, runs
/// YuNet on each frame, and computes a keypoint path the render graph
/// can pan along.
#[derive(Debug)]
pub struct ReframeRequest {
    pub clip_id: uuid::Uuid,
    pub source_path: std::path::PathBuf,
    /// Offset inside the source file where the clip begins.
    pub t_start_ms: u64,
    /// Length of the source window to analyse.
    pub duration_ms: u64,
    /// Target aspect ratio (width / height). E.g. 9.0/16.0 for a
    /// vertical short-form output; the crop rectangle fits this.
    pub target_aspect: f64,
    /// Path to the face detector ONNX file on disk.
    pub model_path: std::path::PathBuf,
    /// True = SCRFD-500M (640x640, distance decode). False = YuNet
    /// (320x320, anchor decode).
    pub model_is_scrfd: bool,
    /// Frames per second to sample. Auto-reframe only needs a rough
    /// path, so 4 fps is plenty and keeps the job fast.
    pub sample_fps: f64,
    /// Downscale the longer side of each sampled frame to at most this
    /// before running the detector. YuNet runs at 320x320 internally;
    /// 480 gives it a little more detail than a bare 320 without
    /// dominating the job.
    pub max_side: u32,
    /// Inference backend: CPU, CUDA, DirectML, etc.
    pub backend: BackendType,
}

/// Result of an auto-reframe analysis job.
#[derive(Debug)]
pub struct ReframeResult {
    pub clip_id: uuid::Uuid,
    pub keypoints: Vec<caprust_core::clip::ReframeKeypoint>,
    pub frames_analyzed: usize,
    pub frames_with_face: usize,
}

/// Beat analysis request: which stored grid to (re)compute and from
/// which file. `key` is the media id (or clip id fallback) under
/// which the grid is stored in `ProjectState::beat_grids`.
pub struct BeatRequest {
    pub key: uuid::Uuid,
    pub source_path: std::path::PathBuf,
}

/// Result of a beat analysis job, including the file fingerprint the
/// grid was computed from (stale grids are recomputed on demand).
#[derive(Debug)]
pub struct BeatResult {
    pub key: uuid::Uuid,
    pub bpm: f32,
    pub beats_ms: Vec<u64>,
    pub len_bytes: u64,
    pub mtime_secs: u64,
}

/// Spawn a beat analysis job: decode the source to 8 kHz mono and run
/// onset-flux detection. Single result message; `ffmpeg` must exist —
/// caller verifies availability.
pub fn spawn_beat_job(
    ffmpeg: std::path::PathBuf,
    req: BeatRequest,
) -> std::sync::mpsc::Receiver<Result<BeatResult, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = run_beat_job(&ffmpeg, &req);
        let _ = tx.send(result);
    });
    rx
}

fn run_beat_job(ffmpeg: &std::path::Path, req: &BeatRequest) -> Result<BeatResult, String> {
    let samples = caprust_media_io::waveform::decode_mono_s16(ffmpeg, &req.source_path, 8000)
        .map_err(|e| e.to_string())?;
    let meta = std::fs::metadata(&req.source_path).map_err(|e| e.to_string())?;
    let mtime_secs = meta
        .modified()
        .map_err(|e| e.to_string())?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let grid = caprust_core::beat::detect_beats(&samples, 8000);
    Ok(BeatResult {
        key: req.key,
        bpm: grid.bpm,
        beats_ms: grid.beats_ms,
        len_bytes: meta.len(),
        mtime_secs,
    })
}

/// Spawn a background thread that runs the full auto-reframe pipeline:
/// probe the source for its native dimensions, extract sampled RGB
/// frames, run YuNet on each, and compute a keypoint path.
///
/// Returns immediately with a receiver. `ffmpeg` and `ffprobe` must be
/// Some -- caller is responsible for verifying availability.
pub fn spawn_reframe_job(
    ffmpeg: std::path::PathBuf,
    ffprobe: std::path::PathBuf,
    req: ReframeRequest,
) -> std::sync::mpsc::Receiver<Result<ReframeResult, String>> {
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::Builder::new()
        .name("caprust-reframe".into())
        // Whisper.cpp and tract are stack-hungry; the
        // Rust default 2 MB thread stack overflows on
        // Windows during inference. 32 MB is comfortable.
        .stack_size(32 * 1024 * 1024)
        // Whisper.cpp and tract are stack-hungry; the
        // Rust default 2 MB thread stack overflows on
        // Windows during inference. 32 MB is comfortable.
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let result = run_reframe_job(&ffmpeg, &ffprobe, &req);
            let _ = tx.send(result);
        })
        .expect("spawn reframe thread");

    rx
}

fn run_reframe_job(
    ffmpeg: &std::path::Path,
    ffprobe: &std::path::Path,
    req: &ReframeRequest,
) -> Result<ReframeResult, String> {
    // 1. Probe the source to learn its native dimensions.
    let probe = caprust_media_io::ffprobe::probe(ffprobe, &req.source_path)
        .map_err(|e| format!("probe {}: {e}", req.source_path.display()))?;
    if !probe.has_video {
        return Err("source has no video stream".into());
    }
    let src_w = probe
        .width
        .ok_or_else(|| "probe: missing width".to_string())?;
    let src_h = probe
        .height
        .ok_or_else(|| "probe: missing height".to_string())?;
    if src_w < 2 || src_h < 2 {
        return Err(format!("invalid source dimensions {src_w}x{src_h}"));
    }

    // 2. Extract sampled frames.
    let frames = caprust_media_io::frame_extract::extract_rgb_frames(
        ffmpeg,
        &req.source_path,
        src_w,
        src_h,
        req.t_start_ms as f64 / 1000.0,
        req.duration_ms as f64 / 1000.0,
        req.sample_fps,
        req.max_side,
    )
    .map_err(|e| format!("frame extraction: {e}"))?;

    if frames.is_empty() {
        return Ok(ReframeResult {
            clip_id: req.clip_id,
            keypoints: Vec::new(),
            frames_analyzed: 0,
            frames_with_face: 0,
        });
    }

    // 3. Load the detector and run it. Models are cached across jobs by
    //    path + mtime so repeated reframes don't reparse the ONNX graph.
    let samples = if req.model_is_scrfd {
        let key = req.model_path.to_string_lossy().into_owned();
        let detector = model_get_or_load(&SCRFD_CACHE, key, file_mtime(&req.model_path), || {
            caprust_media_io::scrfd::ScrfdDetector::load(
                &req.model_path,
                caprust_media_io::inference_backend::BackendType::Cpu,
            )
            .map_err(|e| format!("load SCRFD: {e}"))
        })?;
        let mut out = Vec::with_capacity(frames.len());
        for f in &frames {
            let best = detector
                .detect_best(&f.rgb, f.width, f.height)
                .map_err(|e| format!("SCRFD inference: {e}"))?;
            out.push((f.t_ms, best.map(|b| b.center())));
        }
        out
    } else {
        let key = req.model_path.to_string_lossy().into_owned();
        let detector = model_get_or_load(&YUNET_CACHE, key, file_mtime(&req.model_path), || {
            caprust_media_io::face_detect::FaceDetector::load(&req.model_path, req.backend)
                .map_err(|e| format!("load YuNet: {e}"))
        })?;
        caprust_media_io::auto_reframe::detect_face_centers(&frames, &detector)
            .map_err(|e| format!("face detection: {e}"))?
    };

    let frames_with_face = samples.iter().filter(|(_, c)| c.is_some()).count();

    // 4. Compute the keypoint path. Frame aspect is the source aspect
    //    (before our downscale, which preserves it).
    let frame_aspect = src_w as f64 / src_h as f64;
    let keypoints = caprust_media_io::auto_reframe::compute_keypoints_from_centers(
        &samples,
        frame_aspect,
        req.target_aspect,
    );

    tracing::info!(
        "reframe: clip {} -- {} frames, {} with face, {} keypoints",
        req.clip_id,
        frames.len(),
        frames_with_face,
        keypoints.len()
    );

    Ok(ReframeResult {
        clip_id: req.clip_id,
        keypoints,
        frames_analyzed: frames.len(),
        frames_with_face,
    })
}

// ---------------------------------------------------------------------------
// Background removal (Phase P3c-2)
// ---------------------------------------------------------------------------

/// Progress events emitted by a background-removal job. Modeled on
/// `caprust_core::models::DownloadEvent`: the worker sends these over
/// an mpsc channel, the UI drains them per frame.
#[derive(Debug)]
pub enum BgRemovalEvent {
    /// Emitted after probe + extraction, before the per-frame loop.
    Started { total_frames: usize, backend: BackendType },
    /// Emitted every N frames (see PROGRESS_STRIDE) so the UI does not
    /// repaint on every single frame.
    Progress { done: usize, total: usize },
    /// Terminal success. `mask_path` is the FFV1 MKV on disk.
    Finished {
        clip_id: uuid::Uuid,
        mask_path: std::path::PathBuf,
        frame_count: usize,
    },
    /// Terminal failure. `reason` is a user-readable string.
    Failed(String),
    /// Terminal cancellation: the user stopped the job mid-way.
    Cancelled { clip_id: uuid::Uuid },
}

/// Progress event stride: emit every Nth frame. A 30s / 30fps clip is
/// 900 frames; emitting 900 events would be wasteful. 25 keeps the
/// progress bar smooth at ~1.2 s per step on typical hardware.
pub const PROGRESS_STRIDE: usize = 5;

/// Request for a background-removal job. Everything the worker needs
/// to run end to end, with no back-reference into the project state.
#[derive(Debug)]
pub struct BgRemovalRequest {
    pub clip_id: uuid::Uuid,
    pub source_path: std::path::PathBuf,
    /// Offset inside the source where the clip begins.
    pub t_start_ms: u64,
    /// Length of the source window to process.
    pub duration_ms: u64,
    /// Frame rate to sample and encode at. Must match the project's
    /// frame rate so the mask aligns with the render output 1:1.
    pub fps: f64,
    /// Path to the u2netp ONNX model on disk.
    pub model_path: std::path::PathBuf,
    /// Where to write the FFV1 MKV mask. Resolved by the caller via
    /// `caprust_core::cache::mask_path`.
    pub mask_output: std::path::PathBuf,
    /// Downscale the longer side of each extracted frame to at most
    /// this before running inference. u2netp runs at 320x320
    /// internally, so 480 gives the mask a little more source detail
    /// than a bare 320 without dominating memory (30 s / 30 fps at
    /// 480p is ~350 MB of frame buffers).
    pub max_side: u32,
    /// Inference backend: CPU, CUDA, DirectML, etc.
    pub backend: BackendType,
    /// Cooperative cancellation flag. The UI keeps a clone and can stop
    /// the frame loop early (1.5).
    pub cancel: CancellationToken,
}

/// Spawn a background thread that runs the full background-removal
/// pipeline: probe source dimensions, extract sampled frames, run
/// u2netp on each, and write the per-frame alpha masks to a single
/// FFV1 Matroska file. Emits `BgRemovalEvent`s as it progresses.
///
/// Returns immediately with a receiver. `ffmpeg` and `ffprobe` must
/// be valid paths.
pub fn spawn_bg_removal_job(
    ffmpeg: std::path::PathBuf,
    ffprobe: std::path::PathBuf,
    req: BgRemovalRequest,
) -> std::sync::mpsc::Receiver<BgRemovalEvent> {
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::Builder::new()
        .name("caprust-bg-removal".into())
        // Whisper.cpp and tract are stack-hungry; the
        // Rust default 2 MB thread stack overflows on
        // Windows during inference. 32 MB is comfortable.
        .stack_size(32 * 1024 * 1024)
        // Whisper.cpp and tract are stack-hungry; the
        // Rust default 2 MB thread stack overflows on
        // Windows during inference. 32 MB is comfortable.
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            run_bg_removal_job(&ffmpeg, &ffprobe, &req, &tx);
        })
        .expect("spawn bg-removal thread");

    rx
}

fn run_bg_removal_job(
    ffmpeg: &std::path::Path,
    ffprobe: &std::path::Path,
    req: &BgRemovalRequest,
    tx: &std::sync::mpsc::Sender<BgRemovalEvent>,
) {
    if let Err(reason) = run_bg_removal_inner(ffmpeg, ffprobe, req, tx) {
        tracing::error!("bg-removal: {reason}");
        let _ = tx.send(BgRemovalEvent::Failed(reason));
    }
}

fn run_bg_removal_inner(
    ffmpeg: &std::path::Path,
    ffprobe: &std::path::Path,
    req: &BgRemovalRequest,
    tx: &std::sync::mpsc::Sender<BgRemovalEvent>,
) -> Result<(), String> {
    // 1. Probe for native dimensions.
    let probe = caprust_media_io::ffprobe::probe(ffprobe, &req.source_path)
        .map_err(|e| format!("probe {}: {e}", req.source_path.display()))?;
    if !probe.has_video {
        return Err("source has no video stream".into());
    }
    let src_w = probe
        .width
        .ok_or_else(|| "probe: missing width".to_string())?;
    let src_h = probe
        .height
        .ok_or_else(|| "probe: missing height".to_string())?;
    if src_w < 2 || src_h < 2 {
        return Err(format!("invalid source dimensions {src_w}x{src_h}"));
    }

    // 2. Extract sampled frames at the project's frame rate.
    let frames = caprust_media_io::frame_extract::extract_rgb_frames(
        ffmpeg,
        &req.source_path,
        src_w,
        src_h,
        req.t_start_ms as f64 / 1000.0,
        req.duration_ms as f64 / 1000.0,
        req.fps,
        req.max_side,
    )
    .map_err(|e| format!("frame extraction: {e}"))?;

    let total = frames.len();
    if total == 0 {
        return Err("no frames extracted".into());
    }
    tracing::info!(
        "bg-removal: clip {} -- {} frames to process",
        req.clip_id,
        total
    );

    // 3. Load the model once, cached across jobs by path + mtime.
    let key = req.model_path.to_string_lossy().into_owned();
    let model = model_get_or_load(&BG_CACHE, key, file_mtime(&req.model_path), || {
        caprust_media_io::background_removal::BackgroundRemover::load(&req.model_path, req.backend)
            .map_err(|e| format!("load u2netp: {e}"))
    })?;

    // Emit Started now that we know the actual backend (after CPU fallback).
    let _ = tx.send(BgRemovalEvent::Started {
        total_frames: total,
        backend: model.backend_type(),
    });

    // 4. Open the FFV1 writer on the output path.

    // 4. Open the FFV1 writer on the output path.
    let mut writer = spawn_mask_writer(ffmpeg, &req.mask_output, &frames[0], req.fps)?;

    // 5. Per-frame inference + write.
    for (i, frame) in frames.iter().enumerate() {
        if req.cancel.is_cancelled() {
            let _ = tx.send(BgRemovalEvent::Cancelled {
                clip_id: req.clip_id,
            });
            return Ok(());
        }
        let mask = model
            .infer_mask(&frame.rgb, frame.width, frame.height)
            .map_err(|e| format!("infer frame {i}: {e}"))?;
        writer
            .write_frame(&mask)
            .map_err(|e| format!("write frame {i}: {e}"))?;

        if (i + 1) % PROGRESS_STRIDE == 0 || i + 1 == total {
            tracing::info!("bg-removal: frame {}/{}", i + 1, total);
            let _ = tx.send(BgRemovalEvent::Progress { done: i + 1, total });
        }
    }

    // 6. Flush and close the writer.
    writer
        .finish()
        .map_err(|e| format!("flush mask writer: {e}"))?;

    let _ = tx.send(BgRemovalEvent::Finished {
        clip_id: req.clip_id,
        mask_path: req.mask_output.clone(),
        frame_count: total,
    });
    tracing::info!(
        "bg-removal: clip {} done -- {} frames -> {}",
        req.clip_id,
        total,
        req.mask_output.display()
    );
    Ok(())
}

/// MobileSAM segmentation (Phase K2 - Object Removal)
///
/// Progress events emitted by a MobileSAM segmentation job. Modeled on
/// `BgRemovalEvent`: the worker sends these over an mpsc channel,
/// the UI drains them per frame.
#[derive(Debug)]
pub enum SegmentEvent {
    /// Emitted after probe + extraction, before the per-frame loop.
    Started { total_frames: usize, backend: BackendType },
    /// Emitted every N frames (see PROGRESS_STRIDE).
    Progress { done: usize, total: usize },
    /// Terminal success. `mask_path` is the FFV1 MKV on disk.
    Finished {
        clip_id: uuid::Uuid,
        mask_path: std::path::PathBuf,
        frame_count: usize,
    },
    /// Terminal failure. `reason` is a user-readable string.
    Failed(String),
    /// Terminal cancellation: the user stopped the job mid-way.
    Cancelled { clip_id: uuid::Uuid },
}

/// Request for a MobileSAM segmentation job. Everything the worker needs
/// to run end to end, with no back-reference into the project state.
#[derive(Debug)]
pub struct SegmentRequest {
    pub clip_id: uuid::Uuid,
    pub source_path: std::path::PathBuf,
    /// Offset inside the source where the clip begins.
    pub t_start_ms: u64,
    /// Length of the source window to process.
    pub duration_ms: u64,
    /// Frame rate to sample and encode at. Must match the project's
    /// frame rate so the mask aligns with the render output 1:1.
    pub fps: f64,
    /// Path to the MobileSAM encoder ONNX model on disk.
    pub encoder_path: std::path::PathBuf,
    /// Path to the MobileSAM decoder ONNX model on disk.
    pub decoder_path: std::path::PathBuf,
    /// Where to write the FFV1 MKV mask. Resolved by the caller via
    /// `caprust_core::cache::mask_path`.
    pub mask_output: std::path::PathBuf,
    /// Click X coordinate in normalized coordinates [0, 1].
    pub click_x: f32,
    /// Click Y coordinate in normalized coordinates [0, 1].
    pub click_y: f32,
    /// Downscale the longer side of each extracted frame to at most
    /// this before running inference.
    pub max_side: u32,
    /// Inference backend: CPU, CUDA, DirectML, etc.
    pub backend: BackendType,
    /// Cooperative cancellation flag (1.5).
    pub cancel: CancellationToken,
}

/// Spawn a background thread that runs the full MobileSAM segmentation
/// pipeline: probe source dimensions, extract sampled frames, run
/// MobileSAM encoder+decoder on each with point prompt, and write the
/// per-frame binary masks to a single FFV1 Matroska file. Emits
/// `SegmentEvent`s as it progresses.
///
/// Returns immediately with a receiver. `ffmpeg` and `ffprobe` must
/// be valid paths.
pub fn spawn_segment_job(
    ffmpeg: std::path::PathBuf,
    ffprobe: std::path::PathBuf,
    req: SegmentRequest,
) -> std::sync::mpsc::Receiver<SegmentEvent> {
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::Builder::new()
        .name("caprust-segment".into())
        // tract inference is stack-hungry; 32 MB is comfortable.
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            if let Err(reason) = run_segment_inner(&ffmpeg, &ffprobe, &req, &tx) {
                tracing::error!("segment: {reason}");
                let _ = tx.send(SegmentEvent::Failed(reason));
            }
        })
        .expect("spawn segment thread");

    rx
}

fn run_segment_inner(
    ffmpeg: &std::path::Path,
    ffprobe: &std::path::Path,
    req: &SegmentRequest,
    tx: &std::sync::mpsc::Sender<SegmentEvent>,
) -> Result<(), String> {
    // 1. Probe for native dimensions.
    let probe = caprust_media_io::ffprobe::probe(ffprobe, &req.source_path)
        .map_err(|e| format!("probe {}: {e}", req.source_path.display()))?;
    if !probe.has_video {
        return Err("source has no video stream".into());
    }
    let src_w = probe
        .width
        .ok_or_else(|| "probe: missing width".to_string())?;
    let src_h = probe
        .height
        .ok_or_else(|| "probe: missing height".to_string())?;
    if src_w < 2 || src_h < 2 {
        return Err(format!("invalid source dimensions {src_w}x{src_h}"));
    }

    // 2. Extract sampled frames at the project's frame rate.
    let frames = caprust_media_io::frame_extract::extract_rgb_frames(
        ffmpeg,
        &req.source_path,
        src_w,
        src_h,
        req.t_start_ms as f64 / 1000.0,
        req.duration_ms as f64 / 1000.0,
        req.fps,
        req.max_side,
    )
    .map_err(|e| format!("frame extraction: {e}"))?;

    let total = frames.len();
    if total == 0 {
        return Err("no frames extracted".into());
    }
    tracing::info!(
        "segment: clip {} -- {} frames to process",
        req.clip_id,
        total
    );

    // 3. Load the MobileSAM model once, cached across jobs. The key and
    //    invalidation token cover both encoder and decoder files.
    let sam_key = format!(
        "{}|{}",
        req.encoder_path.to_string_lossy(),
        req.decoder_path.to_string_lossy()
    );
    let sam_mtime = match (file_mtime(&req.encoder_path), file_mtime(&req.decoder_path)) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    let encoder_path = req.encoder_path.clone();
    let decoder_path = req.decoder_path.clone();
    let model = model_get_or_load(&SAM_CACHE, sam_key, sam_mtime, || {
        caprust_media_io::mobile_sam::MobileSam::load(&encoder_path, &decoder_path, req.backend)
            .map_err(|e| format!("load MobileSAM: {e}"))
    })?;

    // Emit Started now that we know the actual backend (after CPU fallback).
    let _ = tx.send(SegmentEvent::Started {
        total_frames: total,
        backend: model.backend_type(),
    });

    // 4. Open the FFV1 writer on the output path.
    let mut writer = spawn_mask_writer(ffmpeg, &req.mask_output, &frames[0], req.fps)?;

    // 5. Per-frame inference + write.
    for (i, frame) in frames.iter().enumerate() {
        if req.cancel.is_cancelled() {
            let _ = tx.send(SegmentEvent::Cancelled {
                clip_id: req.clip_id,
            });
            return Ok(());
        }
        let mask = model
            .segment_point(
                &frame.rgb,
                frame.width,
                frame.height,
                req.click_x,
                req.click_y,
            )
            .map_err(|e| format!("infer frame {i}: {e}"))?;
        writer
            .write_frame(&mask)
            .map_err(|e| format!("write frame {i}: {e}"))?;

        if (i + 1) % PROGRESS_STRIDE == 0 || i + 1 == total {
            tracing::info!("segment: frame {}/{}", i + 1, total);
            let _ = tx.send(SegmentEvent::Progress { done: i + 1, total });
        }
    }

    // 6. Flush and close the writer.
    writer
        .finish()
        .map_err(|e| format!("flush mask writer: {e}"))?;

    let _ = tx.send(SegmentEvent::Finished {
        clip_id: req.clip_id,
        mask_path: req.mask_output.clone(),
        frame_count: total,
    });
    tracing::info!(
        "segment: clip {} done -- {} frames -> {}",
        req.clip_id,
        total,
        req.mask_output.display()
    );
    Ok(())
}

/// LaMa inpainting (Phase K2 - Object Removal)
///
/// Progress events emitted by a LaMa inpainting job. Modeled on
/// `BgRemovalEvent`: the worker sends these over an mpsc channel,
/// the UI drains them per frame.
#[derive(Debug)]
pub enum InpaintEvent {
    /// Emitted after probe + extraction, before the per-frame loop.
    Started { total_frames: usize, backend: BackendType },
    /// Emitted every N frames (see PROGRESS_STRIDE).
    Progress { done: usize, total: usize },
    /// Terminal success. `output_path` is the inpainted video on disk.
    Finished {
        clip_id: uuid::Uuid,
        output_path: std::path::PathBuf,
        frame_count: usize,
    },
    /// Terminal failure. `reason` is a user-readable string.
    Failed(String),
    /// Terminal cancellation: the user stopped the job mid-way.
    Cancelled { clip_id: uuid::Uuid },
}

/// Request for a LaMa inpainting job. Everything the worker needs
/// to run end to end, with no back-reference into the project state.
#[derive(Debug)]
pub struct InpaintRequest {
    pub clip_id: uuid::Uuid,
    pub source_path: std::path::PathBuf,
    /// Offset inside the source where the clip begins.
    pub t_start_ms: u64,
    /// Length of the source window to process.
    pub duration_ms: u64,
    /// Frame rate to sample and encode at. Must match the project's
    /// frame rate so the output aligns with the render output 1:1.
    pub fps: f64,
    /// Path to the LaMa ONNX model on disk.
    pub model_path: std::path::PathBuf,
    /// Path to the mask video (FFV1 MKV) generated by MobileSAM.
    pub mask_path: std::path::PathBuf,
    /// Where to write the inpainted video.
    pub output_path: std::path::PathBuf,
    /// Downscale the longer side of each extracted frame to at most
    /// this before running inference. LaMa runs at 512x512
    /// internally.
    pub max_side: u32,
    /// Inference backend: CPU, CUDA, DirectML, etc.
    pub backend: BackendType,
    /// Cooperative cancellation flag (1.5).
    pub cancel: CancellationToken,
}

/// Spawn a background thread that runs the full LaMa inpainting
/// pipeline: probe source dimensions, extract sampled frames + masks,
/// run LaMa on each, and write the inpainted frames to a video file.
/// Emits `InpaintEvent`s as it progresses.
///
/// Returns immediately with a receiver. `ffmpeg` and `ffprobe` must
/// be valid paths.
pub fn spawn_inpaint_job(
    ffmpeg: std::path::PathBuf,
    ffprobe: std::path::PathBuf,
    req: InpaintRequest,
) -> std::sync::mpsc::Receiver<InpaintEvent> {
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::Builder::new()
        .name("caprust-inpaint".into())
        // tract inference is stack-hungry; 32 MB is comfortable.
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            if let Err(reason) = run_inpaint_inner(&ffmpeg, &ffprobe, &req, &tx) {
                tracing::error!("inpaint: {reason}");
                let _ = tx.send(InpaintEvent::Failed(reason));
            }
        })
        .expect("spawn inpaint thread");

    rx
}

fn run_inpaint_inner(
    ffmpeg: &std::path::Path,
    ffprobe: &std::path::Path,
    req: &InpaintRequest,
    tx: &std::sync::mpsc::Sender<InpaintEvent>,
) -> Result<(), String> {
    // 1. Probe for native dimensions.
    let probe = caprust_media_io::ffprobe::probe(ffprobe, &req.source_path)
        .map_err(|e| format!("probe {}: {e}", req.source_path.display()))?;
    if !probe.has_video {
        return Err("source has no video stream".into());
    }
    let src_w = probe
        .width
        .ok_or_else(|| "probe: missing width".to_string())?;
    let src_h = probe
        .height
        .ok_or_else(|| "probe: missing height".to_string())?;
    if src_w < 2 || src_h < 2 {
        return Err(format!("invalid source dimensions {src_w}x{src_h}"));
    }

    // 2. Extract sampled frames at the project's frame rate.
    let frames = caprust_media_io::frame_extract::extract_rgb_frames(
        ffmpeg,
        &req.source_path,
        src_w,
        src_h,
        req.t_start_ms as f64 / 1000.0,
        req.duration_ms as f64 / 1000.0,
        req.fps,
        req.max_side,
    )
    .map_err(|e| format!("frame extraction: {e}"))?;

    // 3. Extract mask frames (grayscale) from the mask video.
    let mask_frames = caprust_media_io::frame_extract::extract_gray_frames(
        ffmpeg,
        &req.mask_path,
        src_w,
        src_h,
        req.t_start_ms as f64 / 1000.0,
        req.duration_ms as f64 / 1000.0,
        req.fps,
        req.max_side,
    )
    .map_err(|e| format!("mask frame extraction: {e}"))?;

    let total = frames.len();
    if total == 0 {
        return Err("no frames extracted".into());
    }
    if mask_frames.len() != total {
        return Err(format!(
            "frame count mismatch: {} video frames vs {} mask frames",
            total,
            mask_frames.len()
        ));
    }
    tracing::info!(
        "inpaint: clip {} -- {} frames to process",
        req.clip_id,
        total
    );

    // 4. Load the LaMa model once, cached across jobs by path + mtime.
    let key = req.model_path.to_string_lossy().into_owned();
    let model = model_get_or_load(&LAMA_CACHE, key, file_mtime(&req.model_path), || {
        caprust_media_io::lama::Lama::load(&req.model_path, req.backend)
            .map_err(|e| format!("load LaMa: {e}"))
    })?;

    // Emit Started now that we know the actual backend (after CPU fallback).
    let _ = tx.send(InpaintEvent::Started {
        total_frames: total,
        backend: model.backend_type(),
    });
    let mut writer = spawn_video_writer(ffmpeg, &req.output_path, &frames[0], req.fps)?;

    // 6. Per-frame inpainting + write.
    for (i, (frame, mask)) in frames.iter().zip(mask_frames.iter()).enumerate() {
        if req.cancel.is_cancelled() {
            let _ = tx.send(InpaintEvent::Cancelled {
                clip_id: req.clip_id,
            });
            return Ok(());
        }
        let inpainted = model
            .inpaint(&frame.rgb, &mask.gray, frame.width, frame.height)
            .map_err(|e| format!("inpaint frame {i}: {e}"))?;
        writer
            .write_frame(&inpainted)
            .map_err(|e| format!("write frame {i}: {e}"))?;

        if (i + 1) % PROGRESS_STRIDE == 0 || i + 1 == total {
            tracing::info!("inpaint: frame {}/{}", i + 1, total);
            let _ = tx.send(InpaintEvent::Progress { done: i + 1, total });
        }
    }

    // 7. Flush and close the writer.
    writer
        .finish()
        .map_err(|e| format!("flush video writer: {e}"))?;

    let _ = tx.send(InpaintEvent::Finished {
        clip_id: req.clip_id,
        output_path: req.output_path.clone(),
        frame_count: total,
    });
    tracing::info!(
        "inpaint: clip {} done -- {} frames -> {}",
        req.clip_id,
        total,
        req.output_path.display()
    );
    Ok(())
}

/// Request for a face-anonymize job. Mirrors `BgRemovalRequest`:
/// detect faces per extracted frame with YuNet, paint expanded
/// filled boxes onto a black canvas, encode the sequence as FFV1.
/// Everything the worker needs, no back-reference into the project.
#[derive(Debug)]
pub struct AnonRequest {
    pub clip_id: uuid::Uuid,
    pub source_path: std::path::PathBuf,
    /// Offset inside the source where the clip begins.
    pub t_start_ms: u64,
    /// Length of the source window to process.
    pub duration_ms: u64,
    /// Frame rate to sample and encode at. Must match the project's
    /// frame rate so the mask aligns with the render output 1:1.
    pub fps: f64,
    /// Path to the face detector ONNX model on disk.
    pub model_path: std::path::PathBuf,
    /// True = SCRFD model (working decoder, preferred). False = YuNet
    /// (its 2022mar decoder is still a stub returning no boxes — kept
    /// as a fallback so a YuNet-only setup degrades to empty masks
    /// instead of erroring).
    pub use_scrfd: bool,
    /// Where to write the FFV1 MKV mask. Resolved by the caller via
    /// `caprust_core::cache::anon_path`.
    pub mask_output: std::path::PathBuf,
    /// Downscale the longer side of each extracted frame to at most
    /// this before running inference. YuNet runs at 320x320
    /// internally, so 480 keeps box precision without dominating
    /// memory.
    pub max_side: u32,
    /// Inference backend: CPU, CUDA, DirectML, etc.
    pub backend: BackendType,
    /// Cooperative cancellation flag (1.5).
    pub cancel: CancellationToken,
}

/// Progress events emitted by a face-anonymize job. Same shape as
/// `BgRemovalEvent`.
#[derive(Debug)]
pub enum AnonEvent {
    /// Emitted after probe + extraction, before the per-frame loop.
    Started { total_frames: usize },
    /// Emitted every N frames (see PROGRESS_STRIDE).
    Progress { done: usize, total: usize },
    /// Terminal success. `mask_path` is the FFV1 MKV on disk.
    Finished {
        clip_id: uuid::Uuid,
        mask_path: std::path::PathBuf,
        frame_count: usize,
    },
    /// Terminal failure. `reason` is a user-readable string.
    Failed(String),
    /// Terminal cancellation: the user stopped the job mid-way.
    Cancelled { clip_id: uuid::Uuid },
}

/// Spawn a background thread that runs the full anonymize pipeline.
/// Returns immediately with a receiver. `ffmpeg` and `ffprobe` must
/// be valid paths.
pub fn spawn_anon_job(
    ffmpeg: std::path::PathBuf,
    ffprobe: std::path::PathBuf,
    req: AnonRequest,
) -> std::sync::mpsc::Receiver<AnonEvent> {
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::Builder::new()
        .name("caprust-anon".into())
        // Same stack-hungry tract inference as the other jobs.
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            if let Err(reason) = run_anon_inner(&ffmpeg, &ffprobe, &req, &tx) {
                tracing::error!("anon: {reason}");
                let _ = tx.send(AnonEvent::Failed(reason));
            }
        })
        .expect("spawn anon thread");

    rx
}

fn run_anon_inner(
    ffmpeg: &std::path::Path,
    ffprobe: &std::path::Path,
    req: &AnonRequest,
    tx: &std::sync::mpsc::Sender<AnonEvent>,
) -> Result<(), String> {
    // 1. Probe for native dimensions.
    let probe = caprust_media_io::ffprobe::probe(ffprobe, &req.source_path)
        .map_err(|e| format!("probe {}: {e}", req.source_path.display()))?;
    if !probe.has_video {
        return Err("source has no video stream".into());
    }
    let src_w = probe
        .width
        .ok_or_else(|| "probe: missing width".to_string())?;
    let src_h = probe
        .height
        .ok_or_else(|| "probe: missing height".to_string())?;
    if src_w < 2 || src_h < 2 {
        return Err(format!("invalid source dimensions {src_w}x{src_h}"));
    }

    // 2. Extract sampled frames at the project's frame rate.
    let frames = caprust_media_io::frame_extract::extract_rgb_frames(
        ffmpeg,
        &req.source_path,
        src_w,
        src_h,
        req.t_start_ms as f64 / 1000.0,
        req.duration_ms as f64 / 1000.0,
        req.fps,
        req.max_side,
    )
    .map_err(|e| format!("frame extraction: {e}"))?;

    let total = frames.len();
    if total == 0 {
        return Err("no frames extracted".into());
    }
    let _ = tx.send(AnonEvent::Started {
        total_frames: total,
    });

    // 3. Load the face detector once. SCRFD has a real decoder and
    // is preferred (same preference as auto-reframe); YuNet 2022mar
    // decode is still a stub returning no boxes, kept as fallback.
    enum Detector {
        Scrfd(Arc<caprust_media_io::scrfd::ScrfdDetector>),
        YuNet(Arc<caprust_media_io::face_detect::FaceDetector>),
    }
    let key = req.model_path.to_string_lossy().into_owned();
    let mtime = file_mtime(&req.model_path);
    let detector = if req.use_scrfd {
        let d = model_get_or_load(&SCRFD_CACHE, key, mtime, || {
            caprust_media_io::scrfd::ScrfdDetector::load(
                &req.model_path,
                caprust_media_io::inference_backend::BackendType::Cpu,
            )
            .map_err(|e| format!("load SCRFD: {e}"))
        })?;
        Detector::Scrfd(d)
    } else {
        let d = model_get_or_load(&YUNET_CACHE, key, mtime, || {
            caprust_media_io::face_detect::FaceDetector::load(&req.model_path, req.backend)
                .map_err(|e| format!("load YuNet: {e}"))
        })?;
        Detector::YuNet(d)
    };

    // 4. Open the FFV1 writer on the output path.
    let mut writer = spawn_mask_writer(ffmpeg, &req.mask_output, &frames[0], req.fps)?;

    // 5. Per-frame detection + box paint.
    for (i, frame) in frames.iter().enumerate() {
        if req.cancel.is_cancelled() {
            let _ = tx.send(AnonEvent::Cancelled {
                clip_id: req.clip_id,
            });
            return Ok(());
        }
        let boxes: Vec<(f32, f32, f32, f32)> = match &detector {
            Detector::Scrfd(d) => d
                .detect(&frame.rgb, frame.width, frame.height)
                .map_err(|e| format!("detect frame {i}: {e}"))?
                .into_iter()
                .map(|b| (b.x, b.y, b.w, b.h))
                .collect(),
            Detector::YuNet(d) => d
                .detect(&frame.rgb, frame.width, frame.height)
                .map_err(|e| format!("detect frame {i}: {e}"))?
                .into_iter()
                .map(|b| (b.x, b.y, b.w, b.h))
                .collect(),
        };
        let mask = paint_face_boxes(&boxes, frame.width, frame.height);
        writer
            .write_frame(&mask)
            .map_err(|e| format!("write frame {i}: {e}"))?;

        if (i + 1) % PROGRESS_STRIDE == 0 || i + 1 == total {
            let _ = tx.send(AnonEvent::Progress { done: i + 1, total });
        }
    }

    // 6. Flush and close the writer.
    writer
        .finish()
        .map_err(|e| format!("flush mask writer: {e}"))?;

    let _ = tx.send(AnonEvent::Finished {
        clip_id: req.clip_id,
        mask_path: req.mask_output.clone(),
        frame_count: total,
    });
    Ok(())
}

/// Paint detected face boxes onto a black canvas (white = blur here).
/// Boxes are expanded ~25% so hairline and chin stay covered, then
/// clamped to the frame. Frames with no faces stay black.
fn paint_face_boxes(boxes: &[(f32, f32, f32, f32)], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let mut mask = vec![0u8; w * h];
    for &(bx, by, bw, bh) in boxes {
        let pad_w = bw * 0.25;
        let pad_h = bh * 0.25;
        let x0 = ((bx - pad_w).max(0.0) * width as f32) as usize;
        let y0 = ((by - pad_h).max(0.0) * height as f32) as usize;
        let x1 = ((bx + bw + pad_w).min(1.0) * width as f32) as usize;
        let y1 = ((by + bh + pad_h).min(1.0) * height as f32) as usize;
        for y in y0..y1.min(h) {
            for x in x0..x1.min(w) {
                mask[y * w + x] = 255;
            }
        }
    }
    mask
}

/// Subprocess wrapper for the ffmpeg that writes the mask sequence.
/// Raw grayscale frames are piped on stdin; ffmpeg encodes them as
/// FFV1 in Matroska, lossless, single channel.
struct MaskWriter {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    frame_bytes: usize,
}

impl MaskWriter {
    fn write_frame(&mut self, mask: &[u8]) -> std::io::Result<()> {
        debug_assert_eq!(mask.len(), self.frame_bytes);
        std::io::Write::write_all(&mut self.stdin, mask)
    }

    fn finish(mut self) -> Result<(), String> {
        // Drop stdin first so ffmpeg sees EOF and can finalize the
        // container. Then wait for the process.
        drop(self.stdin);
        let status = self
            .child
            .wait()
            .map_err(|e| format!("wait ffmpeg mask writer: {e}"))?;
        if !status.success() {
            return Err(format!("ffmpeg mask writer exited {status}"));
        }
        Ok(())
    }
}

fn spawn_mask_writer(
    ffmpeg: &std::path::Path,
    output: &std::path::Path,
    first_frame: &caprust_media_io::frame_extract::RgbFrame,
    fps: f64,
) -> Result<MaskWriter, String> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let w = first_frame.width;
    let h = first_frame.height;
    let args: Vec<String> = vec![
        "-y".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        "gray".into(),
        "-s".into(),
        format!("{w}x{h}"),
        "-r".into(),
        format!("{fps:.6}"),
        "-i".into(),
        "-".into(),
        "-c:v".into(),
        "ffv1".into(),
        "-pix_fmt".into(),
        "gray".into(),
        output.to_string_lossy().into_owned(),
    ];
    tracing::debug!("bg-removal: mask writer args: {:?}", args);
    let mut child = caprust_media_io::silent_cmd::silent_command(ffmpeg)
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn ffmpeg mask writer: {e}"))?;

    if let Some(mut err) = child.stderr.take() {
        std::thread::spawn(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(&mut err);
            for line in reader.lines().map_while(std::result::Result::ok) {
                if !line.trim().is_empty() {
                    tracing::warn!("bg-removal ffmpeg: {line}");
                }
            }
        });
    }

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "ffmpeg mask writer: stdin missing".to_string())?;

    Ok(MaskWriter {
        child,
        stdin,
        frame_bytes: (w as usize) * (h as usize),
    })
}

/// Video writer for inpainted frames (color output).
pub struct VideoWriter {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    frame_bytes: usize,
}

impl VideoWriter {
    pub fn write_frame(&mut self, rgb: &[u8]) -> Result<(), String> {
        use std::io::Write;
        if rgb.len() != self.frame_bytes {
            return Err(format!(
                "frame size mismatch: expected {}, got {}",
                self.frame_bytes,
                rgb.len()
            ));
        }
        self.stdin
            .write_all(rgb)
            .map_err(|e| format!("write frame to ffmpeg: {e}"))?;
        Ok(())
    }

    pub fn finish(mut self) -> Result<(), String> {
        drop(self.stdin);
        let status = self
            .child
            .wait()
            .map_err(|e| format!("wait ffmpeg video writer: {e}"))?;
        if !status.success() {
            return Err(format!("ffmpeg video writer exited {}", status));
        }
        Ok(())
    }
}

fn spawn_video_writer(
    ffmpeg: &std::path::Path,
    output: &std::path::Path,
    first_frame: &caprust_media_io::frame_extract::RgbFrame,
    fps: f64,
) -> Result<VideoWriter, String> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let w = first_frame.width;
    let h = first_frame.height;
    let args: Vec<String> = vec![
        "-y".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        "rgb24".into(),
        "-s".into(),
        format!("{w}x{h}"),
        "-r".into(),
        format!("{fps:.6}"),
        "-i".into(),
        "-".into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-crf".into(),
        "23".into(),
        output.to_string_lossy().into_owned(),
    ];
    tracing::debug!("inpaint: video writer args: {:?}", args);
    let mut child = caprust_media_io::silent_cmd::silent_command(ffmpeg)
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn ffmpeg video writer: {e}"))?;

    if let Some(mut err) = child.stderr.take() {
        std::thread::spawn(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(&mut err);
            for line in reader.lines().map_while(std::result::Result::ok) {
                if !line.trim().is_empty() {
                    tracing::warn!("inpaint ffmpeg: {line}");
                }
            }
        });
    }

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "ffmpeg video writer: stdin missing".to_string())?;

    Ok(VideoWriter {
        child,
        stdin,
        frame_bytes: (w as usize) * (h as usize) * 3,
    })
}

// ---------------------------------------------------------------------------
// Stabilization pass 1 (vidstabdetect)
// ---------------------------------------------------------------------------

/// Progress event stride for the detect pass: detect runs at 20-50x
/// realtime, so per-frame events would spam the channel. 30 keeps the
/// bar smooth without repaint pressure.
pub const STAB_PROGRESS_STRIDE: usize = 30;

/// Request for a stabilization detect job. Everything the worker needs
/// to run end to end, with no back-reference into the project state.
/// The source window must be the clip's own window at 1.0x speed; the
/// caller guarantees speed == 1.0 and !reversed (transforms measured
/// otherwise would not align with the rendered frames).
#[derive(Debug)]
pub struct StabRequest {
    pub clip_id: uuid::Uuid,
    pub source_path: PathBuf,
    /// Source-time offset where the clip's visible window begins.
    pub t_start_ms: u64,
    /// Source-time length to analyze.
    pub duration_ms: u64,
    /// Project frame rate, used only to estimate total frames for the
    /// progress bar.
    pub fps: f64,
    /// Where to write the .trf file. Resolved by the caller via
    /// `caprust_core::cache::stab_path`.
    pub trf_output: PathBuf,
}

/// Progress events emitted by a stabilization detect job. Same shape
/// as `BgRemovalEvent`: the worker sends these over an mpsc channel,
/// the UI drains them per frame.
#[derive(Debug)]
pub enum StabEvent {
    /// Emitted before spawning ffmpeg.
    Started { total_frames: usize },
    /// Emitted every STAB_PROGRESS_STRIDE frames.
    Progress { done: usize, total: usize },
    /// Terminal success. `trf_path` is the transforms file on disk.
    Finished {
        clip_id: uuid::Uuid,
        trf_path: PathBuf,
        frame_count: usize,
    },
    /// Terminal failure. `reason` is a user-readable string.
    Failed(String),
}

/// Spawn a background thread that runs `vidstabdetect` over the
/// clip's source window and writes the transforms file. Returns
/// immediately with a receiver. `ffmpeg` must be a valid path to a
/// build with the vidstab filters (our BtbN GPL build has both).
pub fn spawn_stab_job(ffmpeg: PathBuf, req: StabRequest) -> std::sync::mpsc::Receiver<StabEvent> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("caprust-stab-detect".into())
        .spawn(move || {
            run_stab_job(&ffmpeg, &req, &tx);
        })
        .expect("spawn stab-detect thread");
    rx
}

fn run_stab_job(
    ffmpeg: &std::path::Path,
    req: &StabRequest,
    tx: &std::sync::mpsc::Sender<StabEvent>,
) {
    if let Err(reason) = run_stab_inner(ffmpeg, req, tx) {
        tracing::error!("stab-detect: {reason}");
        let _ = tx.send(StabEvent::Failed(reason));
    }
}

/// Escape a path for use as a `vidstabdetect=result=` /
/// `vidstabtransform=input=` filter option: forward slashes, the
/// drive colon backslash-escaped, the whole thing single-quoted.
/// Verified live against the gyan.dev essentials build:
/// `result='C\:/.../x.trf'`. Returns None when the path contains a
/// single quote, which no quoting survives.
fn escape_vidstab_path(path: &std::path::Path) -> Option<String> {
    let s = path.to_string_lossy();
    if s.contains('\'') {
        return None;
    }
    let fwd = s.replace('\\', "/");
    Some(format!("'{}'", fwd.replace(':', "\\:")))
}

fn run_stab_inner(
    ffmpeg: &std::path::Path,
    req: &StabRequest,
    tx: &std::sync::mpsc::Sender<StabEvent>,
) -> Result<(), String> {
    if req.duration_ms == 0 {
        return Err("clip has zero duration".into());
    }
    let result = escape_vidstab_path(&req.trf_output)
        .ok_or_else(|| "trf path cannot be quoted".to_string())?;
    if let Some(parent) = req.trf_output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create stab cache dir: {e}"))?;
    }
    let start_sec = req.t_start_ms as f64 / 1000.0;
    let dur_sec = req.duration_ms as f64 / 1000.0;
    let total = (req.fps * dur_sec).ceil().max(1.0) as usize;
    let _ = tx.send(StabEvent::Started {
        total_frames: total,
    });

    let mut child = caprust_media_io::silent_cmd::silent_command(ffmpeg)
        .arg("-hide_banner")
        .arg("-ss")
        .arg(format!("{start_sec:.3}"))
        .arg("-i")
        .arg(&req.source_path)
        .arg("-t")
        .arg(format!("{dur_sec:.3}"))
        .arg("-vf")
        .arg(format!("vidstabdetect=result={result}"))
        .arg("-an")
        .arg("-f")
        .arg("null")
        .arg("-")
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn ffmpeg vidstabdetect: {e}"))?;

    // Parse `frame= 123` progress out of the stats stream. Stats lines
    // end with \r, trailers with \n, so split on both.
    let mut done = 0usize;
    let mut emitted = 0usize;
    if let Some(stderr) = child.stderr.take() {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(stderr);
        for seg in reader.split(b'\r') {
            let Ok(bytes) = seg else { break };
            let text = String::from_utf8_lossy(&bytes);
            for line in text.split('\n') {
                if let Some(rest) = line.trim().strip_prefix("frame=") {
                    if let Ok(n) = rest.trim().parse::<usize>() {
                        done = n;
                        if done.saturating_sub(emitted) >= STAB_PROGRESS_STRIDE || done >= total {
                            emitted = done;
                            let _ = tx.send(StabEvent::Progress { done, total });
                        }
                    }
                }
            }
        }
    }
    let status = child
        .wait()
        .map_err(|e| format!("wait vidstabdetect: {e}"))?;
    if !status.success() {
        return Err(format!("vidstabdetect exited with {status}"));
    }
    let len = std::fs::metadata(&req.trf_output)
        .map(|m| m.len())
        .map_err(|e| format!("trf file missing after detect: {e}"))?;
    if len == 0 {
        return Err("vidstabdetect wrote an empty transforms file".into());
    }
    let _ = tx.send(StabEvent::Finished {
        clip_id: req.clip_id,
        trf_path: req.trf_output.clone(),
        frame_count: done.max(1),
    });
    Ok(())
}

// --- Real GFPGAN face restoration pre-pass ----------------------------
//
// Runs SCRFD + GFPGAN over every frame of a clip's source window and
// writes an FFV1 MKV to the project cache. Export and preview then use
// that file as the clip's input, so inference happens once per frame
// in a cancellable job rather than on every preview tick.

static GFPGAN_CACHE: ModelSlot<caprust_media_io::face_restore::FaceRestorer> = OnceLock::new();

pub struct GfpganRequest {
    pub clip_id: uuid::Uuid,
    pub source_path: PathBuf,
    pub t_start_ms: u64,
    pub duration_ms: u64,
    pub fps: f64,
    pub detector_path: PathBuf,
    pub restorer_path: PathBuf,
    pub output_path: PathBuf,
    pub max_side: u32,
    pub cancel: CancellationToken,
    /// Inference backend: CPU (tract-onnx), CUDA (NVIDIA GPU), or
    /// DirectML (AMD GPU). Defaults to CPU.
    pub backend: caprust_media_io::inference_backend::BackendType,
}

#[derive(Debug)]
pub enum GfpganEvent {
    Started {
        total_frames: usize,
        /// The inference backend actually in use (after Auto resolution
        /// and any CPU fallback).
        backend: caprust_media_io::inference_backend::BackendType,
    },
    Progress {
        done: usize,
        total: usize,
    },
    Finished {
        clip_id: uuid::Uuid,
        output_path: PathBuf,
        frame_count: usize,
    },
    Failed(String),
    /// Emitted when the requested GPU backend failed to load and the job
    /// fell back to CPU. Helps users understand why GPU acceleration
    /// appears inactive.
    FallbackToCpu {
        requested: caprust_media_io::inference_backend::BackendType,
    },
}

pub fn spawn_gfpgan_job(
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    req: GfpganRequest,
) -> Receiver<GfpganEvent> {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("caprust-gfpgan".into())
        // tract inference is stack-hungry on Windows; see other AI jobs.
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            if let Err(reason) = run_gfpgan_inner(&ffmpeg, &ffprobe, &req, &tx) {
                tracing::error!("gfpgan job: {reason}");
                let _ = tx.send(GfpganEvent::Failed(reason));
            }
        })
        .expect("spawn gfpgan thread");
    rx
}

/// FFV1/RGB writer for restored frames. FFV1 is lossless so the AI
/// work is not degraded before the export encoder sees it.
struct RgbWriter {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    frame_bytes: usize,
}

impl RgbWriter {
    fn write_frame(&mut self, rgb: &[u8]) -> std::io::Result<()> {
        debug_assert_eq!(rgb.len(), self.frame_bytes);
        std::io::Write::write_all(&mut self.stdin, rgb)
    }

    fn finish(mut self) -> Result<(), String> {
        drop(self.stdin);
        let status = self
            .child
            .wait()
            .map_err(|e| format!("wait ffmpeg gfpgan writer: {e}"))?;
        if !status.success() {
            return Err(format!("ffmpeg gfpgan writer exited {status}"));
        }
        Ok(())
    }
}

fn spawn_rgb_writer(
    ffmpeg: &Path,
    output: &Path,
    w: u32,
    h: u32,
    fps: f64,
) -> Result<RgbWriter, String> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let args: Vec<String> = vec![
        "-y".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        "rgb24".into(),
        "-s".into(),
        format!("{w}x{h}"),
        "-r".into(),
        format!("{fps:.6}"),
        "-i".into(),
        "-".into(),
        "-an".into(),
        "-c:v".into(),
        "ffv1".into(),
        "-pix_fmt".into(),
        "rgb24".into(),
        output.to_string_lossy().into_owned(),
    ];
    let mut child = caprust_media_io::silent_cmd::silent_command(ffmpeg)
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn ffmpeg gfpgan writer: {e}"))?;
    if let Some(mut err) = child.stderr.take() {
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(&mut err)
                .lines()
                .map_while(std::result::Result::ok)
            {
                if !line.trim().is_empty() {
                    tracing::warn!("gfpgan ffmpeg: {line}");
                }
            }
        });
    }
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "ffmpeg gfpgan writer: stdin missing".to_string())?;
    Ok(RgbWriter {
        child,
        stdin,
        frame_bytes: (w as usize) * (h as usize) * 3,
    })
}

fn run_gfpgan_inner(
    ffmpeg: &Path,
    ffprobe: &Path,
    req: &GfpganRequest,
    tx: &Sender<GfpganEvent>,
) -> Result<(), String> {
    let probe = caprust_media_io::ffprobe::probe(ffprobe, &req.source_path)
        .map_err(|e| format!("probe {}: {e}", req.source_path.display()))?;
    if !probe.has_video {
        return Err("source has no video stream".into());
    }
    let src_w = probe
        .width
        .ok_or_else(|| "probe: missing width".to_string())?;
    let src_h = probe
        .height
        .ok_or_else(|| "probe: missing height".to_string())?;

    let frames = caprust_media_io::frame_extract::extract_rgb_frames(
        ffmpeg,
        &req.source_path,
        src_w,
        src_h,
        req.t_start_ms as f64 / 1000.0,
        req.duration_ms as f64 / 1000.0,
        req.fps,
        req.max_side,
    )
    .map_err(|e| format!("frame extraction: {e}"))?;
    let total = frames.len();
    if total == 0 {
        return Err("no frames extracted".into());
    }
    let key = format!(
        "{}|{}",
        req.detector_path.to_string_lossy(),
        req.restorer_path.to_string_lossy()
    );
    let restorer = model_get_or_load(&GFPGAN_CACHE, key, file_mtime(&req.restorer_path), || {
        caprust_media_io::face_restore::FaceRestorer::load(
            &req.detector_path,
            &req.restorer_path,
            req.backend,
        )
        .map_err(|e| format!("load GFPGAN: {e}"))
    })?;
    let restorer = restorer.as_ref();

    // Report which backend is actually in use (may differ from requested
    // if CUDA/DirectML failed to load and fell back to CPU).
    let _ = tx.send(GfpganEvent::Started {
        total_frames: total,
        backend: restorer.backend_type(),
    });

    // Warn if the requested backend was not actually used (CPU fallback).
    if restorer.backend_type() != req.backend {
        let _ = tx.send(GfpganEvent::FallbackToCpu {
            requested: req.backend,
        });
        tracing::warn!(
            "gfpgan: requested {:?} backend but fell back to CPU",
            req.backend
        );
    }

    let (fw, fh) = (frames[0].width, frames[0].height);
    let mut writer = spawn_rgb_writer(ffmpeg, &req.output_path, fw, fh, req.fps)?;
    let mut processed = 0usize;
    for frame in &frames {
        if req.cancel.is_cancelled() {
            break;
        }
        let restored = restorer
            .restore_frame(&frame.rgb, frame.width, frame.height)
            .map_err(|e| format!("restore frame {processed}: {e}"))?;
        writer
            .write_frame(&restored)
            .map_err(|e| format!("write frame {processed}: {e}"))?;
        processed += 1;
        if processed.is_multiple_of(PROGRESS_STRIDE) || processed == total {
            let _ = tx.send(GfpganEvent::Progress {
                done: processed,
                total,
            });
        }
    }
    writer.finish()?;
    if processed == 0 {
        return Err("cancelled before the first frame".into());
    }
    let _ = tx.send(GfpganEvent::Finished {
        clip_id: req.clip_id,
        output_path: req.output_path.clone(),
        frame_count: processed,
    });
    tracing::info!("gfpgan: clip {} restored {processed} frames", req.clip_id);
    Ok(())
}

#[cfg(test)]
mod model_cache_tests {
    use super::{file_mtime, model_get_or_load, ModelSlot};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, OnceLock};
    use std::time::{Duration, SystemTime};

    static TEST_CACHE: ModelSlot<String> = OnceLock::new();

    #[test]
    fn model_structs_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<caprust_media_io::background_removal::BackgroundRemover>();
        assert_send_sync::<caprust_media_io::mobile_sam::MobileSam>();
        assert_send_sync::<caprust_media_io::lama::Lama>();
        assert_send_sync::<caprust_media_io::face_detect::FaceDetector>();
        assert_send_sync::<caprust_media_io::scrfd::ScrfdDetector>();
        assert_send_sync::<caprust_media_io::face_restore::FaceRestorer>();
    }

    #[test]
    fn same_key_and_mtime_loads_once() {
        // Reset the shared slot so this test is independent of order.
        let fresh: ModelSlot<String> = OnceLock::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let mtime = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1000));

        let load = || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<String, String>("model-v1".to_string())
        };
        let a = model_get_or_load(&fresh, "k".into(), mtime, load).unwrap();
        let b = model_get_or_load(&fresh, "k".into(), mtime, load).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "loader must run once");
        assert_eq!(*a, "model-v1");
        assert!(Arc::ptr_eq(&a, &b), "second call returns the same Arc");
    }

    #[test]
    fn changed_mtime_reloads() {
        let fresh: ModelSlot<String> = OnceLock::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let t1 = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1000));
        let t2 = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(2000));

        let v1 = model_get_or_load(&fresh, "k".into(), t1, || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<String, String>("v1".into())
        })
        .unwrap();
        let v2 = model_get_or_load(&fresh, "k".into(), t2, || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<String, String>("v2".into())
        })
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2, "mtime change reloads");
        assert_eq!(*v1, "v1");
        assert_eq!(*v2, "v2");
    }

    #[test]
    fn failed_load_is_not_cached() {
        let fresh: ModelSlot<String> = OnceLock::new();
        let mtime = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1000));
        let first = model_get_or_load(&fresh, "k".into(), mtime, || {
            Err::<String, String>("boom".into())
        });
        assert!(first.is_err());
        let second = model_get_or_load(&fresh, "k".into(), mtime, || {
            Ok::<String, String>("ok".into())
        });
        assert_eq!(
            *second.unwrap(),
            "ok",
            "a failed load must not poison the cache"
        );
    }

    #[test]
    fn different_keys_are_independent() {
        let fresh: ModelSlot<String> = OnceLock::new();
        let mtime = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1000));
        let a = model_get_or_load(&fresh, "a".into(), mtime, || {
            Ok::<String, String>("A".into())
        })
        .unwrap();
        let b = model_get_or_load(&fresh, "b".into(), mtime, || {
            Ok::<String, String>("B".into())
        })
        .unwrap();
        assert_eq!((a.as_str(), b.as_str()), ("A", "B"));
    }

    #[test]
    fn file_mtime_none_for_missing_path() {
        assert_eq!(file_mtime(std::path::Path::new("no-such-file.onnx")), None);
    }

    // Keep the module-level static referenced so it is type-checked too.
    #[test]
    fn static_slot_is_usable() {
        let v = model_get_or_load(&TEST_CACHE, "static".into(), None, || {
            Ok::<String, String>("s".into())
        })
        .unwrap();
        assert_eq!(*v, "s");
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::{run_cancellable_frames, CancellationToken};
    use std::sync::Arc;

    #[test]
    fn cancel_token_starts_false_and_flips_to_true() {
        let t = CancellationToken::new();
        assert!(!t.is_cancelled());
        t.cancel();
        assert!(t.is_cancelled());
    }

    #[test]
    fn cancel_token_clones_share_state() {
        let a = CancellationToken::new();
        let b = a.clone();
        assert!(!b.is_cancelled());
        a.cancel();
        assert!(b.is_cancelled());
    }

    #[test]
    fn cancellable_frames_stops_early_on_cancel() {
        let total = 100;
        let cancel = CancellationToken::new();
        let processed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let p = processed.clone();

        let result = run_cancellable_frames(total, &cancel, |i| {
            p.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // Cancel after processing frame 5 (i == 5)
            if i == 5 {
                cancel.cancel();
            }
            Ok::<(), ()>(())
        });

        assert_eq!(result, Ok(6), "should process frames 0..=5 then stop");
        assert_eq!(processed.load(std::sync::atomic::Ordering::SeqCst), 6);
    }

    #[test]
    fn cancellable_frames_completes_all_when_not_cancelled() {
        let total = 10;
        let cancel = CancellationToken::new();
        let processed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let p = processed.clone();

        let result = run_cancellable_frames(total, &cancel, |i| {
            let _ = i;
            p.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok::<(), ()>(())
        });

        assert_eq!(result, Ok(10));
        assert_eq!(processed.load(std::sync::atomic::Ordering::SeqCst), 10);
    }

    #[test]
    fn cancellable_frames_stops_immediately_when_already_cancelled() {
        let total = 100;
        let cancel = CancellationToken::new();
        cancel.cancel();
        let processed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let p = processed.clone();

        let result = run_cancellable_frames(total, &cancel, |_| {
            p.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok::<(), ()>(())
        });

        assert_eq!(result, Ok(0));
        assert_eq!(processed.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn cancellable_frames_propagates_body_error() {
        let total = 100;
        let cancel = CancellationToken::new();
        let result: Result<usize, &str> = run_cancellable_frames(total, &cancel, |_| Err("boom"));
        assert_eq!(result, Err("boom"));
    }
}

#[cfg(test)]
mod stab_job_tests {
    use super::escape_vidstab_path;

    #[test]
    fn escape_uses_forward_slashes_and_escaped_colon() {
        let p = std::path::Path::new("C:\\proj\\cache\\stab\\x.trf");
        assert_eq!(
            escape_vidstab_path(p).as_deref(),
            Some("'C\\:/proj/cache/stab/x.trf'")
        );
    }

    #[test]
    fn escape_rejects_single_quote() {
        let p = std::path::Path::new("/tmp/a'b.trf");
        assert!(escape_vidstab_path(p).is_none());
    }

    #[test]
    fn escape_leaves_unix_paths_quoted() {
        let p = std::path::Path::new("/tmp/x.trf");
        assert_eq!(escape_vidstab_path(p).as_deref(), Some("'/tmp/x.trf'"));
    }
}
