# CapRust

Social-first video editor in Rust (egui + wgpu + FFmpeg).

## Features

- Timeline with command-pattern undo/redo
- Ripple-on-drop: shifts clips right when a drop would overlap
- Missing-media relink dialog
- Clear project cache (regenerable artwork + waveforms)
- Multi-language UI (English, Croatian) via Fluent
- Media bin sorting
- Aspect ratio presets: 16:9, 9:16, 4:5, 1:1, 2:1, 21:9
- Export resolution: named tiers, Original, 1.5×, 2×
- Export frame rate: 24/30/60 fps or original (exact fraction)
- VBR / CBR bitrate modes
- Real-time export ETA
- Sample-counter audio mixing (robust to timestamp discontinuities)
- PTS-ordering mux

## Layout

    crates/
      core/         timeline model, commands, project state
      media-io/     export settings, ETA, audio mix, mux
      ui/           egui app, theme, panels
      i18n/         Fluent localization
      app/          binary entry point

## Local development

    cargo run -p caprust-app

## Build

    cargo build --release -p caprust-app

## Nightly builds

Push to `main` — GitHub Actions produces `caprust-nightly-win64.exe`.
