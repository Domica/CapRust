Release 0.9.25: Complete keyframe animation system (generic tracks per property, easing curves, curve editor), trajectory drag visual feedback fix, drag persistence fix, Windows binary stripped to ~10 MB.

### What's new

- **Complete keyframe animation system (#84-87).** Generic `KeyframeTrack` per property (scale, rotation, opacity, blur.amount, position, etc.). `EaseCurve` enum (Linear/EaseIn/EaseOut/EaseInOut) on each keyframe. `trajectory_expr()` generates ffmpeg easing expressions for export.
- **Curve editor in Trajectory tab.** Draggable keyframe points, playhead line, easing selector per keyframe, grid visible even with 0/1 keyframes.
- **Keyframe Tracks tab.** Add/remove tracks, property name editor, per-track curve editor with easing. No-flicker Add Track UX (immediate pending push).
- **i18n for keyframe UI.** English, Croatian, German, Spanish, Italian translations.
- **Windows binary stripping in CI.** `cargo strip` reduces release binary to ~10 MB.

### Fixes

- **Add Track button flicker.** The button no longer appears/disappears; track is added to pending edits immediately.
- **Trajectory graph missing with 0/1 keyframes.** Grid and points now render in all states with guidance text.
- **Borrow checker in KeyframeTracks loop.** Fixed by deferring pending push until after the loop.
- **Trajectory Y coordinate.** Fixed using `centre_y` instead of `centre_x` for vertical axis.
- **Drag visual feedback.** Added `request_repaint()` during drag for immediate graph updates.
- **Drag persistence.** Fixed using thread-local storage (egui 0.31 API workaround).
- **Right-click delete on keyframes.** Works in both trajectory and keyframe tracks tabs.

### Downloads

| Platform | Artifact |
|----------|----------|
| Windows | `CapRust-0.9.25-win64-setup.exe` (Inno Setup installer) |
| Linux (Debian/Ubuntu) | `caprust_0.9.25_amd64.deb` |
| Android | `caprust-0.9.25.apk` (debug build, sideloadable) |

### Installation

- **Windows**: Run the `.exe` installer
- **Linux**: `sudo dpkg -i caprust_0.9.25_amd64.deb`
- **Android**: Enable "Install unknown apps", then tap the `.apk` to install

### Notes

- Android APK is a debug build (signed with debug key), suitable for testing
- Linux .deb requires `libgtk-3-0` and `libasound2` (auto-installed by apt)
- Windows installer creates Start Menu shortcut and desktop icon

### Known limitations

- Keyframe easing curves on export: ffmpeg filtergraph easing works for trajectory, per-property track easing on export is not yet wired.
- AI Enhance (Real-ESRGAN): blocked by tract-onnx runtime speed; the settings download prepares the model file but does not run it.
- Android editor: feature parity with desktop is partial (no keyframe tracks tab, no plugin UI).

Full Changelog: https://github.com/Domica/CapRust/compare/v0.9.24...v0.9.25