//! CLAP plugin scanner (Phase I).
//!
//! Non-recursive walk over the two standard CLAP locations:
//! - `C:\Program Files\Common Files\CLAP` (system-wide)
//! - `%APPDATA%/CapRust/plugins` (user-local)
//!
//! Each `.clap` file is loaded with `clack-host` to read its first
//! descriptor. Plugin binaries are untrusted code (DIRECTIVES 12) —
//! every load is wrapped in `catch_unwind`.
//!
//! Feature-gated behind `clap` so Linux CI without the CLAP SDK can
//! build the workspace without pulling `clack-host` (same pattern as
//! `ffmpeg`, DIRECTIVES 10.6).

use std::ffi::CStr;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use clack_host::prelude::PluginEntry;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginInfo {
    pub path: PathBuf,
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
}

/// Directories scanned by default, in order.
pub fn default_scan_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(target_os = "windows")]
    {
        dirs.push(PathBuf::from(r"C:\Program Files\Common Files\CLAP"));
    }
    if let Some(base) = appdata_base() {
        dirs.push(base.join("plugins"));
    }
    dirs
}

fn appdata_base() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("CapRust"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config").join("caprust"))
    }
}

/// True when `path` is a `.clap` file sitting directly in one of `dirs`.
///
/// Plugin paths stored in a project file are untrusted: loading one is
/// native code execution. Only plugins from the scanned directories are
/// allowed; paths are canonicalized so `..`, symlinks and UNC shares
/// cannot sneak past the prefix check.
pub fn is_trusted_plugin_path_in(path: &Path, dirs: &[PathBuf]) -> bool {
    let Ok(p) = path.canonicalize() else {
        return false;
    };
    if p.extension().and_then(|e| e.to_str()) != Some("clap") {
        return false;
    }
    let Some(parent) = p.parent() else {
        return false;
    };
    dirs.iter()
        .filter_map(|d| d.canonicalize().ok())
        .any(|d| d == parent)
}

/// [`is_trusted_plugin_path_in`] against the default scan directories.
pub fn is_trusted_plugin_path(path: &Path) -> bool {
    is_trusted_plugin_path_in(path, &default_scan_dirs())
}

/// Scan a list of directories. Missing directories are silently skipped.
pub fn scan_dirs(dirs: &[PathBuf]) -> Vec<PluginInfo> {
    let mut out = Vec::new();
    for d in dirs {
        out.extend(scan_dir(d));
    }
    out
}

/// Scan a single directory (non-recursive) for `.clap` files.
pub fn scan_dir(dir: &Path) -> Vec<PluginInfo> {
    if !dir.is_dir() {
        return Vec::new();
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("clap") {
            continue;
        }
        if let Some(info) = load_descriptor(&path) {
            out.push(info);
        }
    }
    out
}

fn load_descriptor(path: &Path) -> Option<PluginInfo> {
    // Untrusted code (DIRECTIVES 12). catch_unwind so a bad plugin
    // cannot take the scanner down with it. PluginEntry::load is unsafe
    // because the CLAP ABI is not verifiable by the type system.
    catch_unwind(AssertUnwindSafe(|| unsafe {
        let entry = PluginEntry::load(path).ok()?;
        let factory = entry.get_plugin_factory()?;
        let desc = factory.plugin_descriptors().next()?;
        Some(PluginInfo {
            path: path.to_path_buf(),
            id: cstr_to_string(desc.id()),
            name: cstr_to_string(desc.name()),
            vendor: cstr_to_string(desc.vendor()),
            version: cstr_to_string(desc.version()),
        })
    }))
    .ok()
    .flatten()
}

fn cstr_to_string(c: Option<&CStr>) -> String {
    c.map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(label: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("caprust_clap_{}_{}", label, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn missing_dir_yields_empty() {
        assert!(scan_dir(Path::new("Z:/caprust_definitely_missing_xyz")).is_empty());
    }

    #[test]
    fn empty_dir_yields_empty() {
        let d = tmp_dir("empty");
        assert!(scan_dir(&d).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ignores_non_clap_extensions() {
        let d = tmp_dir("ext");
        std::fs::write(d.join("readme.txt"), b"nope").unwrap();
        std::fs::write(d.join("lib.dll"), b"nope").unwrap();
        assert!(scan_dir(&d).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn trust_requires_scan_dir_and_clap_extension() {
        let trusted = tmp_dir("trusted");
        let other = tmp_dir("other");
        let ok = trusted.join("a.clap");
        let wrong_ext = trusted.join("a.dll");
        let outside = other.join("a.clap");
        for f in [&ok, &wrong_ext, &outside] {
            std::fs::write(f, b"x").unwrap();
        }
        let dirs = vec![trusted.clone()];
        assert!(is_trusted_plugin_path_in(&ok, &dirs));
        assert!(!is_trusted_plugin_path_in(&wrong_ext, &dirs));
        assert!(!is_trusted_plugin_path_in(&outside, &dirs));
        // `..` traversal back out of the trusted dir resolves outside it.
        let sneaky = trusted
            .join("..")
            .join(other.file_name().unwrap())
            .join("a.clap");
        assert!(!is_trusted_plugin_path_in(&sneaky, &dirs));
        assert!(!is_trusted_plugin_path_in(
            &trusted.join("missing.clap"),
            &dirs
        ));
        let _ = std::fs::remove_dir_all(&trusted);
        let _ = std::fs::remove_dir_all(&other);
    }

    #[test]
    fn bogus_clap_does_not_panic() {
        let d = tmp_dir("bogus");
        std::fs::write(d.join("fake.clap"), b"not a real plugin").unwrap();
        let out = scan_dir(&d);
        assert!(out.is_empty(), "bogus .clap must not yield an entry");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn plugin_info_serde_roundtrip() {
        let p = PluginInfo {
            path: PathBuf::from(r"C:\plugins\Demo.clap"),
            id: "com.example.Demo".into(),
            name: "Demo".into(),
            vendor: "Example".into(),
            version: "1.0".into(),
        };
        let j = serde_json::to_string(&p).unwrap();
        let back: PluginInfo = serde_json::from_str(&j).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn scan_dirs_with_missing_dirs_is_empty() {
        let out = scan_dirs(&[
            PathBuf::from("Z:/caprust_nope_1"),
            PathBuf::from("Z:/caprust_nope_2"),
        ]);
        assert!(out.is_empty());
    }

    #[test]
    #[ignore = "requires ZebraHZ at a fixed path; run locally with --ignored"]
    fn scan_zebra_returns_metadata() {
        let dir = Path::new(r"F:\Minimax H3\ZEBRA CLAP");
        if !dir.is_dir() {
            eprintln!("skipping: ZebraHZ not at {}", dir.display());
            return;
        }
        let out = scan_dir(dir);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "com.u-he.ZebraHZ");
        assert_eq!(out[0].name, "ZebraHZ");
        assert_eq!(out[0].vendor, "u-he");
    }
}
