Release 0.9.30: Phase 1 AI subsystem stabilization — ONNX loader unification, model cache, cooperative cancellation.

### What's new

- **AI cancellation (1.5):** Jobs bar shows a ✕ cancel button for background-removal, object-removal, and face-anonymize jobs. Cancel is cooperative — the worker thread checks a `CancellationToken` before each frame and emits a `Cancelled` event so the UI can stop cleanly without killing the process.
- **AI model cache (1.6):** Loaded ONNX models are cached across jobs by path + file mtime. Re-running background removal, segmentation, inpainting, or anonymize on the same model avoids re-parsing and re-optimizing the graph each time.
- **Shared ONNX loaders (1.1, 1.2):** All five model loaders (YuNet, SCRFD, LaMa, MobileSAM, u2netp) now share `onnx_common` helpers — `load_typed_pinned`, `load_typed`, `load_inference` — removing duplicated boilerplate while keeping the two tract-onnx type families intentional.
- **Loud decoder failure (1.3):** YuNet 2022mar 3-output layout now fails loudly instead of silently returning empty boxes, so auto-reframe errors surface rather than degrading invisibly.

### Under the hood

- `ModelSlot<T>` static cache with `Send + Sync` compile-time proof for all five model types.
- `CancellationToken` (Relaxed `AtomicBool`) wired into `BgRemovalRequest`, `SegmentRequest`, `InpaintRequest`, `AnonRequest` and all six app.rs spawn call sites.
- `Cancelled` variants added to all four event enums; app.rs drain functions handle them and clear state/toast accordingly.
- `CancellationToken`/`cancel` already existed in all five locales (en, hr, de, es, it); no `.ftl` edits needed for the UI itself — only new `toast-*-cancelled` keys were added.

### Version

Bumped from `0.9.29` to `0.9.30` (minor, user-facing cancellation feature).
