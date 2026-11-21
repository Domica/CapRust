Release 0.9.34: Real GFPGAN v1.4 face restoration.

### Face Restore is now real GFPGAN

Face Restore is no longer a smartblur approximation. Enabling it on a clip in the **Video** tab now runs a genuine AI pre-pass:

1. **SCRFD-500M** detects faces and returns five facial landmarks.
2. Each face is aligned to the canonical 512x512 FFHQ frame with a least-squares similarity transform.
3. **GFPGAN v1.4 ONNX** runs through tract-onnx to restore facial detail while preserving identity, expression and mouth motion.
4. The restored face is inverse-warped back into the original frame with a feathered mask.
5. All frames are written to a lossless FFV1 cache at `<project>/cache/gfpgan/<clip_id>.mkv`.

The cache becomes the clip's input for **both preview and export** — inference runs once per clip, not on every preview frame. When a restored cache exists, the old smartblur + hqdn3d fallback is skipped so the AI result is not blurred again. Clearing the cached result falls back to the smoothing filter.

**Requirements:** enable both the `GFPGAN v1.4 Face Restore` and `SCRFD-500M face detector` models in Settings → AI Models. The job needs ffmpeg, works on video and image clips, and requires normal playback speed without reverse. Progress and cancellation are available in the jobs bar.

### Fixes

- **Corrected the GFPGAN ONNX contract.** The inference module was feeding raw 0..255 values into a model that expects `(v - 127.5) / 127.5`; outputs were garbage of magnitude 1e17. The model also emits BGR and needed a channel swap to RGB. Both were verified against the real model and validated visually on a face photo.
- **SCRFD now decodes its five facial keypoints** instead of discarding the `kps` output, which is required for alignment.
- **Restored clips skip the smartblur fallback**, guarded by a regression test.
- **Stale cache invalidation.** Changing a clip's duration, speed or direction clears its face-restore result so an outdated restored video can never be reused.

### Downloads

| Platform | Artifact |
|----------|----------|
| Windows | `CapRust-0.9.34-win64-setup.exe` (Inno Setup installer) |
| Windows | `caprust-0.9.34-windows.zip` (portable) |
| Linux (Debian/Ubuntu) | `caprust_0.9.34_amd64.deb` |
| Android | `caprust-0.9.34.apk` (debug build, sideloadable) |

### Installation

- **Windows**: Run the `.exe` installer, or extract the portable `.zip`
- **Linux**: `sudo dpkg -i caprust_0.9.34_amd64.deb`
- **Android**: Sideload the `.apk`
