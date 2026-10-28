# Building from source

## Prerequisites

- Rust 1.91+ (see `rust-toolchain.toml`)
- FFmpeg 5.1+ in PATH
- Visual Studio Build Tools (Windows) / GTK+X11+ALSA (Linux)
- For Android: Android NDK, Gradle

## Build desktop

```bash
git clone https://github.com/Domica/CapRust
cd CapRust
cargo build --release -p caprust-app --no-default-features
./target/release/caprust-app
```

FFmpeg is called as a subprocess, never linked.

## Build Android APK

```bash
# Requires Android NDK and Gradle
cd android
gradle --no-daemon assembleDebug
```

## Testing

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Architecture

- `crates/core` — project model, commands, settings, media
- `crates/ui` — egui UI, panels, widgets
- `crates/media-io` — ffmpeg wrapper, export, preview
- `crates/android` — phone engine + daemon
- `crates/mcp` — MCP server (JSON-RPC)
- `crates/i18n` — translations

See [full_readme.md](https://github.com/Domica/CapRust/blob/main/full_readme.md) for full architecture.
EOF