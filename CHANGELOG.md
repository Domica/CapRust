# Changelog

All notable changes to CapRust. Format loosely follows Keep a Changelog.

## [0.9.9] — 2026-10-07

### Added

- **Media library hygiene.** Re-importing the same file no longer
  duplicates the library (matched case- and separator-insensitively)
  — skipped files report a toast. Clear all asks first in a centered
  modal (Cancel / Yes); closing the dialog cancels.
- **Menu polish.** Open File/Edit/View menus keep a persistent gray
  highlight with an accent underline while open. Working shortcut
  hints on File and Edit items (Ctrl+O/S/Z/Y, S, Del), gated on the
  shortcuts setting. New bindings: Ctrl+S saves, Ctrl+O opens.

### Fixed

- **New transitions lost their overlap.** Dissolve, smooth, circle
  close and pixelize were mapped for render but missing from the
  core xfade list, so project load stripped them and no follower
  shift was computed. Both lists are now bound by a regression test
  in both directions.

## [0.9.8] — 2026-10-07

### Added

- **SRT sidecar export.** New switch in the export dialog plus File
  menu entry writes `.srt` next to the video (or standalone) from
  normalized caption cues. Shared `CueWindow` utilities in core.
- **Beat-sync (analysis + markers).** Onset-flux BPM detection runs
  as a background job from the Sound panel, grids persist per media
  with size/mtime fingerprinting, stale grids prune on load, and
  markers draw on the ruler. Snap-to-beat stays a follow-up.
- **4 effects, 7 transitions, 4 voice FX.** Negative, pixelate,
  sketch, sharpen; dissolve, smooth ×4, circle-close, pixelize
  (xfade path, overlap handling included); telephone, bass, echo,
  chipmunk per-clip voice filters. Docs: CONTRIBUTING +
  TRANSLATING guides.

### Fixed

- **Edit menu Split/Delete/Ripple Delete were stubs** since PR 7;
  they now dispatch the same undoable commands as the keys.
- **Caption rows no longer stack on overlaps.** Whisper segments
  overlap in time but enable windows clamped only within a segment;
  a normalization pass clamps across segments (preview and export
  share the plan, one fix covers both).

## [0.9.7] — 2026-10-06

### Fixed

- **Export 'Open folder' now opens the output folder.** An empty or
  relative destination used to resolve who-knows-where, and even a
  correct path fell back to Documents: Explorer parses its own
  command line, so `/select` and the path go literally via `raw_arg`.
  The output dir is ensured up front (toast + abort otherwise), and a
  missing file falls back to its parent dir.
- **Timeline pan and layout fixes.** `Esc` exits the sticky pan tool
  (Grab cursor shows while it is on), wheel/touchpad scroll pans only
  while hovering the timeline, track rows are resizable by dragging
  the header divider (40–160px, saved per project), and the lane
  viewport no longer overflows by item spacing.

### Changed

- **Export finish actions use button widgets and i18n.** Open folder
  is `primary` and Dismiss is `secondary` with new `exp-open-folder`
  / `exp-dismiss` keys (en + hr); removes the last hardcoded English
  strings in the export flow.

## [0.9.6] — 2026-10-06

### Added

- **Disabled states for chip, switch and segmented_control.** New
  `*_enabled` variants dim to 40% alpha, use hover-only hit-testing
  and a `NotAllowed` cursor when disabled; the existing widgets are
  thin wrappers passing `enabled = true`. Button rows across
  Settings, the media bin and six more panels now use the `primary`
  / `secondary` / `ghost` / `icon` variants instead of raw
  `ui.button`, per the widget policy. No behaviour change.

### Fixed

- **Save, load, export, undo and redo failures now surface error
  toasts.** They previously died in the application log; a red toast
  now names the failure next to the log line. Adds `toast-*-failed`
  keys in English and Croatian.

## [0.9.5] — 2026-10-06

### Changed

- **Panel polish (P3-all).** Boolean checkboxes in Settings and the
  FFmpeg prompt become `switch`. Settings theme mode and playhead
  size, media bin preview size, clip properties tabs, model prompt
  tabs, and multicam angle pickers become `segmented_control` or
  `chip`, matching the widget library added in 0.9.4. Menu items in
  View -> Sort / Size stay `selectable_label` (egui idiom).
  `export_window` advanced stays a `checkbox`: it is a disclosure,
  not a state flag.

### Fixed

- **Clip.reversed now renders.** The flag was stored, hashed into
  `render_hash()`, and toggled by `SetClipCommand` since it was added,
  but never reached the render plan or the filtergraph, so toggling
  Reverse produced identical output in preview and export. Now emits
  `,reverse` after `trim=duration=N` (video) and `,areverse` at the
  head of the post-speed tail (audio). The Reverse toggle is disabled
  above `REVERSE_MAX_SEC = 60.0` seconds with an explanatory tooltip,
  since ffmpeg's `reverse` filter buffers the whole trimmed window in
  RAM before emitting frame 0. Closes #29.

## [0.9.4] — 2026-10-06

### Added

- **Widget library (P2).** Five new reusable primitives under
  `crates/ui/src/widgets/`, every one routing its dimensions through
  `theme::tokens`: `chip` (compact selectable pill), `switch` (animated
  boolean toggle), `badge` (colored status pill), `segmented_control`
  (horizontal one-of-N selector), `tooltip_rich` / `tooltip_body`
  (two-line tooltip with wrapped body). Each ships with a
  renders-without-panic test.

### Changed

- **Design tokens sweep finished (P1).** Removed the last hardcoded
  `add_space` values from `crates/ui/src/panels/`; `grep -rnE
  "add_space\([0-9]" crates/ui/src/panels/` now returns nothing. All
  spacing, radii, typography and elevation come from
  `crate::theme::tokens`.

### Fixed

- **Mirror H / Mirror V now render in preview and export.** The
  `flip_h` / `flip_v` flags have been stored on `Clip`, hashed into
  `render_hash()`, and toggled by the UI since they were added, but the
  render plan's `VideoClip` never carried them and the filtergraph
  never emitted `hflip` / `vflip`. Both preview and export produced
  identical output regardless of the toggle. The flags are now plumbed
  through and emitted between `setpts` and `trim`, so the mirror
  applies before any crop or scale pans over the frame. Five
  regression tests cover each combination and the filter ordering.
  (`Clip.reversed` has the same class of bug; tracked separately.)

- **Audio ducking with a single ducked clip** (#28). The
  `sidechaincompress` filter takes `[main][sidechain]`; the
  single-consumer branch of `build_audio_chain` emitted
  `[control][ducked]`, so with exactly one ducked clip the voice was
  attenuated under the music instead of the other way round. The
  multi-consumer branch was already correct. Regression tests cover
  both branches.

## [0.9.3] — 2026-10-06

### Fixed — Timeline overlay follow-ups (#14)

- **Polyline vertices on every keyframe.** The audio envelope polyline
  now places a vertex on every `volume_keyframe`, not only on the 129
  evenly-spaced samples. On long clips the even spacing alone is several
  seconds wide, so short ramps and dips were smoothed over. Interpolation
  between keyframes stays linear in the dB domain, matching the export.
- **Overlay anchored during zoom and scroll.** Duck zones are stored as
  fractions of the clip duration instead of pixel offsets, so the cache
  (keyed on `render_hash()`) no longer goes stale after a zoom change.
  The overlay is laid out against the full clip rect and clipped by the
  painter to the visible portion; a clip scrolled partly out of view
  keeps its zone and polyline aligned with the time axis.
- **Duck zones match what the export wires.** The overlay only draws a
  zone when the export actually builds a sidechain between the two
  clips. A zone is suppressed when the control is the clip itself, either
  track is muted, the control is a video with `audio_detached`, the video
  probe found no audio, or the control clip type carries no audio. An
  integration test compares the overlay against the real
  `plan_from_project` filtergraph.

### Known issues

- `build_audio_chain` may emit sidechain inputs in the wrong order
  (`[ctrl][ducked]` instead of `[ducked][ctrl]`). Reported in #28;
  not confirmed through a full export yet.


## [0.9.2] - 2026-10-05

Timeline audio envelope overlay plus a probe-backfill fix. Small
release; the audio envelope is the headline.

### Added
- **Timeline audio envelope overlay (#14).** Audio clips now draw an
  amplitude-style overlay directly on the clip rect, between the
  waveform strip and the clip label. Two layers share one vertical
  scale (+12 dB at the top, 0 dB at 60% height, -60 dB at the bottom):
  - **Volume automation** — a polyline through ``volume_keyframes``,
    sampled at 128+1 evenly spaced fractions and interpolated in the
    **dB domain** to match the ffmpeg ``volume`` filter expression
    built by ``export_graph``. When the clip has no keyframes the
    overlay draws a flat line at ``volume_db``.
  - **Ducking zones** — a shaded region wherever the
    ``duck_against`` control clip overlaps this clip's span. Fill
    opacity scales with ``duck_reduction_db`` (0.10..0.45), so a
    deeper duck reads darker in the timeline.

  The overlay is cached per clip and invalidated only when
  ``ProjectState::render_hash()`` changes, so scroll and zoom do not
  recompute anything.

### Fixed
- **Probe backfill for media auto-created on load.** Media items
  synthesized by ``relink_orphan_media_refs`` at project load start
  with ``probe_done = false`` and no duration. A new backfill pass
  (``backfill_missing_probes``) enqueues a probe job for every item
  whose cache is empty, so durations and thumbnails arrive even when
  the project was opened on a fresh machine.

## [0.9.1] - 2026-10-05

Nine bug fixes and two audio UX improvements on top of 0.9.0. All
reported from real use of the 0.9.0 beta installer.

### Fixed
- **Split-at-playhead: right half plays from source 0.** A clip
  split at the playhead restarted at 0:00 of the source file instead
  of continuing where the left half stopped. The same missing model
  field broke fade in/out on split halves. ``Clip.source_offset_ms``
  now carries the source window start and the render plan threads it
  into ``-ss`` (video) and ``atrim`` (audio).
- **Volume automation with 2+ keyframes blocked preview.** The
  ``volume=EXPRdB:eval=frame`` form is evaluated once at filter-config
  time in every ffmpeg build we tested. With 2+ keyframes the
  expression references ``t``, which is NAN at init, so the pipeline
  hung. The expression is now in linear-multiplier form
  (``pow(10, dB/20)``) so ``eval=frame`` actually runs per frame.
- **Auto-duck dropdown showed UUIDs.** Labels are now
  ``<kind> @ MM:SS (Ns)``, or the clip name when set.
- **FFmpeg subprocess windows stole focus.** Every spawn from
  media-io and screen-record now goes through a ``silent_command``
  helper that sets ``CREATE_NO_WINDOW`` on Windows release builds.
- **Update checker never saw prerelease builds.** It called
  ``/releases/latest``, which GitHub excludes prereleases from.
  Switched to ``/releases?per_page=20`` and pick the newest non-draft
  entry.
- **Thumbnail strip and waveform missing after project reopen.**
  Media items re-imported (or moved between machines) got fresh
  UUIDs while timeline clips kept the old ones. Project load now
  relinks by clip path and creates a fresh MediaItem for orphan paths.
- **Audio clip dropped before probe finishes stayed at 3s.** The
  probe handler now extends ``duration_ms`` on clips whose
  ``source_duration_ms`` was still 0 (the placeholder state).
- **Clear cache did not regenerate waveforms.** Both the clear-cache
  menu and Regenerate thumbnails now run the thumbnail and waveform
  backfill passes.

### Added
- **Ducking reduction amount.** Per-clip ``duck_reduction_db``
  (default -12 dB), mapped to a sidechaincompress ratio via
  ``2^(-dB/6)``. Slider appears in the Sound tab when a control
  clip is selected.
- **File-based logs in release builds.** Every run writes INFO+ to
  ``%APPDATA%/CapRust/logs/caprust.log.YYYY-MM-DD``; the console is
  hidden in release, ``CAPRUST_DEBUG=1`` restores stderr. Settings →
  Appearance → Open logs folder.

## [0.9.0] - 2026-10-05

Chroma key, multi-camera groups, and screen recording. First
minor bump since the beta line, so expect rough edges in all
three.

### Added
- **Chroma key (green screen)** (core, media-io, ui). Per-clip
  ``ChromaKeySpec`` with key color, similarity, and edge blend. The
  filtergraph inserts ``chromakey=color=...:similarity=...:blend=...``
  after ``format=yuva420p`` in Phase 1 of the base chain, so it
  composes with the existing overlay stack. Properties panel gets
  a color picker, two sliders, and Enable / Remove buttons.
- **Multi-camera groups** (core, media-io, ui). A new
  ``MultiCamGroup`` references existing timeline clips as camera
  angles. Right-click any two-or-more selected clips to create a
  group. The render path suppresses every non-active angle, so
  switching angle changes preview and export in one undoable
  step.
- **Multi-camera audio sync** (media-io). RMS envelope + naive
  cross-correlation on a 100 Hz envelope finds the per-angle
  offset. ``Sync angles`` button in the Multicam dock panel;
  offsets are written through ``SetMultiCamSyncCommand``.
- **Screen recording** (new ``caprust-screen-record`` crate). DXGI
  Desktop Duplication, single-frame + continuous capture to H.264
  mp4 via an ffmpeg stdin pipe. Cursor overlay supports both COLOR
  and MONOCHROME shapes. Modal in File -> Record screen with
  monitor picker, duration, and fps; recordings land in
  ``%APPDATA%\CapRust\recordings\`` and auto-import into the
  media bin. Isolated from ``core`` and ``media-io`` so the
  DIRECTIVES 23.1 portability rule stays intact.

### Fixed
- **Video clips without an audio stream no longer break preview
  and export.** ``plan_from_project`` assumed every ``ClipType::Video``
  carried an embedded audio track, so the audio filtergraph
  emitted ``[N:a]`` references for silent sources and ffmpeg failed
  at filter-graph binding time. ``MediaItem.has_audio`` now carries
  the ffprobe result, and the plan skips audio harvest for a
  source that has none.

### Changed
- Old ``.caprust`` files load as before: ``Clip.chroma_key`` and
  ``ProjectState.multicam_groups`` are ``#[serde(default)]``.

## [0.8.4] - 2026-10-03

Security follow-up, contributed by @llvm-x86 (PR #13). Closes #1.

### Security
- **Settings import and sync-folder load no longer replace local
  executable paths** (settings, app). A settings backup or a file in
  the sync folder could previously set ``ffmpeg_path`` /
  ``ffprobe_path`` to any executable, which the app then runs.
  ``AppSettings::replace_from_untrusted`` takes everything from the
  file except those two paths, which stay machine-local. The sync
  path still requires an explicit user click, so nothing loads
  automatically.
- **MyMemory disclosure** (i18n, settings). The Translation settings
  now state that caption text, and the contact email if set, is sent
  to MyMemory, a third-party service. The request is HTTPS; the
  concern was that a third party receives the text.
- **MCP trust model documented** (docs/mcp.md). ``add_media`` accepts
  any path the user's account can read, by design. The server trusts
  its client, so restricting individual tools would not add
  meaningful protection. A short Trust model section states this
  explicitly.

## [0.8.3] - 2026-10-03

Security hardening pass on top of 0.8.2, contributed by @llvm-x86.
Addresses items 1-5, 7, and the update-URL part of item 6 from #1.

### Security
- **CLAP plugin path trust** (clap_host). A ``.clap`` path from a
  project file was dlopen'd unchecked. Now the path must canonicalize
  inside a scanned plugin directory, have a ``.clap`` extension, and
  sit directly under that directory.
- **Piper archive verification** (piper). Per-platform SHA-256 is
  pinned and verified before extraction; the archive is deleted on
  failure.
- **Model registry distrust** (models). ``merge_missing_defaults``
  now drops unknown entries from project files and always takes
  ``kind``, ``url`` and ``sha256`` from the built-in defaults,
  keeping only user state (enabled, status, progress).
- **SHA-256 pinning for every downloadable model** (models,
  download). Downloads now verify against a pinned hash before
  ``.part`` is renamed into place. Streaming hash (64 KB buffer)
  instead of reading the file into RAM.
- **FFmpeg archive verification** (ffmpeg). The BtbN archive is
  checked against the release's published ``checksums.sha256``
  before extraction. Same-origin, so it catches corruption and
  archive tampering, not a compromised release.
- **``-protocol_whitelist file`` on all media-path inputs**
  (media-io, 11 call sites). Blocks ``http:``, ``concat:`` and
  similar on user-supplied media paths. ``-i -`` (stdin, mask
  writer) and lavfi sources are deliberately exempt.
- **Update checker URL allowlist** (update_checker). Only release
  URLs under this repo are handed to the OS opener; everything else
  is rejected.
- **YuNet URL moved to the official source**. Pinned to opencv_zoo
  at commit ``c97242c`` with a verified SHA-256 (matches the Git
  LFS pointer and the file served from ``media.githubusercontent.com``),
  replacing the unofficial ``casual02`` mirror.

### Changed
- Removed four committed ``*.log`` files; ``.gitignore`` already
  ignored them but they predated the rule.

### Open
- Item 6 from #1 (MCP ``add_media`` path trust, settings-import
  ``ffmpeg_path``, MyMemory disclosure) is tracked in #13 and not
  addressed here.

## [0.8.2] - 2026-10-02

Audio tail and waveform fixes on top of 0.8.1.

### Fixed
- **Audio playback stopped at the end of video content** (#12). The
  render plan's total duration now includes audio and text clips, not
  just video, so ffmpeg keeps writing PCM past the last video clip
  and the audio tail plays through to its real end.
- **Playhead froze and the timeline stopped scrolling** at video EOF.
  The advance is now driven every frame while playback is active,
  gated on `play_anchor_set`, not on a fresh preview frame (#12).
- **Waveform froze at zoom 4x or higher** (#12). The draw loop always
  started at `peaks[0]` regardless of scroll position; it now slices
  the peaks array by the visible fraction of the clip, so the
  waveform scrolls with the clip.

### Changed
- **Background removal in debug builds** is now usable. tract-onnx,
  tract-linalg and tract-core get `opt-level = 3` in the dev profile
  so `cargo run` does not spend minutes per frame on inference.
  `background_removal::load` logs a debug-build warning so the
  trade-off is visible (#3).

## [0.8.1] - 2026-10-02

Maintenance release: three user-facing fixes, two security/correctness
fixes, and an AI-tell docs cleanup. No new features.

### Fixed
- **CBR bitrate ignored on Advanced export** (#5). The VBR/CBR toggle
  and bitrate field were never read by the export pipeline:
  ``build_command`` always emitted ``-crf`` (or ``-cq`` / ``-qp`` for
  hardware encoders). ``RenderPlan`` now carries ``rate_mode`` and
  ``bitrate_kbps``; CBR emits ``-b:v`` / ``-maxrate`` / ``-bufsize``
  on CPU, switches ``-rc cbr`` on NVENC and AMF. Two new unit tests.
- **JobRunner never refreshed ffmpeg/ffprobe paths** (#6). The worker
  thread captured paths at construction, so a mid-session install or
  Settings override was invisible until restart. Jobs now carry their
  own resolved paths, sourced from ``ffmpeg_status`` at enqueue time.
- **Timeline thumbnails stayed empty after a cache clear** (#9).
  ``regen_missing_thumbnails`` enqueued jobs but left ``thumb_done``
  set, and the ``ThumbDone`` drain loop skipped reload when a stale
  ``TextureHandle`` was still held. Both fixed; ``backfill_waveforms``
  had the same flag gap and is fixed too.

### Security
- **BG-removal mask path bypassed protocol whitelist** (#7). A
  malicious ``.caprust`` could point ``Clip.bg_removal`` at an
  absolute path, a ``..`` traversal, or an ffmpeg filter source like
  ``http://attacker/``. ``movie=`` honors all protocols and
  ``-protocol_whitelist`` does not apply to filter-level sources.
  ``resolve_bg_removal_path`` now requires the mask to live directly
  under ``<project>/cache/masks/`` after canonicalization, with a
  filename stem matching the clip UUID and a ``.mkv`` extension.
  Seven new unit tests.

### Changed
- Replaced em and en dashes with ASCII hyphens in README and
  CHANGELOG (75 + 9 occurrences). Cosmetic, no content change.

## [0.8.0] - 2026-10-02

Trim, transitions, audio duration, and theming fixes on top of the
0.7.0-beta.1 polish pass. Waveform display, reattach-audio, ripple
move, and per-track audio processing all land here.

### Added
- **Waveform display** on audio and video clips. Peaks extracted at
  8 kHz into a 2048-bucket cache at ``<project>/cache/waveforms/`` and
  drawn as vertical bars under the clip tint.
- **Per-track volume fader**: right-click the mute chip in the track
  header. Range -60 to +6 dB with Reset and Mute presets.
- **Audio processing flags** per clip: ``denoise`` (afftdn),
  ``voice_boost`` (dynaudnorm), ``normalize`` (loudnorm -16 LUFS).
  Wire order: denoise, voice_boost, normalize, then user gain,
  fades, and the track fader.
- **Reattach audio** (P7). A video whose audio was separated shows
  "Reattach audio" in the context menu; the detached Audio clip
  shows "Reattach to video". Symmetric to Separate audio.
- **Ripple move** toolbar toggle: dragging shifts every later
  same-track clip by the same delta. Independent of magnetic mode.
- **Theme colors** for xfade overlap shading (RGBA) and waveform
  strips (RGB), editable in Settings > Appearance.

### Fixed
- **Audio import duration** was stuck at the 3000 ms fallback. The
  clip's ``source_duration_ms`` now mirrors the media item's real
  length; the probe handler tightens the cap for clips dropped
  before the probe finished.
- **Trim grow** past the shrink point was blocked by the same
  fallback. Growing now caps at the true source duration.
- **Right-trim visual drift**: the timeline rendered the clip at
  ``drag.current_ms`` instead of the clip's real start, so growing
  the right edge looked like a body drag. Now uses the live
  ``start_time_ms``.
- **Trim edge detection** used the live pointer position, which is
  already past the 8 px zone by the time ``drag_started()`` fires.
  Now uses ``press_origin()``.
- **Xfade duration change** dropped the transition and shifted the
  clip right. ``SetTransitionDurationCommand`` looked for a
  predecessor using the already-shifted start position; now uses
  the natural start.
- **Progress bar** for background jobs was invisible in the docked
  layout. ``show_jobs_bar`` is now called from ``show_editor_dock``
  too.
- **Indeterminate progress** on BG removal: the bar stayed at 0%
  until frame 25 of 72. Now shows a pulse from the ``Started`` event.

### Changed
- ``PROGRESS_STRIDE`` for background removal 25 to 5. Adds a
  ``frame i/N`` trace line per stride.

## [0.7.0-beta.1] - 2026-10-01

First beta of the 0.7 line. A UI polish pass (design tokens,
widget library, feedback states, start-screen redesign), a crash
handler, and a Windows installer.

### Added
- **Design tokens** in `theme/tokens.rs` (space, radius, text,
  anim, elev). 143 hardcoded spacing/radius/font sites migrated
  across 14 files; `apply_style(ctx)` folded into `Theme::apply`.
- **Widget library** at `crates/ui/src/widgets/`: `button`,
  `dialog`, `empty`, `loading`, `banner`, `property`, `section`,
  `toolbar`.
- **Loading widget** (`spinner`, `spinner_with`, `centered`).
  FFmpeg downloads without a `Content-Length` now show a spinner
  plus a running MiB counter instead of a frozen 0% bar.
- **Banner widget** (`Info` / `Warning` / `Error`) with an
  optional action slot. First use: a persistent warning above the
  timeline when project media files are missing, with a Relink
  call to action.
- **Error toasts.** `ToastKind::{Info, Error}`; error toasts
  render with a red accent and stay up 6 s (vs 4 s for Info).
  13 failure paths migrated from `toast` to `toast_error`.
- **Crash handler.** `crates/app/src/crash.rs` installs a panic
  hook that writes `%APPDATA%/CapRust/crashes/crash-<unix>.log`
  with version, timestamp, panic location, message, and a full
  backtrace. Last 20 reports kept; older ones pruned on write.
- **Windows installer.** Inno Setup 6 script at
  `installer/caprust.iss` plus a one-shot `installer/build.ps1`.
  Per-user install (no UAC prompt), Start Menu shortcut, optional
  desktop shortcut, LZMA2/max compression (22 MB exe → 8 MB
  setup). Output: `CapRust-<ver>-win64-setup.exe`.
- **Start screen redesign.** Left sidebar with CapRust wordmark,
  `+ New Project` (opens a modal), `Open Project`, `Settings`,
  `Quit`, and a version label. Recent projects render as a
  dynamic grid of 16:9 tiles with cached thumbnails and metadata
  (duration · clips · age, then resolution · fps).

### Changed
- Version bumped `0.6.0-alpha.1` → `0.7.0-beta.1`; `winres`
  embeds the new version in the executable's file properties.
- `RecentProject` carries `base_resolution`, `frame_rate_label`,
  and `first_media_id` (all `#[serde(default)]` for backward
  compatibility).
- Preview transport bar sizes extracted into file-local consts;
  settings window is a fixed 640×700 (min) so tabs do not
  resize it, opens centred.
- Dock tab strip: darker background, full-accent active tab with
  luminance-based text colour, dimmed 55% when the zone loses
  keyboard focus.
- Five panels migrated to `empty::placeholder`; four headings to
  `section::header`; `ffmpeg_prompt` and `relink_dialog` to the
  dialog widget.

### Fixed
- **Invisible text on selected `selectable_value` widgets.**
  `Theme` set `selection.stroke = Stroke::NONE`, whose colour is
  transparent; egui uses that colour for the label. Affected the
  properties tab bar and every ComboBox dropdown. Now uses a
  luminance-based `contrast_on(accent)`.
- **`hr.ftl` literal `\uXXXX` escapes** (`\u0161`, `\u010d`,
  `\u017e`, `\u2026`) replaced with real UTF-8 characters; the
  start screen was showing them verbatim.

### Tests
- 295 passing, 8 skipped.

## [0.6.0-alpha.1] - 2026-09-30

First alpha of the 0.6 line. Settings portability, user-selectable
fonts, and captions parity.

### Added
- **Settings export/import as JSON.** File → Settings → Paths →
  Backup. Export writes a versioned `SettingsFile` envelope; import
  reads it back. Missing fields are defaulted, a newer format
  version is refused with a clear message.
- **Settings auto-sync via cloud folder.** Point CapRust at a folder
  that Google Drive, OneDrive, Dropbox, Box, iCloud Drive,
  Syncthing, or Nextcloud keeps in sync. On every save a snapshot
  is written to `<folder>/caprust-settings.json`. On startup, if
  the folder holds a newer snapshot, a modal offers Load them /
  Keep local. Transport is the cloud client's folder sync; CapRust
  never talks to a cloud API.
- **User-selectable app font family + size.** Settings → Appearance
  → Font. Families: Default (egui), Segoe UI, Arial, Consolas.
  Size 0.8-1.5× scales the entire UI via pixels-per-point. No font
  files bundled; the system font is read from `%WINDIR%\Fonts`.
- **Captions motion + procedural effects parity with text
  overlays.** Captions clips now carry `TextMotion` (X/Y/Scale) and
  `TextEffect` (Blink/Pulse/ColorCycle), editable from the clip
  properties panel. Same command, same Ctrl+Z semantics as
  TextOverlay.
- **Issue templates** for bug reports and feature requests.
- **Drift logger.** The periodic `preview state` INFO line now
  includes `audio_ms` and `delta` for future sync regressions.

### Changed
- `crates/mcp/Cargo.toml` uses `version.workspace = true`.

### Fixed
- Captions drawtext now respects the clip's motion and effect
  instead of hardcoding identity/none.
- The `preview state` sync measurement no longer reports a bogus
  drift when the audio player has not started yet.

### Tests
- 257 passing, 8 skipped.

## [0.5.0-alpha.1] - 2026-09-29

First alpha of the 0.5 line. CLAP plugin hosting, a standalone MCP
server, and the xfade preview drift fix that had been outstanding
since 0.4.

### Added
- **CLAP plugin hosting (Faza I).** Non-recursive scan of the two
  standard CLAP directories (`C:\Program Files\Common Files\CLAP`
  and `%APPDATA%\CapRust\plugins`), feature-gated behind `clap`
  so Linux CI without the CLAP SDK still builds. Plugin binaries
  are untrusted code; every load is wrapped in `catch_unwind`.
- **Master plugin chain.** `ProjectState.master_plugins:
  Vec<PluginInstance>`. A new Plugins tab in the asset browser
  lists discovered plugins; a `+` on any card adds it to the
  chain. A new Master dock tab shows the active chain with Bypass
  and Remove per instance. All mutations go through the undo
  stack (`AddMasterPluginCommand`, `RemoveMasterPluginCommand`,
  `SetPluginParamCommand`). `render_hash` includes the chain, so
  any change respawns the preview.
- **CLAP audio processing.** The master chain runs on the PCM
  reader thread between the ffmpeg decode and the cpal ring
  buffer. Interleaved stereo is deinterleaved to planar, processed
  by each plugin in turn, then reinterleaved. Verified against
  u-he ZebraHZ 2.9.4 on Windows: the plugin loads, activates,
  and processes a test sine without NaN.
- **MCP server (Faza N).** Standalone `caprust-mcp` binary crate.
  Line-delimited JSON-RPC 2.0 on stdio; loads a `.caprust`
  project file and exposes read-only inspection plus mutating
  tools. 15 tools total: `get_project_summary`, `list_tracks`,
  `list_clips`, `list_media`, `add_media`,
  `add_clip_to_timeline`, `move_clip`, `split_clip`,
  `set_transition`, `set_clip_volume`, `set_clip_speed`,
  `set_clip_fade`, `undo`, `redo`, `save_project`. Every mutating
  tool goes through `UndoStack`, same as the UI. No tokio, no
  rmcp - 200-line hand-rolled dispatch.
- **`AddMediaCommand`** in core (was missing for the MCP
  `add_media` tool to satisfy DIRECTIVES 6).
- **`docs/mcp.md`** with a Claude Desktop config template, tool
  reference, and known limits.

### Fixed
- **Xfade audio drift in preview.** The audio player started on
  the Play click, but the playhead anchors only on the first
  consumed video frame. A project without any transition
  delivered that frame in ~370 ms, so audio and playhead stayed
  in step. A project with an xfade or slide forces ffmpeg to
  decode both clips from their beginning to build the blend;
  the first frame arrived 2.2-2.4 s later. By then audio was
  already 2.2-2.4 s ahead of the playhead. Fix: audio is now
  deferred until the first frame is consumed. Audio and playhead
  both start from the same position, so `audio_baseline` is near
  zero regardless of transition complexity.
- **`SetTransitionCommand` clamp.** The model shifted followers
  by the raw `transition_duration_ms`; the render clamps to half
  of the shorter clip. For a 3000 ms slide on a 4 s clip the
  model shifted 3000 ms while the render shortened by 2000 ms,
  adding a 1000 ms gap after the transition. The model now uses
  the same clamp as the render.
- **`commands.rs` module ordering.** Doc comment / `pub mod` order
  was fragile; the module list is now always below the doc block.

### Changed
- **`crates/mcp/Cargo.toml`** switched from a hardcoded `version`
  to `version.workspace = true`, so a release bump touches one
  place.

### Tests
- 241 passing, 8 skipped (6 existing + 2 new CLAP tests requiring
  ZebraHZ at a fixed path). New since 0.4: master plugin model +
  commands, CLAP scanner + chain, MCP rpc/server/tools,
  `AddMediaCommand`, xfade clamp, audio shift guards.

## [0.4.0-alpha.2] - 2026-09-29

Second alpha of the 0.4 line. Transitions on the timeline, batch
undo, and UX polish.

### Added
- **Transition easing.** Clips carry `transition_in_easing` and
  `transition_out_easing` (both `EaseCurve`, default Linear). The
  clip properties panel shows an Easing dropdown under any
  transition that is present. The render currently emits plain
  linear fades because the gyan.dev essentials build we test
  against (2026-01-26) rejects `fade=curve=` with "Option not
  found". The field is stored and preserved for the day we pin a
  newer ffmpeg.
- **Configurable transition duration.** New
  `Clip.transition_duration_ms` (default 500) with a 100..3000 ms
  slider in the clip properties panel. Applies to both in and out
  edges of a clip.
- **Xfade shifts followers.** Attaching an xfade between two
  adjacent clips on the same track now moves the follower clip and
  every clip after it on that track left by the transition
  duration. Removing the transition shifts them back. Changing the
  duration shifts by the delta. All in one command so Ctrl+Z
  restores the whole batch. Applied in `SetTransitionCommand` and
  `SetTransitionDurationCommand`.
- **Timeline overlap shading.** Clips that carry an xfade on their
  in-edge show an accent-tinted slice with thin diagonal hatching
  over the overlap region. The crossfade footprint is visible at a
  glance.
- **Fade-out on the last clip of an xfade run.** The earlier
  clips' ends are consumed by the next clip's xfade, but the tail
  of the run can now fade back to black when its clip has
  `transition_out == Some("fade")`.
- **MacroCommand.** Runs a list of commands as a single undoable
  step, with rollback on mid-batch failure. Used by the media bin
  batch drop so a 4-clip drop is one Ctrl+Z.
- **Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z** keyboard shortcuts for
  undo/redo. Previously only reachable through the timeline
  toolbar icons.
- **App icon** embedded in the .exe via winres, runtime window
  icon, README header logo.
- **Material-style egui_dock theme.** Tab bar matches the panel
  fill, 30px tall, active tab uses an accent wash. Close button
  subtle, brightens on hover.
- **Brighter toasts.** Accent border, top highlight line, and a
  fill lift so notifications read as elevated.

### Fixed
- **Media bin multi-select and long-press drag.** Ctrl+click
  toggles membership, plain click replaces. Holding the primary
  button for 500 ms on a card starts a drag; a shorter click only
  selects. The X button on a multi-selection removes every
  selected item at once.
- **Batch drag to timeline.** Dropping a multi-selection chains
  the clips left-to-right at the drop position and selects the
  result so a follow-up move or delete acts on the whole batch.
- **Group move on the timeline.** Ctrl+click or marquee selects
  multiple clips; dragging one of them moves the entire group by
  the same time and track delta in a single undoable step.
- **Captions and text overlays seek-shift correctly.** Their
  `timeline_start_sec` now follows the playhead seek, matching
  video and audio. Previously a seek into the middle of a long
  project left their enable windows on the original timeline.
- **Apostrophes in caption text no longer break the preview.**
  ASCII apostrophe and double quote substitute to U+2019 / U+201C
  at render time; both are safe inside a filtergraph single-quoted
  value.
- **Media bin X button clickable on hover.** The enclosing drag
  source claimed the pointer, so the button vanished mid-click.
  Rewritten to use the raw pointer position for visibility and the
  raw primary-click edge for the action.
- **Total render duration accounts for xfade shortening.** A
  3-second crossfade used to leave a 3-second black tail at the
  end of the render because the base and audio bed were sized to
  the unshortened sum.
- **Overlapping clip detection.** Run grouping now accepts a
  positive overlap equal to the transition duration, not just
  butt-joined clips.
- **Dragging a clip that carries an xfade now removes the
  transition** with a toast, restoring every follower to its
  original position. The user re-adds after the move.

### Changed
- `SetTransitionCommand` rewritten: the shift is applied to the
  follower clip and every clip after it on the same track.
- `Clip` gained `applied_xfade_shift_ms` to track the shift the
  transition command applied.
- `move_many::MoveManyCommand` and `macro_command::MacroCommand`
  new core modules.
- `VideoEncoder` enum (H264/H265/AV1 x CPU/NVENC/AMF) added to
  `ExportSettings` in the previous alpha; unchanged here.

### Known issues
- No installer yet; download the nightly ZIP and extract it.
- FFmpeg is not bundled. First run offers to download a BtbN build
  or to point at an existing install.
- Windows only.
- Transition easing stored but not rendered on the current ffmpeg
  build. Needs a build with `fade=curve=` support.
- Text overlay rotation is stored in the model but neither
  rendered nor exposed in the UI.
- Preview A/V sync has a residual 100-300 ms lead/lag on some
  hardware. Tunable via `CAPRUST_AV_DELAY_MS`.

## [0.4.0-alpha.1] - 2026-09-28

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

## [0.3.0-alpha.1] - 2026-09-28

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

## [0.2.0-alpha.1] - 2026-09-29

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

## [0.1.0-alpha.1] - 2026-09-27

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
