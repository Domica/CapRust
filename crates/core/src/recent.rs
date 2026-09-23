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
    }
}

pub fn now_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
