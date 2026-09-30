---
name: Bug report
about: Something is broken or crashes
title: "[Bug] "
labels: bug
---

## What happened

A clear, short description of what went wrong.

## Steps to reproduce

1.
2.
3.

## Expected behaviour

What you expected instead.

## Environment

- **CapRust version:** (see Help → About or the title bar)
- **Windows version:** (e.g. Windows 11 24H2)
- **FFmpeg source:** PATH / managed auto-download / manual override
  (Settings → Paths shows the resolved path)

## Media

If the bug is about a specific file:
- Container/codec (mp4/h264, mov/prores, ...)
- Approximate duration and resolution

## Logs

Run from a terminal with logging enabled and paste the last ~50 lines:
$env:RUST_LOG = "caprust=debug,caprust_media_io=debug,info"
.\caprust-app.exe 2>&1 | Tee-Object -FilePath caprust_debug.log

text

Then attach `caprust_debug.log`.

## Screenshots / video

If relevant, drop a screenshot or a short screen recording.
