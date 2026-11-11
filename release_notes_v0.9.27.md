Release 0.9.27: Trajectory & keyframe graph now follows the dragged cursor, preview/export invalidated on trajectory/keyframe change, double-buffer no longer freezes on a backwards seek, playback/scrub performance improvements.

### What's new

- **Trajectory & keyframe graph follows the dragged cursor (3.0.1).** The point now tracks the mouse on the graph in real time while dragging.
- **Preview/export invalidated on trajectory or keyframe change (3.0.4).** Editing a clip's trajectory or keyframe tracks refreshes the preview and re-renders export instead of showing the stale cached frame.
- **Double-buffer preview no longer freezes on a backwards seek (6.1).** Scrubbing back now publishes the freshly decoded frame instead of a stale buffer.
- **Playback/scrub performance (6.x).** Stable preview texture (no per-frame texture churn) and zero-copy frame upload (no full-frame copy per frame).

### Fixes

- **Graph points snapped back / did not follow the cursor when dragged.** The dragged point now stays under the cursor and the preview updates live.
- **Preview and export showed a stale frame after a trajectory/keyframe edit.** The render/preview hash now changes with the edit, so the cache is invalidated.
- **Preview froze (showed the previous frame) after a backwards seek.** The double buffer no longer swaps a stale back-buffer frame to the front; the decoded frame is published directly.

### Downloads

| Platform | Artifact |
|----------|----------|
| Windows | `CapRust-0.9.27-win64-setup.exe` (Inno Setup installer) |
| Linux (Debian/Ubuntu) | `caprust_0.9.27_amd64.deb` |
| Android | `caprust-0.9.27.apk` (debug build, sideloadable) |

### Installation

- **Windows**: Run the `.exe` installer
- **Linux**: `sudo dpkg -i caprust_0.9.27_amd64.deb`
- **Android**: Enable "Install unknown apps", then tap the `.apk` to install

### Notes

- Android APK is a debug build (signed with debug key), suitable for testing
- Linux .deb requires `libgtk-3-0` and `libasound2` (auto-installed by apt)
- Windows installer creates Start Menu shortcut and desktop icon

### Known limitations

- Keyframe easing curves on export: ffmpeg filtergraph easing works for trajectory, per-property track easing on export is not yet wired.
- AI Enhance (Real-ESRGAN): blocked by tract-onnx runtime speed; the settings download prepares the model file but does not run it.
- Android editor: feature parity with desktop is partial (no keyframe tracks tab, no plugin UI).

Full Changelog: https://github.com/Domica/CapRust/compare/v0.9.26...v0.9.27
