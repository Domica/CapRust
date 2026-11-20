Release 0.9.32: Bug fixes for model downloads — file no longer vanishes after failed preuzimanje, stuck downloads self-heal.

### Fixes

- **Downloaded file no longer vanishes on SHA-256 mismatch.** `download_impl` reached 100 %, failed SHA verification, deleted the `.part` file, and never created the `.onnx` — user saw "dođe do kraja i onda nestane file". Now `.part` is kept for inspection and the error toast reports the actual vs. expected hash plus the partial file path. Combined with Cancel (Downloading) / Retry (Error) buttons in Settings → AI Models, failed downloads can be retried or inspected instead of disappearing silently.
- **Stranded model download self-heals.** `scan_local()` resets a `Downloading` model back to `NotDownloaded` (progress → 0.0) when neither the final `.onnx` file nor the `.part` file exists — covers manual `.part` deletion or a dead download thread. `drain_model_download()` also resets on `Disconnected` (thread vanished without Done/Failed). Restarting the app brings back the Download button instead of leaving it stuck forever.

### Downloads

| Platform | Artifact |
|----------|----------|
| Windows | `CapRust-0.9.32-win64-setup.exe` (Inno Setup installer) |
| Windows | `CapRust-0.9.32-win64.zip` (portable) |
| Linux (Debian/Ubuntu) | `caprust_0.9.32_amd64.deb` |
| Android | `caprust-0.9.32.apk` (debug build, sideloadable) |

### Installation

- **Windows**: Run the `.exe` installer
- **Linux**: `sudo dpkg -i caprust_0.9.32_amd64.deb`
- **Android**: Enable "Install unknown apps", then tap the `.apk` to install

### Notes

- Android APK is a debug build (signed with debug key), suitable for testing
- Linux .deb requires `libgtk-3-0` and `libasound2` (auto-installed by apt)
- Windows installer creates Start Menu shortcut and desktop icon

### Known limitations

- Keyframe easing curves on export: ffmpeg filtergraph easing works for trajectory, per-property track easing on export is not yet wired.
- Android editor: feature parity with desktop is partial (no keyframe tracks tab, no plugin UI).

Full Changelog: https://github.com/Domica/CapRust/compare/v0.9.31...v0.9.32
