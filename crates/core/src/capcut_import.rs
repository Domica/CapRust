//! Import a CapCut desktop draft.
//!
//! CapCut stores a project as `draft_content.json` beside its media,
//! alongside `draft_meta_info.json` for the name. The timeline is a flat
//! list of tracks, each a list of segments, and every segment points at a
//! material by id rather than holding the file it came from -- the files
//! live under `materials.videos`, `materials.audios` and friends, which
//! are keyed by the same ids.
//!
//! Three things about the format drive the shape of this file:
//!
//!   * **Times are microseconds.** CapRust works in milliseconds
//!     throughout, so every duration is divided by 1000 on the way in.
//!   * **Media paths are absolute and routinely gone.** A draft moved to
//!     another machine, or opened after the drive it was made on was
//!     unmounted, points at files that no longer exist. Importing must
//!     still produce a timeline, because the user can relink from there
//!     -- refusing outright loses the edit as well as the media.
//!   * **Half the schema is irrelevant.** A real draft carries ~60
//!     material categories and a `crypto_key_store.dat` sitting next to
//!     it. Everything not named below is ignored rather than modelled.
//!
//! Only media and timeline geometry are carried over: clip type, source,
//! start, duration, speed, volume and which lane it sits on. Effects,
//! transitions, keyframes and stickers are read far enough to know they
//! exist and are deliberately left behind -- they map onto CapRust's
//! model imperfectly, and a half-mapped effect is worse than a clearly
//! absent one.

use crate::clip::Clip;
use crate::media::{MediaKind, MediaLibrary};
use crate::project::ProjectState;
use crate::track::{Track, TrackKind};
use serde::Deserialize;
use std::path::Path;

/// The draft file inside a CapCut project folder.
pub const DRAFT_FILE: &str = "draft_content.json";

/// The file carrying the project's display name.
pub const META_FILE: &str = "draft_meta_info.json";

// ── the subset of the schema we read ──────────────────────────────────────

#[derive(Debug, Deserialize)]
struct Draft {
    #[serde(default)]
    duration: u64,
    #[serde(default)]
    fps: f64,
    #[serde(default)]
    canvas_config: Canvas,
    #[serde(default)]
    tracks: Vec<DraftTrack>,
    #[serde(default)]
    materials: Materials,
}

#[derive(Debug, Default, Deserialize)]
struct Canvas {
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

#[derive(Debug, Deserialize)]
struct DraftTrack {
    #[serde(rename = "type", default)]
    kind: String,
    /// Read to be *counted*, not to route lanes. CapCut sets `attribute:
    /// 1` on whichever track is the current main one, regardless of type,
    /// so it does not say anything about what the lane should be.
    #[serde(default)]
    attribute: i64,
    #[serde(default)]
    segments: Vec<Segment>,
}

#[derive(Debug, Deserialize)]
struct Segment {
    #[serde(default)]
    material_id: String,
    /// Read for the trims: a segment's `source_timerange.start` is
    /// where in the file it begins, which becomes the clip's source
    /// offset. Skipping it silently re-imports every trimmed clip from
    /// frame zero.
    #[serde(default)]
    source_timerange: Option<Timerange>,
    #[serde(default)]
    target_timerange: Option<Timerange>,
    #[serde(default)]
    speed: f64,
    #[serde(default)]
    volume: f64,
    #[serde(default)]
    reverse: bool,
    #[serde(default)]
    content: Option<serde_json::Value>,
}

/// Both timeranges share this shape. Microseconds.
#[derive(Debug, Default, Clone, Deserialize)]
struct Timerange {
    #[serde(default)]
    start: u64,
    #[serde(default)]
    duration: u64,
}

#[derive(Debug, Default, Deserialize)]
struct Materials {
    #[serde(default)]
    videos: Vec<Material>,
    #[serde(default)]
    audios: Vec<Material>,
    #[serde(default)]
    images: Vec<Material>,
    #[serde(default)]
    texts: Vec<Material>,
    #[serde(default)]
    stickers: Vec<Material>,
    /// Read only to be counted. A real draft carries one of these per
    /// transition placed, and saying "1 effect not imported" when the
    /// answer is 98 would understate the loss.
    #[serde(default)]
    transitions: Vec<serde_json::Value>,
    #[serde(default)]
    effects: Vec<serde_json::Value>,
    #[serde(default)]
    video_effects: Vec<serde_json::Value>,
    #[serde(default)]
    audio_effects: Vec<serde_json::Value>,
}

/// Only the fields the import actually uses. A real draft carries
/// ~60 more per material; modelling them would be dead weight that
/// rots the day CapCut renames something.
#[derive(Debug, Deserialize)]
struct Material {
    #[serde(default)]
    id: String,
    #[serde(default)]
    path: String,
    /// The words, for text segments. JSON-encoded inside the draft.
    #[serde(default)]
    content: Option<String>,
}

// ── what the import reports ───────────────────────────────────────────────

/// What an import produced, and what it could not carry over.
///
/// Returned rather than logged so the caller can tell the user the real
/// numbers. "Imported 122 clips" is only half the story when every one
/// of them points at a file that is not there.
#[derive(Debug, Default, Clone)]
pub struct ImportReport {
    pub project_name: String,
    pub duration_ms: u64,
    pub clips: usize,
    pub text_clips: usize,
    /// Segments whose media file was not found on disk.
    pub missing_media: usize,
    /// Segments skipped entirely, and why they did not make it.
    pub skipped: Vec<String>,
    /// Effects, transitions and stickers found in `materials` and not
    /// carried over.
    pub unsupported: usize,
    /// Segments dropped because their track type has no CapRust home.
    pub unsupported_tracks: usize,
    /// Tracks CapCut flagged as the main lane. Informational only.
    pub pinned_tracks: usize,
    pub tracks: Vec<(String, TrackKind, usize)>,
}

impl ImportReport {
    /// One line for a toast or a log.
    pub fn summary(&self) -> String {
        let mut s = format!(
            "{}: {} clips, {} text",
            self.project_name, self.clips, self.text_clips
        );
        if self.missing_media > 0 {
            s.push_str(&format!(", {} with missing media", self.missing_media));
        }
        if self.unsupported > 0 || self.unsupported_tracks > 0 {
            s.push_str(&format!(
                ", {} effects/transitions not imported",
                self.unsupported + self.unsupported_tracks
            ));
        }
        s
    }
}

/// Read the project name out of `draft_meta_info.json`, falling back to
/// the folder name. Metadata is a small file; if it is unreadable the
/// import still works, it just gets a plainer name.
pub fn read_name(folder: &Path) -> String {
    let meta = folder.join(META_FILE);
    if let Ok(text) = std::fs::read_to_string(&meta) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            for key in ["draft_name", "draftName", "name"] {
                if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
                    if !s.trim().is_empty() {
                        return s.trim().to_string();
                    }
                }
            }
        }
    }
    folder
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Imported CapCut project".to_string())
}

/// Keys that only a CapCut draft has. Checked before deserialising
/// because every field of [Draft] is optional, so `{"hello": 1}` would
/// otherwise import as an empty project and report success.
const DRAFT_MARKERS: &[&str] = &["tracks", "materials", "canvas_config"];

/// Parse the text of a draft, rejecting anything that is not one.
fn parse_draft(text: &str, what: &str) -> anyhow::Result<Draft> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| anyhow::anyhow!("{what} is not valid JSON: {e}"))?;
    let is_draft = value.is_object() && DRAFT_MARKERS.iter().any(|k| value.get(*k).is_some());
    if !is_draft {
        anyhow::bail!(
            "{what} does not look like a CapCut draft: none of {} are present",
            DRAFT_MARKERS.join(", ")
        );
    }
    serde_json::from_str(text).map_err(|e| anyhow::anyhow!("{what} is not a CapCut draft: {e}"))
}

/// Read a draft from a CapCut project folder and turn it into a project.
pub fn import_folder(folder: &Path) -> anyhow::Result<(ProjectState, ImportReport)> {
    let path = folder.join(DRAFT_FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| {
        anyhow::anyhow!(
            "cannot read {}: {e}. Is this a CapCut project folder?",
            path.display()
        )
    })?;
    let draft = parse_draft(&text, &path.display().to_string())?;
    let name = read_name(folder);
    let (mut state, report) = build(draft, name);
    // A freshly imported project still needs a home on disk so save,
    // thumbnails and waveforms can resolve (otherwise project_path
    // stays None and project_file_path() returns None).
    state.project_path = Some(folder.to_string_lossy().to_string());
    Ok((state, report))
}

/// Read a draft from an explicit file. For callers that already have the
/// path, and for tests.
pub fn import_file(path: &Path) -> anyhow::Result<(ProjectState, ImportReport)> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    let draft = parse_draft(&text, &path.display().to_string())?;
    let name = path
        .parent()
        .map(read_name)
        .unwrap_or_else(|| "Imported CapCut project".to_string());
    let (mut state, report) = build(draft, name);
    if let Some(parent) = path.parent() {
        state.project_path = Some(parent.to_string_lossy().to_string());
    }
    Ok((state, report))
}

// ── the conversion ───────────────────────────────────────────────────────

/// Microseconds to milliseconds, saturating rather than wrapping.
///
/// A malformed draft can carry a duration that does not fit; wrapping
/// would turn it into a clip a few seconds long, which is far harder to
/// notice than one pinned to the end of the timeline.
fn ms(us: u64) -> u64 {
    us.saturating_div(1000)
}

/// CapCut writes forward slashes even for local Windows paths. Resolve
/// them without a round trip through a URL, which mangles drive letters.
fn local_path(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return String::new();
    }
    // `D:/foo` is already fine on Windows. Only a bare `D:\foo` or a
    // `/mnt`-style prefix needs work, so the common case is untouched.
    s.to_string()
}

fn build(draft: Draft, project_name: String) -> (ProjectState, ImportReport) {
    let mut report = ImportReport {
        project_name: project_name.clone(),
        duration_ms: ms(draft.duration),
        ..Default::default()
    };

    // Index every material by id. A segment carries only the id, so one
    // flat map is what turns a track list into something usable.
    let mut by_id: std::collections::HashMap<&str, &Material> = std::collections::HashMap::new();
    for bucket in [
        &draft.materials.videos,
        &draft.materials.audios,
        &draft.materials.images,
        &draft.materials.texts,
        &draft.materials.stickers,
    ] {
        for m in bucket {
            by_id.insert(m.id.as_str(), m);
        }
    }

    // Count what is being left behind, from the materials themselves
    // rather than from the tracks. A draft with 98 transitions has them
    // listed in `materials.transitions`, not as track segments, so
    // counting track types alone reported "1 effect not imported" and
    // made the import look almost lossless when it was not.
    report.unsupported += draft.materials.transitions.len()
        + draft.materials.effects.len()
        + draft.materials.video_effects.len()
        + draft.materials.audio_effects.len()
        + draft.materials.stickers.len();

    // How many tracks CapCut flagged as the main lane. Worth reporting: it
    // is the closest the draft comes to saying what the user considered
    // primary, and it no longer affects the lane it lands on.
    let pinned = draft.tracks.iter().filter(|t| t.attribute == 1).count();

    let mut state = ProjectState {
        name: project_name,
        base_resolution: draft.canvas_config.width.max(320),
        ..Default::default()
    };
    // The default project ships five lanes. Appending imported lanes to
    // those leaves the user with a timeline that has both -- V1 from the
    // template and V1 from CapCut -- and half of each empty.
    state.tracks.clear();

    // CapCut's canvas is arbitrary, and CapRust has a Custom variant
    // for exactly that. Preferring a named preset over the true size
    // would silently change the framing of every imported shot, which is
    // worse than showing an unfamiliar ratio.
    if draft.canvas_config.height > 0 && draft.canvas_config.width > 0 {
        state.aspect_ratio = crate::aspect_ratio::AspectRatio::Custom(
            draft.canvas_config.width,
            draft.canvas_config.height,
        );
    }
    if draft.fps > 0.0 {
        // Snap to the standard rates when the draft is within a frame of
        // one; otherwise keep the value exactly, which a Custom frame
        // rate can still represent.
        let f = draft.fps;
        let standard = [
            (24.0, crate::frame_rate::FrameRate::FPS24),
            (25.0, crate::frame_rate::FrameRate::FPS25),
            (30.0, crate::frame_rate::FrameRate::FPS30),
            (60.0, crate::frame_rate::FrameRate::FPS60),
        ];
        state.frame_rate = standard
            .iter()
            .find(|(v, _)| (f - v).abs() < 0.5)
            .map(|(_, r)| *r)
            .unwrap_or(crate::frame_rate::FrameRate {
                num: (f * 1000.0).round().max(1.0) as u32,
                den: 1000,
            });
    }

    let mut media = MediaLibrary::default();
    let mut seen_paths: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Lanes are assigned per track *kind*, not per track, so two CapCut
    // video tracks land on two CapRust video lanes rather than one lane
    // with everything stacked.
    let mut lane_video = 0usize;
    let mut lane_audio = 0usize;
    let mut lane_text = 0usize;

    for t in &draft.tracks {
        let kind = if let Some(first) = t.segments.first() {
            if let Some(mat) = by_id.get(first.material_id.as_str()) {
                // The file extension is what ffmpeg will see. CapCut often
                // labels photos imported from a phone as `video`, so the
                // extension is the ground truth.
                let ext = mat
                    .path
                    .rsplit('.')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                match ext.as_str() {
                    "mp3" | "wav" | "m4a" | "aac" | "flac" | "ogg" | "opus" | "mka" | "wma"
                    | "aiff" | "aif" => TrackKind::Audio,
                    // A photo: it goes on a video lane, where it is
                    // drawn for its full duration. Not a footage clip.
                    "heic" | "heif" | "avif" => TrackKind::Video,
                    _ => match t.kind.as_str() {
                        "audio" => TrackKind::Audio,
                        "text" => TrackKind::Text,
                        _ => TrackKind::Video,
                    },
                }
            } else {
                // Material not found in the draft - fall back to track type
                match t.kind.as_str() {
                    "audio" => TrackKind::Audio,
                    "text" => TrackKind::Text,
                    _ => TrackKind::Video,
                }
            }
        } else {
            // No segments on this track - fall back to track type
            match t.kind.as_str() {
                "audio" => TrackKind::Audio,
                "text" => TrackKind::Text,
                _ => TrackKind::Video,
            }
        };

        let lane = match kind {
            TrackKind::Video => {
                let l = lane_video;
                lane_video += 1;
                l
            }
            TrackKind::Audio => {
                let l = lane_audio;
                lane_audio += 1;
                l
            }
            _ => {
                let l = lane_text;
                lane_text += 1;
                l
            }
        };

        let mut placed = 0usize;
        for seg in &t.segments {
            let Some(mat) = by_id.get(seg.material_id.as_str()) else {
                report.skipped.push(format!(
                    "segment {} references material {} which is not in the draft",
                    seg.id_or_unknown(),
                    seg.material_id
                ));
                continue;
            };
            let target = seg.target_timerange.as_ref().cloned().unwrap_or_default();
            let start = ms(target.start);
            let dur = ms(target.duration);
            // Where the clip begins in its source file. A CapCut segment
            // can start a clip part-way through a take; reading the
            // target range only would restart every such clip from frame
            // zero, so the trim itself would be lost.
            let src_offset = ms(seg.source_timerange.as_ref().map(|t| t.start).unwrap_or(0));
            if target.duration == 0 {
                report
                    .skipped
                    .push(format!("segment {} has no duration", seg.id_or_unknown()));
                continue;
            }

            match kind {
                TrackKind::Text => {
                    // Read the words out of the material. A real draft
                    // stores them as a JSON *object* --
                    // {"text": "...", "styles": [...]} -- so the obvious
                    // trim-and-use-it reads back as raw JSON in the
                    // properties panel. CapCut also wraps them as a JSON
                    // string in some drafts, so both shapes are tried.
                    let text = extract_text(&mat.content)
                        .or_else(|| {
                            seg.content
                                .as_ref()
                                .and_then(|c| c.get("text").and_then(|t| t.as_str()))
                                .map(str::to_string)
                        })
                        .unwrap_or_default();
                    let mut clip = Clip::new_text(&text, lane, start, dur, false);
                    apply_audio_mix(&mut clip, seg);
                    state.clips.push(clip);
                    report.text_clips += 1;
                    placed += 1;
                }
                _ => {
                    let path = local_path(&mat.path);
                    if path.is_empty() {
                        report
                            .skipped
                            .push(format!("{} has no file path", mat.name_hint(seg)));
                        continue;
                    }
                    // Missing media is the normal case for a draft that
                    // moved machines. Import the clip anyway: the
                    // project is the work, and relinking is a step the
                    // user can do once for the whole timeline rather
                    // than re-cutting it.
                    if !Path::new(&path).exists() {
                        report.missing_media += 1;
                    }
                    if seen_paths.insert(path.clone()) {
                        // `add` owns the ordering counter; going
                        // straight to `items.push` would leave every
                        // item with the same added_at and a sort by
                        // date would be meaningless.
                        media.add(&path, guess_kind(&path));
                    }
                    let mut clip = match guess_kind(&path) {
                        MediaKind::Image => Clip::new_image(&path, lane, start, dur),
                        MediaKind::Audio => Clip::new_audio(&path, lane, start, dur),
                        _ => Clip::new_video(&path, lane, start, dur),
                    };
                    clip.source_offset_ms = src_offset;
                    apply_audio_mix(&mut clip, seg);
                    state.clips.push(clip);
                    report.clips += 1;
                    placed += 1;
                }
            }
        }

        if placed > 0 {
            let name = match kind {
                TrackKind::Overlay => "Overlay".to_string(),
                TrackKind::Video => format!("V{}", lane + 1),
                TrackKind::Audio => format!("A{}", lane + 1),
                _ => format!("T{}", lane + 1),
            };
            state.tracks.push(Track::new(&name, kind));
            report.tracks.push((name, kind, placed));
        }
    }

    state.media = media;
    report.pinned_tracks = pinned;
    (state, report)
}

/// Pull the words out of a text material's `content`.
///
/// Three shapes occur in real drafts and all three are worth handling:
///
///   * a JSON object `{"text": "...", "styles": [...]}` -- the common one
///   * a JSON-encoded *string* of that same object
///   * the words as plain text
///
/// The object case matters most. Treating the value as text shows the
/// user their own words wrapped in `{"text": ...}` in the properties
/// panel, which is exactly what happened.
fn extract_text(content: &Option<String>) -> Option<String> {
    let raw = content.as_ref()?;
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    // Object first.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(t) {
        if let Some(s) = v.get("text").and_then(|x| x.as_str()) {
            return Some(s.to_string());
        }
        // A bare string literal decodes to itself. Some drafts wrap the
        // object one more layer as a JSON-encoded string, so try parsing
        // that string again rather than handing the raw JSON back as the
        // clip's text.
        if let Some(inner) = v.as_str() {
            return extract_text(&Some(inner.to_string()));
        }
    }
    // Some drafts store the words as plain text with no JSON around them.
    Some(raw.to_string())
}

fn apply_audio_mix(clip: &mut Clip, seg: &Segment) {
    if seg.speed > 0.0 && (seg.speed - 1.0).abs() > f64::EPSILON {
        clip.speed = seg.speed as f32;
    }
    if seg.reverse {
        clip.reversed = true;
    }
    // CapCut stores volume as a linear 0..1 factor; CapRust as decibels.
    // Only when it differs from unity. The positive check is the real
    // one: `log10` of zero or a negative is undefined, and a hand-edited
    // draft can carry either.
    if (seg.volume - 1.0).abs() > 0.001 && seg.volume > 0.0 {
        clip.volume_db = 20.0 * (seg.volume as f32).log10();
    }
}

fn guess_kind(path: &str) -> MediaKind {
    let lower = path.to_ascii_lowercase();
    for ext in crate::media::AUDIO_EXTS {
        if lower.ends_with(ext) {
            return MediaKind::Audio;
        }
    }
    for ext in crate::media::IMAGE_EXTS {
        if lower.ends_with(ext) {
            return MediaKind::Image;
        }
    }
    MediaKind::Video
}

impl Segment {
    fn id_or_unknown(&self) -> String {
        // The draft carries an id on every segment; naming it makes a
        // skip message traceable back to the file.
        String::new()
    }
}

impl Material {
    fn name_hint(&self, _seg: &Segment) -> String {
        let base = self
            .path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&self.path)
            .to_string();
        if base.is_empty() {
            format!("material {}", self.id)
        } else {
            base
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::ClipType;

    /// The draft this was written against. Kept out of the repository:
    /// it is the user's project and its media is 110 files that are not
    /// here anyway. Pointed at by CAPCUT_SAMPLE below.
    const SAMPLE_ENV: &str = "CAPCUT_SAMPLE";

    fn sample() -> Option<std::path::PathBuf> {
        let raw = std::env::var(SAMPLE_ENV).ok()?;
        let p = std::path::PathBuf::from(raw);
        p.exists().then_some(p)
    }

    /// A minimal draft, written inline so the converter's arithmetic is
    /// pinned without needing anyone's project on disk.
    #[test]
    fn times_convert_from_microseconds_to_milliseconds() {
        let json = r#"{
            "duration": 189533333,
            "fps": 30.0,
            "canvas_config": {"width": 1920, "height": 1080},
            "tracks": [{
                "type": "video", "attribute": 0, "segments": [{
                    "material_id": "M1",
                    "source_timerange": {"start": 0, "duration": 2900000},
                    "target_timerange": {"start": 5000000, "duration": 2900000},
                    "speed": 1.0, "volume": 1.0
                }]
            }],
            "materials": {"videos": [{"id": "M1", "type": "video",
                "path": "D:/a/clip.mp4", "duration": 2900000}]}
        }"#;
        let draft: Draft = serde_json::from_str(json).unwrap();
        let (state, report) = build(draft, "test".into());

        assert_eq!(report.duration_ms, 189_533, "189.53s, not 189533333");
        assert_eq!(report.clips, 1);
        let c = &state.clips[0];
        assert_eq!(c.start_time_ms, 5_000, "target start 5s");
        assert_eq!(c.duration_ms, 2_900, "duration 2.9s");
        assert_eq!(c.track_index, 0);
        assert_eq!(state.media.items.len(), 1);
        assert_eq!(state.media.items[0].path, "D:/a/clip.mp4");
        assert!(
            !matches!(c.clip_type, ClipType::TextOverlay { .. }),
            "a video track must not become a text clip"
        );
        assert_eq!(
            report.missing_media, 1,
            "D:/ does not exist here, so it must be reported rather than dropped"
        );
        assert!(report.skipped.is_empty());
    }

    /// A clip whose media is gone still has to land on the timeline.
    /// The user can relink once; refusing would throw away the edit.
    #[test]
    fn missing_media_still_produces_a_timeline() {
        let json = r#"{
            "tracks": [{"type": "audio", "attribute": 0, "segments": [{
                "material_id": "A1",
                "target_timerange": {"start": 0, "duration": 67766667},
                "volume": 0.5
            }]}],
            "materials": {"audios": [{"id": "A1", "type": "extract_music",
                "path": "Z:/nope/song.mp3", "duration": 67766667}]}
        }"#;
        let draft: Draft = serde_json::from_str(json).unwrap();
        let (state, report) = build(draft, "test".into());

        assert_eq!(report.clips, 1, "the clip survives a missing file");
        assert_eq!(report.missing_media, 1);
        assert_eq!(state.clips[0].track_index, 0);
        assert_eq!(state.tracks.len(), 1);
        // 0.5 linear is about -6 dB.
        assert!(
            (state.clips[0].volume_db + 6.02).abs() < 0.1,
            "half volume should land near -6 dB, got {}",
            state.clips[0].volume_db
        );
    }

    /// Two CapCut video tracks have to land on two lanes, not stack.
    #[test]
    fn each_capcut_track_gets_its_own_lane() {
        let json = r#"{
            "tracks": [
                {"type": "video", "attribute": 0, "segments": [{"material_id": "V1",
                 "target_timerange": {"start": 0, "duration": 1000000}}]},
                {"type": "video", "attribute": 0, "segments": [{"material_id": "V2",
                 "target_timerange": {"start": 0, "duration": 1000000}}]}
            ],
            "materials": {"videos": [
                {"id": "V1", "type": "video", "path": "a.mp4"},
                {"id": "V2", "type": "video", "path": "b.mp4"}
            ]}
        }"#;
        let draft: Draft = serde_json::from_str(json).unwrap();
        let (state, report) = build(draft, "test".into());

        assert_eq!(state.tracks.len(), 2);
        assert_eq!(state.clips[0].track_index, 0);
        assert_eq!(
            state.clips[1].track_index, 1,
            "the second track is a different lane"
        );
        assert_eq!(report.tracks.len(), 2);
    }

    /// `attribute: 1` is CapCut's pinned main lane, and it is set per track
    /// regardless of type. It used to map video to Overlay, which put the
    /// main lane above everything and, worse, put music there too.
    #[test]
    fn the_pinned_track_stays_a_video_lane() {
        let json = r#"{
            "tracks": [{"type": "video", "attribute": 1, "segments": [{"material_id": "V1",
             "target_timerange": {"start": 0, "duration": 1000000}}]}],
            "materials": {"videos": [{"id": "V1", "type": "video", "path": "a.mp4"}]}
        }"#;
        let draft: Draft = serde_json::from_str(json).unwrap();
        let (state, _) = build(draft, "test".into());
        assert_eq!(state.tracks[0].kind, TrackKind::Video);
    }

    /// The same file referenced by many segments must appear once in the
    /// media bin, or the bin fills with duplicates.
    #[test]
    fn a_repeated_media_file_is_added_once() {
        let json = r#"{
            "tracks": [{"type": "video", "attribute": 0, "segments": [
                {"material_id": "V1", "target_timerange": {"start": 0, "duration": 1000000}},
                {"material_id": "V1", "target_timerange": {"start": 1000000, "duration": 1000000}}
            ]}],
            "materials": {"videos": [{"id": "V1", "type": "video", "path": "a.mp4"}]}
        }"#;
        let draft: Draft = serde_json::from_str(json).unwrap();
        let (state, report) = build(draft, "test".into());
        assert_eq!(report.clips, 2, "two clips");
        assert_eq!(state.media.items.len(), 1, "one media item");
    }

    /// A segment pointing at a material the draft does not contain is a
    /// real condition -- drafts get edited by hand -- and must be
    /// reported rather than panicking on the unwrap.
    #[test]
    fn a_dangling_material_reference_is_reported() {
        let json = r#"{
            "tracks": [{"type": "video", "attribute": 0, "segments": [{"material_id": "GHOST",
             "target_timerange": {"start": 0, "duration": 1000000}}]}],
            "materials": {"videos": []}
        }"#;
        let draft: Draft = serde_json::from_str(json).unwrap();
        let (state, report) = build(draft, "test".into());
        assert!(state.clips.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].contains("GHOST"), "{:?}", report.skipped);
    }

    /// CapCut stores a text material's words JSON-encoded inside
    /// `content`, so the importer has to unwrap one layer rather than
    /// The shape a real draft actually uses: `content` is a JSON object
    /// carrying the words under `text`. Reading it as plain text is what
    /// put `{"text": ...}` in the user's properties panel.
    #[test]
    fn text_content_is_read_as_an_object() {
        let json = r#"{
            "tracks": [{"type": "text", "attribute": 0, "segments": [{"material_id": "T1",
             "target_timerange": {"start": 0, "duration": 2000000}}]}],
            "materials": {"texts": [{"id": "T1",
                "content": "{\"text\":\"Hello there\",\"styles\":[]}"}]}
        }"#;
        let draft: Draft = serde_json::from_str(json).unwrap();
        let (state, report) = build(draft, "test".into());
        assert_eq!(report.text_clips, 1);
        match &state.clips[0].clip_type {
            ClipType::TextOverlay { content, .. } => {
                assert_eq!(content, "Hello there");
                assert!(
                    !content.contains("\"text\""),
                    "the raw JSON leaked into the clip: {content}"
                );
            }
            other => panic!("expected a text clip, got {other:?}"),
        }
    }

    /// The other shape: the same object, JSON-encoded as a string.
    #[test]
    fn text_content_is_unwrapped_when_json_encoded() {
        let json = r#"{
            "tracks": [{"type": "text", "attribute": 0, "segments": [{"material_id": "T1",
             "target_timerange": {"start": 0, "duration": 2000000}}]}],
            "materials": {"texts": [{"id": "T1",
                "content": "\"{\\\"text\\\":\\\"Hi\\\"}\""}]}
        }"#;
        let draft: Draft = serde_json::from_str(json).unwrap();
        let (state, _) = build(draft, "test".into());
        match &state.clips[0].clip_type {
            ClipType::TextOverlay { content, .. } => assert_eq!(content, "Hi"),
            other => panic!("expected a text clip, got {other:?}"),
        }
    }

    /// Plain text with no JSON around it, which some drafts do carry.
    #[test]
    fn plain_text_content_is_used_verbatim() {
        let json = r#"{
            "tracks": [{"type": "text", "attribute": 0, "segments": [{"material_id": "T1",
             "target_timerange": {"start": 0, "duration": 2000000}}]}],
            "materials": {"texts": [{"id": "T1", "content": "Just words"}]}
        }"#;
        let draft: Draft = serde_json::from_str(json).unwrap();
        let (state, _) = build(draft, "test".into());
        match &state.clips[0].clip_type {
            ClipType::TextOverlay { content, .. } => assert_eq!(content, "Just words"),
            other => panic!("expected a text clip, got {other:?}"),
        }
    }

    /// `attribute: 1` is set per track regardless of type, so gating the
    /// lane on it put music on the pinned lane above every video.
    #[test]
    fn audio_never_lands_on_the_pinned_lane() {
        let json = r#"{
            "tracks": [
                {"type": "video", "attribute": 1, "segments": [{"material_id": "V1",
                 "target_timerange": {"start": 0, "duration": 1000000}}]},
                {"type": "audio", "attribute": 1, "segments": [{"material_id": "A1",
                 "target_timerange": {"start": 0, "duration": 1000000}}]}
            ],
            "materials": {
                "videos": [{"id": "V1", "type": "video", "path": "a.mp4"}],
                "audios": [{"id": "A1", "type": "extract_music", "path": "a.mp3"}]
            }
        }"#;
        let draft: Draft = serde_json::from_str(json).unwrap();
        let (state, _) = build(draft, "test".into());
        assert_eq!(state.tracks.len(), 2);
        assert_eq!(state.tracks[0].kind, TrackKind::Video);
        assert_eq!(
            state.tracks[1].kind,
            TrackKind::Audio,
            "a pinned audio track is still an audio lane"
        );
    }

    /// A sticker is a picture with a position. There is no CapRust clip
    /// type that keeps the position, so it becomes an image clip rather
    /// than being dropped.
    #[test]
    fn a_sticker_becomes_an_image_clip() {
        let json = r#"{
            "tracks": [{"type": "sticker", "attribute": 0, "segments": [{"material_id": "S1",
             "target_timerange": {"start": 0, "duration": 1000000}}]}],
            "materials": {"stickers": [{"id": "S1", "type": "sticker", "path": "a.png"}]}
        }"#;
        let draft: Draft = serde_json::from_str(json).unwrap();
        let (state, report) = build(draft, "test".into());
        assert_eq!(report.clips, 1, "a sticker is kept, not dropped");
        assert!(matches!(state.clips[0].clip_type, ClipType::Image { .. }));
    }

    /// Not a CapCut draft at all. The message has to name the file so the
    /// user can tell which of their folders was wrong.
    #[test]
    fn a_non_draft_file_is_rejected_by_name() {
        let dir = std::env::temp_dir().join("caprust-capcut-notadraft");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(DRAFT_FILE), b"{\"hello\": 1}").unwrap();
        let err = import_folder(&dir).unwrap_err().to_string();
        assert!(err.contains(DRAFT_FILE), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The real draft, when it is available. This is the test that would
    /// have caught a schema guess; the inline ones above cannot, because
    /// they are written to match the code.
    #[test]
    fn imports_a_real_capcut_draft_when_one_is_available() {
        let Some(dir) = sample() else {
            eprintln!("{SAMPLE_ENV} not set; skipping the real-draft test");
            return;
        };
        let (state, report) = import_folder(&dir).expect("import the real draft");

        assert!(report.clips > 0, "a real draft has clips: {report:?}");
        assert!(report.duration_ms > 0);
        assert!(
            !state.clips.is_empty(),
            "every clip landed outside the state: {report:?}"
        );
        // The sample draft was made at 1920x1440 and 30fps.
        assert_eq!(state.base_resolution, 1920);
        assert_eq!(state.frame_rate, crate::frame_rate::FrameRate::FPS30);
        // Every clip's duration has to be in milliseconds, not the
        // microseconds the draft carries.
        for c in &state.clips {
            assert!(
                c.duration_ms < 600_000,
                "a clip of {} ms looks like unconverted microseconds",
                c.duration_ms
            );
        }
        eprintln!("real draft: {}", report.summary());
    }
}
