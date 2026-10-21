//! The tests drive the engine the way the phone will: import a file,
//! put it on the timeline, look at a frame, render the video.
//!
//! The frames come out of the real ffmpeg, not out of a graph, so a
//! plan that renders nothing fails here.

use std::path::{Path, PathBuf};
use std::process::Command;

use caprust_android::{Paths, Session};
use caprust_core::aspect_ratio::AspectRatio;

fn ffmpeg() -> PathBuf {
    PathBuf::from("ffmpeg")
}

fn ffprobe() -> PathBuf {
    PathBuf::from("ffprobe")
}

/// A short clip with sound, made by the same ffmpeg the app uses.
fn sample_video(dir: &Path, name: &str) -> PathBuf {
    let out = dir.join(name);
    let o = Command::new(ffmpeg())
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=10:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ])
        .arg(&out)
        .output()
        .expect("ffmpeg runs");
    assert!(
        o.status.success(),
        "the sample clip failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    out
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("caprust-android-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn session(dir: &Path) -> Session {
    Session::vertical(Paths {
        ffmpeg: ffmpeg(),
        ffprobe: ffprobe(),
        models_dir: dir.join("models"),
        work_dir: dir.join("work"),
    })
}

#[test]
fn a_fresh_session_is_vertical() {
    let dir = scratch("vertical");
    let s = session(&dir);
    assert_eq!(s.aspect(), AspectRatio::Portrait9x16);
    let (w, h) = s.size();
    assert!(h > w, "a Story is taller than it is wide: {w}x{h}");
    assert_eq!(s.clips().len(), 0);
}

#[test]
fn importing_a_file_probes_it() {
    let dir = scratch("import");
    let clip = sample_video(&dir, "clip.mp4");
    let mut s = session(&dir);

    let imported = s.import(&clip).expect("the file is imported");
    assert_eq!(
        imported.duration_ms, 2000,
        "the duration comes from the probe"
    );
    assert!(imported.has_audio);
    assert_eq!(s.media().len(), 1);
}

#[test]
fn an_imported_file_can_go_on_the_timeline() {
    let dir = scratch("timeline");
    let clip = sample_video(&dir, "clip.mp4");
    let mut s = session(&dir);

    let imported = s.import(&clip).expect("the file is imported");
    let track = s.first_video_track().expect("a video lane");
    let id = s
        .add_to_timeline(imported.id, track, 0)
        .expect("the clip lands");

    assert_eq!(s.clips().len(), 1);
    let clip = s
        .clips()
        .iter()
        .find(|c| c.id == id)
        .expect("the clip is there");
    assert_eq!(clip.duration_ms, 2000);
    assert_eq!(clip.track_index, track);
}

#[test]
fn a_preview_frame_comes_out_of_the_real_ffmpeg() {
    let dir = scratch("preview");
    let clip = sample_video(&dir, "clip.mp4");
    let mut s = session(&dir);

    let imported = s.import(&clip).expect("the file is imported");
    let track = s.first_video_track().expect("a video lane");
    s.add_to_timeline(imported.id, track, 0)
        .expect("the clip lands");

    let frame = s.preview_frame(0).expect("a frame is rendered");
    assert!(frame.exists(), "the frame is at {}", frame.display());
    let bytes = std::fs::read(&frame).expect("the frame is readable");
    assert!(bytes.len() > 1000, "the frame is a picture, not a stub");
    // A JPEG starts with the SOI marker.
    assert_eq!(&bytes[0..2], &[0xff, 0xd8], "the frame is a JPEG");
}

#[test]
fn an_export_produces_a_video() {
    let dir = scratch("export");
    let clip = sample_video(&dir, "clip.mp4");
    let mut s = session(&dir);

    let imported = s.import(&clip).expect("the file is imported");
    let track = s.first_video_track().expect("a video lane");
    s.add_to_timeline(imported.id, track, 0)
        .expect("the clip lands");

    let out = dir.join("story.mp4");
    let events = s.export(&out).expect("the export starts");

    let mut finished = false;
    let mut failure = None;
    for ev in events {
        match ev {
            caprust_media_io::exporter::ExportEvent::Finished { .. } => finished = true,
            caprust_media_io::exporter::ExportEvent::Failed(msg) => failure = Some(msg),
            _ => {}
        }
    }
    assert!(failure.is_none(), "the export failed: {failure:?}");
    assert!(finished, "the export did not finish");
    assert!(out.exists(), "the video is at {}", out.display());
    assert!(std::fs::metadata(&out).unwrap().len() > 1000);
}

#[test]
fn a_project_survives_a_round_trip() {
    let dir = scratch("save");
    let clip = sample_video(&dir, "clip.mp4");
    let mut s = session(&dir);

    let imported = s.import(&clip).expect("the file is imported");
    let track = s.first_video_track().expect("a video lane");
    s.add_to_timeline(imported.id, track, 0)
        .expect("the clip lands");

    let path = dir.join("story.caprust");
    s.save(&path).expect("the project is saved");

    let back = Session::open(session(&dir).paths().clone(), &path).expect("the project opens");
    assert_eq!(back.clips().len(), 1);
    assert_eq!(back.media().len(), 1);
    assert_eq!(back.aspect(), AspectRatio::Portrait9x16);
}
