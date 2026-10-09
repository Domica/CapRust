## Added

- **27 new transitions.** Cover, reveal, diagonal, corner wipes, vertical/horizontal open/close, fade-grays, zoom-blur fade, circle/rect crop, squeeze.
- **10 new effects.** Cinematic bars, teal & orange, cross process, bleach bypass, moonlight, dreamy haze, 3D flip, freezeframe, echo, beauty (skin smoothing).
- **Color LUT pipeline.** Per-clip LUT with custom `.cube` upload + 4 bundled film LUTs (Kodak, Fuji, B&W contrast, vintage sepia).
- **Beat snap-to-grid + auto-cut.** Clip edges snap to beat markers; "Auto-cut to beats" splits video clips at each beat in one undo step.
- **Media-bin tab layout persistence.** Filtered / No filter toggle and sidebar width survive restarts.

### Fixed

- **Save Frame EINVAL.** Single-frame capture now renders a video-only filtergraph (was: unconnected `[a_final]` audio output → ffmpeg -22).
- **5 broken effects/filters.** Cinematic bars, freezeframe, grain, kaleido, fade filter — all root-caused via pixel-stats harness and repaired.

## Install

- `CapRust-0.9.12-win64-setup.exe` — signed Inno Setup installer
- `caprust-nightly-linux.zip` — Linux nightly binary archive

## SHA256

- `CapRust-0.9.12-win64-setup.exe`: `a08cc6a80d40ecb673ffb6194129b4998bc7b0370c485b99d565c89daa4d1ace`
- `caprust-nightly-linux.zip`: `5071c7bcf643f09522117b4c9bb789064dbe461a06c3a534f2cfee33db099901`
