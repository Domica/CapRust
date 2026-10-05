//! Multi-camera grouping.
//!
//! A MultiCamGroup references existing timeline clips by UUID
//! (one per camera angle). It does not own tracks or clips; the
//! render path uses the group to decide which angle is live at
//! each timeline position. See DIRECTIVES 28.3 (Faza Q).
//!
//! PR 1 stores the grouping only. Sync offsets (PR 2) and the
//! render path (PR 3) are follow-ups.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MultiCamGroup {
    pub id: Uuid,
    pub name: String,
    /// One clip id per camera angle, in display order (A, B, C, ...).
    /// The clips live on whatever tracks the user placed them on;
    /// the group does not own their layout.
    pub angle_clip_ids: Vec<Uuid>,
    /// Index into `angle_clip_ids` of the angle the render
    /// path currently uses. Non-active angles are suppressed
    /// at plan time. Defaults to 0 so old files render as
    /// before (all angles present, first wins z-order ties).
    #[serde(default)]
    pub active_angle: usize,
    /// Per-angle offset in milliseconds relative to angle[0].
    /// Empty = no sync applied yet (PR 2 fills it).
    #[serde(default)]
    pub sync_offsets_ms: Vec<i64>,
}

impl MultiCamGroup {
    pub fn new(name: impl Into<String>, angle_clip_ids: Vec<Uuid>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            angle_clip_ids,
            active_angle: 0,
            sync_offsets_ms: Vec::new(),
        }
    }

    pub fn angle_count(&self) -> usize {
        self.angle_clip_ids.len()
    }
    /// Clip id of the currently active angle, if the index is in
    /// bounds. Returns None when the group has no clips yet.
    pub fn active_clip_id(&self) -> Option<Uuid> {
        self.angle_clip_ids.get(self.active_angle).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults() {
        let ids = vec![Uuid::new_v4(), Uuid::new_v4()];
        let g = MultiCamGroup::new("cam", ids.clone());
        assert_eq!(g.angle_clip_ids, ids);
        assert!(g.sync_offsets_ms.is_empty());
        assert_eq!(g.angle_count(), 2);
    }

    #[test]
    fn serde_round_trip() {
        let g = MultiCamGroup::new("x", vec![Uuid::new_v4()]);
        let j = serde_json::to_string(&g).unwrap();
        let back: MultiCamGroup = serde_json::from_str(&j).unwrap();
        assert_eq!(g, back);
    }

    #[test]
    fn legacy_json_missing_sync_offsets() {
        let id = Uuid::new_v4();
        let clip = Uuid::new_v4();
        let json = format!(r#"{{"id":"{id}","name":"x","angle_clip_ids":["{clip}"]}}"#);
        let g: MultiCamGroup = serde_json::from_str(&json).unwrap();
        assert!(g.sync_offsets_ms.is_empty());
    }
}
