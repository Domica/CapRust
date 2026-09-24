# CapRust — Global Directives

> Project constitution. Read before every PR. Rules override convenience.

**Version:** 3 (post Faza G)
**Last updated:** 2026-09-24

---

## 1. Product

**CapRust** is a social-first video editor in Rust for Windows desktop.
Target users: short-form creators (TikTok, Reels, Shorts).

**Non-goals for v1:** professional NLE, cloud sync, mobile (planned for later phases — see §23).

**UX inspirations:** CapCut (social presets, quick export), Premiere Pro (magnetic timeline), DaVinci Resolve (color vocabulary). Feature ideas only — no code reuse, no attribution.

---

## 2. Toolchain

| Component | Version | Notes |
|---|---|---|
| Rust | stable 1.83+ | edition = 2021 |
| egui / eframe | 0.31 | do not upgrade without API audit |
| egui-phosphor | 0.9 | must match egui 0.31 |
| uuid | 1.x | features = ["v4", "serde"] |
| serde / serde_json | 1.x | derive on all persisted types |
| anyhow / thiserror | 1.x / 2.x | anyhow public API, thiserror typed |
| rfd | 0.15 | native file dialogs |
| tracing / tracing-subscriber | 0.1 / 0.3 | env-filter |
| image | 0.25 | jpeg feature only |
| cpal | 0.15 | audio playback (Faza H) |
| ringbuf | 0.4 | lock-free SPSC |
| ffmpeg / ffprobe | external 7.x | **NEVER link** — subprocess only |

**Windows-first.** Linux/macOS compile in CI but UX polish is Windows.

---

## 3. Workspace Layout

```
caprust/
|-- Cargo.toml              workspace + shared deps + release profile
|-- DIRECTIVES.md           this file
|-- .devcontainer/          Codespaces / VS Code dev container
|-- .github/workflows/      CI + Nightly
|-- crates/
|   |-- core/               domain: timeline, commands, project IO, settings, media, models, cache
|   |-- media-io/           ffprobe, thumbnail, mux, export spec, audio_mix, player, preview_render, streamer, export_graph, exporter
|   |-- ui/                 egui app, theme, panels, timeline widgets, media jobs
|   |-- i18n/               Fluent bundles (en, hr)
|   `-- app/                binary entry point
```

**One crate = one bounded context.** Crate dependencies are strictly:

```
app -> ui -> media-io -> core
app -> ui -> i18n
```

No cycles. `core` depends on nothing internal. UI logic stays in `ui`; domain logic stays in `core`.

---

## 4. Code Rules

### 4.1 Compilation gates (every commit)

    cargo fmt --all
    cargo check --workspace
    cargo clippy --workspace --all-targets -- -D warnings
    cargo nextest run --workspace

**`-D warnings` is non-negotiable.** No `#[allow]` without an inline comment explaining why.

### 4.2 Serialization

Every persisted type derives `Serialize, Deserialize`.
Storage keys are lower-snake-case, versioned on schema change:
`"theme"`, `"settings"`, `"recent"`. Future: `"theme_v2"`, etc.

Never break an existing key silently — bump the version and migrate.

New struct fields must have `#[serde(default)]` unless migration is explicit.

### 4.3 Errors

- Public API returns `anyhow::Result<T>`.
- `thiserror` only for typed domain errors that callers match on.
- Never `unwrap()` outside tests. `expect()` only for truly impossible branches.

### 4.4 Logging

Use `tracing::{trace, info, warn, error}`. No `println!` outside `main.rs`.
Log every user-visible mutation: file saved, clip dropped, model downloaded.
Job/probe/thumbnail logs must be emitted for every step so we can diagnose from `caprust-app.exe` in a cmd window.

### 4.5 Naming

- Types: `PascalCase`
- Fields / functions: `snake_case`
- Constants: `SCREAMING_SNAKE_CASE`
- egui ids: `("noun", id)` tuples
- Storage keys: `"<thing>"` or `"<thing>_v<n>"`
- FTL keys: `<section>-<thing>` (e.g. `menu-file-save`)

---

## 5. egui 0.31 — Do Not Regress

These API points each caused a real bug. Do not revert.

| Correct | Wrong (breaks) |
|---|---|
| `egui::CornerRadius::same(6)` | `Rounding::same(6)` |
| `visuals.window_corner_radius` | `visuals.window_rounding` |
| `painter.rect_stroke(rect, r, stroke, StrokeKind::Inside)` | 3-arg `rect_stroke` |
| `egui::DragAndDrop::payload::<T>(ctx)` | `ctx.dnd_payload()` |
| `ui.new_child(UiBuilder::new()...)` | `ui.child_ui(rect, layout, None)` |
| `ui.ctx().load_texture(name, img, opts)` | `TextureHandle::new` |
| `egui::Frame::NONE` | `egui::Frame::none()` |

**`Stroke::new(1.0_f32, ...)` — always suffix `_f32`.** Rust is phasing out float literal fallback.

**ScrollAreas inside panels are dangerous:**
- `drag_to_scroll(true)` steals drags from DnD sources. Use `.drag_to_scroll(false)` on any ScrollArea that hosts drop targets.
- Nested ScrollAreas (vertical inside horizontal) can collapse available height to 0. Prefer manual scroll via a `scroll_x` offset field on the app.
- `ui.available_height()` returns 0 inside a horizontal layout. Capture height *before* entering `ui.horizontal_top(...)` and pass it explicitly.

---

## 6. Command Pattern (mandatory)

Every mutation to `ProjectState` goes through a `Command`:

```rust
pub trait Command: Send + Sync {
    fn execute(&mut self, state: &mut ProjectState) -> anyhow::Result<()>;
    fn undo(&mut self, state: &mut ProjectState) -> anyhow::Result<()>;
    fn description(&self) -> String;
}
```

Execute via `UndoStack::execute(Box::new(cmd), &mut project)`.

- Every command must implement `undo` correctly.
- Every command must have a unit test in its own module.
- Every command must capture "before" state for undo.
- Pure reads bypass the stack; nothing that mutates does.
- Commands live in `crates/core/src/commands/`.

Existing commands:
- `RippleInsertCommand`
- `MoveClipCommand`
- `DeleteClipCommand` (with `ripple: bool`)
- `SplitClipCommand`
- `RelinkCommand`
- `SetClipCommand` (generic field editor)
- `AddEffectCommand`, `RemoveEffectCommand`, `SetTransitionCommand`

---

## 7. Timeline Semantics

### 7.1 Track kinds

| Kind | Multiplicity | Notes |
|---|---|---|
| Video | multiple | V1, V2, V3... |
| Audio | multiple | A1, A2... |
| Text | multiple | lower thirds, titles |
| Overlay | singleton | pinned; always renders above non-pinned |
| Captions | singleton | dedicated captions lane |

Display order: `track::display_order(tracks)` — pinned first, then normal.

### 7.2 Drag & drop (media bin -> timeline)

Payload is `uuid::Uuid` (media id). **The media bin renders before the timeline**, so egui clears the payload before the timeline sees it on the release frame.

Solution (do not remove):
1. In `show_timeline`, read `egui::DragAndDrop::payload::<Uuid>(ctx)` each frame; if `Some`, store in `self.last_dnd_payload`.
2. On `pointer.any_released()`, use `self.last_dnd_payload` (not the current `dnd_payload`).
3. Highlight the lane under `pointer.hover_pos()` while a payload is active.

Never assume `ctx.dnd_payload()` returns `Some` on the release frame.

### 7.3 Cross-track drag

`ClipDrag.track_index` is updated each frame from `track_for_y()` which maps `pointer.y` -> track index using the cached `timeline_row_layout`.

### 7.4 Magnetic mode

- Toggle ON: pack all tracks end-to-end **once**.
- Does not re-pack after drag, drop, or delete.
- Ripple delete is implied by magnetic ON.
- Toggle OFF: gaps preserved.
- Manual drag placement must stick.

### 7.5 Snap

Threshold: 10 px converted to ms. Snaps to clip start, clip end, and playhead.

### 7.6 Trim

Left handle: moves `start_time_ms`, adjusts `duration_ms`.
Right handle: adjusts `duration_ms` only, capped at `clip.source_duration_ms` (0 = unlimited).

Handles are edge zones (8 px wide) detected via pointer position.

---

## 8. i18n

- Bundles are thread-local (`thread_local!` with `RefCell<HashMap<...>>`). `FluentBundle` is not `Sync` — never put it in a `static Lazy`.
- Current language is stored in `AppSettings.language` and applied via `caprust_i18n::set_current_lang()` at the top of `CapRustApp::update()`.
- Every user-facing string goes through `tr("key")` (alias for `caprust_i18n::t("key")` in `crates/ui/src/i18n_helper.rs`).
- Supported: `en`, `hr`. Adding a language = one `.ftl` file + one entry in `LANGUAGES`.
- Never hardcode a translated string in `ui`. Never write to FTL keys not defined in both bundles.

---

## 9. Settings & Persistence

`AppSettings` (app-wide, persisted with eframe storage):

| Field | Type | Notes |
|---|---|---|
| `language` | String | "en" or "hr" |
| `models_dir` | String | where AI models live |
| `ffmpeg_path` | Option<String> | override; None = PATH |
| `ffprobe_path` | Option<String> | override; None = PATH |
| `enable_shortcuts` | bool | gates R/H/V/Delete/S/Ctrl+A/Ctrl+S |

`ProjectState` (per project, `.caprust` JSON):
name, aspect_ratio, base_resolution, frame_rate, tracks, clips, media, models, project_path.

`RecentList` — capped at 12, persisted with key `"recent"`.

**eframe `persistence` feature is required** — without it `save()` never fires and settings/recent silently don't persist.

**Never write to a path the user did not choose.** `%APPDATA%/CapRust/` is the only implicit-write location.

---

## 10. ffmpeg Integration

- **Never link** libav* — always call `ffmpeg.exe` / `ffprobe.exe` as subprocess.
- `media-io` provides safe wrappers: `ffprobe::probe()`, `thumbnail::extract_jpeg()`, `player::decode_frame_rgba()`.
- Detect at startup (`detect_ffmpeg(&settings)`); user overrides in Settings -> Paths.
- Long-running jobs run on a background thread (`JobRunner`) and post results through `std::sync::mpsc`.
- Job result types: `ProbeDone`, `ThumbDone { media_id, temp_jpg }`, `Failed`.
- Missing binaries = graceful degradation: import works, thumbnails are placeholders, export is disabled with a clear message.

**Devcontainer** ships ffmpeg 7.0.2 static (johnvansickle.com). Windows nightly uses the same major version.

### 10.1 Thumbnail pipeline

1. Import -> `MediaItem` created with `probe_done: false, thumb_done: false`.
2. `JobRunner::enqueue()` -> thread runs `ffprobe` -> posts `ProbeDone`.
3. Same thread runs `ffmpeg -ss <t> -frames:v 1 -vf scale=320:-1 <tmp.jpg>` -> posts `ThumbDone`.
4. UI `drain()` copies temp JPEG to `<project>/cache/thumbnails/<id>.jpg`.
5. **`std::fs::rename` fails across drives on Windows.** Always `std::fs::copy` + `std::fs::remove_file`.
6. UI loads JPEG -> `egui::TextureHandle` -> stores in `clip_textures: HashMap<Uuid, TextureHandle>`.

### 10.2 Mux PTS-ordering (critical for export)

`media-io::mux::compare_pts()` normalizes PTS to a common unit before comparison. The naive "alternate one packet per input" approach truncates audio because audio produces ~46 packets/s vs video's 24-60.

**Rule:** never assume video and audio PTS are directly comparable. Always call `compare_pts(pts, tb)`.

### 10.3 Audio sample-counter (critical for export)

Do **not** use `atrim=duration` in the filtergraph for clip out-points. Files with edit lists or paused recordings have broken timestamp series that park frames outside the trim window. Instead:
- Count samples fed per input in Rust.
- Stamp each frame's PTS from a running sample counter.
- Stop feeding the input after `duration * speed * sample_rate` source samples.
- Let `apad` fill silence until duration ends.

Reference impl: `media-io::audio_mix::MixInputState`.

### 10.4 Windows CI

`ffmpeg-next` links libav* at compile time. Windows CI doesn't have them -> build fails. Solution:
- `crates/media-io/Cargo.toml`: `default = []`, `ffmpeg = ["dep:ffmpeg-next"]`.
- Nightly build: `cargo build --release -p caprust-app --no-default-features`.
- Runtime subprocess calls work fine without the crate.

---

## 11. AI Models

- **Never bundle** weights in the binary.
- Default dir: `%APPDATA%/CapRust/models/`; overridable in Settings.
- Registry: `core::models::ModelRegistry` with `ModelStatus` (NotDownloaded, Downloading, Ready, Error).
- Captions: Whisper family (tiny/base/small/medium).
- Narration: Piper voices (en_US lessac/amy, hr_HR ivan, de_DE thorsten).
- Downloads: HTTP via `reqwest` in a later phase. Until then `start_download` writes a `.placeholder` file.
- When no model is ready and the user clicks a caption/narration button, open the model-prompt dialog.

---

## 12. Audio (CLAP — planned)

- Host: `clack-host` crate.
- Built-in effects: `nih_plug`.
- Plugin scan paths (Windows): `C:\Program Files\Common Files\CLAP` and `%APPDATA%/CapRust/plugins`.
- CLAP plugins are **untrusted code**. Wrap every call in `catch_unwind`.
- Audio processing must run on a dedicated thread; use `ringbuf` for UI <-> audio transfer.

---

## 13. UI Design Language

- Dark-first, Material-like: 6 px corners, subtle borders, no harsh strokes.
- Accent-driven: selection, hover, active states derive from `theme.accent`.
- Toggle buttons show **pressed-in** state: darker bg + inset border + accent icon.
- Icons: Phosphor font (`egui_phosphor::regular`).
- Panels are resizable. The timeline panel is user-resizable and caches row geometry.
- Media bin grid: dynamic column count based on panel width.
- Start screen content is centered both axes.
- Export / Settings / model-prompt windows are anchored `Align2::CENTER_CENTER`.

---

## 14. Keyboard Shortcuts

Active only when `AppSettings.enable_shortcuts == true` and `AppMode::Editor`.
Skip all keys while `ctx.wants_keyboard_input()`.

| Key | Action |
|---|---|
| `R` | Reverse selected clips |
| `H` | Mirror horizontally |
| `V` | Mirror vertically |
| `Delete` / `Backspace` | Delete selected clips |
| `S` | Split at playhead |
| `Ctrl+A` | Select all clips |
| `Ctrl+S` | Save project |

When disabled, right-click menu items lose their key hints and R/H/V entries are hidden.

---

## 15. Git & Commits

- Conventional Commits: `feat:`, `fix:`, `refactor:`, `chore:`, `ci:`, `docs:`.
- Scope when non-obvious: `feat(timeline): ...`.
- **Include phase reference:** `feat(export Phase E2-A): ...`
- No references to external projects in commit messages.
- No `Co-Authored-By` trailers.
- One logical change per commit.

---

## 16. CI & Releases

`.github/workflows/ci.yml`:
- Triggers: `push: main` + all PRs.
- Matrix: ubuntu / windows / macos.
- Installs X11/GTK/ALSA libs on Linux (for eframe + cpal).
- Steps: `fmt --check`, `clippy -D warnings`, `cargo nextest run --workspace --no-fail-fast`.

`.github/workflows/nightly.yml`:
- Triggers: `push: main` + `workflow_dispatch`.
- Windows: `cargo build --release -p caprust-app --no-default-features` -> artifact `caprust-nightly-win64`.
- Linux: same, artifact `caprust-nightly-linux`.
- `--no-default-features` is required — ffmpeg linking breaks Windows CI.

**Test runner:** `cargo nextest` (not `cargo test`). Runs each test in its own process -> no OOM.

---

## 17. Definition of Done

A feature is done only when all are true:

1. Compiles under `-D warnings` on all three OSes.
2. All existing tests pass; new behaviour has at least one unit test.
3. Every user-visible string goes through `tr("...")` and exists in both `.ftl` files.
4. Every mutation goes through a `Command`.
5. Every new persisted field has `#[serde(default)]` or a migration.
6. No regression of the egui 0.31 API points in §5.
7. `DIRECTIVES.md` updated if a rule or convention changed.
8. Tested on Windows: import media, drop to timeline, play, save, reopen.

---

## 18. Open Work (v3 — post Faza G)

| Phase | Status | Scope |
|---|---|---|
| Core, Media, Timeline, i18n, Themes, ProjectIO | ✅ | |
| A — i18n kroz UI | ✅ | |
| C — ffprobe + thumbnails | ✅ | |
| D — Preview streaming | ✅ | Radi; perf fix u toku |
| E — Export (video) | ✅ | Audio treba testirati |
| E2-A — Multi-track + image + text + effects | ✅ | |
| E2-B — xfade tranzicije | ✅ | |
| G — Preview kroz filtergraph | ✅ | Zadnji fix: perf (457a8c9) |
| **H — Audio playback u preview** | ⏳ Next | cpal + ringbuf |
| **F — AI modeli (Whisper + Piper)** | ⏳ Planned | Nakon H |
| **I — CLAP audio pluginovi** | ⏳ Planned | |
| **J — Animacije (zoom_pulse, shake, particle)** | ⏳ Planned | |
| **K — Real-time efekt preview bez restarta** | ⏳ Planned | |
| **L — Media hash + sync-friendly format** | ⏳ Planned | Prije mobile |
| **M — I/O trait abstraction** | ⏳ Planned | Prije mobile |
| **N — MCP server** | ⏳ Planned | Nakon F |
| **O — Cloud sync** | ⏳ Planned | Nakon M |
| **P — Mobile (iOS/Android)** | ⏳ Planned | Nakon M + L |

---

## 19. When In Doubt

- Prefer small, isolated PRs. A feature that touches 3 crates is 3 PRs.
- If a change would violate a rule, update the rule first in a separate commit and explain why.
- If two rules conflict, the one higher in this document wins.
- When a bug has an easy fix and a correct fix, do the correct fix and add a test.

---

## 20. Dev Container (Codespaces / VS Code)

Config in `.devcontainer/`.

**Base:** `mcr.microsoft.com/devcontainers/rust:1-1-bookworm`

**Installed:**
- ffmpeg 7.0.2 static (johnvansickle.com) -> `/usr/local/bin/ffmpeg`, `ffprobe`
- cargo-nextest 0.9.85 -> `/usr/local/cargo/bin/cargo-nextest`
- System deps: `clang`, `libclang-dev`, `libgtk-3-dev`, `libxcb-*`, `libxkbcommon-dev`, `libasound2-dev`, `libxrandr-dev`, `libxi-dev`, `libxcursor-dev`

**Persistent volumes:**
- `caprust-cargo-registry` -> `/usr/local/cargo/registry`
- `caprust-target` -> `${workspaceFolder}/target`

**Why:**
- ffmpeg 7 static — never linked, subprocess only. Avoids build issues.
- cargo-nextest — each test in its own process, solves OOM that forced `-j 1`.
- ALSA (`libasound2-dev`) — needed for cpal in Phase H.

---

## 21. Preview Architecture

**Rule:** Preview **NEVER** decodes raw video. It always renders through the same filtergraph as export.

**Flow:**
1. `plan_from_project()` -> `RenderPlan` (same as export)
2. `PreviewRenderer::spawn()` -> one ffmpeg process with `-filter_complex`, output raw RGBA to stdout
3. Reader thread throttles to target fps (`std::thread::sleep`)
4. UI reads frames, uploads as egui texture

**Seek:** `-ss` as **OUTPUT** option (not input). ffmpeg decodes as fast as it can and skips the pre-roll.

**Playhead:**
- Wall-clock derived: `playhead = playback_started_ms + (now - playback_started_at)`
- **NEVER** accumulate `dt` per frame (UI FPS would cause 5-10x speedup)
- Re-anchor `playback_started_at` on the **FIRST frame** (not on spawn) — ffmpeg has 200-500ms startup
- Gate: playhead advances ONLY after first frame (`preview_player.has_frame == true`)

**Renderer restart:**
- Only on: `renderer_dead` (is_none) or `explicit_seek_ms.is_some()`
- **NEVER** on "drift > X ms" — causes restart loop
- `explicit_seek_ms` set on: toggle_play (start), seek buttons, SetPlayhead (>500ms diff), end-of-timeline loop reset

**Audio:** currently drains to `/dev/null`. Phase H adds cpal + ringbuf.

---

## 22. Git Push Workflow

**Resolved:** Consolidated to single repo at `/workspaces/CapRust/` on 2026-09-24.

**Rule:** Always `git pull --rebase origin main` before push.

**Alias:**

    git config --global alias.sync "!git pull --rebase origin main && git push"

Then just: `git sync`

---

## 23. Future Directions (Planning Constraints)

Long-term goals. Architectural decisions in other sections are made to not block them.

### 23.1 Mobile (iOS + Android)

**Constraint:** `core`, `media-io`, `i18n` must be portable — no `std::process::Command`, no `rfd`, no hardcoded paths.

**Rule:** all I/O through traits:
- `MediaDecoder` — desktop uses ffmpeg subprocess, mobile native
- `MediaEncoder` — same
- `FilePicker` — desktop `rfd`, mobile native

**Forbidden in core/media-io/i18n:**
- `std::process::Command`
- `rfd::`
- Absolute paths in data structures

**Timeline:** Phase P, after M (traits) and L (media hash).

### 23.2 Cloud Sync

**Constraint:** project format must be sync-friendly.

**Rule:** media referenced by content hash (blake3), not path.
- `MediaItem.hash: [u8; 32]` — hash of file contents
- `Clip.media_ref: Option<MediaHash>` — reference into media library
- Path is cache, not part of project

**Project format:**

    {
      "media": [
        { "hash": "abc123...", "filename": "video.mp4", "size": 1234 }
      ],
      "clips": [
        { "media_ref": "abc123...", "start": 0, ... }
      ]
    }

When file moves, media library re-finds it by hash (local folder or cloud folder).

**Timeline:** Phase O, after M.

### 23.3 MCP Server

**Goal:** AI clients (Claude Desktop, Cursor) can drive the editor.

**Constraint:** every command must have JSON-serializable input and output.

**Rule:** MCP tools are thin wrappers around existing Command patterns. No duplicated logic.

**Planned tools (Phase N):**
- `caprust_list_media` -> `Vec<MediaItem>`
- `caprust_import_media(path)` -> `MediaHash`
- `caprust_add_clip(media_hash, track, start_ms, dur_ms)` -> `clip_id`
- `caprust_apply_effect(clip_id, effect_id)` -> `()`
- `caprust_generate_captions(clip_id, model_id)` -> `Vec<CaptionSegment>`
- `caprust_narrate(text, voice_id)` -> `audio_path`
- `caprust_export(settings)` -> `output_path`

**Architecture:**

    crates/
      mcp/                <- NEW crate (Phase N)
        src/
          server.rs       - MCP protocol (rmcp)
          tools.rs        - tool definitions
          dispatch.rs     - tool -> Command mapping

**Timeline:** Phase N, after F (AI models are the reason to have MCP at all).

---

## 24. Sync Constraints (for Future Phases)

When designing new data structures, apply these rules:

1. **No absolute paths in persisted state.** Store hashes, filenames, or relative paths.
2. **No OS-specific types in core.** No `std::fs::File`, no `std::path::PathBuf` in serializable structs (use `String`).
3. **Content-addressed media.** Every imported file gets a blake3 hash at import time.
4. **Idempotent imports.** Importing the same file twice should produce the same MediaHash -> dedup.
5. **Portable timestamps.** Use u64 milliseconds since Unix epoch, never system-specific `Instant`.
