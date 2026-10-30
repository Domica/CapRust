Release 0.9.24: Android editor (arm64-v8a, armeabi-v7a, x86, x86_64), Debian .deb, Linux file dialogs via GTK3, platform-aware FFmpeg download, keyframe curve editor (basic), AI model download in Settings

### What's new

- **Android editor.** The editor runs on the phone: Rust engine cross-compiled for arm64-v8a, armeabi-v7a, x86, x86_64, JSON-RPC daemon over loopback, Kotlin UI with media bin, timeline, preview and export. The APK carries libffmpeg.so for all four architectures.
- **Debian .deb package.** caprust_0.9.24_amd64.deb installs with dpkg -i, includes desktop entry and icon. Runs on Debian 12 (glibc 2.35).
- **Linux FFmpeg auto-download.** Picks the Linux build (.tar.xz) instead of a Windows zip, preserves the exec bit, reports only a binary that actually runs.

### Fixed

- **Android import with .mp4 files.** The picker returned .mp4 files that infer_kind missed (extension matched, but guess_kind magic bytes needed for the .bin case). sniffKind now checks typ at offset 4.
- **Android dd_media created media but no timeline clip.** The daemon's dd_media now creates a Clip entry linked to the media via media_id, so list_clips and efresh() show it.
- **Android dd_media duration_ms was 0.** When unprobed, uses 5000 ms default so timeline shows length.
- **Android manifest launcher.** EditorActivity is the launcher; MainActivity (remote/test) has no MAIN/LAUNCHER filter.
- **Linux FFmpeg download fetched Windows build.** The download picks .tar.xz for Linux, preserves exec bit on extraction, verifies with -version before reporting installed.
- **Linux managed dir.** Follows XDG (~/.local/share/caprust/ffmpeg), not literal $HOME/CapRust.
- **FFmpeg "installed" false positive.** ind_tool runs -version, not is_file().

