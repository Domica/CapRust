//! Timeline templates: save / export / import / apply.
//!
//! A template is a `ProjectState` serialized as JSON with the
//! `.caprust-template` extension. Applying a template replaces the
//! current timeline (with a confirm step in the UI); media that is
//! missing on disk flows through the normal missing-media banner +
//! relink dialog, so templates shared between machines keep working.
//!
//! Sources:
//! * bundled — `templates/*.caprust-template.json` in the repo root,
//!   embedded via `include_str!`. Community contributions land here
//!   via PR (see templates/README.md).
//! * user — `%APPDATA%/CapRust/templates/` (written by Save as
//!   template and Import in the app).

use crate::project::ProjectState;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Extension for template files (JSON, same schema as `.caprust`).
pub const TEMPLATE_EXT: &str = "caprust-template";

/// One bundled starter template: (id, display name, embedded JSON).
pub struct BundledTemplate {
    pub id: &'static str,
    pub display: &'static str,
    pub json: &'static str,
}

macro_rules! bundled {
    ($id:literal, $display:literal, $file:literal) => {
        BundledTemplate {
            id: $id,
            display: $display,
            json: include_str!($file),
        }
    };
}

/// Starter templates shipped with the app. Files live in
/// `templates/` at the repo root so contributors can add more.
pub fn bundled_templates() -> [BundledTemplate; 3] {
    [
        bundled!(
            "intro-title",
            "Intro title card",
            "../../../templates/intro-title.caprust-template.json"
        ),
        bundled!(
            "lower-third",
            "Lower third",
            "../../../templates/lower-third.caprust-template.json"
        ),
        bundled!(
            "outro-subscribe",
            "Outro subscribe card",
            "../../../templates/outro-subscribe.caprust-template.json"
        ),
    ]
}

/// Parse a bundled template into a project state.
pub fn parse_bundled(t: &BundledTemplate) -> Result<ProjectState> {
    let mut state: ProjectState =
        serde_json::from_str(t.json).with_context(|| format!("parse bundled template {}", t.id))?;
    state.normalize_xfade_shifts();
    Ok(state)
}

/// User templates folder: `%APPDATA%/CapRust/templates/`.
pub fn user_templates_dir() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("CapRust").join("templates")
}

/// A template file on disk.
#[derive(Debug, Clone)]
pub struct TemplateFile {
    pub name: String,
    pub path: PathBuf,
}

/// List user templates, sorted by name. Missing dir = empty list.
pub fn list_user_templates() -> Vec<TemplateFile> {
    let dir = user_templates_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<TemplateFile> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(TEMPLATE_EXT))
        .map(|p| TemplateFile {
            name: p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("template")
                .to_string(),
            path: p,
        })
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// Sanitize a template name into a safe filename stem.
pub fn sanitize_template_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim().to_string();
    if s.is_empty() {
        "template".to_string()
    } else {
        s
    }
}

/// Save the current project as a named user template. Overwrites a
/// same-named template. Returns the written path.
pub fn save_template(state: &ProjectState, name: &str) -> Result<PathBuf> {
    let dir = user_templates_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("create dir {}", dir.display()))?;
    save_template_to(state, name, &dir)
}

/// `save_template` into an explicit directory (used by tests).
pub fn save_template_to(state: &ProjectState, name: &str, dir: &Path) -> Result<PathBuf> {
    let path = dir.join(format!("{}.{TEMPLATE_EXT}", sanitize_template_name(name)));
    let json = serde_json::to_string_pretty(state).context("serialize template")?;
    std::fs::write(&path, json).with_context(|| format!("write {}", path.display()))?;
    tracing::info!("template saved to {}", path.display());
    Ok(path)
}

/// Load a template file (user or imported) into a project state.
pub fn load_template(path: &Path) -> Result<ProjectState> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut state: ProjectState = serde_json::from_str(&text).context("parse template JSON")?;
    state.normalize_xfade_shifts();
    Ok(state)
}

/// Copy an external `.caprust-template` file into the user folder.
pub fn import_template_file(src: &Path) -> Result<PathBuf> {
    let dir = user_templates_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("create dir {}", dir.display()))?;
    import_template_file_to(src, &dir)
}

/// `import_template_file` into an explicit directory (used by tests).
pub fn import_template_file_to(src: &Path, dir: &Path) -> Result<PathBuf> {
    let name = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("template");
    // Validate before copying: garbage in, error out (not a broken entry).
    let text = std::fs::read_to_string(src).with_context(|| format!("read {}", src.display()))?;
    let _: ProjectState = serde_json::from_str(&text).context("parse template JSON")?;
    let dst = dir.join(format!("{}.{TEMPLATE_EXT}", sanitize_template_name(name)));
    std::fs::copy(src, &dst).with_context(|| format!("copy to {}", dst.display()))?;
    tracing::info!("template imported to {}", dst.display());
    Ok(dst)
}

/// Delete a user template file. Missing file = Ok.
pub fn delete_template(path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_file(path).with_context(|| format!("delete {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_state() -> ProjectState {
        let mut s = ProjectState {
            name: "tmpl".into(),
            ..ProjectState::default()
        };
        s.clips
            .push(crate::clip::Clip::new_text("Hello", 0, 0, 2000, false));
        s
    }

    #[test]
    fn template_roundtrip_preserves_clips() {
        let dir = std::env::temp_dir().join(format!("caprust_tmpl_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let s = sample_state();
        let path = save_template_to(&s, "My Intro!", &dir).unwrap();
        assert!(path.ends_with(format!("My Intro_.{TEMPLATE_EXT}")));
        let back = load_template(&path).unwrap();
        assert_eq!(back.clips.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sanitize_handles_garbage_names() {
        assert_eq!(sanitize_template_name("a/b:c"), "a_b_c");
        assert_eq!(sanitize_template_name("   "), "template");
        assert_eq!(sanitize_template_name("Intro 01"), "Intro 01");
    }

    #[test]
    fn bundled_templates_parse() {
        for t in bundled_templates() {
            let state = parse_bundled(&t).expect(t.id);
            assert!(!state.clips.is_empty(), "{} has no clips", t.id);
        }
    }

    #[test]
    fn import_rejects_garbage_json() {
        let dir = std::env::temp_dir().join(format!("caprust_tmpl_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("bad.caprust-template");
        std::fs::write(&bad, b"{not json").unwrap();
        assert!(import_template_file_to(&bad, &dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
