# AI Feature Status for CapRust v0.9.23

## Working (alpha)
- Android standalone editor (import, preview, export, timeline drag)
- Basic keyframe curve editor (#84)
- Settings AI model download (u2netp.onnx)

## Requires mask file — not auto-generated in 0.9.23
- **Anonymize (face blur)**: Needs `cache/anon/<clip>.mkv` mask from detection pass.
- **Background removal**: Needs `cache/bg_removal/<clip>.mkv` mask from segmentation.
- **Auto-reframe**: Needs u2netp model probed at runtime.

If mask is missing, the filtergraph skips the merge or produces incorrect output (B/W or black).
Do not use for export until mask pipeline generates automatically.

## Next
- #84 Full (easing curves, per-property tracks)
- #81 Google Photos watch
- #78 SignPath code signing
- #39 Real-ESRGAN AI upscale (deferred)
