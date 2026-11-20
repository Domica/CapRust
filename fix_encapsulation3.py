#!/usr/bin/env python3
"""
Round 3: Fix remaining errors after rounds 1 and 2.

Issues:
1. .models().models() double conversion (multiline, created by round 2's
   .models\n -> .models()\n running before double-conversion fix).
2. .tracks().push() -> .tracks_mut().push() (push needs &mut Vec).
3. .models() / .media() used where mutable access needed -> models_mut() /
   media_mut().
4. .tracks) / .media) before closing paren -> .tracks()) / .media()).
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

    # --- Fix .models().models() double conversion (multiline) ---
    # Pattern: .models()\n    .models()  ->  .models()\n    .models
    text = re.sub(
        r"\.models\(\)(\n\s*)\.models\(\)",
        r".models()\1.models",
        text,
    )
    # --- Fix .models().models() double conversion (single line) ---
    text = re.sub(r"\.models\(\)\.models\(\)", ".models().models", text)

    # --- Fix .tracks().push( -> .tracks_mut().push( ---
    text = re.sub(r"\.tracks\(\)\.push\(", ".tracks_mut().push(", text)

    # --- Fix .media().add( -> .media_mut().add( ---
    text = re.sub(r"\.media\(\)\.add\(", ".media_mut().add(", text)

    # --- Fix .media().remove( -> .media_mut().remove( ---
    text = re.sub(r"\.media\(\)\.remove\(", ".media_mut().remove(", text)

    # --- Fix .models().scan_local( -> .models_mut().scan_local( ---
    text = re.sub(r"\.models\(\)\.scan_local\(", ".models_mut().scan_local(", text)

    # --- Fix .models().merge_missing_defaults( -> .models_mut()... ---
    text = re.sub(
        r"\.models\(\)\.merge_missing_defaults\(",
        ".models_mut().merge_missing_defaults(",
        text,
    )

    # --- Fix .tracks) -> .tracks()) ---
    text = re.sub(r"\.tracks\)", ".tracks())", text)

    # --- Fix .media) -> .media()) ---
    text = re.sub(r"\.media\)", ".media())", text)

    # --- Fix .models) -> .models()) ---
    text = re.sub(r"\.models\)", ".models())", text)

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
