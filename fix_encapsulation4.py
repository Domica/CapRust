#!/usr/bin/env python3
"""
Round 4: Fix remaining errors after rounds 1-3.

Issues:
1. .models()\n.scan_local( -> .models_mut()\n.scan_local( (mutable method on immutable ref)
2. .models()\n.models\n.iter_mut( -> .models_mut()\n.models\n.iter_mut( (iter_mut needs &mut)
3. .media()\n.add( -> .media_mut()\n.add( (add needs &mut)
4. .tracks()\n.push( -> .tracks_mut()\n.push( (push needs &mut)
5. .tracks().clone() -> .tracks().to_vec() (clone on slice clones ref, to_vec clones data)
6. updated_tracks.to_vec() -> updated_tracks (already Vec, no need to convert again)
7. &mut self.project.models() -> &mut self.project.models_mut() (&mut on &ModelRegistry doesn't work)
"""

import re
from pathlib import Path

BASE = Path(r"C:\Users\domag\CapRust")

FILES = [
    "crates/ui/src/app.rs",
]

def fix_file(path: Path) -> int:
    text = path.read_text(encoding="utf-8")
    original = text

    # --- .models()\n.scan_local( -> .models_mut()\n.scan_local( ---
    text = re.sub(
        r"\.models\(\)(\n\s*)\.scan_local\(",
        r".models_mut()\1.scan_local(",
        text,
    )

    # --- .models()\n.models\n.iter_mut( -> .models_mut()\n.models\n.iter_mut( ---
    text = re.sub(
        r"\.models\(\)(\n\s*)\.models(\n\s*)\.iter_mut\(",
        r".models_mut()\1.models\2.iter_mut(",
        text,
    )

    # --- .media()\n.add( -> .media_mut()\n.add( ---
    text = re.sub(
        r"\.media\(\)(\n\s*)\.add\(",
        r".media_mut()\1.add(",
        text,
    )

    # --- .tracks()\n.push( -> .tracks_mut()\n.push( ---
    text = re.sub(
        r"\.tracks\(\)(\n\s*)\.push\(",
        r".tracks_mut()\1.push(",
        text,
    )

    # --- .tracks().clone() -> .tracks().to_vec() ---
    text = re.sub(
        r"self\.project\.tracks\(\)\.clone\(\)",
        "self.project.tracks().to_vec()",
        text,
    )

    # --- updated_tracks.to_vec() -> updated_tracks (already Vec) ---
    text = re.sub(
        r"\*self\.project\.tracks_mut\(\) = updated_tracks\.to_vec\(\);",
        "*self.project.tracks_mut() = updated_tracks;",
        text,
    )

    # --- &mut self.project.models() -> &mut self.project.models_mut() ---
    text = re.sub(
        r"&mut self\.project\.models\(\)",
        "&mut self.project.models_mut()",
        text,
    )

    if text != original:
        path.write_text(text, encoding="utf-8")
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
