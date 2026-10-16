//! Process-based import plugins.
//!
//! An importer is a separate executable that speaks newline-delimited
//! JSON on stdin/stdout. The host sends one request line and reads one
//! response line. Because the plugin is a process, it can be written in
//! any language and a crash cannot take the editor down with it.
//!
//! A plugin declares itself with a small manifest file next to the
//! executable, or the host can probe it by sending `{"method":"hello"}`.
//! The manifest is plain JSON so a plugin author does not need to know
//! anything about CapRust to get started:
//!
//! ```json
//! {
//!   "id": "com.example.my-importer",
//!   "name": "My Importer",
//!   "version": "0.1.0",
//!   "protocol": 1,
//!   "kinds": ["capcut", "otio"]
//! }
//! ```
//!
//! Protocol (v1):
//!   host → plugin  {"method":"hello"}
//!   plugin → host  {"id":<int>,"result":{"protocol":1,"name":"…"}}
//!
//!   host → plugin  {"method":"import","path":"<path>","kind":"<kind>"}
//!   plugin → host  {"id":<int>,"result":{"project":<ProjectState JSON>}}
//!
//!   host → plugin  {"method":"describe"}
//!   plugin → host  {"id":<int>,"result":{"kinds":["…"],"name":"…"}}
//!
//! The `project` field is a serialised [`ProjectState`]. The plugin
//! needs no link against CapRust to produce one: it writes the same
//! JSON the `.caprust` file contains. That keeps the plugin ABI a
//! stable data format rather than a Rust API, so a plugin written in
//! Python or Go works the same as one written in Rust.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Protocol version this host speaks.
pub const PROTOCOL: u32 = 1;

/// What the manifest file looks like on disk. Also the shape of the
/// `hello`/`describe` response, minus `protocol`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImporterManifest {
    /// Reverse-DNS identifier. Used as the stable key in settings.
    pub id: String,
    /// Human-readable name shown in the File menu.
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default = "default_protocol")]
    pub protocol: u32,
    /// Import kinds this plugin handles ("capcut", "otio", ...).
    #[serde(default)]
    pub kinds: Vec<String>,
    /// Whether the user picks one file or a folder. Defaults to `File`,
    /// which is what an interchange format is.
    #[serde(default)]
    pub target: ImportTarget,
}

/// What the user is asked to pick before a plugin runs.
///
/// The host used to decide this by matching the plugin's kind against a
/// hardcoded list, which meant a plugin was either in that list and got
/// a folder dialog or out of it and got a file dialog -- with no way for
/// the plugin to say which it actually needs. An OTIO timeline is a
/// single JSON file and was being asked for as a folder.
///
/// The plugin states it. `kinds` stays descriptive: it is what the
/// format calls itself, not a UI instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImportTarget {
    /// One file -- the common case for an interchange format.
    #[default]
    File,
    /// A folder the plugin reads a project out of, like a video
    /// editor's own project directory.
    Folder,
}

fn default_version() -> String {
    "0.0.0".to_string()
}

fn default_protocol() -> u32 {
    PROTOCOL
}

/// One request/response exchange with a plugin process.
struct PluginProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl PluginProcess {
    fn spawn(exe: &Path) -> Result<Self> {
        let mut child = Command::new(exe)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("spawn importer {}", exe.display()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("no stdin for {}", exe.display()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("no stdout for {}", exe.display()))?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        })
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let req = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.flush()?;

        let mut resp = String::new();
        let n = self.stdout.read_line(&mut resp)?;
        if n == 0 {
            anyhow::bail!("plugin closed before answering {method}");
        }
        let v: Value = serde_json::from_str(resp.trim())?;
        if let Some(err) = v.get("error") {
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("plugin error");
            anyhow::bail!("{msg}");
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }
}

impl Drop for PluginProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A discovered importer: its manifest plus the path to the executable.
#[derive(Debug, Clone, PartialEq)]
pub struct ImporterPlugin {
    pub manifest: ImporterManifest,
    pub exe: PathBuf,
}

/// Scan `dir` for `*.json` manifest files. Each manifest's `exe` field,
/// when present, points at the binary relative to the manifest; without
/// it the binary is assumed to sit beside the manifest with the same
/// stem and no extension (or `.exe` on Windows).
///
/// A malformed manifest is skipped rather than aborting the scan: one
/// bad file must not hide every good plugin.
pub fn discover(dir: &Path) -> Vec<ImporterPlugin> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<ImporterManifest>(&text) else {
            continue;
        };
        if manifest.protocol != PROTOCOL {
            continue;
        }
        // The exe is either named in a sibling field or derived from the
        // manifest stem.
        let exe = path.with_extension(if cfg!(windows) { "exe" } else { "" });
        if exe.is_file() {
            found.push(ImporterPlugin { manifest, exe });
        }
    }
    found
}

/// Overlay a plugin's project JSON onto a default `ProjectState`.
///
/// A plugin only has to know the fields it actually produced — name,
/// clips, tracks, media, frame rate, aspect ratio. Everything else
/// comes from the default, so a model change on this side (a new field,
/// a renamed one) does not break plugins that were written against an
/// older shape. That matters for third-party plugins: recompiling them
/// on every release would make the whole feature unusable.
///
/// The overlay recurses rather than replacing whole top-level fields. A
/// plugin that sends `{"media": {"items": [...]}}` must not have to know
/// that `MediaLibrary` also carries a private `next_added_at` counter
/// with no serde default — dropping it silently restarts the media bin's
/// sort order every time the user adds a file. Nested objects merge key
/// by key; arrays and scalars replace wholesale, because there is no
/// sensible partial merge of a list of clips.
///
/// Keys the host does not know are dropped at every level rather than
/// rejected, so a plugin built against a newer model still loads the
/// parts this build understands. The mirror-image gap — a field the host
/// has since renamed — resolves to the default rather than an error,
/// which is the right way round: the user still gets a project.
pub fn merge_project(base: &crate::ProjectState, incoming: &Value) -> Result<crate::ProjectState> {
    let mut merged = serde_json::to_value(base)?;
    overlay(&mut merged, incoming);
    Ok(serde_json::from_value(merged)?)
}

/// Merge `patch` into `base` in place, keeping only keys `base` already
/// has. Objects recurse; every other value replaces.
fn overlay(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            for (key, val) in p {
                if let Some(slot) = b.get_mut(key) {
                    overlay(slot, val);
                }
            }
        }
        (b, p) => *b = p.clone(),
    }
}

/// Ask a plugin to import `path` and return the project JSON it
/// produced. The caller overlays it on a default project with
/// [`merge_project`] and deserialises into `ProjectState`.
///
/// The plugin process is spawned fresh per import. Import is a
/// one-shot operation, so keeping it around would only leak handles;
/// a plugin that wants to stay warm can be implemented as a separate
/// long-lived kind later.
pub fn run_import(plugin: &ImporterPlugin, path: &Path) -> Result<crate::ProjectState> {
    let mut proc = PluginProcess::spawn(&plugin.exe)?;
    let result = proc.call("import", json!({"path": path, "kind": plugin.manifest.kinds.first().map(String::as_str).unwrap_or("")}))?;
    let patch = result
        .get("project")
        .ok_or_else(|| anyhow::anyhow!("plugin returned no project"))?;
    merge_project(&crate::ProjectState::default(), patch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_without_a_target_asks_for_a_file() {
        // Every plugin written before `target` existed means "a file",
        // which is the only reading of the old protocol that does not
        // change behaviour under it.
        let m: ImporterManifest =
            serde_json::from_str(r#"{"id":"org.x.y","name":"Y","protocol":1,"kinds":["otio"]}"#)
                .unwrap();
        assert_eq!(m.target, ImportTarget::File);
    }

    #[test]
    fn a_target_reads_back_from_its_stored_names() {
        for (target, text) in [
            (ImportTarget::File, r#""file""#),
            (ImportTarget::Folder, r#""folder""#),
        ] {
            let m = ImporterManifest {
                id: "org.x.y".into(),
                name: "Y".into(),
                version: "1".into(),
                protocol: PROTOCOL,
                kinds: vec!["y".into()],
                target,
            };
            let json = serde_json::to_string(&m).unwrap();
            assert!(json.contains(text), "{target:?} should serialise as {text}");
            let back: ImporterManifest = serde_json::from_str(&json).unwrap();
            assert_eq!(back.target, target);
        }
    }

    #[test]
    fn manifest_defaults_are_stable() {
        let m: ImporterManifest = serde_json::from_str(r#"{"id":"a","name":"A"}"#).unwrap();
        assert_eq!(m.protocol, PROTOCOL);
        assert_eq!(m.version, "0.0.0");
        assert!(m.kinds.is_empty());
    }

    #[test]
    fn merge_keeps_defaults_for_absent_fields() {
        let base = crate::ProjectState::default();
        let patch = json!({"name": "from plugin", "base_resolution": 3840});
        let merged = merge_project(&base, &patch).unwrap();
        assert_eq!(merged.name, "from plugin");
        assert_eq!(merged.base_resolution, 3840);
        // Untouched fields still hold their default -- including the
        // default track set, which is what a plugin gets for free when
        // it does not care about lanes.
        assert_eq!(merged.locale, base.locale);
        assert_eq!(merged.tracks.len(), base.tracks.len());
    }

    #[test]
    fn merge_keeps_nested_fields_the_plugin_does_not_know_about() {
        // `media` carries a private `next_added_at` with no serde
        // default. A plugin sending only `items` must not drop it.
        let base = crate::ProjectState::default();
        let as_json = serde_json::to_value(&base).unwrap();
        assert!(
            as_json["media"].get("next_added_at").is_some(),
            "premise: the media object does carry the counter"
        );
        let patch = json!({"media": {"items": []}});
        // Deserialising would fail outright if the counter were missing.
        merge_project(&base, &patch).expect("items-only media");
    }

    /// An array replaces wholesale. There is no partial merge of a list
    /// of clips, and there is no way to fill in a clip's missing fields
    /// from the default track set anyway -- the default is an empty
    /// clip list, not a template. A plugin therefore has to write every
    /// field its model types do not default, which is why both bundled
    /// plugins emit ids and the four track flags explicitly.
    #[test]
    fn merge_replaces_arrays_wholesale() {
        let base = crate::ProjectState::default();
        assert!(!base.tracks.is_empty(), "premise: defaults have tracks");
        let id = uuid::Uuid::new_v4();
        let patch = json!({"tracks": [{
            "id": id.to_string(),
            "name": "V1",
            "kind": "Video",
            "locked": false,
            "visible": true,
            "muted": true,
            "height": 80.0,
        }]});
        let merged = merge_project(&base, &patch).expect("a complete track");
        assert_eq!(merged.tracks.len(), 1, "the array replaced, not merged");
        assert_eq!(merged.tracks[0].id, id);
        assert!(merged.tracks[0].muted, "the plugin's own value wins");
        assert!(
            !merged.tracks[0].locked,
            "a field the plugin sent false and the default also has false"
        );
    }

    /// The other shape of the recursion: a nested *object* merges key by
    /// key, so a plugin that only knows the frame rate it read does not
    /// have to restate anything else about it.
    #[test]
    fn merge_fills_in_a_partial_nested_object() {
        let base = crate::ProjectState::default();
        let patch = json!({"frame_rate": {"num": 60}});
        let merged = merge_project(&base, &patch).expect("partial frame rate");
        assert_eq!(merged.frame_rate.num, 60);
        // `den` was not sent, so the default's denominator stands.
        assert_eq!(merged.frame_rate.den, base.frame_rate.den);
    }

    #[test]
    fn merge_ignores_keys_this_build_does_not_know() {
        let base = crate::ProjectState::default();
        let patch = json!({
            "name": "x",
            "field_from_the_future": {"a": 1},
            "media": {"items": [], "also_from_the_future": 2},
        });
        let merged = merge_project(&base, &patch).unwrap();
        assert_eq!(merged.name, "x");
    }

    #[test]
    fn merge_roundtrips_a_serialised_project_unchanged() {
        let base = crate::ProjectState {
            name: "round trip".into(),
            ..Default::default()
        };
        let as_json = serde_json::to_value(&base).unwrap();
        let merged = merge_project(&crate::ProjectState::default(), &as_json).unwrap();
        assert_eq!(merged.name, "round trip");
        assert_eq!(merged.base_resolution, base.base_resolution);
    }

    #[test]
    fn discover_skips_missing_dir() {
        let found = discover(Path::new("no/such/dir/anywhere"));
        assert!(found.is_empty());
    }

    #[test]
    fn discover_finds_manifest_beside_exe() {
        let dir = std::env::temp_dir().join("caprust-importer-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = json!({"id":"test.x","name":"Test","protocol":1,"kinds":["demo"]});
        std::fs::write(dir.join("p.json"), manifest.to_string()).unwrap();
        // Touch the exe the scan looks for.
        let exe = if cfg!(windows) {
            dir.join("p.exe")
        } else {
            dir.join("p")
        };
        std::fs::write(&exe, b"").unwrap();
        let found = discover(&dir);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].manifest.id, "test.x");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
