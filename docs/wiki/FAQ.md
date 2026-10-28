# FAQ

## Does CapRust have watermarks?
No. MIT licensed, no watermarks, no payments, no telemetry, no account required.

## Does it work offline?
Yes. All AI runs on your machine. Cloud sync is just a folder.

## What platforms are supported?
Windows 10/11 (64-bit), Linux (Debian/Ubuntu), Android (alpha).

## Why is the binary 26 MB?
The AI runtime (face detection + background removal) is 7.8 MB. Getting under 20 MB means dropping offline AI. See size-audit.yml.

## How do I translate CapRust?
See [TRANSLATING.md](https://github.com/Domica/CapRust/blob/main/TRANSLATING.md). Copy `en.ftl`, translate values, open a PR.

## Where are settings stored?
Windows: `%APPDATA%\CapRust\`. Linux: `~/.config/CapRust/` (or XDG). Android: app's data directory.

## How do I report a bug?
Open an issue on [GitHub](https://github.com/Domica/CapRust/issues) with steps to reproduce.
EOF