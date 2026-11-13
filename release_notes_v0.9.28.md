Release 0.9.28: Preview playback robustness — streamed frames no longer dropped as stale, single pause teardown, remote-frame copy only when a phone is connected, exact paused seek, lock-free double buffer.

### What's new

- **Streamed playback no longer drops frames (6.2).** Each frame pulled from the live stream now carries a strictly increasing sequence, so the double buffer accepts every frame instead of rejecting all but the first as stale.
- **Pause runs the teardown once (6.5).** The playing→paused teardown (`stop_stream` + `cancel_pending` + audio drop) now runs exactly on the transition, not on every repaint while paused.
- **Remote-frame copy only when a phone is connected (6.10).** The per-frame pixel copy for the MCP remote reader is skipped during ordinary desktop editing.
- **Exact paused one-shot seek (6.11).** The paused fallback seeks to the exact playhead instead of snapping to a 100 ms grid.
- **Lock-free double buffer (6.3).** The unused `Arc<Mutex>` inside `DoubleBufferPreview` is gone.

### Fixes

- **Preview stalled after the first streamed frame.** The streamed sequence was constant, so the double buffer flagged every subsequent frame as stale and the preview froze.
- **Pause repeatedly killed the stream every frame.** The teardown ran on every repaint, clearing the buffer continuously. It now runs once, on entering pause.
- **Paused scrub requested stale/duplicate frames.** Grid rounding in the one-shot path is removed; it seeks exactly.
- **Wasted ~1 MB/frame at 30 fps during desktop editing.** The remote-reader copy now runs only while a remote bridge is listening.

### Downloads

| Platform | Artifact |
|----------|----------|
| Windows | `CapRust-0.9.28-win64-setup.exe` (Inno Setup installer) |
| Linux (Debian/Ubuntu) | `caprust_0.9.28_amd64.deb` |
| Android | `caprust-0.9.28.apk` (debug build, sideloadable) |

### Installation

- **Windows**: Run the `.exe` installer
- **Linux**: `sudo dpkg -i caprust_0.9.28_amd64.deb`
- **Android**: Enable "Install unknown apps", then tap the `.apk` to install

### Notes

- Android APK is a debug build (signed with debug key), suitable for testing
- Linux .deb requires `libgtk-3-0` and `libasound2` (auto-installed by apt)
- Windows installer creates Start Menu shortcut and desktop icon

### Known limitations

- Keyframe easing curves on export: ffmpeg filtergraph easing works for trajectory, per-property track easing on export is not yet wired.
- AI Enhance (Real-ESRGAN): blocked by tract-onnx runtime speed; the settings download prepares the model file but does not run it.
- Android editor: feature parity with desktop is partial (no keyframe tracks tab, no plugin UI).

Full Changelog: https://github.com/Domica/CapRust/compare/v0.9.27...v0.9.28
