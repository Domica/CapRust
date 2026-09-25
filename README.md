# CapRust

> Social-first video editor in Rust. Native Windows .exe, built for short-form creators.

[![CI](https://github.com/Domica/CapRust/actions/workflows/ci.yml/badge.svg)](https://github.com/Domica/CapRust/actions/workflows/ci.yml)
[![Nightly](https://github.com/Domica/CapRust/actions/workflows/nightly.yml/badge.svg)](https://github.com/Domica/CapRust/actions/workflows/nightly.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.83%2B-orange.svg)](https://www.rust-lang.org/)
[![AI: local](https://img.shields.io/badge/AI-Whisper%20%2B%20Piper%20(local)-purple.svg)](#what-is-caprust)

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
| Zero-config install | Yes — Portable .exe |

---

## Features

### Timeline
- **Magnetic mode** — clips pack end-to-end, ripple delete
- **Snap to clips** — edges and playhead
- **Multi-track** — V1/V2, A1, Text, pinned Overlay, Captions
- **Drag and drop** from media library
- **Cross-track drag**, trim handles, split at playhead
- **Follow playhead** during playback
- **Pan tool** for horizontal navigation

### Media
- Import **video** (MP4, MOV, AVI, MKV, WebM), **audio** (MP3, WAV, M4A, FLAC), **images** (PNG, JPG, WebP)
- Auto **thumbnails** via ffmpeg extraction + per-project cache
- **Background probe** — duration, resolution, FPS
- **Media bin** with sort (added / name / type), filter (all / video / audio / image), preview size (S / M / L)

### Effects and Transitions
- **14 effect presets** — blur, vignette, glitch, RGB split, sepia, B&W, cinematic, vintage, noir, sunset, ocean, pastel, neon, gold
- **10 transitions** — fade, slide (L/R/U/D), wipe (L/R), zoom (in/out), rotate
- **Text overlays** via `drawtext`
- **Per-clip effects chain** with undo/redo

### Preview
- **Real-time preview** through the same filtergraph as export
- Quality selector: 1/4, 1/2, 1:1
- Frame-accurate playhead
- Wall-clock playback timing — immune to UI frame rate

### Export
- **Multi-track compositing** — V1 / V2 / Overlay z-order
- **xfade transitions** between adjacent clips
- **Effects rendering** in filtergraph
- Resolution: 4K, QHD, 1080p, 720p, 21:9, Original project, 1.5x, 2x
- Frame rate: 24, 30, 60, or Original (exact fraction, preserves 29.97)
- Codec: H.264 (H.265 / AV1 planned)
- Quality: Small / Regular / Large (CRF 26 / 20 / 16)
- Advanced: VBR / CBR + bitrate, Color range
- Real-time ETA, progress bar, Reveal in folder

### Internationalization
- **English + Croatian** via Fluent
- Runtime language switching — no restart

### Theming
- **Dark / Light / Custom**
- 9 accent presets + color picker

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

- Rust 1.83+ (`rustup install stable`)
- **FFmpeg 7** binary in PATH
- **Windows:** Visual Studio Build Tools (C++ workload)
- **Linux:** `libgtk-3-dev libxcb-shape0-dev libxkbcommon-dev libasound2-dev`
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

Everything pre-installed: ffmpeg 7 static, ALSA, GTK, X11 libs, cargo-nextest.

---

## Architecture

**Cargo workspace** with strict dependency direction:

    app  ->  ui  ->  media-io  ->  core
    app  ->  ui  ->  i18n

| Crate | Purpose |
|---|---|
| **core** | Domain: timeline, commands, project state, settings, media library, models registry |
| **media-io** | ffprobe, thumbnail extraction, mux, export filtergraph, preview renderer |
| **ui** | egui app, theme, panels, timeline widgets, background jobs |
| **i18n** | Fluent bundles (en, hr) |
| **app** | Binary entry point |

### Design principles

- **Command pattern everywhere** — every mutation is undoable, testable, isolated
- **Never link libav\*** — ffmpeg is always a subprocess
- **Preview = Export** — same RenderPlan, same filtergraph, no surprises
- **Wall-clock playhead** — playback timing from `Instant`, not UI frame count
- **Thread-local i18n** — `FluentBundle` is not `Sync`, use `thread_local`

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
| Preview | Real-time through export filtergraph | Done — perf polish ongoing |
| Export | Video with effects + transitions | Done — audio mux test in progress |
| **H** | Audio playback in preview | Done |
| **F** | AI models — Whisper, Piper | Done |
| **I** | CLAP audio plugins | Planned |
| **J** | Animations — zoom pulse, shake, particle | Planned |
| **N** | MCP server — AI-driven editing | Planned |
| **O** | Cloud sync | Planned |
| **P** | Mobile — iOS / Android | Planned |

See `DIRECTIVES.md` section 18 for the full roadmap.

---

## Testing

Uses **cargo-nextest** for parallel test execution:

    cargo nextest run --workspace
    cargo nextest run --workspace -E "test(ripple)"
    cargo nextest run --workspace --retries 2

CI runs on Ubuntu, Windows, macOS with `clippy -D warnings`.

---

## Contributing

**Open source, MIT licensed.**

Before opening a PR:

1. Read `DIRECTIVES.md` — the project design rules
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
- **Fast** — native Rust
- **Social-first** — 9:16, quick export, TikTok / Reels presets
- **Open** — MIT, no telemetry
- **Modern** — Rust 2021, egui, fluent

---

## License

MIT — see [LICENSE](LICENSE).

---

## Credits

Icons: Phosphor Icons

---

Made with Rust.
