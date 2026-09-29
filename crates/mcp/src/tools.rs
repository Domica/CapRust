//! MCP tool definitions and handlers.
//!
//! Two groups:
//! - read-only — pure inspection of `ProjectState`
//! - mutating  — every change goes through `UndoStack::execute`, per
//!   DIRECTIVES §6 and §35.3. No direct mutation of `ProjectState`.

use std::path::Path;

use anyhow::{bail, Context, Result};
use caprust_core::clip::{Clip, ClipType};
use caprust_core::commands::ripple::RippleInsertCommand;
use caprust_core::commands::set_clip::SetClipCommand;
use caprust_core::commands::set_effect::SetTransitionCommand;
use caprust_core::commands::split_clip::SplitClipCommand;
use caprust_core::commands::UndoStack;
use caprust_core::media::MediaKind;
use caprust_core::project::ProjectState;
use caprust_core::project_io;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn definitions() -> Vec<Value> {
    vec![
        // ── read-only ───────────────────────────────────────────────────────
        json!({
            "name": "get_project_summary",
            "description": "Return project name, dimensions, frame rate, total duration, and counts.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "list_tracks",
            "description": "List all tracks with index, kind, name, and header chips.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "list_clips",
            "description": "List clips. Optional track_index filters to a single track.",
            "inputSchema": {
                "type": "object",
                "properties": { "track_index": { "type": "integer", "minimum": 0 } },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "list_media",
            "description": "List media library items.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        // ── mutating ────────────────────────────────────────────────────────
        json!({
            "name": "move_clip",
            "description": "Move a clip to a new start time (and optionally a new track).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_id":  { "type": "string" },
                    "to_ms":    { "type": "integer", "minimum": 0 },
                    "to_track": { "type": "integer", "minimum": 0 }
                },
                "required": ["clip_id", "to_ms"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "split_clip",
            "description": "Split a clip at an absolute timeline position.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_id": { "type": "string" },
                    "at_ms":   { "type": "integer", "minimum": 0 }
                },
                "required": ["clip_id", "at_ms"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "set_transition",
            "description": "Set or clear the in/out transition on a clip. transition_id=null clears.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_id":       { "type": "string" },
                    "edge":          { "type": "string", "enum": ["in", "out"] },
                    "transition_id": { "type": ["string", "null"] }
                },
                "required": ["clip_id", "edge", "transition_id"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "set_clip_volume",
            "description": "Set static volume in dB (-60.0 .. 20.0).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_id": { "type": "string" },
                    "db":      { "type": "number" }
                },
                "required": ["clip_id", "db"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "set_clip_speed",
            "description": "Set playback speed (0.1 .. 100.0). 1.0 is normal.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_id": { "type": "string" },
                    "speed":   { "type": "number" }
                },
                "required": ["clip_id", "speed"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "set_clip_fade",
            "description": "Set audio fade in/out durations in ms. At least one must be supplied.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_id":     { "type": "string" },
                    "fade_in_ms":  { "type": "integer", "minimum": 0 },
                    "fade_out_ms": { "type": "integer", "minimum": 0 }
                },
                "required": ["clip_id"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "add_clip_to_timeline",
            "description": "Insert a clip from the media library onto a track at a given time.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "media_id":    { "type": "string" },
                    "track_index": { "type": "integer", "minimum": 0 },
                    "start_ms":    { "type": "integer", "minimum": 0 }
                },
                "required": ["media_id", "track_index", "start_ms"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "undo",
            "description": "Undo the last mutating operation.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "redo",
            "description": "Redo the last undone operation.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "save_project",
            "description": "Save the project to the path it was loaded from.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
    ]
}

pub fn call(
    name: &str,
    args: &Value,
    project: &mut ProjectState,
    undo: &mut UndoStack,
    project_path: &Path,
) -> Result<String> {
    match name {
        // read-only
        "get_project_summary" => get_project_summary(project),
        "list_tracks" => list_tracks(project),
        "list_clips" => list_clips(project, args),
        "list_media" => list_media(project),
        // mutating
        "move_clip" => move_clip(args, project, undo),
        "split_clip" => split_clip(args, project, undo),
        "set_transition" => set_transition(args, project, undo),
        "set_clip_volume" => set_clip_volume(args, project, undo),
        "set_clip_speed" => set_clip_speed(args, project, undo),
        "set_clip_fade" => set_clip_fade(args, project, undo),
        "add_clip_to_timeline" => add_clip_to_timeline(args, project, undo),
        "undo" => do_undo(project, undo),
        "redo" => do_redo(project, undo),
        "save_project" => save_project(project, project_path),
        other => bail!("unknown tool: {other}"),
    }
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn parse_uuid(args: &Value, key: &str) -> Result<Uuid> {
    let s = args
        .get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("missing or non-string `{key}`"))?;
    Uuid::parse_str(s).with_context(|| format!("invalid UUID in `{key}`: {s}"))
}

fn parse_u64(args: &Value, key: &str) -> Result<u64> {
    args.get(key)
        .and_then(Value::as_u64)
        .with_context(|| format!("missing or non-integer `{key}`"))
}

fn parse_f32(args: &Value, key: &str) -> Result<f32> {
    let n = args
        .get(key)
        .and_then(Value::as_f64)
        .with_context(|| format!("missing or non-number `{key}`"))?;
    Ok(n as f32)
}

fn clip_kind_str(t: &ClipType) -> String {
    let dbg = format!("{t:?}");
    dbg.split([' ', '{', '('])
        .next()
        .unwrap_or("Unknown")
        .to_string()
}

fn clip_exists(project: &ProjectState, id: Uuid) -> Result<()> {
    if project.clips.iter().any(|c| c.id == id) {
        Ok(())
    } else {
        bail!("no clip with id {id}")
    }
}

// ── read-only handlers ──────────────────────────────────────────────────────

fn get_project_summary(p: &ProjectState) -> Result<String> {
    let total_duration_ms: u64 = p
        .clips
        .iter()
        .map(|c| c.start_time_ms.saturating_add(c.duration_ms))
        .max()
        .unwrap_or(0);

    let summary = json!({
        "name": p.name,
        "aspect_ratio": format!("{:?}", p.aspect_ratio),
        "base_resolution": p.base_resolution,
        "frame_rate": format!("{:?}", p.frame_rate),
        "locale": p.locale,
        "total_duration_ms": total_duration_ms,
        "clip_count": p.clips.len(),
        "track_count": p.tracks.len(),
        "media_count": p.media.items.len(),
    });
    Ok(serde_json::to_string_pretty(&summary)?)
}

fn list_tracks(p: &ProjectState) -> Result<String> {
    let arr: Vec<Value> = p
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            json!({
                "index": i,
                "id": t.id,
                "name": t.name,
                "kind": format!("{:?}", t.kind),
                "muted": t.muted,
                "visible": t.visible,
                "locked": t.locked,
                "pinned": t.pinned,
                "height": t.height,
            })
        })
        .collect();
    Ok(serde_json::to_string_pretty(&arr)?)
}

fn list_clips(p: &ProjectState, args: &Value) -> Result<String> {
    let track_filter: Option<usize> = args
        .get("track_index")
        .and_then(Value::as_u64)
        .map(|n| n as usize);

    let arr: Vec<Value> = p
        .clips
        .iter()
        .filter(|c| track_filter.is_none_or(|t| c.track_index == t))
        .map(|c| {
            json!({
                "id": c.id,
                "track_index": c.track_index,
                "start_time_ms": c.start_time_ms,
                "duration_ms": c.duration_ms,
                "kind": clip_kind_str(&c.clip_type),
                "media_id": c.media_id,
                "speed": c.speed,
                "reversed": c.reversed,
                "flip_h": c.flip_h,
                "flip_v": c.flip_v,
                "volume_db": c.volume_db,
                "transition_in": c.transition_in,
                "transition_out": c.transition_out,
            })
        })
        .collect();
    Ok(serde_json::to_string_pretty(&arr)?)
}

fn list_media(p: &ProjectState) -> Result<String> {
    let arr: Vec<Value> = p
        .media
        .items
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "name": m.name,
                "path": m.path,
                "kind": format!("{:?}", m.kind),
                "duration_ms": m.duration_ms,
                "size_bytes": m.size_bytes,
                "added_at": m.added_at,
                "probe_done": m.probe_done,
                "thumb_done": m.thumb_done,
            })
        })
        .collect();
    Ok(serde_json::to_string_pretty(&arr)?)
}

// ── mutating handlers ───────────────────────────────────────────────────────

fn move_clip(args: &Value, project: &mut ProjectState, undo: &mut UndoStack) -> Result<String> {
    let clip_id = parse_uuid(args, "clip_id")?;
    let to_ms = parse_u64(args, "to_ms")?;
    clip_exists(project, clip_id)?;

    let mut cmd = SetClipCommand::new(clip_id).start_time_ms(to_ms);
    if let Some(t) = args.get("to_track").and_then(Value::as_u64) {
        cmd = cmd.track_index(t as usize);
    }

    undo.execute(Box::new(cmd), project)?;
    Ok(json!({ "ok": true, "clip_id": clip_id }).to_string())
}

fn split_clip(args: &Value, project: &mut ProjectState, undo: &mut UndoStack) -> Result<String> {
    let clip_id = parse_uuid(args, "clip_id")?;
    let at_ms = parse_u64(args, "at_ms")?;
    clip_exists(project, clip_id)?;

    undo.execute(Box::new(SplitClipCommand::new(clip_id, at_ms)), project)?;
    Ok(json!({ "ok": true }).to_string())
}

fn set_transition(
    args: &Value,
    project: &mut ProjectState,
    undo: &mut UndoStack,
) -> Result<String> {
    let clip_id = parse_uuid(args, "clip_id")?;
    let edge = args
        .get("edge")
        .and_then(Value::as_str)
        .context("missing or non-string `edge`")?;
    let in_edge = match edge {
        "in" => true,
        "out" => false,
        other => bail!("`edge` must be \"in\" or \"out\", got {other:?}"),
    };
    let transition_id = args.get("transition_id").and_then(|v| {
        if v.is_null() {
            None
        } else {
            v.as_str().map(String::from)
        }
    });
    clip_exists(project, clip_id)?;

    undo.execute(
        Box::new(SetTransitionCommand::new(clip_id, in_edge, transition_id)),
        project,
    )?;
    Ok(json!({ "ok": true }).to_string())
}

fn set_clip_volume(
    args: &Value,
    project: &mut ProjectState,
    undo: &mut UndoStack,
) -> Result<String> {
    let clip_id = parse_uuid(args, "clip_id")?;
    let db = parse_f32(args, "db")?;
    if !(-60.0..=20.0).contains(&db) {
        bail!("db out of range -60.0 .. 20.0 (got {db})");
    }
    clip_exists(project, clip_id)?;

    let cmd = SetClipCommand::new(clip_id).volume_db(db);
    undo.execute(Box::new(cmd), project)?;
    Ok(json!({ "ok": true, "db": db }).to_string())
}

fn set_clip_speed(
    args: &Value,
    project: &mut ProjectState,
    undo: &mut UndoStack,
) -> Result<String> {
    let clip_id = parse_uuid(args, "clip_id")?;
    let speed = parse_f32(args, "speed")?;
    if !(0.1..=100.0).contains(&speed) {
        bail!("speed out of range 0.1 .. 100.0 (got {speed})");
    }
    clip_exists(project, clip_id)?;

    let cmd = SetClipCommand::new(clip_id).speed(speed);
    undo.execute(Box::new(cmd), project)?;
    Ok(json!({ "ok": true, "speed": speed }).to_string())
}

fn set_clip_fade(args: &Value, project: &mut ProjectState, undo: &mut UndoStack) -> Result<String> {
    let clip_id = parse_uuid(args, "clip_id")?;
    let fade_in = args.get("fade_in_ms").and_then(Value::as_u64);
    let fade_out = args.get("fade_out_ms").and_then(Value::as_u64);
    if fade_in.is_none() && fade_out.is_none() {
        bail!("at least one of `fade_in_ms` / `fade_out_ms` is required");
    }
    clip_exists(project, clip_id)?;

    let mut cmd = SetClipCommand::new(clip_id);
    if let Some(v) = fade_in {
        cmd = cmd.fade_in_ms(v);
    }
    if let Some(v) = fade_out {
        cmd = cmd.fade_out_ms(v);
    }

    undo.execute(Box::new(cmd), project)?;
    Ok(json!({ "ok": true }).to_string())
}

fn add_clip_to_timeline(
    args: &Value,
    project: &mut ProjectState,
    undo: &mut UndoStack,
) -> Result<String> {
    let media_id = parse_uuid(args, "media_id")?;
    let track_index = args
        .get("track_index")
        .and_then(Value::as_u64)
        .context("missing or non-integer `track_index`")? as usize;
    let start_ms = parse_u64(args, "start_ms")?;

    if track_index >= project.tracks.len() {
        bail!(
            "track_index {track_index} out of bounds (0..{})",
            project.tracks.len()
        );
    }

    let media = project
        .media
        .items
        .iter()
        .find(|m| m.id == media_id)
        .with_context(|| format!("no media with id {media_id}"))?;

    let path = media.path.clone();
    let dur = media.duration_ms;
    let kind = media.kind;

    let clip = match kind {
        MediaKind::Video => Clip::new_video(&path, track_index, start_ms, dur),
        MediaKind::Audio => Clip::new_audio(&path, track_index, start_ms, dur),
        MediaKind::Image => Clip::new_image(&path, track_index, start_ms, dur),
    };
    let new_id = clip.id;

    undo.execute(Box::new(RippleInsertCommand::new(clip)), project)?;
    Ok(json!({ "ok": true, "clip_id": new_id }).to_string())
}

fn do_undo(project: &mut ProjectState, undo: &mut UndoStack) -> Result<String> {
    undo.undo(project)?;
    Ok(json!({ "ok": true }).to_string())
}

fn do_redo(project: &mut ProjectState, undo: &mut UndoStack) -> Result<String> {
    undo.redo(project)?;
    Ok(json!({ "ok": true }).to_string())
}

fn save_project(project: &ProjectState, path: &Path) -> Result<String> {
    project_io::save_project(project, path)?;
    Ok(json!({ "ok": true, "path": path.display().to_string() }).to_string())
}

// ── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_project() -> ProjectState {
        let mut p = ProjectState::default();
        p.tracks.clear();
        p.clips.clear();
        p.media.items.clear();
        p
    }

    fn project_with_one_clip() -> (ProjectState, Uuid) {
        let mut p = ProjectState::default();
        let c = Clip::new_video("x.mp4", 0, 0, 1000);
        let id = c.id;
        p.clips.push(c);
        (p, id)
    }

    // ── read-only ───────────────────────────────────────────────────────────

    #[test]
    fn summary_of_empty_project_is_zero_duration() {
        let p = empty_project();
        let s = get_project_summary(&p).unwrap();
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["total_duration_ms"], 0);
        assert_eq!(v["clip_count"], 0);
    }

    #[test]
    fn summary_reflects_default_tracks() {
        let p = ProjectState::default();
        let s = get_project_summary(&p).unwrap();
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["track_count"], p.tracks.len());
        assert!(!p.tracks.is_empty(), "default state has seed tracks");
    }

    #[test]
    fn list_tracks_empty_project() {
        let p = empty_project();
        let s = list_tracks(&p).unwrap();
        let v: Value = serde_json::from_str(&s).unwrap();
        assert!(v.as_array().unwrap().is_empty());
    }

    #[test]
    fn list_clips_empty_with_filter() {
        let p = empty_project();
        let s = list_clips(&p, &json!({ "track_index": 0 })).unwrap();
        let v: Value = serde_json::from_str(&s).unwrap();
        assert!(v.as_array().unwrap().is_empty());
    }

    #[test]
    fn list_media_empty_project() {
        let p = empty_project();
        let s = list_media(&p).unwrap();
        let v: Value = serde_json::from_str(&s).unwrap();
        assert!(v.as_array().unwrap().is_empty());
    }

    // ── mutating ────────────────────────────────────────────────────────────

    #[test]
    fn move_clip_changes_start_and_undo_restores() {
        let (mut p, id) = project_with_one_clip();
        let mut u = UndoStack::new();

        move_clip(&json!({ "clip_id": id, "to_ms": 5000 }), &mut p, &mut u).unwrap();
        assert_eq!(p.clips[0].start_time_ms, 5000);

        u.undo(&mut p).unwrap();
        assert_eq!(p.clips[0].start_time_ms, 0);
    }

    #[test]
    fn move_clip_with_track_change() {
        let (mut p, id) = project_with_one_clip();
        let mut u = UndoStack::new();

        move_clip(
            &json!({ "clip_id": id, "to_ms": 100, "to_track": 2 }),
            &mut p,
            &mut u,
        )
        .unwrap();
        assert_eq!(p.clips[0].start_time_ms, 100);
        assert_eq!(p.clips[0].track_index, 2);
    }

    #[test]
    fn move_clip_missing_arg_errors() {
        let (mut p, _) = project_with_one_clip();
        let mut u = UndoStack::new();
        let err = move_clip(&json!({ "to_ms": 100 }), &mut p, &mut u).unwrap_err();
        assert!(err.to_string().contains("clip_id"));
    }

    #[test]
    fn move_clip_unknown_clip_errors() {
        let (mut p, _) = project_with_one_clip();
        let mut u = UndoStack::new();
        let other = Uuid::new_v4();
        let err =
            move_clip(&json!({ "clip_id": other, "to_ms": 100 }), &mut p, &mut u).unwrap_err();
        assert!(err.to_string().contains("no clip"));
    }

    #[test]
    fn split_clip_creates_two_and_undo_reverts() {
        let (mut p, id) = project_with_one_clip();
        let mut u = UndoStack::new();

        split_clip(&json!({ "clip_id": id, "at_ms": 500 }), &mut p, &mut u).unwrap();
        assert_eq!(p.clips.len(), 2);

        u.undo(&mut p).unwrap();
        assert_eq!(p.clips.len(), 1);
    }

    #[test]
    fn set_volume_validates_range() {
        let (mut p, id) = project_with_one_clip();
        let mut u = UndoStack::new();
        let err =
            set_clip_volume(&json!({ "clip_id": id, "db": 50.0 }), &mut p, &mut u).unwrap_err();
        assert!(err.to_string().contains("out of range"));
    }

    #[test]
    fn set_volume_changes_db_and_undo() {
        let (mut p, id) = project_with_one_clip();
        let mut u = UndoStack::new();
        let before = p.clips[0].volume_db;

        set_clip_volume(&json!({ "clip_id": id, "db": -6.0 }), &mut p, &mut u).unwrap();
        assert_eq!(p.clips[0].volume_db, -6.0);

        u.undo(&mut p).unwrap();
        assert_eq!(p.clips[0].volume_db, before);
    }

    #[test]
    fn set_speed_validates_range() {
        let (mut p, id) = project_with_one_clip();
        let mut u = UndoStack::new();
        let err =
            set_clip_speed(&json!({ "clip_id": id, "speed": 0.01 }), &mut p, &mut u).unwrap_err();
        assert!(err.to_string().contains("out of range"));
    }

    #[test]
    fn set_speed_changes_value() {
        let (mut p, id) = project_with_one_clip();
        let mut u = UndoStack::new();
        set_clip_speed(&json!({ "clip_id": id, "speed": 2.0 }), &mut p, &mut u).unwrap();
        assert_eq!(p.clips[0].speed, 2.0);
    }

    #[test]
    fn set_fade_requires_at_least_one_arg() {
        let (mut p, id) = project_with_one_clip();
        let mut u = UndoStack::new();
        let err = set_clip_fade(&json!({ "clip_id": id }), &mut p, &mut u).unwrap_err();
        assert!(err.to_string().contains("at least one"));
    }

    #[test]
    fn set_transition_rejects_bad_edge() {
        let (mut p, id) = project_with_one_clip();
        let mut u = UndoStack::new();
        let err = set_transition(
            &json!({ "clip_id": id, "edge": "left", "transition_id": null }),
            &mut p,
            &mut u,
        )
        .unwrap_err();
        assert!(err.to_string().contains("edge"));
    }

    #[test]
    fn undo_on_empty_stack_is_noop() {
        let mut p = ProjectState::default();
        let mut u = UndoStack::new();
        let clips_before = p.clips.len();
        let tracks_before = p.tracks.len();
        // UndoStack::undo on an empty stack is a silent no-op, not an error.
        // Mutating tools are the only producers of stack entries, so this
        // path is only reachable when a client calls undo() before any
        // mutating call.
        let _ = do_undo(&mut p, &mut u);
        assert_eq!(p.clips.len(), clips_before);
        assert_eq!(p.tracks.len(), tracks_before);
    }

    #[test]
    fn redo_after_undo_restores_move() {
        let (mut p, id) = project_with_one_clip();
        let mut u = UndoStack::new();

        move_clip(&json!({ "clip_id": id, "to_ms": 900 }), &mut p, &mut u).unwrap();
        u.undo(&mut p).unwrap();
        assert_eq!(p.clips[0].start_time_ms, 0);

        do_redo(&mut p, &mut u).unwrap();
        assert_eq!(p.clips[0].start_time_ms, 900);
    }

    #[test]
    fn add_clip_rejects_unknown_media() {
        let mut p = ProjectState::default();
        let mut u = UndoStack::new();
        let bogus = Uuid::new_v4();
        let err = add_clip_to_timeline(
            &json!({ "media_id": bogus, "track_index": 0, "start_ms": 0 }),
            &mut p,
            &mut u,
        )
        .unwrap_err();
        assert!(err.to_string().contains("no media"));
    }

    #[test]
    fn add_clip_rejects_out_of_bounds_track() {
        let mut p = ProjectState::default();
        let mut u = UndoStack::new();
        let err = add_clip_to_timeline(
            &json!({ "media_id": Uuid::new_v4(), "track_index": 999, "start_ms": 0 }),
            &mut p,
            &mut u,
        )
        .unwrap_err();
        assert!(err.to_string().contains("out of bounds"));
    }

    #[test]
    fn unknown_tool_errors() {
        let mut p = ProjectState::default();
        let mut u = UndoStack::new();
        let err = call(
            "nope",
            &json!({}),
            &mut p,
            &mut u,
            std::path::Path::new("x.caprust"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown tool"));
    }
}
