# Installation

## Windows

1. Download `CapRust-0.9.23-win64-setup.exe` from [Releases](https://github.com/Domica/CapRust/releases/latest).
2. Run the installer (per-user by default, no UAC prompt).
3. Launch CapRust from the Start Menu.

## Linux (Debian/Ubuntu)

1. Download `caprust_0.9.23_amd64.deb` from [Releases](https://github.com/Domica/CapRust/releases/latest).
2. `sudo dpkg -i caprust_0.9.23_amd64.deb`
3. Launch from applications menu or run `caprust`.

## Android (Alpha)

1. Download `CapRust-0.9.23-android.apk` from [Releases](https://github.com/Domica/CapRust/releases/latest).
2. Install the APK (allow unknown sources if prompted).
3. Open CapRust on your phone.

## FFmpeg

CapRust requires FFmpeg 5.1+. On first run, CapRust offers to download a static build (~110 MB) if neither `ffmpeg` nor `ffprobe` is on your PATH. On Linux, the download picks the Linux build (`.tar.xz`) automatically.

## AI models

AI models (Whisper, Piper, ONNX) download on demand from **Settings → AI Models**. The settings page has a Download button for `u2netp.onnx` (used for auto-reframe, background removal, face detection).
EOF