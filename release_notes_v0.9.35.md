## CapRust 0.9.35

### AI inference backends with CUDA / DirectML
- MobileSAM (object removal) and LaMa (inpainting) are now ported to the unified `InferenceSession` abstraction with opt-in CUDA and DirectML support.
- Face detection and background removal already run on `InferenceSession`; face restore (GFPGAN) now reports the active inference backend in the Properties panel and via toast notifications.
- Three Windows variants are released: **standard** (CPU, ~27 MB), **CUDA** (NVIDIA, ~50 MB), and **DirectML** (AMD, ~50 MB). Each is a separate portable EXE, installer, and ZIP.

### Release artifacts
- `caprust-app.exe`, `caprust-app-cuda.exe`, `caprust-app-directml.exe` — portable EXEs
- `CapRust-0.9.35-win64-setup.exe` / `-cuda` / `-directml` — Inno Setup installers
- `caprust-0.9.35-windows` / `-cuda` / `-directml` — portable ZIPs
- `caprust_0.9.35_amd64.deb` — Debian/Ubuntu package
- `caprust-0.9.35.apk` — Android APK

### Other fixes
- CI now builds each Windows variant into its own `--target-dir`, so CUDA/DirectML variants actually include GPU support (previously all three were identical CPU-only builds).
- `cargo fmt --all -- --check`, `cargo clippy -- -D warnings`, and `cargo check` all pass on the full workspace.
