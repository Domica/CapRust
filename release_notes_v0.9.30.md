Release 0.9.30: Faza 2.1–2.5 — UndoStack batching, clips privatization, production .unwrap() cleanup.

### What's new

- **Faza 2.1 UndoStack atomic batch execution.** `UndoStack::execute_batch(Vec<Box<dyn Command>>)` wraps multiple commands in a `MacroCommand` for a single undo step with atomic rollback on mid-batch failure. Ctrl+Z reverts an entire batch at once instead of per-command.

### Fixes

- **Faza 2.2 clips privatization.** `ProjectState.clips` field changed from `pub` to `pub(crate)`; all clip mutations go through controlled accessor/mutator methods (`clips()`, `modify_clip_by_id`, `modify_clip_by_index`, `retain_clips`, `clip_by_id_mut`, `with_clip_and_models_mut`, `add_clip_for_test`, `clear_clips_for_test`) so every mutation is undoable via `UndoStack`. Updated `caprust-ui`, `caprust-media-io`, `caprust-mcp`, and `caprust-android` crates accordingly.
- **Faza 2.3 Panics → bail!.** Replaced bare `.unwrap()` calls in core test code with `.expect("descriptive message")` so failures carry context instead of silent panics. Fixed integration test `set_clip_auto_reframe.rs` to use the public `add_clip_for_test()` / `clips()` API for `pub(crate)` access.
- **Faza 2.5 Production .unwrap() → error handling.** `detect_beats()` in `crates/core/src/beat.rs` no longer panics when no BPM candidates fall in the 60–240 BPM range; returns an empty `BeatGrid` instead, consistent with existing early-return behavior.

### Downloads

| Platform | Artifact |
|----------|----------|
| Windows | `CapRust-0.9.30-win64-setup.exe` (Inno Setup installer) |
| Linux (Debian/Ubuntu) | `caprust_0.9.30_amd64.deb` |
| Android | `caprust-0.9.30.apk` (debug build, sideloadable) |

### Installation

- **Windows**: Run the `.exe` installer
- **Linux**: `sudo dpkg -i caprust_0.9.30_amd64.deb`
- **Android**: Enable "Install unknown apps", then tap the `.apk` to install

### Notes

- Android APK is a debug build (signed with debug key), suitable for testing
- Linux .deb requires `libgtk-3-0` and `libasound2` (auto-installed by apt)
- Windows installer creates Start Menu shortcut and desktop icon

### Known limitations

- Keyframe easing curves on export: ffmpeg filtergraph easing works for trajectory, per-property track easing on export is not yet wired.
- AI Enhance (Real-ESRGAN): blocked by tract-onnx runtime speed; the settings download prepares the model file but does not run it.
- Android editor: feature parity with desktop is partial (no keyframe tracks tab, no plugin UI).

Full Changelog: https://github.com/Domica/CapRust/compare/v0.9.29...v0.9.30
