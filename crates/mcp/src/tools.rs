//! MCP tool definitions and handlers.
//!
//! Two groups:
//! - read-only — pure inspection of `ProjectState`
//! - mutating  — every change goes through `UndoStack::execute`, per
//!   DIRECTIVES §6 and §35.3. No direct mutation of `ProjectState`.

use std::path::Path;

use anyhow::{bail, Context, Result};
use caprust_core::clip::{Clip, ClipType};
use caprust_core::commands::add_media::AddMediaCommand;
use caprust_core::commands::ripple::RippleInsertCommand;
use caprust_core::commands::set_clip::SetClipCommand;
use caprust_core::commands::set_effect::{
    AddEffectCommand, RemoveEffectCommand, SetTransitionCommand, SetTransitionDurationCommand,
};
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
            "name": "remove_effect",
            "description": "Remove one applied effect or filter from a clip, by the same id `apply_preset` used to add it. `list_clips` reports the applied ids.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_id":   { "type": "string" },
                    "effect_id": { "type": "string" }
                },
                "required": ["clip_id", "effect_id"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "set_transition",
            "description": "Set or clear the in/out transition on a clip. transition_id=null clears. Pass `duration_ms` to also change the transition length; it is ignored when clearing. Effective length is clamped to half of either neighbouring clip, so a request above that returns the clamped value.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_id":       { "type": "string" },
                    "edge":          { "type": "string", "enum": ["in", "out"] },
                    "transition_id": { "type": ["string", "null"] },
                    "duration_ms":   { "type": "integer", "minimum": 1, "maximum": 5000,
                                       "description": "Optional. Changes the transition length in milliseconds. The clip and every clip after it on the same track shift so the overlap stays exact." }
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
            "name": "preview_transport",
            "description": "Drive the preview: `play`, `pause`, `toggle`, or `seek` to `at_ms`. Seeks are clamped to the project duration.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["play", "pause", "toggle", "seek", "seek_back", "seek_fwd"] },
                    "at_ms": { "type": "integer", "minimum": 0, "description": "Required for `seek`." },
                    "by_ms": { "type": "integer", "minimum": 1, "description": "Step for `seek_back` and `seek_fwd`. Defaults to 5000." }
                },
                "required": ["action"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "preview_status",
            "description": "Whether the preview is playing and where its playhead is, in milliseconds.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "export_settings",
            "description": "The export settings the computer will use, each with the accepted values. Change one with `set_export_setting`.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "set_export_setting",
            "description": "Change one export setting, using a value from `export_settings`.",
            "inputSchema": {
                "type": "object",
                "properties": { "key": { "type": "string" }, "value": { "type": "string" } },
                "required": ["key", "value"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "preview_frame",
            "description": "The current preview frame as base64 JPEG. Returns `frame: null` before the first frame has decoded, which is normal. `width` caps the longest side; 320 is a sensible default for a phone.",
            "inputSchema": {
                "type": "object",
                "properties": { "width": { "type": "integer", "minimum": 64, "maximum": 1280 } },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "start_export",
            "description": "Start an export of the open project to `destination`. Returns as soon as it is accepted; poll `export_status` for progress.",
            "inputSchema": {
                "type": "object",
                "properties": { "destination": { "type": "string" } },
                "required": ["destination"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "export_status",
            "description": "Export progress as `progress` (0..1) and `destination`, plus whether anything has started.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "list_recent_projects",
            "description": "List recently opened projects, newest first. Each entry carries the `path` that `open_project` expects.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "open_project",
            "description": "Switch the running CapRust session to another project. Undo history and open media are not carried over.",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "list_assets",
            "description": "List the ids of the transition, effect and filter presets that can be applied to a clip. Returns three arrays under `transitions`, `effects` and `filters`.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "apply_preset",
            "description": "Apply a transition, effect or filter preset to a clip. `preset_id` \"none\" clears a transition or filter. Effects are added; there is no \"none\" effect.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_id":   { "type": "string" },
                    "kind":      { "type": "string", "enum": ["transition", "effect", "filter"] },
                    "preset_id": { "type": "string" },
                    "edge":      { "type": "string", "enum": ["in", "out"], "description": "Which edge a transition attaches to. Defaults to \"in\". Ignored for effects and filters." }
                },
                "required": ["clip_id", "kind", "preset_id"],
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
        json!({
            "name": "add_media",
            "description": "Register a file from disk into the project's media library. Kind is inferred from extension unless kind is supplied.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "kind": { "type": "string", "enum": ["video", "audio", "image"] }
                },
                "required": ["path"],
                "additionalProperties": false
            }
        }),
    ]
}

pub fn call<'a, 's>(
    name: &str,
    args: &Value,
    project: &mut ProjectState,
    undo: &mut UndoStack,
    project_path: &Path,
    // Two lifetimes, deliberately. `&mut dyn Trait` would tie the
    // borrow to the trait object's lifetime, and `&mut` is invariant in
    // its target -- so the reborrow a caller has at hand could never be
    // passed in. Naming the object lifetime separately lets the short
    // borrow and the long object lifetime be different values.
    session: Option<&'s mut (dyn crate::server::SessionControl + 'a)>,
) -> Result<String> {
    match name {
        // read-only
        "get_project_summary" => get_project_summary(project),
        "list_tracks" => list_tracks(project),
        "list_clips" => list_clips(project, args),
        "list_media" => list_media(project),
        "list_assets" => list_assets(),
        "list_recent_projects" => list_recent_projects(session),
        "preview_frame" => preview_frame(args, session),
        "preview_transport" => preview_transport(args, session),
        "preview_status" => preview_status(session),
        "export_settings" => export_settings(session),
        "set_export_setting" => set_export_setting(args, session),
        "start_export" => start_export(args, session),
        "export_status" => export_status(session),
        "open_project" => open_project(args, session),
        // mutating
        "move_clip" => move_clip(args, project, undo),
        "split_clip" => split_clip(args, project, undo),
        "set_transition" => set_transition(args, project, undo),
        "set_clip_volume" => set_clip_volume(args, project, undo),
        "set_clip_speed" => set_clip_speed(args, project, undo),
        "set_clip_fade" => set_clip_fade(args, project, undo),
        "add_clip_to_timeline" => add_clip_to_timeline(args, project, undo),
        "apply_preset" => apply_preset(args, project, undo),
        "remove_effect" => remove_effect(args, project, undo),
        "undo" => do_undo(project, undo),
        "redo" => do_redo(project, undo),
        "save_project" => save_project(project, project_path),
        "add_media" => add_media(args, project, undo),
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

/// The preset ids a client can choose from.
///
/// Ids only. A client that can draw previews wants thumbnails and
/// labels; one that cannot -- a phone over a socket -- wants a list it
/// can render in a plain ListView without shipping an asset pipeline
/// for it. Both are served by this; the extra fields are simply
/// absent rather than null, so a client checking `has("thumb")` gets
/// a clean false.
fn list_assets() -> Result<String> {
    use caprust_core::presets::{table, PresetKind};
    let ids = |k: PresetKind| -> Vec<Value> { table(k).iter().map(|p| json!(p.id)).collect() };
    Ok(json!({
        "transitions": ids(PresetKind::Transition),
        "effects": ids(PresetKind::Effect),
        "filters": ids(PresetKind::Filter),
    })
    .to_string())
}

/// The recent-projects list, newest first.
///
/// Returns the full path alongside the display name because `path` is
/// what `open_project` takes. A client showing a name and sending back
/// an index would break the moment the list is re-sorted, which is
/// exactly what opening a project does.
fn list_recent_projects<'a, 's>(
    session: Option<&'s mut (dyn crate::server::SessionControl + 'a)>,
) -> Result<String> {
    let Some(s) = session else {
        bail!("this server is pinned to one project and has no recent list");
    };
    let items = s.recent();
    let arr: Vec<Value> = items
        .iter()
        .map(|r| {
            json!({
                "path": r.path,
                "name": r.name,
                "clip_count": r.clip_count,
                "duration_ms": r.duration_ms,
                "last_opened": r.last_opened,
            })
        })
        .collect();
    // `count` is redundant with the array length and stays in the
    // response deliberately: an empty array and an empty recent list
    // are the same thing here, and a client that cannot tell "the
    // desktop has no recent projects" from "the call silently failed"
    // has no way to report which one happened.
    Ok(json!({ "projects": arr, "count": items.len() }).to_string())
}

/// Switch the host to another project.
///
/// Validates the path before asking. The host is the one that decides
/// whether a project can actually be loaded -- it may need to save
/// first, or may have media locked -- so a failure here is the host's
/// message, passed through rather than replaced.
fn open_project<'a, 's>(
    args: &Value,
    session: Option<&'s mut (dyn crate::server::SessionControl + 'a)>,
) -> Result<String> {
    let Some(s) = session else {
        bail!("this server is pinned to one project and cannot switch");
    };
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .context("missing or non-string `path`")?;
    if path.trim().is_empty() {
        bail!("`path` is empty");
    }
    let p = std::path::PathBuf::from(path);
    if !p.exists() {
        bail!("no file at {path}");
    }
    if p.extension().and_then(|e| e.to_str()) != Some("caprust") {
        bail!("{path} is not a .caprust file");
    }
    s.request_open(&p).map_err(anyhow::Error::msg)?;
    Ok(json!({ "ok": true, "opening": path }).to_string())
}

/// Drive the preview transport.
fn preview_transport<'a, 's>(
    args: &Value,
    session: Option<&'s mut (dyn crate::server::SessionControl + 'a)>,
) -> Result<String> {
    let Some(s) = session else {
        bail!("this server has no preview");
    };
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .context("missing or non-string `action`")?;
    let at_ms = args.get("at_ms").and_then(Value::as_u64);
    let by_ms = args
        .get("by_ms")
        .and_then(Value::as_u64)
        // Five seconds matches the desktop transport. A client that
        // omits it gets the same step the buttons on the computer do.
        .unwrap_or(5000)
        .max(1);
    if action == "seek" && at_ms.is_none() {
        bail!("`seek` needs `at_ms`");
    }
    // The relative seeks need the current position to be relative to,
    // and the host is the only one that knows it.
    let at_ms = match action {
        "seek_back" => Some(
            s.preview_status()
                .map(|(_, at)| at)
                .unwrap_or(0)
                .saturating_sub(by_ms),
        ),
        "seek_fwd" => Some(s.preview_status().map(|(_, at)| at).unwrap_or(0) + by_ms),
        _ => at_ms,
    };
    let handled = s
        .preview_transport(action, at_ms)
        .map_err(anyhow::Error::msg)?;
    Ok(json!({ "ok": true, "handled": handled }).to_string())
}

fn preview_status<'a, 's>(
    session: Option<&'s mut (dyn crate::server::SessionControl + 'a)>,
) -> Result<String> {
    let Some(s) = session else {
        bail!("this server has no preview");
    };
    match s.preview_status() {
        Some((playing, at_ms)) => Ok(json!({ "playing": playing, "at_ms": at_ms }).to_string()),
        // No status is a valid answer from a host that has no preview,
        // and reporting it as an error would put a red line on a screen
        // that is working exactly as the host allows.
        None => Ok(json!({ "playing": false, "at_ms": Value::Null }).to_string()),
    }
}

/// The export settings and the values each one accepts.
///
/// The host owns the vocabulary. A client that hardcoded a list of
/// resolutions would break the day the desktop adds one, and would
/// happily send a value the encoder has never heard of.
fn export_settings<'a, 's>(
    session: Option<&'s mut (dyn crate::server::SessionControl + 'a)>,
) -> Result<String> {
    let Some(s) = session else {
        bail!("this server cannot export");
    };
    let Some(v) = s.export_settings() else {
        bail!("this host does not expose its export settings");
    };
    Ok(v.to_string())
}

fn set_export_setting<'a, 's>(
    args: &Value,
    session: Option<&'s mut (dyn crate::server::SessionControl + 'a)>,
) -> Result<String> {
    let Some(s) = session else {
        bail!("this server cannot export");
    };
    let key = args
        .get("key")
        .and_then(Value::as_str)
        .context("missing or non-string `key`")?;
    let value = args
        .get("value")
        .and_then(Value::as_str)
        .context("missing or non-string `value`")?;
    s.set_export_setting(key, value)
        .map_err(anyhow::Error::msg)?;
    Ok(json!({ "ok": true, "key": key, "value": value }).to_string())
}

/// The current preview frame, base64 JPEG.
///
/// `Ok(None)` from the host is a real answer, not an error: nothing has
/// decoded yet. Reporting that as a failure would make the client show
/// an error on a preview that is merely still warming up.
fn preview_frame<'a, 's>(
    args: &Value,
    session: Option<&'s mut (dyn crate::server::SessionControl + 'a)>,
) -> Result<String> {
    let width = args
        .get("width")
        .and_then(Value::as_u64)
        .unwrap_or(320)
        .clamp(64, 1280) as u32;
    let Some(s) = session else {
        bail!("this server has no preview");
    };
    match s.preview_frame(width).map_err(anyhow::Error::msg)? {
        Some(jpeg) => Ok(json!({
            "frame": jpeg,
            "encoding": "jpeg",
            "width": width,
        })
        .to_string()),
        None => Ok(json!({ "frame": Value::Null }).to_string()),
    }
}

/// Ask the host to export.
///
/// The destination is validated here, not by the host: an empty path
/// would otherwise be reported as a vague failure after the UI has
/// already opened its export window.
fn start_export<'a, 's>(
    args: &Value,
    session: Option<&'s mut (dyn crate::server::SessionControl + 'a)>,
) -> Result<String> {
    let Some(s) = session else {
        bail!("this server cannot export");
    };
    let dest = args
        .get("destination")
        .and_then(Value::as_str)
        .context("missing or non-string `destination`")?;
    if dest.trim().is_empty() {
        bail!("`destination` is empty");
    }
    s.request_export(dest).map_err(anyhow::Error::msg)?;
    Ok(json!({ "ok": true, "destination": dest }).to_string())
}

fn export_status<'a, 's>(
    session: Option<&'s mut (dyn crate::server::SessionControl + 'a)>,
) -> Result<String> {
    let Some(s) = session else {
        bail!("this server cannot export");
    };
    let p = s.export_progress();
    Ok(json!({
        "started": p.is_some(),
        "progress": p,
    })
    .to_string())
}

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
                // Duration and the shift it produced. Without these a
                // client can set a transition but cannot read back what
                // was actually stored, and the render clamps the request
                // to half of either neighbouring clip, so the value the
                // client sent is not always the value in effect.
                "transition_duration_ms": c.transition_duration_ms,
                "applied_xfade_shift_ms": c.applied_xfade_shift_ms,
                // The applied effect/filter instances, so a client can
                // show what is on a clip and remove one by id rather
                // than guessing which is which.
                "effects": c.effects.iter().map(|e| json!({
                    "effect_id": e.effect_id,
                    "amount": e.amount,
                    "enabled": e.enabled,
                })).collect::<Vec<_>>(),
                "lut": c.lut,
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

/// Apply one asset-browser preset to a clip.
///
/// Mirrors exactly what the desktop does with a clicked preset card,
/// including which command backs each kind. The reason it is one tool
/// rather than three is that the desktop's own handler already treats
/// them as one action -- Effects and Filters are the same command on
/// different tables, and Transitions differ only in which edge they
/// touch -- so splitting them here would invite a client to invent a
/// fourth interpretation.
///
/// The id is validated against the table before anything runs. An
/// unknown id would otherwise reach `build_one_effect`, match no arm,
/// and render as no change at all: a silent no-op that looks like
/// success on both sides.
fn apply_preset(args: &Value, project: &mut ProjectState, undo: &mut UndoStack) -> Result<String> {
    use caprust_core::presets::{self, PresetKind};

    let clip_id = parse_uuid(args, "clip_id")?;
    let kind_str = args
        .get("kind")
        .and_then(Value::as_str)
        .context("missing or non-string `kind`")?;
    let kind = PresetKind::parse(kind_str).with_context(|| {
        format!("`kind` must be transition, effect or filter, got {kind_str:?}")
    })?;
    let preset_id = args
        .get("preset_id")
        .and_then(Value::as_str)
        .context("missing or non-string `preset_id`")?;

    if !presets::contains(kind, preset_id) {
        bail!(
            "`{preset_id}` is not a {} preset; call list_assets for the valid ids",
            kind.as_str()
        );
    }
    clip_exists(project, clip_id)?;

    match kind {
        PresetKind::Transition => {
            let edge = args.get("edge").and_then(Value::as_str).unwrap_or("in");
            let in_edge = match edge {
                "in" => true,
                "out" => false,
                other => bail!("`edge` must be \"in\" or \"out\", got {other:?}"),
            };
            let t = (preset_id != presets::CLEAR).then(|| preset_id.to_string());
            undo.execute(
                Box::new(SetTransitionCommand::new(clip_id, in_edge, t)),
                project,
            )?;
        }
        PresetKind::Effect | PresetKind::Filter => {
            // No "none" in the effects table, so there is nothing to
            // clear here: the caller removes the instance by id.
            undo.execute(Box::new(AddEffectCommand::new(clip_id, preset_id)), project)?;
        }
    }
    Ok(json!({ "ok": true, "kind": kind.as_str(), "preset_id": preset_id }).to_string())
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

    let still_set = transition_id.is_some();

    undo.execute(
        Box::new(SetTransitionCommand::new(clip_id, in_edge, transition_id)),
        project,
    )?;

    // Duration is a second, separate command so that setting a
    // transition and then changing its length are two undoable steps,
    // matching what the desktop panel does. Skipped when clearing: a
    // cleared transition has no length to set, and writing one would
    // leave a stale value behind for the next transition to inherit.
    let requested_duration = args.get("duration_ms").and_then(Value::as_u64);
    if still_set {
        if let Some(ms) = requested_duration {
            if !(1..=5000).contains(&ms) {
                bail!("duration_ms out of range 1 .. 5000 (got {ms})");
            }
            undo.execute(
                Box::new(SetTransitionDurationCommand::new(clip_id, ms)),
                project,
            )?;
        }
    }

    // Report what was actually stored rather than what was asked for.
    // The render clamps to half of either neighbouring clip, so a
    // request for 3000 on a 2s clip is honoured as 1000 and a client
    // that trusted its own input would draw the wrong overlap.
    let (stored_id, stored_dur, stored_shift, stored_start) = project
        .clips
        .iter()
        .find(|c| c.id == clip_id)
        .map(|c| {
            let id = if in_edge {
                c.transition_in.clone()
            } else {
                c.transition_out.clone()
            };
            (
                id,
                c.transition_duration_ms,
                c.applied_xfade_shift_ms,
                c.start_time_ms,
            )
        })
        .unwrap_or((None, 0, 0, 0));

    Ok(json!({
        "ok": true,
        "transition_id": stored_id,
        "duration_ms": stored_dur,
        "applied_shift_ms": stored_shift,
        "start_time_ms": stored_start,
    })
    .to_string())
}

fn remove_effect(
    args: &Value,
    project: &mut ProjectState,
    undo: &mut UndoStack,
) -> Result<String> {
    let clip_id = parse_uuid(args, "clip_id")?;
    let effect_id = args
        .get("effect_id")
        .and_then(Value::as_str)
        .context("missing or non-string `effect_id`")?
        .to_string();
    clip_exists(project, clip_id)?;

    // Say so when there was nothing to remove. Silently succeeding
    // leaves the client showing an effect the project does not have,
    // and the user has no way to tell which side is wrong.
    let present = project
        .clips
        .iter()
        .find(|c| c.id == clip_id)
        .is_some_and(|c| c.effects.iter().any(|e| e.effect_id == effect_id));
    if !present {
        bail!("clip has no effect `{effect_id}`; call list_clips to see what is applied");
    }

    undo.execute(
        Box::new(RemoveEffectCommand::new(clip_id, effect_id.clone())),
        project,
    )?;
    Ok(json!({ "ok": true, "removed": effect_id }).to_string())
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

fn add_media(args: &Value, project: &mut ProjectState, undo: &mut UndoStack) -> Result<String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .context("missing or non-string `path`")?;

    if !Path::new(path).is_file() {
        bail!("file does not exist: {path}");
    }

    let kind = match args.get("kind").and_then(Value::as_str) {
        Some("video") => MediaKind::Video,
        Some("audio") => MediaKind::Audio,
        Some("image") => MediaKind::Image,
        Some(other) => bail!("unknown media kind: {other:?}"),
        None => infer_kind(path).with_context(|| {
            format!("could not infer media kind from {path}; pass `kind` explicitly")
        })?,
    };

    undo.execute(Box::new(AddMediaCommand::new(path, kind)), project)?;

    let new_id = project.media.items.last().map(|m| m.id);
    Ok(json!({ "ok": true, "media_id": new_id }).to_string())
}

fn infer_kind(path: &str) -> Option<MediaKind> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())?
        .to_ascii_lowercase();
    match ext.as_str() {
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v" => Some(MediaKind::Video),
        "mp3" | "wav" | "m4a" | "aac" | "flac" | "ogg" => Some(MediaKind::Audio),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" => Some(MediaKind::Image),
        _ => None,
    }
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

    /// Setting a transition with a length is the whole point of the
    /// `duration_ms` argument. Before it existed, `SetTransitionDurationCommand`
    /// was unreachable over MCP: a client could apply `diag_bl` and had
    /// no way to change how long it ran.
    #[test]
    fn set_transition_with_duration_moves_the_overlap() {
        let mut p = ProjectState::default();
        let a = Clip::new_video("a.mp4", 0, 0, 8000);
        let b = Clip::new_video("b.mp4", 0, 8000, 8000);
        let id = b.id;
        p.clips.push(a);
        p.clips.push(b);
        let mut u = UndoStack::new();

        let out = set_transition(
            &json!({
                "clip_id": id,
                "edge": "in",
                "transition_id": "fade",
                "duration_ms": 1000,
            }),
            &mut p,
            &mut u,
        )
        .unwrap();

        let v: Value = serde_json::from_str(&out).unwrap();
        let clip = p.clips.iter().find(|c| c.id == id).unwrap();
        assert_eq!(clip.transition_duration_ms, 1000);
        // 8s clips: half of either is 4000, so 1000 is honoured whole.
        assert_eq!(clip.applied_xfade_shift_ms, 1000);
        assert_eq!(clip.start_time_ms, 7000, "b pulled 1000ms left of a");

        // The reply reports the stored value, so a client that draws
        // the overlap uses the same number the render will.
        assert_eq!(v["duration_ms"], 1000);
        assert_eq!(v["applied_shift_ms"], 1000);
        assert_eq!(v["start_time_ms"], 7000);
    }

    /// A length above half of either neighbouring clip is clamped, and
    /// the reply has to say so. A client that trusted its own input
    /// would draw an overlap the render never produces.
    #[test]
    fn set_transition_duration_reports_the_clamped_value() {
        let mut p = ProjectState::default();
        // 2s clips, so half is 1000ms.
        let a = Clip::new_video("a.mp4", 0, 0, 2000);
        let b = Clip::new_video("b.mp4", 0, 2000, 2000);
        let id = b.id;
        p.clips.push(a);
        p.clips.push(b);
        let mut u = UndoStack::new();

        let out = set_transition(
            &json!({ "clip_id": id, "edge": "in", "transition_id": "fade", "duration_ms": 1800 }),
            &mut p,
            &mut u,
        )
        .unwrap();

        let v: Value = serde_json::from_str(&out).unwrap();
        // The field keeps what was asked for; the shift is what the
        // render actually applies.
        assert_eq!(v["duration_ms"], 1800, "the request is stored as sent");
        assert_eq!(
            v["applied_shift_ms"], 1000,
            "but the overlap is clamped to half the clip"
        );
        let clip = p.clips.iter().find(|c| c.id == id).unwrap();
        assert_eq!(clip.start_time_ms, 1000);
    }

    /// Clearing a transition must not write a duration. A stale length
    /// left behind is inherited by the next transition the user applies
    /// and silently changes it.
    #[test]
    fn clearing_a_transition_ignores_duration() {
        let (mut p, id) = project_with_one_clip();
        let mut u = UndoStack::new();

        set_transition(
            &json!({ "clip_id": id, "edge": "in", "transition_id": "fade", "duration_ms": 1200 }),
            &mut p,
            &mut u,
        )
        .unwrap();

        let out = set_transition(
            &json!({
                "clip_id": id, "edge": "in", "transition_id": null,
                "duration_ms": 900,
            }),
            &mut p,
            &mut u,
        )
        .unwrap();

        let clip = p.clips.iter().find(|c| c.id == id).unwrap();
        assert!(clip.transition_in.is_none());
        assert_eq!(
            clip.transition_duration_ms, 1200,
            "clearing leaves the old length alone rather than writing a new one"
        );
        let v: Value = serde_json::from_str(&out).unwrap();
        assert!(v["transition_id"].is_null());
    }

    #[test]
    fn remove_effect_reports_when_there_is_nothing_to_remove() {
        let (mut p, id) = project_with_one_clip();
        let mut u = UndoStack::new();
        let err = remove_effect(
            &json!({ "clip_id": id, "effect_id": "grain" }),
            &mut p,
            &mut u,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("list_clips"),
            "the message should point at the tool that lists what is applied: {err}"
        );
    }

    #[test]
    fn remove_effect_takes_one_off_and_leaves_the_rest() {
        let (mut p, id) = project_with_one_clip();
        let mut u = UndoStack::new();
        apply_preset(
            &json!({ "clip_id": id, "kind": "effect", "preset_id": "grain" }),
            &mut p,
            &mut u,
        )
        .unwrap();
        apply_preset(
            &json!({ "clip_id": id, "kind": "effect", "preset_id": "zoom_pulse" }),
            &mut p,
            &mut u,
        )
        .unwrap();

        remove_effect(
            &json!({ "clip_id": id, "effect_id": "grain" }),
            &mut p,
            &mut u,
        )
        .unwrap();

        let clip = p.clips.iter().find(|c| c.id == id).unwrap();
        let ids: Vec<&str> = clip.effects.iter().map(|e| e.effect_id.as_str()).collect();
        assert_eq!(ids, vec!["zoom_pulse"]);
    }

    #[test]
    fn list_clips_reports_transition_duration_and_effects() {
        let (mut p, id) = project_with_one_clip();
        let mut u = UndoStack::new();
        apply_preset(
            &json!({ "clip_id": id, "kind": "effect", "preset_id": "grain" }),
            &mut p,
            &mut u,
        )
        .unwrap();

        let out = list_clips(&p, &json!({})).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let first = &v[0];
        assert!(first.get("transition_duration_ms").is_some());
        assert!(first.get("applied_xfade_shift_ms").is_some());
        assert_eq!(first["effects"][0]["effect_id"], "grain");
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
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown tool"));
    }

    #[test]
    fn add_media_rejects_missing_file() {
        let mut p = ProjectState::default();
        let mut u = UndoStack::new();
        let err = add_media(
            &json!({ "path": "Z:/caprust_definitely_missing_xyz.mp4" }),
            &mut p,
            &mut u,
        )
        .unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn add_media_infers_video_kind_and_adds() {
        let tmp = std::env::temp_dir().join(format!("caprust_mcp_{}.mp4", Uuid::new_v4()));
        std::fs::write(&tmp, b"fake").unwrap();

        let mut p = ProjectState::default();
        let mut u = UndoStack::new();
        let before = p.media.items.len();

        add_media(&json!({ "path": tmp.to_string_lossy() }), &mut p, &mut u).unwrap();
        assert_eq!(p.media.items.len(), before + 1);
        assert!(matches!(
            p.media.items.last().unwrap().kind,
            MediaKind::Video
        ));

        u.undo(&mut p).unwrap();
        assert_eq!(p.media.items.len(), before);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn add_media_unknown_extension_errors() {
        let tmp = std::env::temp_dir().join(format!("caprust_mcp_{}.qqq", Uuid::new_v4()));
        std::fs::write(&tmp, b"fake").unwrap();

        let mut p = ProjectState::default();
        let mut u = UndoStack::new();
        let err = add_media(&json!({ "path": tmp.to_string_lossy() }), &mut p, &mut u).unwrap_err();
        assert!(err.to_string().contains("could not infer"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn add_media_explicit_kind_overrides_inference() {
        let tmp = std::env::temp_dir().join(format!("caprust_mcp_{}.bin", Uuid::new_v4()));
        std::fs::write(&tmp, b"fake").unwrap();

        let mut p = ProjectState::default();
        let mut u = UndoStack::new();
        add_media(
            &json!({ "path": tmp.to_string_lossy(), "kind": "audio" }),
            &mut p,
            &mut u,
        )
        .unwrap();
        assert!(matches!(
            p.media.items.last().unwrap().kind,
            MediaKind::Audio
        ));
        let _ = std::fs::remove_file(&tmp);
    }

    // ── presets ─────────────────────────────────────────────────────────────

    #[test]
    fn list_assets_returns_all_three_tables() {
        let v: Value = serde_json::from_str(&list_assets().unwrap()).unwrap();
        use caprust_core::presets::{table, PresetKind};
        for (key, kind) in [
            ("transitions", PresetKind::Transition),
            ("effects", PresetKind::Effect),
            ("filters", PresetKind::Filter),
        ] {
            assert_eq!(
                v[key].as_array().unwrap().len(),
                table(kind).len(),
                "{key} length disagrees with the core table"
            );
        }
    }

    #[test]
    fn list_assets_ids_are_actually_applicable() {
        // The list is the only thing a phone shows the user. An id in
        // it that apply_preset then rejects is a dead row in the list.
        use caprust_core::presets::PresetKind;
        let v: Value = serde_json::from_str(&list_assets().unwrap()).unwrap();
        for (key, kind) in [
            ("transitions", PresetKind::Transition),
            ("effects", PresetKind::Effect),
            ("filters", PresetKind::Filter),
        ] {
            for id in v[key].as_array().unwrap() {
                let id = id.as_str().unwrap();
                let (mut p, clip) = project_with_one_clip();
                let mut u = UndoStack::new();
                apply_preset(
                    &json!({ "clip_id": clip, "kind": kind.as_str(), "preset_id": id }),
                    &mut p,
                    &mut u,
                )
                .unwrap_or_else(|e| panic!("listed {key} id `{id}` was rejected: {e}"));
            }
        }
    }

    #[test]
    fn apply_preset_rejects_an_id_from_the_wrong_table() {
        // "fade" is both a transition and a filter. Applying it as an
        // effect would reach build_one_effect, match no arm, and change
        // nothing while reporting ok.
        let (mut p, clip) = project_with_one_clip();
        let mut u = UndoStack::new();
        let e = apply_preset(
            &json!({ "clip_id": clip, "kind": "effect", "preset_id": "fade" }),
            &mut p,
            &mut u,
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("not a effect preset"), "{e}");
    }

    #[test]
    fn apply_preset_rejects_unknown_kind() {
        let (mut p, clip) = project_with_one_clip();
        let mut u = UndoStack::new();
        assert!(apply_preset(
            &json!({ "clip_id": clip, "kind": "texture", "preset_id": "blur" }),
            &mut p,
            &mut u
        )
        .is_err());
    }

    #[test]
    fn transition_none_clears_rather_than_appends() {
        let (mut p, clip) = project_with_one_clip();
        let mut u = UndoStack::new();
        apply_preset(
            &json!({ "clip_id": clip, "kind": "transition", "preset_id": "fade" }),
            &mut p,
            &mut u,
        )
        .unwrap();
        assert_eq!(p.clips[0].transition_in.as_deref(), Some("fade"));

        apply_preset(
            &json!({ "clip_id": clip, "kind": "transition", "preset_id": "none" }),
            &mut p,
            &mut u,
        )
        .unwrap();
        assert_eq!(p.clips[0].transition_in, None);
    }

    #[test]
    fn effect_attaches_to_the_clip() {
        let (mut p, clip) = project_with_one_clip();
        let mut u = UndoStack::new();
        apply_preset(
            &json!({ "clip_id": clip, "kind": "effect", "preset_id": "vignette" }),
            &mut p,
            &mut u,
        )
        .unwrap();
        assert!(p.clips[0].effects.iter().any(|e| e.effect_id == "vignette"));
    }

    #[test]
    fn filter_uses_the_same_command_as_an_effect() {
        // Both go through AddEffectCommand on the desktop too; the
        // only difference is which table the id came from.
        let (mut p, clip) = project_with_one_clip();
        let mut u = UndoStack::new();
        apply_preset(
            &json!({ "clip_id": clip, "kind": "filter", "preset_id": "cinematic" }),
            &mut p,
            &mut u,
        )
        .unwrap();
        assert!(p.clips[0]
            .effects
            .iter()
            .any(|e| e.effect_id == "cinematic"));
    }

    #[test]
    fn transition_edge_out_is_honoured() {
        let (mut p, clip) = project_with_one_clip();
        let mut u = UndoStack::new();
        apply_preset(
            &json!({
                "clip_id": clip, "kind": "transition",
                "preset_id": "wipe_l", "edge": "out"
            }),
            &mut p,
            &mut u,
        )
        .unwrap();
        assert_eq!(p.clips[0].transition_out.as_deref(), Some("wipe_l"));
        assert_eq!(p.clips[0].transition_in, None);
    }

    #[test]
    fn apply_preset_rejects_an_unknown_clip() {
        let mut p = empty_project();
        let mut u = UndoStack::new();
        assert!(apply_preset(
            &json!({ "clip_id": Uuid::new_v4(), "kind": "effect", "preset_id": "blur" }),
            &mut p,
            &mut u
        )
        .is_err());
    }

    #[test]
    fn apply_preset_is_undoable() {
        // A phone has no undo shortcut worth relying on, but the desktop
        // stack is shared, so a remote edit has to leave the same
        // trail a local one does.
        let (mut p, clip) = project_with_one_clip();
        let mut u = UndoStack::new();
        apply_preset(
            &json!({ "clip_id": clip, "kind": "effect", "preset_id": "vignette" }),
            &mut p,
            &mut u,
        )
        .unwrap();
        assert!(p.clips[0].effects.iter().any(|e| e.effect_id == "vignette"));
        do_undo(&mut p, &mut u).unwrap();
        assert!(p.clips[0].effects.is_empty());
    }

    // ── session / project switching ────────────────────────────────────────

    /// Stand-in for the running app.
    struct FakeSession {
        items: Vec<caprust_core::recent::RecentProject>,
        opened: Option<String>,
        reject: Option<String>,
    }

    impl FakeSession {
        fn new() -> Self {
            Self {
                items: Vec::new(),
                opened: None,
                reject: None,
            }
        }

        fn with(name: &str, path: &str) -> Self {
            Self {
                items: vec![caprust_core::recent::RecentProject {
                    path: path.into(),
                    name: name.into(),
                    duration_ms: 1500,
                    last_opened: 1_700_000_000,
                    clip_count: 4,
                    base_resolution: 1080,
                    frame_rate_label: "30.00".into(),
                    first_media_id: None,
                }],
                opened: None,
                reject: None,
            }
        }
    }

    impl crate::server::SessionControl for FakeSession {
        fn recent(&self) -> Vec<caprust_core::recent::RecentProject> {
            self.items.clone()
        }
        fn request_open(&mut self, path: &std::path::Path) -> Result<(), String> {
            if let Some(why) = &self.reject {
                return Err(why.clone());
            }
            self.opened = Some(path.to_string_lossy().to_string());
            Ok(())
        }
        fn request_export(&mut self, _destination: &str) -> Result<(), String> {
            Err("this fake cannot export".into())
        }
    }

    #[test]
    fn list_recent_projects_returns_name_and_path() {
        let mut s = FakeSession::with("My Cut", "C:/p/MyCut.caprust");
        let v: Value = serde_json::from_str(&list_recent_projects(Some(&mut s)).unwrap()).unwrap();
        let first = &v["projects"][0];
        assert_eq!(first["name"], "My Cut");
        // The path is what open_project takes. Sending an index instead
        // would break the moment the list re-sorts, which opening does.
        assert_eq!(first["path"], "C:/p/MyCut.caprust");
    }

    #[test]
    fn open_project_asks_the_host_to_load() {
        let mut s = FakeSession::new();
        let f = std::env::temp_dir().join(format!("caprust_{}.caprust", Uuid::new_v4()));
        std::fs::write(&f, b"{}").unwrap();
        let args = json!({ "path": f.to_string_lossy() });
        open_project(&args, Some(&mut s)).unwrap();
        assert_eq!(s.opened.as_deref(), Some(f.to_string_lossy().as_ref()));
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn open_project_surfaces_the_hosts_own_error() {
        // The host knows things the tool does not: unsaved changes,
        // locked media. Replacing its message with a generic one would
        // hide the only useful part.
        let mut s = FakeSession::new();
        s.reject = Some("unsaved changes on the open project".into());
        let f = std::env::temp_dir().join(format!("caprust_{}.caprust", Uuid::new_v4()));
        std::fs::write(&f, b"{}").unwrap();
        let args = json!({ "path": f.to_string_lossy() });
        let e = open_project(&args, Some(&mut s)).unwrap_err().to_string();
        assert!(e.contains("unsaved changes"), "{e}");
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn open_project_rejects_a_path_that_is_not_there() {
        let mut s = FakeSession::new();
        let args = json!({ "path": "Z:/caprust_missing_xyz.caprust" });
        assert!(open_project(&args, Some(&mut s)).is_err());
        assert!(s.opened.is_none(), "a bad path must not reach the host");
    }

    #[test]
    fn open_project_rejects_a_non_project_extension() {
        let mut s = FakeSession::new();
        let f = std::env::temp_dir().join(format!("caprust_{}.mp4", Uuid::new_v4()));
        std::fs::write(&f, b"x").unwrap();
        let args = json!({ "path": f.to_string_lossy() });
        assert!(open_project(&args, Some(&mut s)).is_err());
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn session_tools_say_so_when_there_is_no_session() {
        // The standalone binary is pinned to one file. A clear message
        // beats a silently ignored call that reports ok.
        assert!(list_recent_projects(None).is_err());
        let args = json!({ "path": "x.caprust" });
        let e = open_project(&args, None).unwrap_err().to_string();
        assert!(e.contains("pinned"), "{e}");
    }
}
