#!/usr/bin/env python3
"""
Round 5: Fix remaining clippy errors in test code and needless-borrow warnings.

caprust-android/src/lib.rs:
  &self.project.tracks() -> self.project.tracks()

caprust-media-io/src/export_graph.rs (test code):
  .tracks -> .tracks() (field access, not already .tracks())
  p.media.items.push(m) -> p.media_mut().items.push(m)
  p.tracks.push(...) -> p.tracks_mut().push(...)
  p.tracks.clear() -> p.tracks_mut().clear()

caprust-ui/src/app.rs (test code):
  p.tracks().clear() -> p.tracks_mut().clear()

caprust-ui/src/timeline/mod.rs (test code):
  p.tracks[5].muted = true -> p.tracks_mut()[5].muted = true
  p.tracks[3].muted = true -> p.tracks_mut()[3].muted = true

caprust-ui/src/app.rs (needless borrow):
  &self.project.tracks() -> self.project.tracks()
  &state.media() -> state.media()
  &mut self.project.models_mut() -> self.project.models_mut()
  &self.project.models() -> self.project.models()
"""

import re
from pathlib import Path

BASE = Path(r"C:\Users\domag\CapRust")

# (file, pattern, replacement)
FIXES = [
    # android needless borrow
    ("crates/android/src/lib.rs",
     r"&self\.project\.tracks\(\)",
     "self.project.tracks()"),

    # media-io export_graph.rs - .tracks -> .tracks() (only when NOT already .tracks() or .tracks_mut())
    ("crates/media-io/src/export_graph.rs",
     r"\.tracks(?![_()])",
     ".tracks()"),

    # media-io export_graph.rs - p.media.items.push(m) -> p.media_mut().items.push(m)
    ("crates/media-io/src/export_graph.rs",
     r"p\.media\.items\.push\(",
     "p.media_mut().items.push("),

    # media-io export_graph.rs - p.tracks.push( -> p.tracks_mut().push(
    ("crates/media-io/src/export_graph.rs",
     r"p\.tracks\.push\(",
     "p.tracks_mut().push("),

    # media-io export_graph.rs - p.tracks.clear() -> p.tracks_mut().clear()
    ("crates/media-io/src/export_graph.rs",
     r"p\.tracks\.clear\(\)",
     "p.tracks_mut().clear()"),

    # ui app.rs test code - p.tracks().clear() -> p.tracks_mut().clear()
    ("crates/ui/src/app.rs",
     r"p\.tracks\(\)\.clear\(\)",
     "p.tracks_mut().clear()"),

    # ui timeline/mod.rs test code - p.tracks[N].muted = true -> p.tracks_mut()[N].muted = true
    ("crates/ui/src/timeline/mod.rs",
     r"p\.tracks\[5\]\.muted",
     "p.tracks_mut()[5].muted"),
    ("crates/ui/src/timeline/mod.rs",
     r"p\.tracks\[3\]\.muted",
     "p.tracks_mut()[3].muted"),

    # ui app.rs needless borrow - &self.project.tracks() -> self.project.tracks()
    ("crates/ui/src/app.rs",
     r"&self\.project\.tracks\(\)",
     "self.project.tracks()"),

    # ui app.rs needless borrow - &state.media() -> state.media()
    ("crates/ui/src/app.rs",
     r"&state\.media\(\)",
     "state.media()"),

    # ui app.rs needless borrow - &mut self.project.models_mut() -> self.project.models_mut()
    ("crates/ui/src/app.rs",
     r"&mut self\.project\.models_mut\(\)",
     "self.project.models_mut()"),

    # ui app.rs needless borrow - &self.project.models() -> self.project.models()
    ("crates/ui/src/app.rs",
     r"&self\.project\.models\(\)",
     "self.project.models()"),
]

def fix_file(path: Path, pattern: str, replacement: str) -> int:
    text = path.read_text(encoding="utf-8")
    original = text
    text = re.sub(pattern, replacement, text)
    if text != original:
        path.write_text(text, encoding="utf-8")
        return 1
    return 0

def main():
    total = 0
    for rel, pattern, replacement in FIXES:
        p = BASE / rel
        if not p.exists():
            print(f"SKIP (not found): {rel}")
            continue
        changed = fix_file(p, pattern, replacement)
        print(f"{'CHANGED' if changed else 'unchanged'}: {rel} ({pattern[:50]}...)")
        total += changed
    print(f"\n{total} file(s) modified.")

if __name__ == "__main__":
    main()
