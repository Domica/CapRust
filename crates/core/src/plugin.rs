//! Master-bus CLAP plugin chain model (Faza I).
//!
//! A `PluginInstance` is a loaded CLAP plugin attached to the project's
//! master bus. The instance stores its CLAP descriptor id + path so it
//! can be reloaded on project open, plus a parameter vector.
//!
//! Parameter values are stored as `Vec<(u32, f32)>` rather than a
//! `HashMap`: serde-JSON is cleaner, ordering is deterministic (useful
//! for `render_hash`), and real chains have <50 params per instance.
//!
//! Live scanning and instantiation happen in `caprust-media-io`. This
//! module is model + serialization only — no CLAP linkage in `core`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginInstance {
    /// Per-instance id. Stable across reloads of the same project; used
    /// by `SetPluginParamCommand` to target exactly one instance.
    pub id: Uuid,
    /// CLAP descriptor id, e.g. `com.u-he.ZebraHZ`.
    pub plugin_id: String,
    /// Path to the `.clap` binary at the time of insertion. Re-resolved
    /// against the scanner on project open.
    pub path: PathBuf,
    /// Display name as reported by the plugin descriptor.
    pub name: String,
    #[serde(default)]
    pub bypassed: bool,
    /// `(parameter_id, value)` pairs. Value is always normalized 0..1
    /// per CLAP spec; the plugin maps it to its own range.
    #[serde(default)]
    pub params: Vec<(u32, f32)>,
}

impl PluginInstance {
    pub fn new(plugin_id: impl Into<String>, path: PathBuf, name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            plugin_id: plugin_id.into(),
            path,
            name: name.into(),
            bypassed: false,
            params: Vec::new(),
        }
    }

    /// Set or replace a parameter. Values outside 0..=1 are rejected
    /// (CLAP spec) and leave `params` untouched.
    pub fn set_param(&mut self, param_id: u32, value: f32) -> bool {
        if !(0.0..=1.0).contains(&value) {
            return false;
        }
        for (id, v) in self.params.iter_mut() {
            if *id == param_id {
                *v = value;
                return true;
            }
        }
        self.params.push((param_id, value));
        true
    }

    pub fn get_param(&self, param_id: u32) -> Option<f32> {
        self.params
            .iter()
            .find(|(id, _)| *id == param_id)
            .map(|(_, v)| *v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance() -> PluginInstance {
        PluginInstance::new("com.example.Demo", PathBuf::from("/x/Demo.clap"), "Demo")
    }

    #[test]
    fn new_defaults_are_empty() {
        let p = instance();
        assert!(!p.bypassed);
        assert!(p.params.is_empty());
    }

    #[test]
    fn set_param_appends_then_replaces() {
        let mut p = instance();
        assert!(p.set_param(1, 0.5));
        assert_eq!(p.get_param(1), Some(0.5));
        assert!(p.set_param(1, 0.75));
        assert_eq!(p.params.len(), 1);
        assert_eq!(p.get_param(1), Some(0.75));
    }

    #[test]
    fn set_param_rejects_out_of_range() {
        let mut p = instance();
        assert!(!p.set_param(1, 1.5));
        assert!(!p.set_param(1, -0.1));
        assert!(p.params.is_empty());
    }

    #[test]
    fn serde_roundtrip() {
        let mut p = instance();
        p.set_param(7, 0.3);
        p.bypassed = true;
        let j = serde_json::to_string(&p).unwrap();
        let back: PluginInstance = serde_json::from_str(&j).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn deserializes_without_optional_fields() {
        // Simulate an older file that lacks `bypassed` and `params`.
        let raw = r#"{
            "id": "00000000-0000-0000-0000-000000000001",
            "plugin_id": "x",
            "path": "/x.clap",
            "name": "X"
        }"#;
        let p: PluginInstance = serde_json::from_str(raw).unwrap();
        assert!(!p.bypassed);
        assert!(p.params.is_empty());
    }
}
