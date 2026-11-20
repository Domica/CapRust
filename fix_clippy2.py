#!/usr/bin/env python3
"""
Round 6: Fix remaining clippy errors.

media-io export_graph.rs (test code):
  p.tracks().push( -> p.tracks_mut().push(
  p.tracks().clear() -> p.tracks_mut().clear()

ui app.rs:
  let all = self.project.models().models -> let all = &self.project.models().models
  local_path_static(self.project.models().models, ...) -> local_path_static(&self.project.models().models, ...)
  p.tracks().last_mut() -> p.tracks_mut().last_mut()

ui timeline/mod.rs (test code):
  p.media().items (mutable) -> p.media_mut().items
"""

import re
from pathlib import Path

BASE = Path(r"C:\Users\domag\CapRust")

FIXES = [
    # media-io: p.tracks().push( -> p.tracks_mut().push(
    ("crates/media-io/src/export_graph.rs",
     r"p\.tracks\(\)\.push\(",
     "p.tracks_mut().push("),
    # media-io: p.tracks().clear() -> p.tracks_mut().clear()
    ("crates/media-io/src/export_graph.rs",
     r"p\.tracks\(\)\.clear\(\)",
     "p.tracks_mut().clear()"),

    # ui app.rs: let all = self.project.models().models -> &self.project.models().models
    ("crates/ui/src/app.rs",
     r"let all = self\.project\.models\(\)\.models;",
     "let all = &self.project.models().models;"),
    # ui app.rs: local_path_static(self.project.models().models, ...) -> &self.project.models().models
    ("crates/ui/src/app.rs",
     r"local_path_static\(\s*self\.project\.models\(\)\.models,",
     "local_path_static(&self.project.models().models,"),
    # ui app.rs: p.tracks().last_mut() -> p.tracks_mut().last_mut()
    ("crates/ui/src/app.rs",
     r"p\.tracks\(\)\.last_mut\(\)",
     "p.tracks_mut().last_mut()"),

    # ui timeline/mod.rs: p.media().items (mutable access needed) -> p.media_mut().items
    ("crates/ui/src/timeline/mod.rs",
     r"p\.media\(\)\.items",
     "p.media_mut().items"),
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
