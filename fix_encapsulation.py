#!/usr/bin/env python3
"""
Fix E0616 errors: replace direct field access (pub->pub(crate)) with
accessor method calls on ProjectState fields media, models, tracks.

Replacement rules (applied in order, most specific first):

models:
  .models.merge_missing_defaults()  -> .models_mut().merge_missing_defaults()
  .models.scan_local(               -> .models_mut().scan_local(
  .models.tick_downloads(           -> .models_mut().tick_downloads(
  .models.get_mut(                  -> .models_mut().get_mut(
  .models.ready_captions()          -> .models().ready_captions()
  .models.ready_narration()         -> .models().ready_narration()
  .models.local_path(               -> .models().local_path(
  .models.models                    -> .models().models
  .models.                          -> .models().          (catch-all)

media:
  .media.find_by_path(              -> .media().find_by_path(
  .media.add(                       -> .media_mut().add(
  .media.remove(                    -> .media_mut().remove(
  .media.items.iter_mut()           -> .media_mut().items.iter_mut()
  .media.items.clear()              -> .media_mut().items.clear()
  .media.items.iter()               -> .media().items.iter()
  .media.items.is_empty()           -> .media().items.is_empty()
  .media.                           -> .media().          (catch-all)

tracks:
  .tracks.push(                     -> .tracks_mut().push(
  .tracks.remove(                   -> .tracks_mut().remove(
  .tracks.iter_mut()                -> .tracks_mut().iter_mut()
  .tracks.len()                     -> .tracks().len()
  .tracks.get(                      -> .tracks().get(
  .tracks.get_mut(                  -> .tracks().get_mut(
  .tracks.iter()                    -> .tracks().iter()
  .tracks.clone()                   -> .tracks().clone()
  .tracks =                         -> * .tracks_mut() =   (special: add * )
  .tracks.                          -> .tracks().          (catch-all)

Special:
  std::mem::take(&mut self.project.media)  -> std::mem::take(&mut self.project.media_mut())
  state.media = media                   -> *state.media_mut() = media
"""

import re
import sys
from pathlib import Path

FILES = [
    "crates/ui/src/app.rs",
    "crates/ui/src/media_jobs.rs",
    "crates/ui/src/panels/media_bin.rs",
    "crates/ui/src/timeline/mod.rs",
]

BASE = Path(r"C:\Users\domag\CapRust")

# Replacement rules: list of (pattern, replacement) applied with re.sub
# Order matters: most specific first.
RULES = [
    # === models: mutable first ===
    (r"\.models\.merge_missing_defaults\(\)", ".models_mut().merge_missing_defaults()"),
    (r"\.models\.scan_local\(", ".models_mut().scan_local("),
    (r"\.models\.tick_downloads\(", ".models_mut().tick_downloads("),
    (r"\.models\.get_mut\(", ".models_mut().get_mut("),
    # === models: read-only ===
    (r"\.models\.ready_captions\(\)", ".models().ready_captions()"),
    (r"\.models\.ready_narration\(\)", ".models().ready_narration()"),
    (r"\.models\.local_path\(", ".models().local_path("),
    (r"\.models\.models", ".models().models"),
    (r"\.models\.", ".models()."),
    # === media: mutable first ===
    (r"\.media\.find_by_path\(", ".media().find_by_path("),
    (r"\.media\.add\(", ".media_mut().add("),
    (r"\.media\.remove\(", ".media_mut().remove("),
    (r"\.media\.items\.iter_mut\(\)", ".media_mut().items.iter_mut()"),
    (r"\.media\.items\.clear\(\)", ".media_mut().items.clear()"),
    (r"\.media\.items\.iter\(\)", ".media().items.iter()"),
    (r"\.media\.items\.is_empty\(\)", ".media().items.is_empty()"),
    (r"\.media\.", ".media()."),
    # === tracks: mutable first ===
    (r"\.tracks\.push\(", ".tracks_mut().push("),
    (r"\.tracks\.remove\(", ".tracks_mut().remove("),
    (r"\.tracks\.iter_mut\(\)", ".tracks_mut().iter_mut()"),
    # === tracks: read-only ===
    (r"\.tracks\.len\(\)", ".tracks().len()"),
    (r"\.tracks\.get\(", ".tracks().get("),
    (r"\.tracks\.get_mut\(", ".tracks().get_mut("),
    (r"\.tracks\.iter\(\)", ".tracks().iter()"),
    (r"\.tracks\.clone\(\)", ".tracks().clone()"),
    (r"\.tracks\.", ".tracks()."),
]

SPECIAL = [
    (r"std::mem::take\(&mut self\.project\.media\)",
     "std::mem::take(&mut self.project.media_mut())"),
    (r"state\.media = media", "*state.media_mut() = media"),
    (r"self\.project\.tracks = updated_tracks",
     "*self.project.tracks_mut() = updated_tracks"),
]

def fix_file(path: Path) -> int:
    text = path.read_text(encoding="utf-8")
    original = text
    for pattern, repl in RULES:
        text = re.sub(pattern, repl, text)
    for pattern, repl in SPECIAL:
        text = re.sub(pattern, repl, text)
    if text != original:
        path.write_text(text, encoding="utf-8")
        # Count changes
        n = sum(1 for _ in re.finditer(r"\.", text)) - sum(1 for _ in re.finditer(r"\.", original))
        return 1
    return 0

def main():
    total = 0
    for rel in FILES:
        p = BASE / rel
        if not p.exists():
            print(f"SKIP (not found): {rel}")
            continue
        changed = fix_file(p)
        print(f"{'CHANGED' if changed else 'unchanged'}: {rel}")
        total += changed
    print(f"\n{total} file(s) modified.")

if __name__ == "__main__":
    main()
