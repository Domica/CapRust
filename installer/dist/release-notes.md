Chroma key, multi-camera groups, and screen recording. First minor
bump since the beta line.

## Install

**Windows 10 / 11, 64-bit.**

1. Download `CapRust-0.9.0-win64-setup.exe` below.
2. Run the wizard. Per-user install by default (no UAC prompt);
   pick "Install for all users" if you prefer Program Files.
3. Launch from the Start Menu.

On first run CapRust offers to download a static FFmpeg build
(~110 MB) if `ffmpeg` / `ffprobe` are not on your PATH.

## Highlights

- **Chroma key (green screen)** - per-clip key color, similarity,
  and edge blend. Preview and export go through the same
  filtergraph stage, so the keying you see is what you ship.
- **Multi-camera groups** - right-click 2+ selected clips to create
  a group. Switch angle from the Multicam dock panel; preview and
  export follow. `Sync angles` button correlates the audio
  envelopes and writes per-angle offsets.
- **Screen recording** - File -> Record screen. Native DXGI
  Desktop Duplication, H.264 mp4, cursor overlay (color +
  monochrome), monitor picker, duration, fps. Recordings land in
  `%APPDATA%\CapRust\recordings\` and auto-import into the
  media bin.
- **Silent-video export fix** - clips whose source has no audio
  stream no longer break preview and export.

See [CHANGELOG.md](https://github.com/Domica/CapRust/blob/main/CHANGELOG.md)
for the full list.

## Requirements

- Windows 10 1809 or later (x64)
- ~150 MB free disk space, plus ~110 MB for FFmpeg and more for
  AI models if you want auto-captions or narration.

## Known issues

- Binary size is ~24 MB (target was 20). Size optimization is a
  post-beta pass.
- Auto-update check is manual (no silent background updater yet).
- Without code signing, Windows SmartScreen may show a warning;
  click "More info" then "Run anyway".

## Verify

SHA-256 of the installer is listed under the asset below.
