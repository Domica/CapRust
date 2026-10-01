//! Integration tests for the model registry (Phase P2a).
//!
//! Lives outside src/ so it exercises the public API exactly the way
//! the UI does: construct a default registry, look up a model by kind,
//! assert the download metadata is present.

use caprust_core::models::{ModelKind, ModelRegistry, ModelStatus};

#[test]
fn registry_has_face_detector_with_url() {
    let r = ModelRegistry::default();
    let yunet = r
        .models
        .iter()
        .find(|m| m.kind == ModelKind::FaceDetector)
        .expect("YuNet face detector is registered by default");
    assert_eq!(yunet.id, "yunet-face");
    assert!(
        !yunet.url.is_empty(),
        "YuNet entry must carry a download URL; without one the UI reports 'no URL configured'"
    );
    assert_eq!(
        yunet.status,
        ModelStatus::NotDownloaded,
        "a fresh registry never claims a model is on disk"
    );
}

#[test]
fn registry_kinds_are_distinct_and_nonempty() {
    let r = ModelRegistry::default();
    for kind in [
        ModelKind::Caption,
        ModelKind::Narration,
        ModelKind::FaceDetector,
        ModelKind::BackgroundRemover,
    ] {
        let n = r.models.iter().filter(|m| m.kind == kind).count();
        assert!(n >= 1, "registry must have at least one entry for {kind:?}");
    }
}

#[test]
fn registry_has_background_remover_with_url() {
    let r = ModelRegistry::default();
    let bg = r
        .models
        .iter()
        .find(|m| m.kind == ModelKind::BackgroundRemover)
        .expect("u2netp background remover is registered by default");
    assert_eq!(bg.id, "u2netp-bg");
    assert!(
        !bg.url.is_empty(),
        "background remover must carry a download URL"
    );
    assert!(bg.url.ends_with(".onnx"), "ONNX model expected: {}", bg.url);
}

#[test]
fn every_downloadable_model_pins_a_sha256() {
    for m in ModelRegistry::default()
        .models
        .iter()
        .filter(|m| !m.url.is_empty())
    {
        assert!(
            m.sha256.len() == 64 && m.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "{} must pin a SHA-256",
            m.id
        );
    }
}

#[test]
fn merge_distrusts_registry_from_project_file() {
    let mut r = ModelRegistry::default();
    // Attacker-controlled snapshot: unknown id with a traversal path and
    // URL, plus a known id with a swapped URL/hash.
    let mut evil = r.models[0].clone();
    evil.id = "..\\..\\evil".into();
    evil.url = "http://attacker.example/x".into();
    r.models.push(evil);
    r.models[0].url = "http://attacker.example/whisper".into();
    r.models[0].sha256.clear();
    r.models[0].enabled = true;

    r.merge_missing_defaults();

    let d = ModelRegistry::default();
    assert_eq!(r.models.len(), d.models.len());
    assert!(r
        .models
        .iter()
        .all(|m| d.models.iter().any(|x| x.id == m.id)));
    assert_eq!(r.models[0].url, d.models[0].url);
    assert_eq!(r.models[0].sha256, d.models[0].sha256);
    assert!(r.models[0].enabled, "user state survives the merge");
}
