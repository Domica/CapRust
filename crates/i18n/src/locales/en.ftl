# App
app-title = CapRust - Social Video Editor
app-tagline = Social-first video editor

# File menu
menu-file = File
menu-file-new = New Project…
menu-file-open = Open Project…
menu-file-save = Save Project
menu-file-save-as = Save As…
menu-file-import = Import Media…
menu-file-export = Export…
menu-file-clear-cache = Clear Project Cache
menu-file-regen-thumbs = Regenerate Thumbnails
menu-file-settings = Settings…
menu-file-close = Close Project
menu-file-quit = Quit

# Edit menu
menu-edit = Edit
menu-edit-undo = Undo
menu-edit-redo = Redo
menu-edit-split = Split at Playhead
menu-edit-delete = Delete Clip
menu-edit-ripple-delete = Ripple Delete

# View menu
menu-view = View
menu-view-sort = Sort Media by
menu-view-size = Preview Size
menu-view-zoom-in = Zoom In
menu-view-zoom-out = Zoom Out

# Right-click clip menu
clip-ctx-delete = Delete
clip-ctx-split = Split at playhead
clip-ctx-reverse = Reverse
clip-ctx-mirror-h = Mirror horizontally
clip-ctx-mirror-v = Mirror vertically

# New project screen
new-title = New Project
new-field-name = Name
new-field-location = Location
new-field-format = Format
new-field-resolution = Base resolution
new-field-fps = Frame rate
new-button-browse = Browse…
new-button-create = Create Project
new-button-quit = Quit
new-recent-heading = Recent Projects
new-recent-empty = No recent projects yet.
new-recent-open = Open
new-recent-forget = Forget
new-recent-delete = Delete

# Media bin
asset-tab-media = Media
asset-tab-transitions = Transitions
asset-tab-effects = Effects
asset-tab-filters = Filters
asset-tab-text = Text
asset-tab-templates = Templates
asset-search = Search…
asset-empty = No items here yet.
asset-templates-hint = Saved clip templates will appear here.
media-heading = Media Library
media-import-clips = 📥 Clips
media-import-music = 🎵 Music
media-import-images = 🖼 Images
media-sort-label = Sort:
media-filter-label = Show
media-size-label = Size
media-empty-title = No media imported yet.
media-remove-one = Remove from library
media-clear-all = 🗑 Clear all
media-clear-all-tooltip = Remove all media from the library (files on disk are kept)
media-regen-thumbs = Regenerate thumbnails
media-empty-hint = Click Clips / Music / Images above.
media-count = { $count } item(s) in library
media-sort-added = Added
media-sort-name = Name
media-sort-type = Type
media-sort-dir-asc = Sort ascending
media-sort-dir-desc = Sort descending
media-filter-all = All
media-filter-video = Video
media-filter-audio = Audio
media-filter-image = Image
media-size-small = S
media-size-medium = M
media-size-large = L

# Timeline toolbar tooltips
tt-add-track = Add track
tt-pan = Pan tool (drag to scroll)
tt-magnetic = Magnetic timeline
tt-snap = Snap to clips
tt-follow = Follow playhead
tt-captions = Generate captions
tt-narration = Generate narration (TTS)
tt-transcribing = Transcribing…
tt-synthesizing = Synthesizing…
toast-caption-started = Generation of subtitles has started.
toast-caption-no-selection = Select a clip on the timeline first.
toast-caption-no-audio = Selected clip has no audio to transcribe.
toast-caption-multi-first = Multiple clips selected — using the first.
clip-ctx-generate-captions = Generate captions for this clip
toast-caption-added = Captions added
toast-caption-empty = No speech detected in the selected clip.
toast-caption-failed = Transcription failed: { $err }
toast-narration-started = Generating narration audio...
toast-narration-added = Narration clip added
toast-narration-failed = Synthesis failed: { $err }
tt-zoom-out = Zoom out
tt-zoom-in = Zoom in
tt-zoom-fit = Zoom to fit
tt-undo = Undo
tt-redo = Redo

# Track header tooltips
tk-lock = Lock
tk-view = Show in preview
tk-mute = Mute
tk-delete = Delete track

# Preview
preview-heading = Preview
preview-quality = Quality
preview-play = Play
preview-pause = Pause
preview-back-30 = Back 30 s
preview-back-5 = Back 5 s
preview-fwd-5 = Forward 5 s
preview-fwd-30 = Forward 30 s
preview-loop = Loop
preview-mute = Mute
preview-unmute = Unmute
preview-volume = Volume

# Properties panel
props-heading = Properties
props-empty = Select a clip to edit its properties.
props-tab-video = Video
props-tab-sound = Sound
props-tab-effects = Effects
props-video-main = Main
props-video-speed = Speed
props-video-mirror = Mirror
props-video-trim = Trim
props-field-rotation = Rotation
props-field-width = Width
props-field-height = Height
props-field-speed = Speed
props-field-custom = Custom
props-field-reverse = Reverse
props-mirror-h = Mirror horizontally
props-mirror-v = Mirror vertically
props-field-start = Start
props-field-duration = Duration
props-field-source = Source
props-value-unlimited = unlimited
props-trim-hint = Drag the handles on the clip's edges in the timeline.
props-sound-volume = Volume
props-sound-fade = Fade
props-sound-fade-in = Fade in
props-sound-fade-out = Fade out
props-sound-processing = Processing
props-sound-normalize = Normalize sound
props-sound-denoise = Decrease noise
props-sound-boost = Voice volume increase
props-sound-wip = (Wired in audio-engine PR.)
props-effects-empty = No effect selected.
props-effects-hint = Pick a filter, transition, or audio effect from the left panel to attach it to this clip.

# Export window
exp-title = ⬆  Export video
exp-summary = Summary
exp-duration = Duration
exp-destination = Destination
exp-browse = Browse…
exp-output = Output
exp-resolution = Resolution
exp-fps = Frame rate
exp-codec = Codec
exp-quality = Quality
exp-quality-small = Small
exp-quality-regular = Regular
exp-quality-large = Large
exp-advanced = Advanced options
exp-bitrate = Bitrate mode
exp-color-range = Color range
exp-button = ⬇  Export

# Settings dialog
set-title = Settings
set-tab-appearance = Appearance
set-tab-models = AI Models
set-tab-shortcuts = Shortcuts
set-tab-language = Language
set-tab-paths = Paths
set-appearance-mode = Mode:
set-appearance-accent = Accent:
set-appearance-tracks = Track colors
set-appearance-tracks-hint = Header and lane tint for each track family.
set-appearance-track-video = Video / Overlay
set-appearance-track-audio = Audio
set-appearance-track-captions = Captions
set-appearance-track-text = Text
set-appearance-custom = Custom colors
set-appearance-panel = Panel background
set-appearance-window = Window background
set-appearance-text = Text color
set-appearance-reset = Reset to defaults
set-models-heading = AI Models
set-models-captions = Captions (speech-to-text)
set-models-narration = Narration (text-to-speech)
set-models-download = ⬇ Download
set-models-enabled = Enabled
set-shortcuts-heading = Keyboard shortcuts
set-shortcuts-enable = Enable keyboard shortcuts
set-language-heading = Interface language
set-language-applied = Applied immediately.
set-paths-models = AI models folder
set-paths-ffmpeg = FFmpeg binaries
set-paths-ffmpeg-hint = Used for media probing, thumbnail extraction, and export. Leave empty to auto-detect from PATH.
set-paths-detect = 🔍 Detect now
set-paths-detected = ✅ ffmpeg + ffprobe detected
set-paths-partial = ⚠ One binary missing
set-paths-not-detected = ❌ Not detected
set-save = 💾 Save
set-cancel = Cancel
set-save-hint = Changes save to storage on Save.

# Model prompt dialog
mp-captions-title = 💬  Captions — choose a model
mp-narration-title = 🎙  Narration — choose a voice
mp-intro = This action needs a model. Download one, then click Use.
mp-use-this = ✓ Use this
mp-cancel = Cancel
mp-hint = Downloads go to Settings → Paths → AI models folder.

# Generic
ok = OK
cancel = Cancel
close = Close
yes = Yes
no = No

set-tab-audio = Audio
set-audio-hint = Controls how the preview plays back through your speakers. Export volume is set per-clip on the timeline.
set-audio-volume = Volume
set-audio-mute = Mute
set-audio-unmute = Unmute
set-audio-preview-only = These settings affect preview only, not export.

# Update checker
update-toast-title = Update available
update-toast-download = Download
update-toast-later = Remind me later
update-toast-skip = Skip this version
set-appearance-updates = Updates
set-appearance-check-updates = Check for updates on startup
set-appearance-updates-hint = Silent check, once per day. You can skip or snooze a specific version.

# Asset browser — coming-soon marker
asset-coming-soon = Coming soon

# Transitions
asset-transition-none = None
asset-transition-fade = Fade
asset-transition-slide_l = Slide Left
asset-transition-slide_r = Slide Right
asset-transition-slide_u = Slide Up
asset-transition-slide_d = Slide Down
asset-transition-zoom_in = Zoom In
asset-transition-zoom_out = Zoom Out
asset-transition-wipe_l = Wipe Left
asset-transition-wipe_r = Wipe Right
asset-transition-rotate = Rotate
asset-transition-blur_t = Blur Cut

# Effects
asset-effect-blur = Blur
asset-effect-vignette = Vignette
asset-effect-glitch = Glitch
asset-effect-rgb_split = RGB Split
asset-effect-shake = Shake
asset-effect-zoom_pulse = Zoom Pulse
asset-effect-flash = Flash
asset-effect-mirror = Mirror
asset-effect-kaleido = Kaleidoscope
asset-effect-old_film = Old Film
asset-effect-vhs = VHS
asset-effect-light_leak = Light Leak
asset-effect-particle = Particle
asset-effect-sparkle = Sparkle
asset-effect-ghost = Ghost
asset-effect-lens_flare = Lens Flare

# Filters
asset-filter-none = None
asset-filter-warm = Warm
asset-filter-cool = Cool
asset-filter-bw = B&W
asset-filter-sepia = Sepia
asset-filter-cinematic = Cinematic
asset-filter-vintage = Vintage
asset-filter-vivid = Vivid
asset-filter-matte = Matte
asset-filter-noir = Noir
asset-filter-sunset = Sunset
asset-filter-ocean = Ocean
asset-filter-fade = Fade
asset-filter-pastel = Pastel
asset-filter-neon = Neon
asset-filter-gold = Golden

# Text styles
asset-text-default = Default
asset-text-bold = Bold Title
asset-text-subtitle = Subtitle
asset-text-lower = Lower Third
asset-text-quote = Quote
asset-text-caption = Caption
asset-text-glow = Neon
asset-text-handwrite = Handwritten
