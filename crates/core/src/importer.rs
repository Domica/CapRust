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

/// Ask a plugin to import `path` and return the project JSON it
/// produced. The caller deserialises into `ProjectState`.
///
/// The plugin process is spawned fresh per import. Import is a
/// one-shot operation, so keeping it around would only leak handles;
/// a plugin that wants to stay warm can be implemented as a separate
/// long-lived kind later.
pub fn run_import(plugin: &ImporterPlugin, path: &Path) -> Result<Value> {
    let mut proc = PluginProcess::spawn(&plugin.exe)?;
    let result = proc.call("import", json!({"path": path, "kind": plugin.manifest.kinds.first().map(String::as_str).unwrap_or("")}))?;
    let project = result
        .get("project")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("plugin returned no project"))?;
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_defaults_are_stable() {
        let m: ImporterManifest = serde_json::from_str(r#"{"id":"a","name":"A"}"#).unwrap();
        assert_eq!(m.protocol, PROTOCOL);
        assert_eq!(m.version, "0.0.0");
        assert!(m.kinds.is_empty());
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
