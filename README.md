# CapRust

> Social-first video editor in Rust. Native Windows .exe, built for short-form creators.

[![CI](https://github.com/Domica/CapRust/actions/workflows/ci.yml/badge.svg)](https://github.com/Domica/CapRust/actions/workflows/ci.yml)
[![Nightly](https://github.com/Domica/CapRust/actions/workflows/nightly.yml/badge.svg)](https://github.com/Domica/CapRust/actions/workflows/nightly.yml)
[![Version](https://img.shields.io/badge/version-0.3.0--alpha.1-blue.svg)](CHANGELOG.md)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org/)
[![AI: local](https://img.shields.io/badge/AI-Whisper%20%2B%20Piper%20%2B%20ONNX%20(local)-purple.svg)](#what-is-caprust)

---

## What is CapRust?

A modern video editor focused on **short-form social content**. Built from scratch in Rust with egui for the UI and ffmpeg for media processing.

**Not another Electron wrapper.** Native binary, starts in under 500 ms.

### Design goals

| Goal | Status |
|---|---|
| Native performance | Yes — Rust + egui (glow backend) |
| CapCut-style UX | Yes — Dark Material theme, docked panels |
| Social format presets (9:16, 4:5, 1:1, 16:9) | Yes |
| Full undo/redo everywhere | Yes — Command pattern |
| Multi-track magnetic timeline | Yes |
| Effects + transitions | Yes — preview + export |
| Preview = Export filtergraph | Yes — same RenderPlan, no surprises |
| AI captions + narration | Yes — local Whisper + Piper |
| Progressive-reveal captions | Yes — whisper token timestamps |
| Auto-reframe (follow largest face) | Yes — SCRFD primary, YuNet fallback |
| Background removal | Yes — u2netp, per-clip mask cache |
| Speed ramps with easing | Yes — 0.1x-10x, ease + range |
| Auto-ducking | Yes — sidechaincompress |
| Update checker | Yes — GitHub Releases, opt-out |
| Hardware encoding | Yes (NVENC + AMF, probed at runtime) |
| Zero-config install | Yes — portable .exe |

---

## Features

### Timeline
- **Magnetic mode** — clips pack end-to-end, ripple delete
- **Snap to clips** — edges and playhead
- **Multi-track** — V1/V2, A1/A2, Text, pinned Overlay, Captions
- **Drag and drop** from media library
- **Cross-track drag**, trim handles, split at playhead
- **Multi-select** — Ctrl+click, marquee rubber-band
- **Follow playhead** during playback, **trim-follow** option
- **Pan tool** for horizontal navigation
- **Per-track chips** — mute, hide, lock, pin (all render-affecting)

### Media
- Import **video** (MP4, MOV, AVI, MKV, WebM), **audio** (MP3, WAV, M4A, FLAC), **images** (PNG, JPG, WebP)
- Auto **thumbnails** via ffmpeg extraction + per-project cache
- **Background probe** — duration, resolution, FPS
- **Missing-media relink** on load. Dialog lists every missing file, folder picker matches by basename, whole batch runs as one undoable `RelinkManyCommand`. Media bin shows a red badge, timeline clips without a source get a red diagonal hatch.
- **Media bin** with sort (added / name / type), filter (all / video / audio / image), preview size (S / M / L)

### Effects, Filters and Transitions
- **16 effect presets** — blur, vignette, glitch, RGB split, B&W, flash, mirror, kaleido, old film, VHS, light leak, particle, sparkle, ghost, lens flare, zoom pulse
- **16 color filters** — warm, cool, B&W, sepia, cinematic, vintage, vivid, matte, noir, sunset, ocean, fade, pastel, neon, gold, none
- **12 transitions** — fade, slide (L/R/U/D), wipe (L/R), zoom (in/out), rotate, blur
- **8 text styles** — default, bold, subtitle, lower, quote, caption, glow, handwrite
- **Amount-aware** — every preset scales with the per-clip amount; disabled stages are skipped
- **Per-clip effects chain** with undo/redo

### Animations
- **Zoom pulse** — breathing crop with drifting window
- **Shake** — deterministic sin/cos camera shake, fixed 8 Hz
- **Ghost** — motion trails via tmix
- **Sparkle** — temporal noise with brightness lift
- **Particle** — softened temporal noise (atmospheric dust/snow)
- **Lens flare** — warm bloom (colorbalance + curves + gblur)

### Audio
- **Fade in/out handles** — drag on the clip's waveform
- **Volume keyframes** — piecewise-linear dB automation with eval=frame
- **Auto-ducking** — music ducks under narration via sidechaincompress
- **Speed ramps** — 0.1x-10x, with easing and range; audio uses a segmented atempo chain that matches the video setpts windows bit-for-bit
- **Preview audio** — cpal output fed by a ring buffer

### AI (all local, no cloud)
- **Whisper captions** — tiny/base/small/medium; token timestamps power the progressive-reveal render
- **Piper narration** — en_US (lessac/amy); voices download from Hugging Face
- **Auto-reframe** — YuNet face detection + keypoint pan, cached per clip
- **Background removal** — u2netp segmentation, FFV1 mask cache, maskedmerge render stage
- **DEMO marker** — zero-byte DEMO file in the models dir exercises the caption pipeline without weights

### Preview
- **Real-time preview** through the same filtergraph as export
- Quality selector: 1/4, 1/2, 1:1
- Frame-accurate playhead
- Wall-clock playback timing — immune to UI frame rate
- **Seek-optimized rendering** - the plan shifts every input to its `-ss`/`-t` for the current playhead. A seek into a 3-minute project renders in ~200-500 ms instead of 5-10 s.
- **Paused seek** re-renders a one-shot frame at the new playhead.
- **Auto-respawn on edit** — hash-based invalidation catches every mutation

### Updates
- **Startup check** — silent GitHub Releases API request, once per 24 h, opt-out in Settings -> Appearance
- **Toast** — three actions: Download (opens release page), Remind me later (snooze 7 days), Skip this version
- **No auto-download** — the app only notifies and links

### Export
- **Multi-track compositing** — V1 / V2 / Overlay / Text / Captions z-order
- **xfade transitions** between adjacent clips
- **Effects rendering** in filtergraph
- **Caption burn-in** — drawtext with style presets
- Resolution: 4K, QHD, 1080p, 720p, 21:9, Original, 1.5x, 2x
- Frame rate: 24, 30, 60, or Original (exact fraction, preserves 29.97)
- Codec: H.264 / H.265 / AV1 on CPU (libx264 / libx265 / libsvtav1) or hardware (NVENC / AMF). The dialog probes the running ffmpeg build and only offers encoders that open on this machine.
- Quality: Small / Regular / Large (CRF 26 / 20 / 16)
- Advanced: VBR / CBR + bitrate, Color range
- Real-time progress bar, Reveal in folder

### Internationalization
- **English + Croatian** via Fluent
- Runtime language switching — no restart

### Theming
- **Dark / Light / Custom**
- 9 accent presets + color picker
- Editable track colors and playhead

---

## Quick Start

### Windows (pre-built)

Download the latest nightly build:

    gh run list --workflow=nightly.yml --limit 1
    gh run download RUN_ID -n caprust-nightly-win64
    .\caprust-app.exe

Or grab it from [Actions — Nightly Build](https://github.com/Domica/CapRust/actions/workflows/nightly.yml).

### Build from source

Prerequisites:

- Rust 1.91+ (pinned via `rust-toolchain.toml`)
- **FFmpeg 5.1+** binary in PATH (tested on 5.1.x and 7.x)
- **Windows:** Visual Studio Build Tools (C++ workload)
- **Linux:** libgtk-3-dev libxcb-shape0-dev libxkbcommon-dev libasound2-dev
- **macOS:** Xcode command line tools

    git clone https://github.com/Domica/CapRust
    cd CapRust
    cargo build --release -p caprust-app --no-default-features
    ./target/release/caprust-app

FFmpeg is called as a **subprocess**, never linked. Binary stays small, no build issues.

### Dev Container (Codespaces / VS Code)

    .devcontainer/
      Dockerfile
      devcontainer.json

Everything pre-installed: ffmpeg (via apt), ALSA, GTK, X11 libs, cargo-nextest.

---

## Architecture

**Cargo workspace** with strict dependency direction:

    app  ->  ui  ->  media-io  ->  core
    app  ->  ui  ->  i18n

| Crate | Purpose |
|---|---|
| **core** | Domain: timeline, commands, project state, settings, media library, models registry, cache |
| **media-io** | ffprobe, thumbnail extraction, mux, export filtergraph, preview renderer, whisper, piper, face detection, frame extraction, background removal |
| **ui** | egui app, theme, panels, timeline widgets, background jobs |
| **i18n** | Fluent bundles (en, hr) |
| **app** | Binary entry point |

### Design principles

- **Command pattern everywhere** — every mutation is undoable, testable, isolated
- **Never link libav*** — ffmpeg is always a subprocess
- **Preview = Export** — same RenderPlan, same filtergraph, no surprises
- **Wall-clock playhead** — playback timing from Instant, not UI frame count
- **Thread-local i18n** — FluentBundle is not Sync, use thread_local
- **Hash-based preview invalidation** — no per-command flags, no forgotten hooks

Full details in [DIRECTIVES.md](DIRECTIVES.md).

---

## Status

| Phase | Feature | Status |
|---|---|---|
| Core | Timeline, commands, undo/redo | Done |
| Media | Import, thumbnails, library | Done |
| Timeline | Multi-track, magnetic, snap, trim | Done |
| UI | Asset browser, properties, settings | Done |
| i18n | English + Croatian | Done |
| Preview | Real-time through export filtergraph | Done |
| Export | Video with effects + transitions + captions | Done |
| **O** | CapCut parity (fade, keyframes, ramps, ducking) | Done |
| **S** | Particle + lens flare effects | Done |
| **F1b** | Model downloader with SHA-256 | Done |
| **K** | Preview auto-respawn on edit | Done |
| **P1** | Progressive-reveal captions | Done |
| **P2** | Auto-reframe (YuNet + crop pan) | Done |
| **P3** | Background removal (u2netp) | Done |
| **I** | CLAP audio plugins | Planned |
| **N** | MCP server — AI-driven editing | Planned |
| **K2** | Seamless double-buffer preview | Planned |
| **G** | Hardware encoding — NVENC / AMF / QSV | Planned |

See DIRECTIVES.md section 18 for the full roadmap.

---

## Testing

Uses **cargo-nextest** for parallel test execution:

    cargo nextest run --workspace
    cargo nextest run --workspace -E "test(ripple)"
    cargo nextest run --workspace --retries 2

CI runs on Ubuntu, Windows, macOS with clippy -D warnings.

Some integration tests are #[ignore]d because they need a real model on disk. Run them manually with:

    CAPRUST_YUNET_PATH=/path/to/yunet.onnx cargo nextest run --run-ignored face_detect
    CAPRUST_U2NETP_PATH=/path/to/u2netp.onnx cargo nextest run --run-ignored background_removal

---

## Contributing

**Open source, MIT licensed.**

Before opening a PR:

1. Read DIRECTIVES.md — the project design rules
2. Run locally:

       cargo fmt --all
       cargo clippy --workspace --all-targets -- -D warnings
       cargo nextest run --workspace

3. Reference existing issues or open a new one to discuss design

---

## Why another video editor?

CapCut is closed-source and phones home. Premiere is a subscription. DaVinci is heavy. Most open source alternatives are abandoned, Electron-based, or built on 20-year-old C++.

CapRust is:

- **Small** — portable binary
- **Fast** — native Rust, no Node runtime
- **Social-first** — 9:16, quick export, TikTok / Reels presets
- **Local AI** — Whisper, Piper, YuNet, u2netp run on your machine
- **Open** — MIT, no telemetry
- **Modern** — Rust 2021, egui, fluent, tract-onnx

---

## Version History

Full details in [CHANGELOG.md](CHANGELOG.md).

### 0.3.0-alpha.1 - 2026-09-28

- **Hardware-accelerated export** - NVENC H.264 / HEVC and AMF wired into the export dialog. 5-15x faster on the RTX 2060 reference machine.
- **Pre-rendered audio PCM cache** - background ffmpeg renders the project audio to a stable file in `%TEMP%`. Fixes the 17-clip cumulative attenuation bug (opening clip was ~1/2^17 of the last).
- **Seek-optimized preview** - plan rewrites inputs to `-ss`/`-t` for the current playhead. Seek into a 3-minute project went from 5-10 s to ~200-500 ms.
- **Missing-media relink on load** - dialog with folder picker, batch `RelinkManyCommand`, media bin badge, timeline hatch.
- Video output carries correct mp4 tags (`hvc1`, `av01`).

### 0.2.0-alpha.1 - 2026-09-29

- **Dockable panel layout** (egui_dock 0.16) with four presets in View -> Layout, persisted across sessions.
- **Captions translation** via MyMemory - right-click a Captions clip, get a new track in the target language.
- Preview skips missing sources instead of dying.
- Speed slider debounced 250 ms; no per-frame respawn.
- eframe built without AccessKit (egui_dock mid-frame panic workaround).

### 0.1.0-alpha.1 - 2026-09-27

First public alpha.

- Timeline: magnetic, snap, ripple, multi-select, marquee, trim-follow
- Preview: filtergraph-based, auto-respawn on edits
- Export: video + audio + xfade sync + caption burn-in, ETA and size estimate
- Captions: Whisper transcription, progressive reveal, per-clip style
- AI: Piper narration, YuNet auto-reframe, u2netp background removal
- Effects: 16 presets, 16 filters, 12 transitions, 8 text styles
- Audio: fades, volume keyframes, auto-ducking, speed ramps
- English + Croatian via Fluent

---

## License

MIT — see [LICENSE](LICENSE).

---

## Credits

Icons: Phosphor Icons
ONNX inference: tract (pure-Rust)
Face detection model: YuNet (opencv_zoo, Apache 2.0)
Background removal model: u2netp (rembg, Apache 2.0)

---

Made with Rust.
