//! The editing engine behind the CapRust Android app.
//!
//! The desktop app is a UI around `caprust-core` and `caprust-media-io`.
//! This crate is the same engine with the phone's questions answered
//! directly: what is on the timeline, what does this moment look like,
//! and make the video.
//!
//! Nothing in here touches an Android API. The crate builds and its
//! tests run on the machine that runs the tests, so the engine can be
//! checked without a phone and without an emulator. The Kotlin side
//! calls it through JNI, and the JNI layer is a thin shell over
//! [`Session`].
//!
//! The one thing the phone has and the desktop does not is where the
//! files are: the gallery, the app's own directory, and the export the
//! user shares. Everything here takes paths, so the caller decides.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use anyhow::{Context, Result};
use caprust_core::aspect_ratio::AspectRatio;
use caprust_core::clip::{Clip, ClipType};
use caprust_core::media::{MediaItem, MediaKind};
use caprust_core::project::{ProjectState, VideoEncoder};
use caprust_core::track::{Track, TrackKind};
use caprust_media_io::export::RateMode;
use caprust_media_io::exporter::ExportEvent;
use uuid::Uuid;

/// Where the engine keeps its files: the preview frames it renders and
/// the export it makes.
#[derive(Debug, Clone)]
pub struct Paths {
    /// The ffmpeg binary. On a phone this is the one shipped in the
    /// APK; on a desktop it is the one on the PATH.
    pub ffmpeg: PathBuf,
    /// The ffprobe binary, next to ffmpeg.
    pub ffprobe: PathBuf,
    /// Where the AI models live.
    pub models_dir: PathBuf,
    /// The app's own directory: preview frames and exports land here.
    pub work_dir: PathBuf,
}

/// What an imported file turned out to be.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedMedia {
    pub id: Uuid,
    pub path: String,
    pub kind: MediaKind,
    pub duration_ms: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub has_audio: bool,
}

/// The project the phone is editing.
pub struct Session {
    project: ProjectState,
    paths: Paths,
    /// The width and height the preview and the export are rendered at.
    width: u32,
    height: u32,
}

impl Session {
    /// A fresh vertical project: 1080x1920, 30 fps, the shape a
    /// Story is.
    pub fn vertical(paths: Paths) -> Self {
        Self::with_aspect(paths, AspectRatio::Portrait9x16)
    }

    /// A fresh project in the given shape.
    pub fn with_aspect(paths: Paths, aspect: AspectRatio) -> Self {
        let project = ProjectState {
            aspect_ratio: aspect,
            frame_rate: caprust_core::frame_rate::FrameRate::default(),
            ..Default::default()
        };
        let (w, h) = project.project_dimensions();
        Self {
            project,
            paths,
            width: w,
            height: h,
        }
    }

    /// Open a project that was saved earlier.
    pub fn open(paths: Paths, project_path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(project_path)
            .with_context(|| format!("read {}", project_path.display()))?;
        let project: ProjectState = serde_json::from_str(&text)
            .with_context(|| format!("parse {}", project_path.display()))?;
        let (w, h) = project.project_dimensions();
        Ok(Self {
            project,
            paths,
            width: w,
            height: h,
        })
    }

    /// The project itself, for the parts of it that have no engine of
    /// their own.
    pub fn project(&self) -> &ProjectState {
        &self.project
    }

    pub fn project_mut(&mut self) -> &mut ProjectState {
        &mut self.project
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// The shape of the project.
    pub fn aspect(&self) -> AspectRatio {
        self.project.aspect_ratio.clone()
    }

    /// Change the shape. The clips stay; the render follows.
    pub fn set_aspect(&mut self, aspect: AspectRatio) {
        self.project.aspect_ratio = aspect;
        let (w, h) = self.project.project_dimensions();
        self.width = w;
        self.height = h;
    }

    /// The width and height the render happens at.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The media in the library.
    pub fn media(&self) -> &[MediaItem] {
        &self.project.media.items
    }

    /// The clips on the timeline.
    pub fn clips(&self) -> &[Clip] {
        &self.project.clips
    }

    /// The tracks.
    pub fn tracks(&self) -> &[Track] {
        &self.project.tracks
    }

    /// Bring a file into the library.
    ///
    /// The file is probed first, so the caller learns how long it is
    /// and what is in it without opening it in an editor.
    pub fn import(&mut self, path: &Path) -> Result<ImportedMedia> {
        let kind = caprust_core::media::guess_kind(&path.to_string_lossy())
            .with_context(|| format!("what kind of file is {}", path.display()))?;
        let probe = caprust_media_io::ffprobe::probe(&self.paths.ffprobe, path)
            .with_context(|| format!("probe {}", path.display()))?;

        let id = self.project.media.add(&path.to_string_lossy(), kind);
        let item = self
            .project
            .media
            .items
            .iter_mut()
            .find(|i| i.id == id)
            .expect("the item was just added");
        item.duration_ms = probe.duration_ms;
        item.has_audio = probe.has_audio;
        item.probe_done = true;

        Ok(ImportedMedia {
            id,
            path: path.to_string_lossy().to_string(),
            kind,
            duration_ms: probe.duration_ms,
            width: probe.width,
            height: probe.height,
            has_audio: probe.has_audio,
        })
    }

    /// Put an imported file on the timeline.
    ///
    /// `track` is the index into the track list. A video with sound
    /// goes to a video lane and keeps its audio; a file that is only
    /// sound goes to the audio lane.
    pub fn add_to_timeline(&mut self, media_id: Uuid, track: usize, start_ms: u64) -> Result<Uuid> {
        let item = self
            .project
            .media
            .items
            .iter()
            .find(|i| i.id == media_id)
            .with_context(|| format!("no media with id {media_id}"))?;
        let path = item.path.clone();
        let duration = item.duration_ms;
        let kind = item.kind;

        let clip = match kind {
            MediaKind::Audio => Clip::new_audio(&path, track, start_ms, duration),
            _ => Clip::new_video(&path, track, start_ms, duration),
        };
        let id = clip.id;
        self.project.add_clip(clip);
        Ok(id)
    }

    /// Render one frame of the preview, as a JPEG in the work
    /// directory, and return where it landed.
    ///
    /// The phone shows the frame and does not need to know that it
    /// came out of ffmpeg.
    pub fn preview_frame(&self, at_ms: u64) -> Result<PathBuf> {
        let plan = self.plan()?;
        let out = self
            .paths
            .work_dir
            .join("preview")
            .join(format!("frame-{at_ms}.jpg"));
        let fps = self.fps();
        caprust_media_io::preview_render::render_single_frame(
            &self.paths.ffmpeg,
            &plan,
            at_ms,
            &out,
            fps,
        )
        .with_context(|| format!("render the preview frame at {at_ms} ms"))?;
        Ok(out)
    }

    /// Render the whole thing. The events carry the progress.
    pub fn export(&self, output: &Path) -> Result<Receiver<ExportEvent>> {
        let plan = self.plan()?;
        if let Some(parent) = output.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).ok();
            }
        }
        Ok(caprust_media_io::exporter::spawn_export(
            self.paths.ffmpeg.clone(),
            plan,
            output.to_path_buf(),
        ))
    }

    /// Save the project.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }
        }
        let json = serde_json::to_string_pretty(&self.project)?;
        std::fs::write(path, json).with_context(|| format!("write {}", path.display()))?;
        Ok(())
    }

    /// The render plan for the current project.
    fn plan(&self) -> Result<caprust_media_io::export_graph::RenderPlan> {
        let (fps_num, fps_den) = (
            self.project.frame_rate.num as i64,
            self.project.frame_rate.den as i64,
        );
        caprust_media_io::export_graph::plan_from_project(
            &self.project,
            self.width,
            self.height,
            fps_num,
            fps_den,
            23,
            RateMode::Vbr,
            8000,
            "veryfast",
            &self.paths.models_dir,
            0,
            VideoEncoder::H264Cpu,
        )
    }

    fn fps(&self) -> f64 {
        let fr = &self.project.frame_rate;
        fr.num as f64 / fr.den as f64
    }

    /// The first video lane, or a new one if the project has none.
    pub fn first_video_track(&self) -> Option<usize> {
        self.project
            .tracks
            .iter()
            .position(|t| t.kind == TrackKind::Video)
    }
}

/// The clip type of a clip, for the UI that lists them.
pub fn clip_kind(clip: &Clip) -> Option<&'static str> {
    match clip.clip_type {
        ClipType::Video { .. } => Some("video"),
        ClipType::Audio { .. } => Some("audio"),
        ClipType::Image { .. } => Some("image"),
        ClipType::TextOverlay { .. } => Some("text"),
        _ => None,
    }
}
