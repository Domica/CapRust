# Changelog

All notable changes to CapRust. Format loosely follows Keep a Changelog.

## [0.4.0-alpha.1] — 2026-09-28

Fourth public alpha. Editor interaction pass: media bin multi-select,
group drag on the timeline, and the caption pipeline.

### Added
- **Media bin multi-select.** Ctrl+click toggles membership, plain
  click replaces. Selected cards get a thicker accent border. The X
  button on a card that belongs to a multi-item selection removes
  every selected item at once, with Ctrl+Z restoring the clips that
  referenced them.
- **Long-press drag from the media bin.** Holding the primary button
  on a card for 500 ms starts a drag; a shorter click only selects.
  The card preview badge follows the cursor and shows how many items
  are being dragged ("1 clip" / "N clips").
- **Batch drag to the timeline.** Dropping a multi-selection chains
  the items left-to-right at the target drop position, using each
  item's duration. The inserted clips come back selected, so a
  follow-up move or delete acts on the whole batch.
- **Group move on the timeline.** Ctrl+click or marquee selects
  multiple clips; dragging one of them moves the entire group by the
  same time and track delta in a single undoable step. Previously a
  multi-select drag moved only the clip under the pointer.

### Fixed
- **Captions and text overlays rendered at the wrong time after a
  seek.** Video and audio already shifted their timeline_start_sec
  by seek_sec and skipped clips before the seek. Captions and text
  overlays did not, so a seek into the middle of a long project left
  their enable windows on the original timeline. Same seek
  arithmetic now applies to both arms.
- **Apostrophes in caption text broke the preview.** Both `\'` and
  the shell-style `'\''` escape fail on the BtbN / gyan ffmpeg
  builds; ffmpeg's filter option parser does not implement
  close-reopen semantics. The first leaks the rest of the option
  list as a new filter name (the classic 'No such filter: <number>'
  error), the second terminates the text value early and renders
  fragments of the filtergraph on the frame. Fix: substitute ASCII
  apostrophe and double quote with U+2019 / U+201C at render time;
  these are not filtergraph special chars and never need escaping.
- **Media bin X button was unclickable on hover.** The button was
  only drawn when the card was hovered, but the enclosing
  dnd_drag_source claimed the pointer the moment it entered the
  card. egui's hover arbitration then reported the card as
  not-hovered as soon as the pointer reached the X, so the button
  vanished mid-click. Rewrote visibility and hit test against the
  raw pointer position.
- **Every plain click in the media bin started a drag.** egui's
  dnd_drag_source uses the whole available UI rect as its interact
  area, so inside a scrollable grid every click anywhere in the
  panel started a drag on the last-registered card. Replaced with
  per-card `allocate_exact_size` + manual `DragAndDrop::set_payload`.

### Changed
- `last_dnd_payload`, `pending_drop` and the timeline drop handler
  now carry `Vec<Uuid>` instead of a single `Uuid`.
- `MediaBinState` gained `selected_media: Vec<Uuid>`.
- New core module `commands::move_many` with `MoveManyCommand`.

### Known issues
- No installer yet; download the nightly ZIP and extract it.
- FFmpeg is not bundled. First run offers to download a BtbN build
  or to point at an existing install.
- Windows only.
- A batch drop emits one `RippleInsertCommand` per item; undo takes
  N presses. A follow-up will collapse them into one command.
- Text overlay rotation is stored in the model but neither rendered
  nor exposed in the UI.
- Preview A/V sync has a residual 100-300 ms lead/lag on some
  hardware. Tunable via `CAPRUST_AV_DELAY_MS`.

## [0.3.0-alpha.1] — 2026-09-28

Third public alpha. Audio pipeline rewritten, seek made usable, and
hardware encoders wired in.

### Added
- **Hardware-accelerated export** (NVENC, AMF). The export dialog
  probes the running ffmpeg build on first open and offers only the
  encoders that actually open on this machine. CPU H.264/H.265/AV1
  remain always available. HEVC and AV1 output carry the correct
  mp4 tags (hvc1, av01). Measured 5-15x faster on an RTX 2060.
- **Pre-rendered audio PCM cache.** A background ffmpeg process
  renders the whole project's audio mix to a stable PCM file in
  %TEMP%. The play path prefers this cache over the preview's own
  PCM output, so audio survives a preview respawn instead of
  restarting on every edit and seek.
- **Seek optimization in the preview renderer.** `plan_from_project`
  now takes the current playhead and rewrites each input to
  `-ss`/`-t` so ffmpeg demuxer-seeks instead of decoding from t=0.
  A seek into the second half of a 3-minute project goes from 5-10 s
  to ~200-500 ms. Projects with transitions fall back to the old
  output-side seek.
- **Missing-media relink flow.** On project load, if any media file
  is gone, a dialog lists the missing items, offers a folder picker,
  and matches files by basename (case-insensitive, depth-capped
  scan). The whole batch runs as one undoable `RelinkManyCommand`.
- Media bin shows a red "Missing" badge on cards whose source file
  is gone.
- Timeline clips without a source file are drawn with a red
  diagonal hatch so a broken clip cannot hide under its normal
  fill colour.
- Paused seek now spawns a one-shot preview render at the new
  playhead. Previously a timeline click while paused moved the
  playhead but left the old frame on screen and no renderer alive.

### Fixed
- **Final audio mix was attenuating clips cumulatively.** A chain
  of N sequential `amix` filters with the default `normalize=1`
  divided the signal by 2 at each stage, so the first clip on a
  17-clip project played at roughly 1/2^17 of the last. Replaced
  with a single `amix` across all inputs at `normalize=0`. The
  opening clip is now at full level and a lone m4a no longer plays
  ~10x louder than video-embedded audio.
- Video frame rate and playback stayed silent on a seek past the
  first clip. PTS was not reset before the filtergraph's trim, so
  the trim saw an empty window and selected zero frames. Reset
  before trim in both video and audio chains.
- `RenderPlan::total_duration_sec` was zero when a seek was applied
  because it had already been derived from the seek-shifted clips;
  subtracting again collapsed the base black / silence bed to 0 s.
- The 650 ms A/V delay compensation is now skipped on
  seek-optimized plans. The fixed value pushed video behind audio
  whenever the audio was cache-fed and the video was already
  seeked.

### Changed
- `Cargo.lock` regenerated; workspace version bumped to
  0.3.0-alpha.1.
- `ExportSettings.codec_index` values 3..=8 now select NVENC and
  AMF variants. 0/1/2 keep their old meaning. Existing projects
  load unchanged.

### Known issues
- No installer yet; download the nightly ZIP and extract it.
- FFmpeg is not bundled. First run offers to download a BtbN build
  (~110 MB) or to point at an existing install. NVENC and AMF
  require a build that was compiled with `--enable-nvenc` /
  `--enable-amf`; the BtbN and gyan.dev builds both satisfy that.
- Windows only.
- Text overlay rotation is stored in the model but neither rendered
  nor exposed in the UI.
- MyMemory translation quality is below DeepL.
- Preview A/V sync has a residual 100-300 ms lead/lag on some
  hardware. Tunable via `CAPRUST_AV_DELAY_MS`; a UI slider is
  planned.

## [0.2.0-alpha.1] — 2026-09-29

Second public alpha. Feature additions since 0.1.0-alpha.1.

### Added
- **Dockable panel layout** (egui_dock 0.16). Tabs can be dragged
  between zones, split horizontally or vertically, and the
  arrangement persists across sessions. Four presets in
  View -> Layout: Classic, Wide timeline, Timeline focus, Preview
  focus. Reset to default restores the 3-zone layout.
- **Captions translation** via MyMemory. Right-click a Captions clip
  -> "Translate captions...". A new Captions track is created per
  language with the same segment timings; the source clip is
  untouched. Settings -> Translation lets you pick source, target,
  and an optional contact email (raises the daily quota 10x).

### Fixed
- Preview no longer dies when a clip points at a missing source
  file. plan_from_project skips such clips and reports the count
  through a toast. A single bad clip used to kill the whole
  renderer, which looked like "play/pause does nothing".
- Speed slider drag no longer causes a preview respawn per frame;
  a 250 ms debounce window absorbs the value changes.
- Audio clips no longer show the Video properties tab.
- Video-only blocks (Auto-reframe, Background removal) are hidden
  on TextOverlay clips.

### Changed
- eframe is built with default-features = false. AccessKit is
  excluded because accesskit_consumer panics when egui_dock
  rearranges widget ids mid-frame. Revisit for the beta a11y pass.
- Nightly builds use a baseline x86-64 target (no AVX-512).
- Media bin toolbars wrap (horizontal_wrapped) so a narrow dock
  zone reflows instead of clipping trailing buttons.

### Known issues
- No installer yet; download the nightly ZIP and extract it.
- FFmpeg is not bundled. First run offers to download a BtbN build
  (~110 MB) or to point at an existing install.
- Windows only.
- Text overlay rotation is stored in the model but neither rendered
  nor exposed in the UI. drawtext has no rotate filter.
- MyMemory translation quality is below DeepL. A local Marian NMT
  backend is on the roadmap.
- The nightly build does not carry AVX-512; older CPUs are safe.

## [0.1.0-alpha.1] — 2026-09-27

First public alpha. Social-first video editor for Windows. Pre-release:
expect rough edges, broken installations, and one or two missing features.

### Added
- Timeline: magnetic mode, snap, ripple, multi-select, marquee, trim-follow
- Preview: filtergraph-based (same plan as export), auto-respawn on edits
- Export: video + audio + xfade sync + caption burn-in, ETA and size estimate
- Captions: Whisper transcription, per-word progressive reveal, per-clip style
  (font size, position, text color, outline, background box)
- Text overlays: content editor, motion (x/y/scale), effects (blink/pulse/
  color-cycle), drag and resize directly in the preview pane
- Effects: 17 built-in (blur, zoom_pulse, chromashift, vhs, ghost, sparkle,
  particle, lens_flare, ...)
- Audio: fade in/out, volume keyframes, auto-ducking, speed ramp with easing
- AI: auto-reframe (SCRFD), background removal (u2netp), karaoke captions
- Models: downloader with SHA-256 verification for Whisper and Piper voices
- FFmpeg: auto-detect on PATH, user override, or managed auto-download with
  first-run prompt
- i18n: English and Croatian

### Known issues
- Installer not yet provided; download the nightly ZIP manually
- FFmpeg is not bundled; the first-run prompt downloads it
- Windows only (Linux/macOS compile in CI but are not supported)
- Rotation on text overlays is stored but not rendered
