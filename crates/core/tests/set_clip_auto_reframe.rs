//! Integration test for SetClipCommand::auto_reframe (Phase P2c-3a).
//!
//! Verifies that the setter writes the keypoints into the clip and
//! that undo restores the pre-command state. The set_clip.rs module
//! has no #[cfg(test)] block because it is the generic field editor
//! shared by many commands; its coverage lives in integration tests.

use caprust_core::clip::{Clip, ReframeKeypoint};
use caprust_core::commands::set_clip::SetClipCommand;
use caprust_core::commands::Command;
use caprust_core::ProjectState;

#[test]
fn auto_reframe_sets_keypoints_and_undo_restores() {
    let mut p = ProjectState::default();
    let clip = Clip::new_video("test.mp4", 0, 0, 1000);
    let id = clip.id;
    p.add_clip(clip);

    let kps = vec![
        ReframeKeypoint {
            t_ms: 0,
            cx_norm: 0.3,
            cy_norm: 0.5,
        },
        ReframeKeypoint {
            t_ms: 1000,
            cx_norm: 0.7,
            cy_norm: 0.5,
        },
    ];

    let mut cmd = SetClipCommand::new(id).auto_reframe(kps.clone());
    cmd.execute(&mut p).expect("execute");
    let c = p.clips.iter().find(|c| c.id == id).expect("clip");
    assert_eq!(c.auto_reframe, kps, "keypoints written");

    cmd.undo(&mut p).expect("undo");
    let c = p.clips.iter().find(|c| c.id == id).expect("clip");
    assert!(
        c.auto_reframe.is_empty(),
        "undo restores the pre-command empty state, got {:?}",
        c.auto_reframe
    );
}

#[test]
fn auto_reframe_empty_vec_clears_existing_keypoints() {
    let mut p = ProjectState::default();
    let mut clip = Clip::new_video("test.mp4", 0, 0, 1000);
    clip.auto_reframe = vec![
        ReframeKeypoint {
            t_ms: 0,
            cx_norm: 0.3,
            cy_norm: 0.5,
        },
        ReframeKeypoint {
            t_ms: 1000,
            cx_norm: 0.7,
            cy_norm: 0.5,
        },
    ];
    let id = clip.id;
    p.add_clip(clip);

    let mut cmd = SetClipCommand::new(id).auto_reframe(Vec::new());
    cmd.execute(&mut p).expect("execute");
    let c = p.clips.iter().find(|c| c.id == id).expect("clip");
    assert!(
        c.auto_reframe.is_empty(),
        "empty Vec must clear the keypoints"
    );
}
