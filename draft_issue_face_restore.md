## Enhancement: AI face restore/enhance for AI-generated clips (GFPGAN)

### Problem
AI video generators (e.g. Minimax H3) frequently produce distorted faces even with correct prompts — melted mouths, wrong eye placement, asymmetrical features. There is currently no way in CapRust to repair faces on already-generated AI clips.

Users have asked for a "replace face" function that swaps a face on AI clips with an AI-generated image, but the approaches evaluated so far have limitations:

- **Warp + LaMa** — static reference face swap; cannot follow speech or head motion (frozen mouth), unsuitable for talking heads.
- **Inswapper / SimSwap** — face swapping, but non-commercial licenses incompatible with CapRust's MIT license.
- **GFPGAN** — face *restoration* (not swap). Operates on existing face geometry and preserves expression/grimaces/mouth motion during speech. Apache 2.0 license (~350 MB). This is the viable path.

### Proposed solution
Add a **GFPGAN face restore/enhance** toggle in the Video tab of clip properties, gated on the `gfpgan-v1.4` model (ModelKind::Enhancer) being Ready + enabled. When enabled, the export render graph runs the GFPGAN ONNX pipeline over the clip's frames to restore facial detail while preserving temporal consistency (speech motion, expression).

Until the tract-onnx GFPGAN pipeline lands, fall back to a smartblur + hqdn3d denoise chain.

### Why this matters
- Fixes distorted faces on remote/low-quality AI video recordings where the model generated faces poorly.
- Preserves speech lip-sync and grimaces (unlike Warp + LaMa which freezes the mouth).
- Fully open-source / Apache 2.0, compatible with MIT.

### Implementation status (reference)
- `crates/core/src/models.rs` — `gfpgan-v1.4` ModelInfo entry added (ModelKind::Enhancer, ~350 MB, Apache 2.0, URL `https://github.com/TencentARC/GFPGAN/releases/download/v1.3.0/GFPGANv1.4.onnx`).
- `crates/core/src/presets.rs` — `gfpgan` added to EFFECTS table.
- `crates/media-io/src/export_graph.rs` — `gfpgan` effect arm + unit tests (fallback chain until ONNX pipeline lands).
- `crates/ui/src/panels/asset_browser.rs` — `gfpgan` card in EFFECTS catalog, `adjust` category.
- `crates/ui/src/panels/clip_properties.rs` — GFPGAN face restore checkbox in `show_effects()`, gated on model Ready + enabled.
- i18n keys added to en/hr/de/es/it locales.

### Future consideration (separate issue)
Face **replace/swap** (substituting an AI-generated face image onto the clip) — requires additional models and license-compatible swap architecture. Track as potential follow-up, not part of this issue.

### Acceptance criteria
- [ ] GFPGAN checkbox appears in clip properties Video tab when `gfpgan-v1.4` model is Ready + enabled
- [ ] Export runs GFPGAN restore on affected frames without breaking lip-sync/expression
- [ ] Falls back gracefully when model not downloaded
- [ ] i18n translations complete in all 5 locales
- [ ] `preset_drift_tests` pass (UI catalog matches core EFFECTS table)
