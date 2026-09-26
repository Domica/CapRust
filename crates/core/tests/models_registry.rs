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
