//! Crash-recovery autosave.
//!
//! The project file on disk is the user's manual Save (Ctrl+S). If the
//! app crashes, panics, or the disk fills, everything since that Save
//! is gone. This module writes a periodic snapshot to a *separate*
//! recovery folder so the session survives without ever touching the
//! user's real project file.
//!
//! Design constraints:
//!
//! * Never write to the user's project path. A corrupt recovery write
//!   must not be able to destroy the real project.
//! * Keep a small ring of previous snapshots. A crash mid-write can
//!   leave a truncated JSON file, so the newest is not always the one
//!   to restore.
//! * Key the snapshot by the project *file* path so two projects in
//!   the same folder never clobber each other, and an unsaved project
//!   (no path yet) cannot be autosaved at all.
//!
//! Files land in `%APPDATA%/CapRust/recovery/` (temp dir fallback on
//! non-Windows), never next to the project.

use crate::project::ProjectState;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How many snapshots to keep per project. The newest is written to
/// slot 0; older ones shift down and the last is dropped.
pub const KEEP_SNAPSHOTS: usize = 3;

/// Bumped if the envelope shape changes. Older snapshots with an
/// unknown-higher version are ignored rather than misparsed.
const RECOVERY_FORMAT_VERSION: u32 = 1;

/// Recovery folder: `%APPDATA%/CapRust/recovery/`.
pub fn recovery_dir() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("CapRust").join("recovery")
}

/// Envelope written around the project snapshot. Keeping the metadata
/// outside the project means a project from a future version can be
/// detected and refused instead of half-loaded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoverySnapshot {
    /// Format version of this envelope.
    pub caprust_recovery_version: u32,
    /// App version that wrote the snapshot.
    pub app_version: String,
    /// Unix seconds when this snapshot was written. Drives the
    /// "recover changes from 12:04?" label.
    pub saved_at: u64,
    /// Path of the project file this recovers, so a moved project can
    /// still be matched up.
    pub project_file: String,
    /// `render_hash` of the project at snapshot time. Lets the app
    /// skip the prompt when nothing actually changed.
    pub state_hash: u64,
    /// The project itself.
    pub state: ProjectState,
}

impl RecoverySnapshot {
    pub fn new(project_file: &Path, state: &ProjectState, saved_at: u64) -> Self {
        Self {
            caprust_recovery_version: RECOVERY_FORMAT_VERSION,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            saved_at,
            project_file: project_file.to_string_lossy().to_string(),
            state_hash: state.render_hash(),
            state: state.clone(),
        }
    }
}

/// Stable per-project filename. Hashes the absolute project path so
/// two projects with the same name in different folders get different
/// slots, and a project whose path changed gets a fresh slot rather
/// than restoring someone else's session.
fn slot_stem(project_file: &Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    // Normalize so "C:\a\T.caprust" and "c:\a\T.caprust" share a slot.
    let lowered = project_file.to_string_lossy().to_lowercase();
    lowered.hash(&mut h);
    let digest = h.finish();

    // Readable prefix keeps the recovery folder browsable by hand.
    let readable: String = project_file
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let readable: String = readable.chars().take(32).collect();

    if readable.is_empty() {
        format!("{digest:016x}")
    } else {
        format!("{readable}-{digest:016x}")
    }
}

/// All snapshot paths for a project, newest slot first.
///
/// Each project gets its own subdirectory named after
/// [`slot_stem`]. The slots live inside it, so one project's temp
/// file can never collide with another project's, and clearing one
/// project cannot disturb a snapshot belonging to a different one.
fn slot_paths(project_file: &Path) -> Vec<PathBuf> {
    let stem = slot_stem(project_file);
    let dir = recovery_dir().join(&stem);
    (0..KEEP_SNAPSHOTS)
        .map(|i| dir.join(format!("{stem}.{i}.recovery")))
        .collect()
}

/// The per-project subdirectory holding every slot for `project_file`.
fn slot_dir(project_file: &Path) -> PathBuf {
    recovery_dir().join(slot_stem(project_file))
}

/// Write a snapshot, rotating older ones down the ring.
///
/// The new snapshot goes to slot 0 only after it is fully written and
/// re-read successfully. Slot 0 is a temp file renamed into place, so
/// a crash mid-write leaves the previous snapshot in slot 1 intact
/// rather than a truncated slot 0.
pub fn write_snapshot(project_file: &Path, state: &ProjectState, saved_at: u64) -> Result<PathBuf> {
    let paths = slot_paths(project_file);
    let dir = slot_dir(project_file);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create recovery dir {}", dir.display()))?;

    let snap = RecoverySnapshot::new(project_file, state, saved_at);
    let json = serde_json::to_string_pretty(&snap).context("serialize recovery snapshot")?;

    // Slot 0 via temp + rename so a partial write never lands there.
    let tmp = paths[0].with_extension("tmp");
    std::fs::write(&tmp, json).with_context(|| format!("write {}", tmp.display()))?;
    // Parse it back: only promote to slot 0 once we know it is loadable.
    let verify: RecoverySnapshot =
        serde_json::from_str(&std::fs::read_to_string(&tmp).context("re-read recovery temp file")?)
            .context("re-parse recovery temp file")?;
    if verify.state_hash != snap.state_hash {
        std::fs::remove_file(&tmp).ok();
        anyhow::bail!("recovery snapshot hash mismatch, discarding write");
    }

    // Shift old slots down before promoting, so slot 0 never holds
    // data we have not yet copied elsewhere.
    for i in (1..KEEP_SNAPSHOTS).rev() {
        if paths[i - 1].exists() {
            std::fs::rename(&paths[i - 1], &paths[i]).ok();
        }
    }
    std::fs::rename(&tmp, &paths[0]).context("promote recovery temp file")?;
    tracing::info!("recovery snapshot written: {}", paths[0].display());
    Ok(paths[0].clone())
}

/// Newest loadable snapshot for a project, or `None` if there is
/// none. Skips unreadable slots so one truncated file does not hide a
/// good older snapshot.
pub fn newest_snapshot(project_file: &Path) -> Option<RecoverySnapshot> {
    for path in slot_paths(project_file) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(snap) = serde_json::from_str::<RecoverySnapshot>(&text) else {
            tracing::warn!("recovery: skipping unreadable snapshot {}", path.display());
            continue;
        };
        if snap.caprust_recovery_version > RECOVERY_FORMAT_VERSION {
            tracing::warn!(
                "recovery: snapshot {} is newer than this build, skipping",
                path.display()
            );
            continue;
        }
        return Some(snap);
    }
    None
}

/// Delete every snapshot for a project. Called after a real Save and
/// when the user discards a recovery prompt.
pub fn clear_snapshots(project_file: &Path) {
    // The whole per-project directory, slots and temp file together.
    //
    // This used to remove the *shared* recovery folder instead, which
    // deleted every other project's snapshots and raced with a
    // concurrent `write_snapshot`: a clear landing between the
    // writer's `create_dir_all` and its first write surfaced as
    // ENOENT on the temp file. It failed on Ubuntu CI while Windows
    // and macOS passed on timing. With one directory per project,
    // removing it only ever disturbs this project.
    let dir = slot_dir(project_file);
    std::fs::remove_dir_all(&dir).ok();
}

/// A recovery worth offering to the user.
///
/// `None` when the snapshot is older than the project file (the user
/// already saved past it), when it matches nothing (a no-op
/// autosave), or when there is no snapshot at all.
#[derive(Debug, Clone)]
pub struct RecoveryOffer {
    /// Unix seconds the snapshot was written.
    pub saved_at: u64,
    /// `render_hash` of the snapshotted project.
    pub state_hash: u64,
}

/// Decide whether `project_file` has a recovery worth prompting for,
/// given the `state_hash` of the project as it was loaded from disk.
pub fn offer_for(project_file: &Path, loaded_hash: u64) -> Option<RecoveryOffer> {
    let snap = newest_snapshot(project_file)?;

    // Nothing changed since the autosave, so there is nothing to
    // recover even if the snapshot is newer.
    if snap.state_hash == loaded_hash {
        return None;
    }

    // The user saved manually after the snapshot: their saved file is
    // the newer intent, do not offer to undo it.
    if let Ok(meta) = std::fs::metadata(project_file) {
        if let Ok(modified) = meta.modified() {
            let project_mtime = modified
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if project_mtime > snap.saved_at {
                return None;
            }
        }
    }

    Some(RecoveryOffer {
        saved_at: snap.saved_at,
        state_hash: snap.state_hash,
    })
}

/// Load the newest snapshot's project state for a project file.
pub fn restore(project_file: &Path) -> Option<ProjectState> {
    newest_snapshot(project_file).map(|s| s.state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::ProjectState;

    fn tmp_project_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("caprust-recovery-test-{tag}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("P.caprust")
    }

    fn project(tag: &str, name: &str) -> ProjectState {
        ProjectState {
            name: name.to_string(),
            project_path: Some(tmp_project_dir(tag).to_string_lossy().to_string()),
            ..Default::default()
        }
    }

    /// Recovery writes are a user-visible feature; the round trip has
    /// to hold or a crash restore would silently lose the session.
    #[test]
    fn snapshot_round_trips() {
        let pf = tmp_project_dir("roundtrip");
        let state = project("roundtrip", "P");
        write_snapshot(&pf, &state, 1_000).unwrap();
        let back = restore(&pf).expect("snapshot restores");
        assert_eq!(back.name, state.name);
        assert_eq!(
            newest_snapshot(&pf).unwrap().state_hash,
            state.render_hash()
        );
        clear_snapshots(&pf);
    }

    /// Slot 0 is written via temp + rename; if that ever regresses, a
    /// crash mid-write would leave an unparseable newest snapshot and
    /// the user would lose everything the ring was supposed to protect.
    #[test]
    fn ring_keeps_previous_snapshot_when_newest_is_corrupt() {
        let pf = tmp_project_dir("ring");
        let first = project("ring", "first");
        write_snapshot(&pf, &first, 1_000).unwrap();

        let second = project("ring", "second");
        write_snapshot(&pf, &second, 2_000).unwrap();

        // Corrupt the newest slot the way a crashed write would.
        let paths = slot_paths(&pf);
        std::fs::write(paths[0].clone(), b"{ truncated json").unwrap();

        // Recovery must still find the older, intact snapshot.
        let found = restore(&pf).expect("falls back to an older snapshot");
        assert_eq!(found.name, "first");
        clear_snapshots(&pf);
    }

    /// `clear_snapshots` used to `remove_dir` the shared recovery
    /// folder. On Linux the folder lands in `/tmp`, where another
    /// test's `write_snapshot` could be between `create_dir_all` and
    /// its first write, and the removal surfaced as ENOENT on the
    /// temp file. Windows and macOS passed on timing, so this is
    /// deterministic: clear, then write, and the write must succeed.
    #[test]
    fn write_survives_clearing_a_different_project() {
        let clearer = tmp_project_dir("clearer");
        let writer = tmp_project_dir("writer");
        write_snapshot(&writer, &project("writer", "first"), 1_000).unwrap();

        clear_snapshots(&clearer);

        let state = project("writer", "second");
        write_snapshot(&writer, &state, 2_000)
            .expect("write after clearing another project must still work");
        assert_eq!(restore(&writer).unwrap().name, "second");
        clear_snapshots(&writer);
        clear_snapshots(&clearer);
    }

    /// One project's directory being cleared must not disturb any
    /// other project's snapshots.
    ///
    /// The reported CI failure was this in reverse: `clear_snapshots`
    /// removed the shared recovery folder, so a clear landing between
    /// another project's `create_dir_all` and its first write produced
    /// ENOENT on the temp file. Ubuntu failed while Windows and macOS
    /// passed on timing. Every project now has its own subdirectory, so
    /// a clear only ever touches one project.
    ///
    /// Writers and clearers run at the same time but never on the same
    /// project: the app serialises both onto the UI thread, so that
    /// combination is not reachable and is deliberately not asserted.
    #[test]
    fn clearing_one_project_leaves_others_intact() {
        use std::sync::Arc;
        use std::sync::Mutex;

        let writers: Vec<PathBuf> = (0..4u32)
            .map(|w| tmp_project_dir(&format!("iso{w}")))
            .collect();
        let victim = tmp_project_dir("iso-victim");
        // Give the victim a snapshot to lose, then leave it alone.
        write_snapshot(&victim, &project("iso-victim", "keep"), 1_000).unwrap();

        let errors = Arc::new(Mutex::new(Vec::<String>::new()));
        let mut handles = Vec::new();
        for (w, path) in writers.iter().enumerate() {
            let errors = Arc::clone(&errors);
            let path = path.clone();
            handles.push(std::thread::spawn(move || {
                for i in 0..25 {
                    let state = project(&format!("iso{w}"), &format!("v{i}"));
                    if let Err(e) = write_snapshot(&path, &state, 1_000 + i as u64) {
                        errors.lock().unwrap().push(format!("w{w}-{i}: {e:#}"));
                    }
                }
            }));
        }
        for _ in 0..2 {
            let path = victim.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..25 {
                    clear_snapshots(&path);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let msgs = errors.lock().unwrap();
        assert!(
            msgs.is_empty(),
            "{} write(s) failed while another project was cleared: {}",
            msgs.len(),
            msgs.iter()
                .take(3)
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" | ")
        );
        // The victim really did lose its snapshot, which is the point.
        assert!(
            newest_snapshot(&victim).is_none(),
            "clearing the victim must remove its snapshots"
        );
        for p in &writers {
            assert!(newest_snapshot(p).is_some(), "writer snapshot survived");
            clear_snapshots(p);
        }
    }

    /// Two projects that share a file name but live in different
    /// folders. Before each project got its own subdirectory, both
    /// wrote the same temp filename into one shared folder, so a
    /// clear for one could delete the temp file the other was about
    /// to rename into place.
    #[test]
    fn same_named_projects_get_separate_recovery_dirs() {
        let a = tmp_project_dir("same-a").join("Clip.caprust");
        let b = tmp_project_dir("same-b").join("Clip.caprust");
        assert_ne!(
            slot_paths(&a),
            slot_paths(&b),
            "same-named projects in different folders must not share slots"
        );
        assert_ne!(slot_dir(&a), slot_dir(&b));
        // Clearing one leaves the other's newest slot readable.
        write_snapshot(&a, &project("same", "a"), 1_000).unwrap();
        clear_snapshots(&b);
        assert!(
            newest_snapshot(&a).is_some(),
            "clearing an unrelated project must not disturb this one"
        );
        clear_snapshots(&a);
    }

    /// The ring must not grow without bound across many autosaves.
    #[test]
    fn ring_is_bounded() {
        let pf = tmp_project_dir("bounded");
        for i in 0..(KEEP_SNAPSHOTS + 4) {
            let state = project("bounded", &format!("v{i}"));
            write_snapshot(&pf, &state, 1_000 + i as u64).unwrap();
        }
        let dir = slot_dir(&pf);
        let on_disk = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".recovery"))
            .count();
        assert_eq!(on_disk, KEEP_SNAPSHOTS);
        // KEEP_SNAPSHOTS + 4 writes means the last one is v6.
        assert_eq!(restore(&pf).unwrap().name, "v6");
        clear_snapshots(&pf);
    }

    /// A snapshot identical to what is on disk means there is nothing
    /// to recover, so no prompt should appear.
    #[test]
    fn offer_skips_when_nothing_changed() {
        let pf = tmp_project_dir("unchanged");
        let state = project("unchanged", "P");
        write_snapshot(&pf, &state, 1_000).unwrap();
        assert!(offer_for(&pf, state.render_hash()).is_none());
        clear_snapshots(&pf);
    }

    /// A real difference in state means the user would lose edits.
    #[test]
    fn offer_fires_on_changed_state() {
        let pf = tmp_project_dir("changed");
        let state = project("changed", "edited");
        write_snapshot(&pf, &state, 1_000).unwrap();
        let offer = offer_for(&pf, 12345).expect("offer made");
        assert_eq!(offer.saved_at, 1_000);
        assert_eq!(offer.state_hash, state.render_hash());
        clear_snapshots(&pf);
    }

    /// No project file means no autosave target. Callers must guard on
    /// this, and this test pins the "no slots for a bogus path" case.
    #[test]
    fn no_snapshot_yields_no_offer() {
        let pf = tmp_project_dir("empty");
        assert!(newest_snapshot(&pf).is_none());
        assert!(offer_for(&pf, 0).is_none());
        assert!(restore(&pf).is_none());
    }
}
