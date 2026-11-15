Release 0.9.29: Phase 0 + 4 + 6.9 hardening — MCP DoS guard, PCM cache cleanup, zero-copy RGBA preview.

### What's new

- **Save panel state (View → Modern zones).** Checkbox next to *Remember custom resize* remembers which panels were open in each Modern zone (Effects, Transitions, Properties, ...) and restores them on the next launch. Off by default.
- **Phase 0 repo hygiene:** removed tracked `crates/core/src/clip.rs.bak`.
- **Phase 0 MCP hardening:** bounded request-line reads (16 MiB cap) across StdioTransport, TcpTransport, and the phone bridge to prevent remote DoS via unbounded line buffering.
- **Phase 0 dead code:** removed the write-only `initialized` field from MCP `Server`; added regression test proving `initialize` alone grants no project access (token is the real gate).
- **Phase 0 Kotlin:** replaced all `!!` non-null assertions in `EditorActivity.kt` and `MainActivity.kt` with safe local `val`s.
- **Phase 4 i18n:** normalized `\uXXXX` escapes in `hr.ftl` to UTF-8 characters; added regression test blocking re-entry.
- **Phase 4 release infra:** hardened Inno Setup 6 lookup in `release.yml` (targeted v6 paths, fail-fast with clear error).
- **Phase 4 media:** stale PCM cache cleanup — previous hash's `.pcm` file removed on cache promotion instead of accumulating in %TEMP%.
- **Phase 6.9 zero-copy preview:** `PreviewFrame.rgba` is now `Arc<[u8]>` so buffer retention for stale detection and the remote-frame publish are refcount bumps instead of full ~1 MB RGBA memcpy per frame.

### Fixes

- **K2 docs claim:** `full_readme.md` roadmap status changed from "Done" to "In progress" (Phase 6 items 6.4, 6.6, 6.7 remain open).

### Downloads

| Platform | Artifact |
|----------|----------|
| Windows | `CapRust-0.9.29-win64-setup.exe` (Inno Setup installer) |
| Linux (Debian/Ubuntu) | `caprust_0.9.29_amd64.deb` |
| Android | `caprust-0.9.29.apk` (debug build, sideloadable) |

### Installation

- **Windows**: Run the `.exe` installer
- **Linux**: `sudo dpkg -i caprust_0.9.29_amd64.deb`
- **Android**: Enable "Install unknown apps", then tap the `.apk` to install

### Notes

- Android APK is a debug build (signed with debug key), suitable for testing
- Linux .deb requires `libgtk-3-0` and `libasound2` (auto-installed by apt)
- Windows installer creates Start Menu shortcut and desktop icon

### Known limitations

- Keyframe easing curves on export: ffmpeg filtergraph easing works for trajectory, per-property track easing on export is not yet wired.
- AI Enhance (Real-ESRGAN): blocked by tract-onnx runtime speed; the settings download prepares the model file but does not run it.
- Android editor: feature parity with desktop is partial (no keyframe tracks tab, no plugin UI).

Full Changelog: https://github.com/Domica/CapRust/compare/v0.9.28...v0.9.29
