Bug fix sweep on top of 0.9.0. Nine fixes plus two audio UX
improvements.

## Install

**Windows 10 / 11, 64-bit.**

1. Download `CapRust-0.9.1-win64-setup.exe` below.
2. Run the wizard. Per-user install by default (no UAC prompt);
   pick "Install for all users" if you prefer Program Files.
3. Launch from the Start Menu.

On first run CapRust offers to download a static FFmpeg build
(~110 MB) if `ffmpeg` / `ffprobe` are not on your PATH.

## Highlights

- **Split-at-playhead fix** - the right half now plays from where
  the left half stopped. Fade in/out on split halves works too.
- **Volume automation fix** - two or more keyframes no longer
  freeze the preview.
- **Orphan media relink** - projects whose media library was
  re-imported now relink on load, so thumbnails and waveforms
  come back without a full re-import.
- **No more cmd windows** - ffmpeg subprocess spawns no longer
  steal focus from the editor.
- **Update checker sees prereleases** - the toast now fires for
  beta releases, not only for stable.
- **Ducking reduction slider** - choose how much a music clip
  drops under narration, from -1 dB to -40 dB.
- **Logs to file in release builds** - `%APPDATA%\CapRust\logs\`.
  Attach the latest log when reporting a bug. Open the folder from
  Settings -> Appearance.

See [CHANGELOG.md](https://github.com/Domica/CapRust/blob/main/CHANGELOG.md)
for the full list.

## Requirements

- Windows 10 1809 or later (x64)
- ~150 MB free disk space, plus ~110 MB for FFmpeg and more for
  AI models if you want auto-captions or narration.

## Known issues

- Auto-update check is manual (no silent background updater yet).
- Without code signing, Windows SmartScreen may show a warning;
  click "More info" then "Run anyway".

## Verify

SHA-256 of the installer is listed under the asset below.
