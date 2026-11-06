## CapRust v0.9.25

### Keyframe System Complete (Issues #84-87)
- **Curve Editor** (#84): Interactive graph editor for trajectory keyframes with grid, easing curves, and playhead sync
- **Easing Curves** (#85): Per-keyframe easing (Linear, Ease In, Ease Out, Ease In/Out) with smooth interpolation
- **Per-Property Tracks** (#86): Generic `KeyframeTrack` system for Scale, Rotation, Opacity, Blur Amount, and more
- **Size/Effect Keyframing** (#87): Keyframe scale, rotation, opacity, and effect parameters with curve editor

### Fixes
- **Issue #92**: Fixed "Add Track" dropdown flicker by removing immediate pending push
- **Trajectory Y coordinate**: Fixed using `centre_y` instead of `centre_x` for vertical axis
- **Drag visual feedback**: Added `request_repaint()` during drag for immediate graph updates
- **Drag persistence**: Fixed using thread-local storage (egui 0.31 API workaround)
- **Right-click delete**: Working on keyframes in both trajectory and keyframe tracks

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