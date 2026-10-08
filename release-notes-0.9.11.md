## Added

- **Three-color waveform amplitude rendering.** Positive amplitude (above 0 dB) renders green, negative amplitude (below 0 dB) renders orange, 0 dB reference line renders subtle white. Independent colors configurable in Settings → Appearance → Timeline.
- **Media bin sidebar with TabLayout.** Two modes: **NoFilter** (legacy flat list with top controls) and **Filtered** (left sidebar with filter, sort, size picker). Resizable sidebar with drag handle, S/M/L size picker at top, persisted width per session.
- **8 new text styles.** Outline, Drop Shadow, Typewriter, Neon Glow, 3D Extrusion, Gradient, Stamp, plus existing 8 styles.
- **Media bin tab layout toggle in View menu.** "No filter" (legacy flat list) / "Filtered" (with sidebar) — consistent with Transitions/Effects/Filters/Text tabs.
- **Waveform color pickers in Settings → Appearance → Timeline.** Separate pickers for positive (above 0 dB), negative (below 0 dB), and center (0 dB line).

### Fixed

- **Save frame output path escaping.** Fixed exit code -22 (EINVAL) on Windows paths with spaces/colons by using same escaping as filter inputs.
- **Timeline overlay test for three-color waveform.** Fixed test assertions to match new positive/negative/center line rendering (previous test expected single-color polyline).
- **Crossing detection logic.** Only triggers on actual sign changes (pos→neg or neg→pos), not when either value is exactly 0.

### Changed

- **Asset browser tab strip vertical.** Media, Transitions, Effects, Filters and Text tabs sit on the left side of the panel instead of a horizontal strip across the top.
- **Playhead time display** moved next to undo/redo for better visibility.
- **Redundant Add Text button** removed from the toolbar.
- **Disk-full hardening.** Pre-flight guards for preview, export and audio cache (256 MB / 1 GB thresholds) with toast + startup cleanup of stale `caprust-audio-*.pcm` files.

## Install

- `CapRust-0.9.11-win64-setup.exe` — signed Inno Setup installer
- `caprust-nightly-linux.zip` — Linux nightly binary archive

## SHA256

- `CapRust-0.9.11-win64-setup.exe`: `05fb73ac52bb2d3f25a7130014c6eb7ea96e1d02e2855e278d5fab0aa1417eb0`
- `caprust-nightly-linux.zip`: `cef298ec533c0c726fe490f4d6677bde38b9db702049b9d35ba2606f4c2a2b81`