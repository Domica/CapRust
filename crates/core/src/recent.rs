//! Recent-projects list, persisted with app settings.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentProject {
    pub path: String,
    pub name: String,
    pub duration_ms: u64,
    /// Unix seconds.
    pub last_opened: u64,
    pub clip_count: usize,
    /// Base resolution in pixels (short side); 0 on old entries
    /// written before this field existed.
    #[serde(default)]
    pub base_resolution: u32,
    /// Frame rate label, e.g. "24.00". Empty on old entries.
    #[serde(default)]
    pub frame_rate_label: String,
    /// Id of the first media item in the project. Used to look up
    /// the cached thumbnail JPEG at
    /// `<project_dir>/cache/thumbnails/<id>.jpg` on the start screen.
    /// None on old entries; the card falls back to a placeholder icon.
    #[serde(default)]
    pub first_media_id: Option<uuid::Uuid>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecentList {
    pub items: Vec<RecentProject>,
}

impl RecentList {
    pub const MAX: usize = 12;

    /// Insert or move to front.
    pub fn push(&mut self, item: RecentProject) {
        self.items.retain(|x| x.path != item.path);
        self.items.insert(0, item);
        self.items.truncate(Self::MAX);
    }

    /// Remove an entry (does not delete the file).
    pub fn forget(&mut self, path: &str) {
        self.items.retain(|x| x.path != path);
    }

    /// Entry currently at the given path, if any.
    pub fn find(&self, path: &str) -> Option<&RecentProject> {
        self.items.iter().find(|x| x.path == path)
    }
}

/// Build a RecentProject from a ProjectState + its on-disk path.
pub fn entry_from(state: &crate::project::ProjectState, path: &str) -> RecentProject {
    let duration_ms = state
        .clips
        .iter()
        .map(|c| c.start_time_ms + c.duration_ms)
        .max()
        .unwrap_or(0);
    RecentProject {
        path: path.to_string(),
        name: state.name.clone(),
        duration_ms,
        last_opened: now_unix(),
        clip_count: state.clips.len(),
        base_resolution: state.base_resolution,
        frame_rate_label: state.frame_rate.label(),
        first_media_id: state.media.items.first().map(|m| m.id),
    }
}

pub fn now_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_recent_json_deserializes_with_defaults() {
        // Shape written before base_resolution / frame_rate_label
        // existed. Must still parse; new fields default to 0 / "".
        let json = r#"
        {
            "path": "/tmp/a.caprust",
            "name": "A",
            "duration_ms": 1000,
            "last_opened": 42,
            "clip_count": 3
        }
        "#;
        let rp: RecentProject = serde_json::from_str(json).expect("parse");
        assert_eq!(rp.base_resolution, 0);
        assert!(rp.frame_rate_label.is_empty());
        assert!(rp.first_media_id.is_none());
    }

    #[test]
    fn new_fields_round_trip() {
        let rp = RecentProject {
            path: "/tmp/b.caprust".into(),
            name: "B".into(),
            duration_ms: 5000,
            last_opened: 99,
            clip_count: 2,
            base_resolution: 1080,
            frame_rate_label: "24.00".into(),
            first_media_id: None,
        };
        let json = serde_json::to_string(&rp).expect("ser");
        let back: RecentProject = serde_json::from_str(&json).expect("de");
        assert_eq!(back.base_resolution, 1080);
        assert_eq!(back.frame_rate_label, "24.00");
    }
}
