# Contributing to CapRust

Short, practical rules. The project is MIT-licensed; by contributing
you agree your work lands under the same license.

## Build gate (required before every commit and PR)

```powershell
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
```

CI runs exactly this on Ubuntu, Windows and macOS. If CI is red,
the PR waits — no exceptions. Run `cargo fmt --all` (not manual
reformatting) whenever `--check` complains.

Rust 1.91+ (pinned via `rust-toolchain.toml`). FFmpeg 5.1+ binary on
`PATH` for media features. Linux needs the GTK/ALSA dev packages
(see README); Windows needs VS Build Tools (C++ workload).

## Commits

- Conventional style: `feat(ui): …`, `fix(render): …`,
  `refactor(ui): …`, `style(ui): …` (fmt-only), `docs: …`,
  `chore(release): …`.
- One logical change per commit, so each stays revertable.
- Commit message body explains *why*, not *what* (the diff shows
  what). Reference issues as `Closes #NN` when applicable.
- Never commit: `DIRECTIVES.md` (local notes, gitignored),
  `installer/dist/`, `*.log`, local scratch files.

## Code conventions

- Every user-visible string goes through `tr("key")` — no hardcoded
  English/Croatian in UI code. Add the `en` + `hr` keys together;
  see TRANSLATING.md.
- UI panels never call `ui.button(...)` directly; use
  `crate::widgets::button::{primary, secondary, ghost, icon}`.
  In menus `ui.button` is the correct idiom.
- Boolean toggles use `switch::switch_labeled`; keep `checkbox` only
  for disclosures (expand/collapse), not state flags.
- New visual effects/filters: one `Preset` row in `asset_browser.rs`
  + one match arm in `build_one_effect` (`export_graph.rs`) +
  `en`/`hr` labels + a chain test. Same for transitions via
  `xfade_name` (overlap/shift logic keys off `is_xfade_id`, so new
  xfade-family ids get it for free).
- Undoable mutations go through `UndoStack` commands
  (`caprust-core/src/commands/`). Read-only header interactions
  (lock/mute/volume) write directly like the existing chips.
- `cargo fmt` owns formatting. Do not hand-format; do not fight it.

## Tests

- `cargo nextest run --workspace` must stay green. New render logic
  needs a filtergraph assertion test (see `export_graph.rs` tests);
  new widgets need a renders-without-panic test.
- Some integration tests are `#[ignore]`d (need real model files);
  run them explicitly when touching that area (see README Testing).

## Issues and discussions

- Bugs: use the bug report template (version, Windows version,
  FFmpeg source, logs with `RUST_LOG=caprust=debug`).
- Ideas and questions: GitHub Discussions.
- Check `DIRECTIVES.md` §18 roadmap context if you have it locally;
  externally, the CHANGELOG shows where the project stands.
