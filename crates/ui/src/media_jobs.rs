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
