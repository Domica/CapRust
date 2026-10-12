# Release 0.9.16 — 2026-10-12

## Fixed

- **Custom autosave interval spinner persisted keystrokes.** The value was re-created from the stored interval every frame; a thread-local draft now holds it until **Postavi** is clicked.
- **Preset clicks produced no visible confirmation.** Now any selection (preset or custom) shows the green "Postavljeno na svakih N min" line.
- **Custom row was unreachable from the dropdown.** It is now always listed below the presets and selecting it enables the spinner immediately.

## Assets

- `caprust-0.9.16-windows.zip` — portable binary (9.3 MB)

## SHA-256

```
caprust-0.9.16-windows.zip: <to be filled after upload>
```

## Full Changelog

See [CHANGELOG.md](../CHANGELOG.md).