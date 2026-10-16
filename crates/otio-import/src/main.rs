//! OTIO (.otio JSON) importer plugin for CapRust.
//!
//! Speaks the plugin protocol on stdin/stdout: one JSON request line,
//! one JSON response line. The host sends `import` with a file path;
//! this returns a CapRust `ProjectState` as JSON.
//!
//! OTIO is a published, open interchange format (Academy Software
//! Foundation). Reading it involves no proprietary schema: every object
//! carries an `OTIO_SCHEMA` key naming its type, and fields are plain
//! JSON. That is all this parser uses.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

fn main() {
    if let Err(e) = serve() {
        // The host reads stderr for diagnostics only; the protocol answer
        // is what matters.
        eprintln!("otio-import: {e}");
        std::process::exit(1);
    }
}

fn serve() -> Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = serde_json::from_str(&line)
            .with_context(|| format!("parse request: {}", &line[..line.len().min(120)]))?;
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");

        let reply = match method {
            "hello" | "describe" => json!({
                "id": id,
                "result": {
                    "protocol": 1,
                    "name": "OTIO Importer",
                    "id": "org.caprust.otio",
                    "kinds": ["otio"],
                }
            }),
            "import" => {
                let path = req
                    .get("params")
                    .and_then(|p| p.get("path"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("import: missing params.path"))?;
                match import(Path::new(path)) {
                    Ok(project) => json!({"id": id, "result": {"project": project}}),
                    Err(e) => json!({"id": id, "error": {"message": e.to_string()}}),
                }
            }
            other => json!({"id": id, "error": {"message": format!("unknown method {other}")}}),
        };

        writeln!(out, "{reply}")?;
        out.flush()?;
    }
    Ok(())
}

/// Convert one `.otio` file into a CapRust `ProjectState` JSON value.
fn import(path: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let doc: Value = serde_json::from_str(&text)
        .with_context(|| format!("parse OTIO JSON in {}", path.display()))?;
    let schema = doc
        .get("OTIO_SCHEMA")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !schema.starts_with("Timeline.") {
        return Err(anyhow!(
            "top level is {schema:?}, expected a Timeline. OTIO files whose \
             root is a Clip or Track cannot be laid out on a timeline"
        ));
    }

    let timeline_name = doc
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("OTIO Import");
    let rate = timeline_rate(&doc);
    let base_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();

    // Tracks live under timeline.tracks (a Stack). Its children are
    // Track objects; a Stack may nest, so this walks to any depth.
    let tracks_stack = doc
        .get("tracks")
        .ok_or_else(|| anyhow!("timeline has no `tracks`"))?;
    let otio_tracks = collect_tracks(tracks_stack);

    let mut out_tracks: Vec<Value> = Vec::new();
    // Lane numbering for *display* counts per kind, so a timeline with a
    // video track and an audio track is called V1/A1 rather than V1/A2.
    // The clip's `track_index`, by contrast, is its position in the lane
    // list -- using the per-kind number there would put the audio on the
    // second video lane.
    let mut named_video = 0usize;
    let mut named_audio = 0usize;
    let mut out_clips: Vec<Value> = Vec::new();
    let mut bin = Bin::default();
    let mut end_ms = 0u64;

    for (kind_name, track) in otio_tracks {
        let lane = out_tracks.len();
        let kind = match kind_name.as_str() {
            "Audio" => "Audio",
            _ => "Video",
        };
        let lane_name = if kind == "Audio" {
            named_audio += 1;
            format!("A{named_audio}")
        } else {
            named_video += 1;
            format!("V{named_video}")
        };
        out_tracks.push(json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "name": lane_name,
            "kind": kind,
            "locked": false,
            "visible": true,
            "muted": false,
            "pinned": false,
            "height": 64.0,
            "volume_db": 0.0,
        }));

        let mut cursor_ms = 0u64;
        for item in children_of(track) {
            let dur_ms = match convert_item(item, rate, kind, &base_dir, &mut bin) {
                Item::Clip {
                    json: mut clip,
                    dur_ms,
                } => {
                    clip["track_index"] = json!(lane);
                    clip["start_time_ms"] = json!(cursor_ms);
                    // Set from the one duration computed for this item,
                    // so a clip can never disagree with itself about how
                    // long it is.
                    clip["duration_ms"] = json!(dur_ms);
                    out_clips.push(clip);
                    dur_ms
                }
                // A gap is empty time. It moves the cursor and nothing
                // else. Emitting a clip for it produced an object with
                // none of the fields the host's Clip requires, which
                // failed deserialisation and took the whole import with
                // it -- and a gap is in most real timelines, so this is
                // the difference between importing a file and not.
                Item::Gap(dur_ms) => dur_ms,
                Item::Skip => continue,
            };
            cursor_ms += dur_ms;
            end_ms = end_ms.max(cursor_ms);
        }
    }

    // Only the fields OTIO actually carries. The host overlays these on
    // a default project, so export settings, model registry and anything
    // added to the model later come from the host, not from here.
    // `end_ms`/`missing_media` are returned to the host for the toast;
    // the host drops keys it does not recognise.
    Ok(json!({
        "name": timeline_name,
        "clips": out_clips,
        // AspectRatio is an externally-tagged enum on the host side.
        "aspect_ratio": "Landscape16x9",
        "base_resolution": 1920,
        "frame_rate": { "num": rate as u32, "den": 1 },
        "locale": "en",
        "media": { "items": bin.items },
        "tracks": out_tracks,
        "otio_report": { "end_ms": end_ms, "missing_media": bin.missing },
    }))
}

/// Timeline rate, from any RationalTime in the file. Falls back to 24.
fn timeline_rate(doc: &Value) -> f64 {
    fn walk(v: &Value) -> Option<f64> {
        match v {
            Value::Object(map) => {
                if let Some(r) = map.get("rate").and_then(Value::as_f64) {
                    if r > 0.0 {
                        return Some(r);
                    }
                }
                map.values().find_map(walk)
            }
            Value::Array(a) => a.iter().find_map(walk),
            _ => None,
        }
    }
    walk(doc).unwrap_or(24.0)
}

/// Flatten the tracks Stack. Handles the nested-Stack case by recursing.
fn collect_tracks(stack: &Value) -> Vec<(String, &Value)> {
    let mut out = Vec::new();
    for child in children_of(stack) {
        let schema = schema_of(child);
        if schema.starts_with("Track.") {
            let kind = child.get("kind").and_then(Value::as_str).unwrap_or("Video");
            out.push((kind.to_string(), child));
        } else if schema.starts_with("Stack.") {
            out.extend(collect_tracks(child));
        }
    }
    out
}

fn children_of(v: &Value) -> Vec<&Value> {
    v.get("children")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

fn schema_of(v: &Value) -> &str {
    v.get("OTIO_SCHEMA").and_then(Value::as_str).unwrap_or("")
}

/// Everything one import accumulates across every track.
///
/// Passed around by `&mut` rather than as seven separate arguments: the
/// media bin wants each file once, in the order it was first seen, and
/// threading those two rules through a call site by hand is how a file
/// ends up listed twice or with a duplicate sort position.
#[derive(Default)]
struct Bin {
    /// Media paths already listed, in first-seen order.
    seen: Vec<String>,
    items: Vec<Value>,
    /// Sort position for the next new file. Not `items.len()`: a file
    /// that appears on three tracks is one entry, and using the length
    /// gave the file after it the same position.
    next_added_at: u64,
    /// Files that are not on disk. Reported so the user knows relinking
    /// is the next step rather than wondering where the edit went.
    missing: usize,
}

impl Bin {
    /// Claim a sort position for `path`, or `None` when the bin already
    /// has it.
    ///
    /// Split from `push` on purpose: the position has to be claimed
    /// before the item is built, because the item carries it. Doing both
    /// in one step meant the first file was written with position 0 and
    /// the second with 1 -- off by one, and the bin sorted from a state
    /// that does not exist.
    fn reserve(&mut self, path: &str) -> Option<u64> {
        if self.seen.iter().any(|p| p == path) {
            return None;
        }
        self.seen.push(path.to_string());
        self.next_added_at += 1;
        Some(self.next_added_at)
    }

    fn push(&mut self, item: Value) {
        self.items.push(item);
    }
}

/// What one child of a Track turns into.
enum Item {
    Clip {
        json: Value,
        dur_ms: u64,
    },
    /// Empty time between two clips. Not a clip.
    Gap(u64),
    /// Nothing to place and nothing to advance past.
    Skip,
}

const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "gif", "bmp", "tiff", "heic", "heif", "avif",
];

/// One child of a Track → a CapRust clip, an empty stretch, or nothing.
///
/// A `Transition` sits between two neighbouring clips and CapRust stores
/// those on the clip edges, so it is not a clip of its own. A `Gap` *is*
/// empty time and does move everything after it, which is why it is not
/// the same as "skip".
fn convert_item(item: &Value, rate: f64, track_kind: &str, base_dir: &Path, bin: &mut Bin) -> Item {
    let schema = schema_of(item);
    if schema.starts_with("Transition.") || schema.starts_with("Filler.") {
        return Item::Skip;
    }
    if schema.starts_with("Gap.") {
        return Item::Gap(range_duration_ms(item.get("source_range"), rate));
    }
    if !schema.starts_with("Clip.") {
        return Item::Skip;
    }

    let dur_ms = range_duration_ms(item.get("source_range"), rate);
    let source_offset_ms = range_start_ms(item.get("source_range"), rate);
    let name = item.get("name").and_then(Value::as_str).unwrap_or("clip");

    let media_ref = item.get("media_reference");
    let ref_schema = media_ref.map(schema_of).unwrap_or("");

    // A text generator holds the words, not a file. CapRust has a clip
    // type for it, so the words survive the import instead of the clip
    // vanishing and taking the user's title with it.
    if ref_schema.starts_with("GeneratorReference") {
        let text = media_ref
            .and_then(|m| m.get("parameters"))
            .and_then(|p| p.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if text.trim().is_empty() {
            return Item::Skip;
        }
        return Item::Clip {
            json: text_clip(text, name),
            dur_ms,
        };
    }

    // A clip with no URL still happened, and its media is usually just
    // on another drive. Dropping it silently lost the whole cut: the
    // user got an empty timeline and no explanation. Import it with the
    // file name as the path, and say in the report that it is missing,
    // so relinking it afterwards is one step rather than a re-cut.
    let url = media_ref
        .and_then(|m| m.get("target_url"))
        .and_then(Value::as_str)
        .filter(|u| !u.is_empty());
    let resolved = match url {
        Some(u) => resolve_media_path(base_dir, u),
        None => base_dir.join(name),
    };
    if !resolved.exists() {
        bin.missing += 1;
    }

    let ext = resolved
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let path_str = resolved.to_string_lossy().to_string();

    let media_kind = if track_kind == "Audio" {
        "Audio"
    } else if IMAGE_EXTS.contains(&ext.as_str()) {
        "Image"
    } else {
        "Video"
    };

    if let Some(added_at) = bin.reserve(&path_str) {
        bin.push(json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "name": resolved
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| name.to_string()),
            "path": path_str,
            "kind": media_kind,
            "duration_ms": dur_ms,
            "size_bytes": 0,
            "added_at": added_at,
            "probe_done": false,
            "thumb_done": false,
            "waveform_done": false,
            "has_audio": media_kind != "Image",
        }));
    }

    let clip_type = match media_kind {
        "Audio" => json!({"Audio": {"path": path_str, "duration_ms": dur_ms}}),
        "Image" => json!({"Image": {"path": path_str, "duration_ms": dur_ms}}),
        _ => json!({"Video": {"path": path_str, "duration_ms": dur_ms}}),
    };

    Item::Clip {
        json: json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "track_index": 0,
            "start_time_ms": 0,
            "duration_ms": dur_ms,
            "clip_type": clip_type,
            "speed": 1.0,
            "reversed": false,
            "flip_h": false,
            "flip_v": false,
            "volume_db": 0.0,
            "source_duration_ms": dur_ms,
            "source_offset_ms": source_offset_ms,
            "media_id": null,
            "effects": [],
            "lut": null,
            "transition_in": null,
            "transition_out": null,
            "transition_in_easing": "Linear",
            "transition_out_easing": "Linear",
            "transition_duration_ms": 500,
            "applied_xfade_shift_ms": 0,
            "audio_detached": false,
            "detached_audio_clip_id": null,
            "name": name,
            "fade_in_ms": 0,
            "fade_out_ms": 0,
            "audio_normalize": false,
            "audio_denoise": false,
            "audio_voice_boost": false,
            "audio_telephone": false,
            "audio_bass": false,
            "audio_echo": false,
            "audio_chipmunk": false,
            "volume_keyframes": [],
            "duck_against": null,
        }),
        dur_ms,
    }
}

fn text_clip(text: &str, name: &str) -> Value {
    json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "track_index": 0,
        "start_time_ms": 0,
        "duration_ms": 0,
        "clip_type": {
            "TextOverlay": {
                "content": text,
                "font_size": 24.0,
                "above": true,
                "style": "default",
            }
        },
        "speed": 1.0,
        "reversed": false,
        "flip_h": false,
        "flip_v": false,
        "volume_db": 0.0,
        "source_duration_ms": 0,
        "source_offset_ms": 0,
        "name": name,
    })
}

/// OTIO media URLs are either absolute paths or relative to the .otio
/// file. `file://` is the form most adapters emit.
fn resolve_media_path(base_dir: &Path, url: &str) -> PathBuf {
    let raw = url.strip_prefix("file://").unwrap_or(url);
    // A URL-encoded space is common in adapter output.
    let decoded = raw.replace("%20", " ");
    let p = PathBuf::from(&decoded);
    if p.is_absolute() {
        p
    } else {
        base_dir.join(p)
    }
}

fn range_duration_ms(range: Option<&Value>, rate: f64) -> u64 {
    let Some(r) = range else { return 0 };
    let d = r
        .get("duration")
        .and_then(|v| v.get("value"))
        .and_then(Value::as_f64);
    match d {
        Some(v) if rate > 0.0 => ((v / rate) * 1000.0).round().max(0.0) as u64,
        _ => 0,
    }
}

fn range_start_ms(range: Option<&Value>, rate: f64) -> u64 {
    let Some(r) = range else { return 0 };
    let s = r
        .get("start_time")
        .and_then(|v| v.get("value"))
        .and_then(Value::as_f64);
    match s {
        Some(v) if rate > 0.0 => ((v / rate) * 1000.0).round().max(0.0) as u64,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
  "OTIO_SCHEMA": "Timeline.1",
  "name": "demo",
  "tracks": {
    "OTIO_SCHEMA": "Stack.1",
    "children": [
      { "OTIO_SCHEMA": "Track.1", "kind": "Video", "name": "V1", "children": [
        { "OTIO_SCHEMA": "Clip.1", "name": "a.mp4",
          "media_reference": { "OTIO_SCHEMA": "ExternalReference.1", "target_url": "file:///tmp/a.mp4" },
          "source_range": { "OTIO_SCHEMA": "TimeRange.1",
            "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": 25, "value": 50 },
            "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": 25, "value": 0 } } },
        { "OTIO_SCHEMA": "Transition.1", "name": "t", "transition_type": "SMPTE_Dissolve",
          "in_offset": { "OTIO_SCHEMA": "RationalTime.1", "rate": 25, "value": 10 },
          "out_offset": { "OTIO_SCHEMA": "RationalTime.1", "rate": 25, "value": 10 } }
      ] },
      { "OTIO_SCHEMA": "Track.1", "kind": "Audio", "name": "A1", "children": [
        { "OTIO_SCHEMA": "Clip.1", "name": "s.mp3",
          "media_reference": { "OTIO_SCHEMA": "ExternalReference.1", "target_url": "s.mp3" },
          "source_range": { "OTIO_SCHEMA": "TimeRange.1",
            "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": 25, "value": 100 },
            "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": 25, "value": 0 } } }
      ] }
    ]
  }
}"#;

    #[test]
    fn imports_a_two_track_timeline() {
        let dir = std::env::temp_dir().join("otio-import-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("demo.otio");
        std::fs::write(&file, SAMPLE).unwrap();

        let project = import(&file).unwrap();
        assert_eq!(project["name"], "demo");
        assert_eq!(project["frame_rate"]["num"], 25);
        assert_eq!(project["tracks"].as_array().unwrap().len(), 2);
        // One video clip: the transition between them is not a clip.
        let clips = project["clips"].as_array().unwrap();
        assert_eq!(clips.len(), 2);
        // 50 frames at 25 fps is two seconds.
        assert_eq!(clips[0]["duration_ms"], 2000);
        assert!(clips[0]["clip_type"].get("Video").is_some());
        // A relative target_url resolves against the .otio directory.
        assert!(clips[1]["clip_type"]["Audio"]["path"]
            .as_str()
            .unwrap()
            .ends_with("s.mp3"));
        // Lane numbering is per kind for the name and global for the
        // index: one video track and one audio track are V1 and A1, and
        // the audio clip sits on lane index 1.
        let tracks = project["tracks"].as_array().unwrap();
        assert_eq!(tracks[0]["name"], "V1");
        assert_eq!(tracks[1]["name"], "A1");
        assert_eq!(clips[0]["track_index"], 0);
        assert_eq!(clips[1]["track_index"], 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A gap is empty time, not a clip. Emitting one as a clip produced
    /// an object missing every field the host's Clip requires, and the
    /// whole import failed deserialising. Real timelines are mostly gaps
    /// -- this is the shape that comes out of a recorder.
    const GAPS: &str = r#"{
        "OTIO_SCHEMA": "Timeline.1",
        "name": "gapped",
        "tracks": { "OTIO_SCHEMA": "Stack.1", "children": [
          { "OTIO_SCHEMA": "Track.1", "kind": "Video", "name": "V1", "children": [
            { "OTIO_SCHEMA": "Clip.1", "name": "a.mp4",
              "media_reference": {"OTIO_SCHEMA":"ExternalReference.1","target_url":"a.mp4"},
              "source_range": {"duration": {"rate": 25, "value": 50}, "start_time": {"rate": 25, "value": 0}} },
            { "OTIO_SCHEMA": "Gap.1", "name": "Filler",
              "source_range": {"duration": {"rate": 25, "value": 100}, "start_time": {"rate": 25, "value": 0}} },
            { "OTIO_SCHEMA": "Clip.1", "name": "b.mp4",
              "media_reference": {"OTIO_SCHEMA":"ExternalReference.1","target_url":"b.mp4"},
              "source_range": {"duration": {"rate": 25, "value": 25}, "start_time": {"rate": 25, "value": 0}} }
          ] }
        ] }
    }"#;

    #[test]
    fn a_gap_moves_the_cursor_instead_of_becoming_a_clip() {
        let dir = std::env::temp_dir().join("otio-import-gaps");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gaps.otio");
        std::fs::write(&file, GAPS).unwrap();

        let project = import(&file).unwrap();
        let clips = project["clips"].as_array().unwrap();
        assert_eq!(clips.len(), 2, "two clips, not three");
        // Every clip must carry what the host's Clip requires. This is
        // the assertion that would have caught the gap-as-clip bug.
        for c in clips {
            assert!(c.get("id").and_then(Value::as_str).is_some(), "{c}");
            assert!(c.get("clip_type").is_some(), "{c}");
            assert!(c.get("duration_ms").is_some(), "{c}");
            assert!(c.get("volume_db").is_some(), "{c}");
        }
        // 50 frames at 25 is 2 s, then the gap pushes the next clip to 6 s.
        assert_eq!(clips[0]["start_time_ms"], 0);
        assert_eq!(clips[1]["start_time_ms"], 6000);
        assert_eq!(project["otio_report"]["end_ms"], 7000);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A clip whose media is gone still lands on the timeline. Dropping
    /// it produced an empty project and no explanation, which is the
    /// worst possible outcome: the user cannot tell whether the import
    /// failed or whether their edit had no media.
    const MISSING: &str = r#"{
        "OTIO_SCHEMA": "Timeline.1",
        "name": "offline",
        "tracks": { "OTIO_SCHEMA": "Stack.1", "children": [
          { "OTIO_SCHEMA": "Track.1", "kind": "Video", "name": "V1", "children": [
            { "OTIO_SCHEMA": "Clip.1", "name": "shot A.mp4",
              "media_reference": {"OTIO_SCHEMA":"MissingReference.1","name":null},
              "source_range": {"duration": {"rate": 24, "value": 240}, "start_time": {"rate": 24, "value": 0}} }
          ] }
        ] }
    }"#;

    #[test]
    fn a_clip_with_no_url_still_imports_and_is_reported_missing() {
        let dir = std::env::temp_dir().join("otio-import-missing");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("missing.otio");
        std::fs::write(&file, MISSING).unwrap();

        let project = import(&file).unwrap();
        let clips = project["clips"].as_array().unwrap();
        assert_eq!(clips.len(), 1, "the cut survives its missing media");
        assert_eq!(clips[0]["duration_ms"], 10_000);
        assert_eq!(
            project["otio_report"]["missing_media"], 1,
            "and the user is told"
        );
        // The file name is the best guess at where the media went.
        let items = project["media"]["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0]["path"].as_str().unwrap().ends_with("shot A.mp4"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A text generator holds words, not a file. Dropping it lost the
    /// user's title.
    #[test]
    fn a_text_generator_becomes_a_text_clip() {
        let json = r#"{
            "OTIO_SCHEMA": "Timeline.1",
            "name": "titled",
            "tracks": { "OTIO_SCHEMA": "Stack.1", "children": [
              { "OTIO_SCHEMA": "Track.1", "kind": "Video", "name": "V1", "children": [
                { "OTIO_SCHEMA": "Clip.1", "name": "Title",
                  "media_reference": {"OTIO_SCHEMA":"GeneratorReference.1","generator_kind":"Text",
                    "parameters": {"text": "Pozdrav Svijete!"}},
                  "source_range": {"duration": {"rate": 24, "value": 72}, "start_time": {"rate": 24, "value": 0}} }
              ] }
            ] }
        }"#;
        let dir = std::env::temp_dir().join("otio-import-text");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("text.otio");
        std::fs::write(&file, json).unwrap();

        let project = import(&file).unwrap();
        let clips = project["clips"].as_array().unwrap();
        assert_eq!(clips.len(), 1);
        assert_eq!(
            clips[0]["clip_type"]["TextOverlay"]["content"],
            "Pozdrav Svijete!"
        );
        assert_eq!(clips[0]["duration_ms"], 3000);
        // A generated title is not a file, so it adds nothing to the bin.
        assert!(project["media"]["items"].as_array().unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `added_at` is what the media bin sorts by. Deriving it from the
    /// item count gives two different files the same position whenever
    /// one file appears on more than one track.
    #[test]
    fn media_added_at_is_unique_per_file() {
        let json = r#"{
            "OTIO_SCHEMA": "Timeline.1",
            "name": "repeats",
            "tracks": { "OTIO_SCHEMA": "Stack.1", "children": [
              { "OTIO_SCHEMA": "Track.1", "kind": "Video", "name": "V1", "children": [
                { "OTIO_SCHEMA": "Clip.1", "name": "a.mp4",
                  "media_reference": {"OTIO_SCHEMA":"ExternalReference.1","target_url":"a.mp4"},
                  "source_range": {"duration": {"rate": 25, "value": 25}, "start_time": {"rate": 25, "value": 0}} }
              ] },
              { "OTIO_SCHEMA": "Track.1", "kind": "Audio", "name": "A1", "children": [
                { "OTIO_SCHEMA": "Clip.1", "name": "a.mp4",
                  "media_reference": {"OTIO_SCHEMA":"ExternalReference.1","target_url":"a.mp4"},
                  "source_range": {"duration": {"rate": 25, "value": 25}, "start_time": {"rate": 25, "value": 0}} },
                { "OTIO_SCHEMA": "Clip.1", "name": "b.mp3",
                  "media_reference": {"OTIO_SCHEMA":"ExternalReference.1","target_url":"b.mp3"},
                  "source_range": {"duration": {"rate": 25, "value": 25}, "start_time": {"rate": 25, "value": 0}} }
              ] }
            ] }
        }"#;
        let dir = std::env::temp_dir().join("otio-import-order");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("order.otio");
        std::fs::write(&file, json).unwrap();

        let project = import(&file).unwrap();
        let items = project["media"]["items"].as_array().unwrap();
        assert_eq!(items.len(), 2, "a.mp4 is shared, not duplicated");
        let ats: Vec<u64> = items
            .iter()
            .map(|i| i["added_at"].as_u64().unwrap())
            .collect();
        assert_eq!(ats, [1, 2], "each file needs its own sort position");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_non_timeline_root_is_rejected_with_a_clear_message() {
        let dir = std::env::temp_dir().join("otio-import-notl");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("x.otio");
        std::fs::write(&file, r#"{"OTIO_SCHEMA":"Clip.1","name":"x"}"#).unwrap();
        let e = import(&file).unwrap_err().to_string();
        assert!(e.contains("expected a Timeline"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn percent_encoded_paths_are_decoded() {
        let p = resolve_media_path(Path::new("/base"), "file:///media/My%20Clip.mov");
        assert_eq!(p, PathBuf::from("/media/My Clip.mov"));
    }
}
