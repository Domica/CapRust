Release 0.9.33: Slide (scrub) tool, working Face Restore fallback, corrected GFPGAN model source.

### ⚠️ Face Restore status — important

**Full GFPGAN v1.4 face restoration (ONNX / tract-onnx inference) is still in implementation.** The ONNX model downloads and activates correctly, but the neural inference is not yet connected to the export pipeline.

**Current Face Restore behaviour: smartblur + hqdn3d face smoothing.** When you enable *Face Restore* on a clip in the **Video** tab, export applies a denoise/smoothing filter chain that softens blurry or noisy faces. It is a visual approximation — not real GFPGAN identity-preserving restoration. This is applied during **export only**, not in the live preview.

A previous build silently did nothing: the effect skipped the ffmpeg filter whenever the ONNX model was present, expecting an external restorer that was never called. That regression is fixed in this release — the smoothing filter is now always emitted, with a regression test guarding it.

### What's new

- **Slide (scrub) tool.** New tool in the timeline toolbar (↔). Drag anywhere on the timeline to move the playhead precisely instead of moving clips. Playback pauses on press and resumes from the new position on release. Pan and Slide are mutually exclusive.
- **Drag-to-seek on the ruler.** The time ruler now supports click-and-drag even without the Slide tool — hold and move the mouse to slowly find the exact frame you want to resume from.
- **Corrected GFPGAN model source.** The GFPGAN v1.4 ONNX entry previously pinned a SHA-256 that never matched the served file, so downloads always failed verification. Switched to a dedicated stable mirror and pinned the verified hash (`cd7311b8…`). The model now downloads completely and can be enabled.

### Fixes

- **Face Restore no longer a no-op.** The `gfpgan` effect always emits the smartblur + hqdn3d smoothing chain; no longer skipped when the ONNX model is on disk.
- **Downloaded file no longer vanishes on SHA-256 mismatch** (from 0.9.32). The `.part` file is kept for inspection and the error toast reports actual vs. expected hash.
- **Stranded model download self-heals** (from 0.9.32). `Downloading` status resets to `NotDownloaded` when neither the final file nor `.part` exists; Cancel/Retry buttons in Settings → AI Models.

### Downloads

| Platform | Artifact |
|----------|----------|
| Windows | `CapRust-0.9.33-win64-setup.exe` (Inno Setup installer) |
| Windows | `CapRust-0.9.33-win64.zip` (portable) |
| Linux (Debian/Ubuntu) | `caprust_0.9.33_amd64.deb` |
| Android | `caprust-0.9.33.apk` (debug build, sideloadable) |

### Installation

- **Windows**: Run the `.exe` installer
- **Linux**: `sudo dpkg -i caprust_0.9.33_amd64.deb`
- **Android**: Enable "Install unknown apps", then tap the `.apk` to install

### Notes

- Android APK is a debug build (signed with debug key), suitable for testing
- Linux .deb requires `libgtk-3-0` and `libasound2` (auto-installed by apt)
- Windows installer creates Start Menu shortcut and desktop icon

### Known limitations

- **Full GFPGAN face restoration is in progress** — current Face Restore is smartblur + hqdn3d smoothing, export-only.
- Keyframe easing curves on export: ffmpeg filtergraph easing works for trajectory, per-property track easing on export is not yet wired.
- Android editor: feature parity with desktop is partial (no keyframe tracks tab, no plugin UI).

Full Changelog: https://github.com/Domica/CapRust/compare/v0.9.32...v0.9.33
