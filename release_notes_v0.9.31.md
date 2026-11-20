Release 0.9.31: GFPGAN face restore, export cancel, model download progress in jobs bar.

### What's new

- **GFPGAN face restore/enhance.** New `gfpgan-v1.4` model (Apache 2.0, ~350 MB) registered in the model registry, exposed as a per-clip checkbox in the **Video** tab of clip properties. Restores blurry or distorted AI-generated faces while keeping expression and speech motion — ideal for distant talking-head AI clips where the generator mangled facial features. Runs during export.
- **Export cancel button.** Cancel ("✕") in the export progress window skips remaining events and swallows `Failed` when cancelled.
- **Model download progress in jobs bar.** Active model downloads now render alongside background jobs in the bottom jobs bar, with progress bar, percentage, and cancel button.
- **Six new per-clip visual effects:** `halftone` (CMYK dot pattern), `solarize` (highlight inversion), `emboss` (relief edge), `gfpgan` (face restore), `ai_enhance` (Real-ESRGAN denoise+deband), plus sticker animations `float` and `pulse`. All in the Effects browser and render graph.

### Fixes

- `cargo fmt --all -- --check` now passes on all CI platforms (was failing CI run 44352985131 on step `cargo fmt`).

### Downloads

| Platform | Artifact |
|----------|----------|
| Windows | `CapRust-0.9.31-win64-setup.exe` (Inno Setup installer) |
| Windows | `CapRust-0.9.31-win64.zip` (portable) |
| Linux (Debian/Ubuntu) | `caprust_0.9.31_amd64.deb` |
| Android | `caprust-0.9.31.apk` (debug build, sideloadable) |

### Installation

- **Windows**: Run the `.exe` installer
- **Linux**: `sudo dpkg -i caprust_0.9.31_amd64.deb`
- **Android**: Enable "Install unknown apps", then tap the `.apk` to install

### Notes

- Android APK is a debug build (signed with debug key), suitable for testing
- Linux .deb requires `libgtk-3-0` and `libasound2` (auto-installed by apt)
- Windows installer creates Start Menu shortcut and desktop icon

### Known limitations

- GFPGAN: the ONNX model is registered and the UI checkbox is wired, but the tract-onnx execution pipeline runs a denoise+deband fall-back until the native GFPGAN pass lands.
- Keyframe easing curves on export: ffmpeg filtergraph easing works for trajectory, per-property track easing on export is not yet wired.
- Android editor: feature parity with desktop is partial (no keyframe tracks tab, no plugin UI).

Full Changelog: https://github.com/Domica/CapRust/compare/v0.9.30...v0.9.31
