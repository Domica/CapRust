Release 0.9.26: Trajectory & keyframe drag visual feedback fixes, drag persistence fix, right-click delete on keyframes.

### What's new

- **Trajectory drag visual feedback.** Točka sada prati miš na grafu u real-time prilikom vučenja (dodano `request_repaint()`).
- **Keyframe Tracks drag visual feedback.** Isto za scale, rotation, opacity, blur.amount keyframe-ove.
- **Drag persistence.** Popravljeno korištenjem thread-local storage (egui 0.31 API workaround).
- **Right-click delete na keyframe-ovima.** Radi u Trajectory tabu i Keyframe Tracks tabu.
- **Trajectory Y coordinate bug fix.** Popravljeno `centre_y` umjesto `centre_x` za vertikalnu os.

### Fixes

- **Trajectory graph ne osvježavalo se prilikom drag-a.** Vrijednosti su se mijenjale u listi, ali graf je stao.
- **Keyframe Tracks graph isto.** Sada oba grafa rade vizualno prilikom vučenja.
- **Drag state gubio se između frame-ova.** Thread-local storage zadržava drag index.

### Downloads

| Platform | Artifact |
|----------|----------|
| Windows | `CapRust-0.9.26-win64-setup.exe` (Inno Setup installer) |
| Linux (Debian/Ubuntu) | `caprust_0.9.26_amd64.deb` |
| Android | `caprust-0.9.26.apk` (debug build, sideloadable) |

### Installation

- **Windows**: Run the `.exe` installer
- **Linux**: `sudo dpkg -i caprust_0.9.26_amd64.deb`
- **Android**: Enable "Install unknown apps", then tap the `.apk` to install

### Notes

- Android APK is a debug build (signed with debug key), suitable for testing
- Linux .deb requires `libgtk-3-0` and `libasound2` (auto-installed by apt)
- Windows installer creates Start Menu shortcut and desktop icon

### Known limitations

- Keyframe easing curves on export: ffmpeg filtergraph easing works for trajectory, per-property track easing on export is not yet wired.
- AI Enhance (Real-ESRGAN): blocked by tract-onnx runtime speed; the settings download prepares the model file but does not run it.
- Android editor: feature parity with desktop is partial (no keyframe tracks tab, no plugin UI).

Full Changelog: https://github.com/Domica/CapRust/compare/v0.9.25...v0.9.26