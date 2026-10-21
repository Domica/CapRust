# The Android app

CapRust on a phone: a real editor, not a remote control.

## What is decided

- **The phone runs the engine itself.** The desktop's crates
  cross-compile for `aarch64-linux-android`: the probe workflow has
  been proving it since October, and it builds `caprust-core`,
  `caprust-media-io` (with whisper.cpp) and now `caprust-android`.
- **The engine is the desktop's engine.** The same crates, the same
  render graph, the same effects. What the phone shows and what it
  exports are the same thing, because they come out of the same code.
- **The bridge is a socket, not JNI.** The phone app starts
  `caprust-androidd` and speaks the same JSON-RPC the companion
  already speaks against the desktop, so the Kotlin side has one
  client and not two. It also means a renderer that dies takes the
  daemon down, not the app.

## What is built

| | |
|---|---|
| `crates/android` | the engine: a project, import, the timeline, a preview frame, an export |
| `crates/android/src/daemon.rs` | the bridge: the MCP tools over a loopback socket |
| `crates/android/tests` | the engine's questions answered, with the real ffmpeg |

## What is left

1. **The Kotlin app.** A screen with a media bin, a timeline, a
   preview and an export button. The companion's `Client` class is
   reused as it is: the protocol is the same.
2. **The APK.** The daemon binary goes into the APK's
   `lib/arm64-v8a/` folder -- the package manager sets the exec bit
   there, and that is the only place an Android app may run a binary
   from -- and the app starts it and reads the port and the token off
   its stdout.
3. **ffmpeg on the phone.** See below.
4. **The AI on the phone.** Not the desktop's models. Captions work
   (one pass over the audio), face detection works (a one-shot job),
   background removal does not: it is per frame, and a phone's CPU is
   five to ten times slower than a desktop's. The phone uses the
   platform's own tools -- MediaPipe for the mask -- as a pre-pass,
   the way the desktop caches a stabilisation mask. The render graph
   does not change, so the preview is the export.

## ffmpeg on Android

The engine renders by calling ffmpeg. Android 10 and later do not let
an app run a binary out of its own data directory, so ffmpeg has to
be in the APK.

Shipping the ffmpeg binary next to the app is not linking it: the
engine runs it as a separate program, which is the arrangement the
desktop has today, where the GPL FFmpeg build is downloaded at runtime
and CapRust itself stays MIT. The APK carries the ffmpeg licence and
a link to its source.

Three ways, and what each costs:

| | |
|---|---|
| A prebuilt Android ffmpeg in the APK | fastest, about +30 MB, and the licence of that build has to be checked |
| ffmpeg built for Android in CI | no third-party binary in the release, and 20 to 40 minutes in CI |
| The desktop renders | the phone is an editor and not a renderer, so the export needs a desktop |

The first is the way, and the second is the way to be sure of it.
