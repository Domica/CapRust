# K2: Seamless double-buffer preview

## Opis
Implementacija seamless double-buffer preview sustava za uklanjanje flickeringa/tearinga tijekom playbacka i scrubanja.

## Problem
Trenutni preview sustav koristi single-buffer rendering sto dovodi do:
- Flickeringa pri brzim prijelazima
- Tearinga pri scrubanju (povlacenju playheada)
- Nesposobnost glatkog prikaza pri brzim prijelazima (xfade, prijelazi)
- Poteskocu pri sinkronizaciji audio/video pri brzim skokovima

## Rjesenje: Double-buffer preview
Implementacija double-buffer sustava gdje:
1. **Front buffer** - prikazuje se korisniku (UI)
2. **Back buffer** - renderira se sljedeci frame u pozadini
3. **Swap** - atomska zamjena buffer-a pri v-sync / frame boundary

## Reference implementacije

### CapCut (ByteDance)
- Koristi **double-buffered preview engine** za timeline scrub
- **Offscreen rendering** u Metal/Vulkan zatim present na UI layer
- **Frame queue** (2-3 frame-a) za smooth scrub
- **Hardware decoding** (VideoToolbox/AMF/NVENC) direktno u texture

### DaVinci Resolve
- **Dual timeline** - source i program monitor koriste double-buffer
- **GPU-accelerated** preview pipeline
- **Frame interpolation** za smooth slow-motion scrub

### Adobe Premiere Pro
- **Mercury Playback Engine** - GPU accelerated, multi-buffer
- **Dynamic Link** s After Effects koristi shared textures
- **Preview files** (rendered cache) za complex sequences

### Final Cut Pro
- **Background rendering** - renderira u pozadini while editing
- **Proxy workflow** - full-res + proxy, seamless switch
- **Metal 2** optimizacije za macOS

### Tehnicke detalje (DaVinci/Resolve/Premiere)
- **Swap chain** (3 buffera: front, back, next)
- **Timeline scrub** koristi **frame stepping** + **optical flow** za interpolation
- **Audio sync** odrzan kroz **sample-accurate** timeline position
- **GPU memory pool** za textures (nema realokacija pri swapu)

## Plan implementacije za CapRust

### Faza 1: Double-buffer infrastruktura
- [ ] `PreviewBuffer` struct s `front`/`back` `egui::TextureHandle`
- [ ] `swap_buffers()` metoda za atomic swap
- [ ] V-sync aware present (egui `request_repaint` + frame timing)

### Faza 2: Preview pipeline
- [ ] Offscreen render u back buffer (egui `offscreen` ili custom render pass)
- [ ] Frame queue (2-3 frame-a) za smooth scrub
- [ ] Frame interpolation za slow-mo scrub (opciono)

### Faza 3: Scrub optimizacija
- [ ] Frame cache za brzo skakanje (keyframe-aware)
- [ ] Async decode + upload u GPU texture (ffmpeg + egui texture)
- [ ] Audio sync zadrzani tijekom scrub

### Faza 4: Napredne features
- [ ] Proxy workflow (full-res + proxy seamless switch)
- [ ] Background rendering (render u pozadini)
- [ ] Multi-view (source + program monitor)

## Prioritet
**High** - Preview je core UX, flickering je top user complaint.

## Reference
- egui `TextureHandle` + `Context::tex_manager()` za texture management
- ffmpeg `AVFrame` -> `egui::TextureHandle` upload
- `egui::Context::request_repaint()` za frame pacing
- `std::sync::Arc<parking_lot::Mutex<>>` za thread-safe buffer swap

## Reference u dokumentaciji
- full_readme.md linija 357 - "K2 Seamless double-buffer preview | Planned"