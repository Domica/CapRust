# Changelog

All notable changes to CapRust. Format loosely follows Keep a Changelog.

## [0.1.0-alpha.1] — 2026-09-27

First public alpha. Social-first video editor for Windows. Pre-release:
expect rough edges, broken installations, and one or two missing features.

### Added
- Timeline: magnetic mode, snap, ripple, multi-select, marquee, trim-follow
- Preview: filtergraph-based (same plan as export), auto-respawn on edits
- Export: video + audio + xfade sync + caption burn-in, ETA and size estimate
- Captions: Whisper transcription, per-word progressive reveal, per-clip style
  (font size, position, text color, outline, background box)
- Text overlays: content editor, motion (x/y/scale), effects (blink/pulse/
  color-cycle), drag and resize directly in the preview pane
- Effects: 17 built-in (blur, zoom_pulse, chromashift, vhs, ghost, sparkle,
  particle, lens_flare, ...)
- Audio: fade in/out, volume keyframes, auto-ducking, speed ramp with easing
- AI: auto-reframe (SCRFD), background removal (u2netp), karaoke captions
- Models: downloader with SHA-256 verification for Whisper and Piper voices
- FFmpeg: auto-detect on PATH, user override, or managed auto-download with
  first-run prompt
- i18n: English and Croatian

### Known issues
- Installer not yet provided; download the nightly ZIP manually
- FFmpeg is not bundled; the first-run prompt downloads it
- Windows only (Linux/macOS compile in CI but are not supported)
- Rotation on text overlays is stored but not rendered
