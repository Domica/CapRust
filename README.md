<p align="center">
  <img src="docs/logo.png" alt="CapRust" width="128" height="128">
</p>

<h1 align="center">CapRust</h1>

<p align="center"><em>Social-first video editor in Rust. Native Windows .exe, built for short-form creators.</em></p>

[![CI](https://github.com/Domica/CapRust/actions/workflows/ci.yml/badge.svg)](https://github.com/Domica/CapRust/actions/workflows/ci.yml)
[![Nightly](https://github.com/Domica/CapRust/actions/workflows/nightly.yml/badge.svg)](https://github.com/Domica/CapRust/actions/workflows/nightly.yml)
[![Downloads](https://img.shields.io/github/downloads-pre/Domica/CapRust/total?label=downloads&color=blue)](https://github.com/Domica/CapRust/releases)
[![Version](https://img.shields.io/badge/version-0.9.14-blue.svg)](CHANGELOG.md)
[![Website](https://img.shields.io/badge/website-caprust-4ade80)](https://domica.github.io/CapRust/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.91%2B-orange.svg)](https://www.rust-lang.org/)
[![AI: local](https://img.shields.io/badge/AI-Whisper%20%2B%20Piper%20%2B%20ONNX%20(local)-purple.svg)](#what-is-caprust)

**Not another Electron wrapper.** Native binary, starts in under 500 ms. No cloud, no telemetry, MIT licensed.

*I started building it in my free time while playing with AI-generated videos (Minimax H3) — I wanted an editor that's fully customizable and works my way. If you like it, enjoy and use it. :)*

📖 **Full documentation: [full_readme.md](full_readme.md)** with every feature, architecture, status table, testing and contributing.

---

## Install

**Windows 10 / 11, 64-bit.** Grab the latest installer from the
[Releases page](https://github.com/Domica/CapRust/releases/latest):

1. Run `CapRust-0.9.14-win64-setup.exe`.
2. The wizard installs per-user by default (no UAC prompt). Pick
   "Install for all users" if you prefer Program Files.
3. Launch CapRust from the Start Menu.

On first run CapRust offers to download a static FFmpeg build
(~110 MB) if neither `ffmpeg` nor `ffprobe` is on your `PATH`.
AI models (Whisper, Piper, ONNX) download on demand from
**Settings → AI Models**.

---

## What is CapRust?

A modern video editor focused on **short-form social content** (9:16, Reels, Shorts, TikTok). Rust + egui UI, ffmpeg under the hood, local AI (Whisper captions, Piper narration, face detection, background removal). Preview and export share the same filtergraph, so what you see is what renders.

## Highlights

- 🆓 **No watermarks. No payments.** MIT licensed, no telemetry, no account.
- 📴 **Local-first, works offline.** AI runs on your machine, cloud sync is just a folder.
- 🗨️ **Auto-captions.** Local Whisper (tiny→medium), styled + progressive-reveal captions, SRT export.
- 🎙️ **Text-to-speech.** Free local Piper voices, straight onto the timeline.
- 🪄 **Background removal.** One click per clip, cached masks, preview = export.
- 🎬 **49 transitions, 32 effects, blend modes, film LUTs.** Live preview, same graph renders export.
- ⚡ **Cut fast.** Split, trim, ripple, freeze frame, 0.1x–10x speed ramps, magnetic timeline, auto-cut to beats.
- 🎛️ **Multi-track audio.** Volume keyframes, auto-ducking, beat detection, one-switch cleanup (denoise/boost/normalize) + voice FX.
- 🎨 **Fully customizable UI.** Window colors, accents, UI + font size, fonts, layouts, clip borders, waveform and overlap colors, plus dark/light/custom themes.
- ✍️ **Titles.** 15 text styles, stroke, shadow, boxes, captions translation.
- 🚀 **Export.** H.264, HEVC, AV1 (CPU or hardware: NVENC/AMF). Up to 4K 60.
- 🦀 **Native Rust engine.** Sub-500 ms startup, ffmpeg subprocess (never linked), full undo/redo.
- 🧠 **Scriptable.** MCP server (JSON-RPC over stdio), so AI agents can drive the editor.
- 💻 **Windows + Linux. 5 UI languages.** EN, HR, ES, IT, DE with runtime switching, no restart.

---

## Screenshots

<p align="center">
  <img src="docs/screenshots/editor.png" alt="CapRust editor" width="900">
  <br>
  <em>Editor: media bin, preview, magnetic timeline, clip properties.</em>
</p>

<details>
<summary><strong>More screenshots</strong> (start screen, captions, properties, effects, transitions, filters, master chain, settings)</summary>
<br>
<p align="center">
  <a href="docs/screenshots/start-screen.png"><img src="docs/screenshots/start-screen.png" alt="Start screen" width="280"></a>
  <a href="docs/screenshots/editor-tracks-caption.png"><img src="docs/screenshots/editor-tracks-caption.png" alt="Captions on timeline" width="280"></a>
  <a href="docs/screenshots/editor-clip-properties.png"><img src="docs/screenshots/editor-clip-properties.png" alt="Clip properties" width="280"></a>
</p>
<p align="center">
  <a href="docs/screenshots/editor-effects.png"><img src="docs/screenshots/editor-effects.png" alt="Effects browser" width="280"></a>
  <a href="docs/screenshots/editor-transitions.png"><img src="docs/screenshots/editor-transitions.png" alt="Transitions" width="280"></a>
  <a href="docs/screenshots/editor-master.png"><img src="docs/screenshots/editor-master.png" alt="Master chain" width="280"></a>
</p>
<p align="center">
  <a href="docs/screenshots/editor-settings.png"><img src="docs/screenshots/editor-settings.png" alt="Settings" width="280"></a>
  <a href="docs/screenshots/editor-settings-backup.png"><img src="docs/screenshots/editor-settings-backup.png" alt="Settings backup" width="280"></a>
  <a href="docs/screenshots/editor-filters.png"><img src="docs/screenshots/editor-filters.png" alt="Filters" width="280"></a>
</p>
</details>

---

## Features

- **Magnetic multi-track timeline:** V1/V2, audio, text, overlay, captions; snap, ripple, trim, split, auto-cut to beats
- **45+ transitions, 30+ effects, 16 filters, 15 text styles**, plus blend modes and per-clip color LUTs (`.cube` upload)
- **Local AI:** Whisper captions + SRT, Piper narration, auto-reframe, background removal
- **Pro audio:** volume automation, auto-ducking, beat detection, fade handles, CLAP plugins
- **Chroma key, multicam with audio sync, screen recording**, hardware encode (NVENC/AMF)
- **Fully customizable UI** (window colors, accents, UI + font size, fonts, layouts, clip borders, waveform and overlap colors)

---

> [!NOTE]
> 🌍 **Want CapRust in your language?** See [TRANSLATING.md](TRANSLATING.md): copy `en.ftl`, translate the values, open a PR. Untranslated keys render as raw keys, so even partial translations help.

## Quick Start

### Build from source

Prerequisites: Rust 1.91+, FFmpeg 5.1+ in PATH, Visual Studio Build Tools (Windows) / GTK+X11+ALSA (Linux).

    git clone https://github.com/Domica/CapRust
    cd CapRust
    cargo build --release -p caprust-app --no-default-features
    ./target/release/caprust-app

FFmpeg is called as a **subprocess**, never linked. See [full_readme.md](full_readme.md) for architecture, testing (`cargo nextest`), and contributing (read DIRECTIVES.md first).

---

## Version History

See [CHANGELOG.md](CHANGELOG.md) for full per-release notes.

| Version | Date | Highlights |
|---|---|---|
| **0.9.14** | 2026-10-10 | Spanish, Italian, German UI; slim README + full_readme; landing screenshot carousel; LUT + CLAP folder pickers in Settings; models rescan fix. |
| **0.9.13** | 2026-10-09 | Text effects, blend modes, clip rename, refresh imports, split display, border options. |
| **0.9.12** | 2026-10-09 | Save Frame fix, 27 transitions, 10 effects, LUT pipeline, beat snap + auto-cut. |
| **0.9.11** | 2026-10-08 | Three-color waveform, media bin sidebar, 8 text styles, waveform pickers. |

---

## License

MIT - see [LICENSE](LICENSE). Icons: Phosphor. ONNX: tract. Face: YuNet. BG removal: u2netp. CLAP: clack-host.

---

Made with Rust.
