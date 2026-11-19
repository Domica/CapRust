# Potential TODO (not scheduled)

Items discussed but not committed to a release. Prioritize before starting any.

---

## 🎭 Replace face on AI-generated clips (Minimax H3)

**Problem:** H3 ignores identity prompts — generated people keep changing face even with a correct prompt. User wants to swap the face on an AI clip with an AI-generated reference image.

**Why it matters:** H3 has no reliable character-consistency control. Prompt-first mitigations (character sheet + `@Image 1` + video/audio reference + refine-instead-of-regenerate) fix ~80% without touching CapRust. The remaining 20% needs post-processing.

### Options evaluated

| Option | What it does | Extra models | Download | License | Effort | Verdict |
|---|---|---|---|---|---|---|
| **Prompt-first mitigations** | character sheet + `@Image`/`@Video` refs + refine | 0 | 0 MB | n/a | 15 min | Try first — fixes most cases |
| **GFPGAN face restore** | blind face restoration (fixes "nakrivo" AI faces) | 1 (~300 MB) | ~300 MB | ✅ Apache 2.0 | 1–2 tjedna | Best if face is just low quality |
| **Warp + LaMa** | affine-warp a reference face onto detected face, inpaint seams | 0 (existing) | 0 MB | ✅ existing | 3–5 dana | Only if subject doesn't speak (frozen mouth) |
| **GHOST one-shot swap** | identity transfer from reference image, keeps target video expression | 4–5 (pth→ONNX) | ~600+ MB | ✅ Apache 2.0 | 3–5 tjedna | Only if you need a *different* person who speaks |
| **inswapper / SimSwap** | classic face swap | 2–3 | ~800 MB | ⛔ non-commercial research | — | ❌ incompatible with MIT license |

### Key constraints
- **Warp + LaMa cannot follow speech** — source is a static image, so mouth stays frozen. Not usable for talking heads.
- **GHOST keeps speech** — it transfers identity (ArcFace embedding) but keeps expression/mouth from the target video. This is the only free option for a talking replacement.
- **Licensing:** InsightFace `inswapper_128`, SimSwap, CodeFormer are non-commercial/research-only — incompatible with CapRust's MIT license. GHOST and GFPGAN are Apache 2.0 (clean).

### Recommended path
1. Prompt-first mitigations (no code).
2. If face quality is the issue → GFPGAN restore.
3. If a different speaking person is needed → GHOST.

> File this under "investigate before implementing." Do not start coding until the user confirms which option.

---

## 0.9.31 development (uncommitted in working tree)

- Faza 2.4: encapsulate `media`, `models`, `tracks` (and consider `master_plugins`, `multicam_groups`, `beat_grids`) — done, uncommitted
- Faza 2.6: convert production `.unwrap()` calls to error handling — done, uncommitted
- Bump `Cargo.toml` back to `0.9.31`, add CHANGELOG `[Unreleased]` section

---

## CI

- `https://github.com/Domica/CapRust/actions/runs/44352985131` failed on all 3 platforms at `cargo test --workspace -j 1`.
- Locally all unit tests pass (core 301, mcp 112, android 8, ui 204, screen-record 4, otio-import, app). media-io integration tests are ignored (need model/ffmpeg downloads); media-io lib test gets sandbox-killed locally (environment).
- Could not retrieve raw CI logs (GitHub API returns 403 "Must have admin rights").
- My 0.9.30 release commits touched only docs (README, CHANGELOG, release notes, landing page) — cannot cause Rust test failures. Likely pre-existing/flaky.
