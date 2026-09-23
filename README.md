# CapRust

Social-first video editor in Rust (egui + wgpu + FFmpeg).
Ported from Domica's contributions to `jub0t/Concat`.

## Features ported
- Ripple-on-drop (Concat #141)
- Missing-media relink (Concat #76)
- Clear project cache (Concat #75)
- Croatian locale (Concat #61)
- Media bin sorting scaffold (Concat #63)
- Aspect ratio presets (Concat #179)
- Export resolution Original/1.5x/2x (Concat #175)
- Export frame rate Original (Concat #175)
- VBR/CBR rate modes (Concat #143)
- Real-time ETA (Concat #64)
- Audio sample-counter mix (Concat #68)
- Mux PTS ordering fix (Concat #68)
- Clean window-close shutdown (Concat #93)
- CI serial tests (Concat #110)
- Nightly build paths (Concat #122)
- Captions-above flag (Concat #142)

## Local dev
    cargo run -p caprust-app

## Nightly
Push to main — GitHub Actions produces caprust-nightly-win64.exe.
