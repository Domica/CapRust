//! Narration cache helpers: deterministic hashing and path layout.

use caprust_core::cache::{narration_exists, narration_hash, narration_path};

#[test]
fn hash_is_stable_and_order_sensitive() {
    // Same input => same hash.
    assert_eq!(
        narration_hash("hello", "voice-a"),
        narration_hash("hello", "voice-a")
    );
    // Different voice => different hash.
    assert_ne!(
        narration_hash("hello", "voice-a"),
        narration_hash("hello", "voice-b")
    );
    // Different text => different hash.
    assert_ne!(
        narration_hash("hello", "voice-a"),
        narration_hash("hello!", "voice-a")
    );
}

#[test]
fn path_lands_in_narration_subdir() {
    let dir = std::path::Path::new("/tmp/caprust-cache-test");
    let p = narration_path(dir, "some text", "voice");
    assert!(p.starts_with(dir.join("narration")));
    let name = p.file_name().unwrap().to_string_lossy();
    assert!(name.ends_with(".wav"));
    assert_eq!(name.len(), 64 + 4); // blake3 hex + ".wav"
}

#[test]
fn exists_returns_false_for_missing_dir() {
    let missing = std::path::Path::new("/tmp/caprust-definitely-missing-xyz");
    assert!(!narration_exists(missing, "text", "voice"));
}
