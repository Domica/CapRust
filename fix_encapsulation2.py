#!/usr/bin/env python3
"""
Round 2: Fix remaining E0616 errors where .models/.media/.tracks appear
at end of line (method chaining), before comma/paren/semicolon, or were
incorrectly converted on non-ProjectState types (AssetBrowserOutput.media).

Also fixes:
- Double .models().models() -> .models().models
- out.media() -> out.media (AssetBrowserOutput, not ProjectState)
- std::mem::take(&mut X) -> std::mem::take(X)
- tracks().get_mut -> tracks_mut().get_mut
- &mut self.project.models -> &mut self.project.models_mut()
- &self.project.models -> &self.project.models()
- *self.project.tracks_mut() = updated_tracks -> needs .to_vec()
- .media; / .models; / .tracks; -> .media(); / .models(); / .tracks();
- Multiline: .models()\n    .models() -> .models().models (fix double conv)
"""

import re
from pathlib import Path

BASE = Path(r"C:\Users\domag\CapRust")

FILES = [
    "crates/ui/src/app.rs",
    "crates/ui/src/media_jobs.rs",
    "crates/ui/src/panels/media_bin.rs",
    "crates/ui/src/timeline/mod.rs",
]

def fix_file(path: Path) -> int:
    text = path.read_text(encoding="utf-8")
    original = text

    # --- Fix out.media() -> out.media (AssetBrowserOutput, not ProjectState) ---
    text = re.sub(r"out\.media\(\)", "out.media", text)

    # --- Fix double conversion (single line): .models().models() -> .models().models ---
    text = re.sub(r"\.models\(\)\.models\(\)", ".models().models", text)

    # --- Fix double conversion (multiline): .models()\n    .models() -> .models().models ---
    text = re.sub(
        r"\.models\(\)(\n\s*)\.models\(\)",
        r".models()\1.models",
        text,
    )

    # --- Fix .models\n -> .models()\n (method chaining across lines) ---
    text = re.sub(r"\.models(\n)", r".models()\1", text)

    # --- Fix .media\n -> .media()\n ---
    text = re.sub(r"\.media(\n)", r".media()\1", text)

    # --- Fix .tracks\n -> .tracks()\n ---
    text = re.sub(r"\.tracks(\n)", r".tracks()\1", text)

    # --- Fix .models, -> .models(), ---
    text = re.sub(r"\.models,", ".models(),", text)

    # --- Fix .models) -> .models()) ---
    text = re.sub(r"\.models\)", ".models())", text)

    # --- Fix .media, -> .media(), ---
    text = re.sub(r"\.media,", ".media(),", text)

    # --- Fix .tracks, -> .tracks(), ---
    text = re.sub(r"\.tracks,", ".tracks(),", text)

    # --- Fix .media; -> .media(); (but NOT out.media;) ---
    text = re.sub(r"(?<!out)\.media;", ".media();", text)

    # --- Fix .models; -> .models(); ---
    text = re.sub(r"(?<!out)\.models;", ".models();", text)

    # --- Fix .tracks; -> .tracks(); (but NOT out.tracks;) ---
    text = re.sub(r"(?<!out)\.tracks;", ".tracks();", text)

    # --- Fix std::mem::take(&mut X) -> std::mem::take(X) ---
    text = re.sub(
        r"std::mem::take\(&mut (self\.project\.media_mut\(\))\)",
        r"std::mem::take(\1)",
        text,
    )

    # --- Fix tracks().get_mut -> tracks_mut().get_mut ---
    text = re.sub(r"\.tracks\(\)\.get_mut\(", ".tracks_mut().get_mut(", text)

    # --- Fix &mut self.project.models -> &mut self.project.models_mut() ---
    text = re.sub(
        r"&mut self\.project\.models,", "&mut self.project.models_mut(),", text
    )

    # --- Fix &self.project.models -> &self.project.models() ---
    text = re.sub(
        r"&self\.project\.models,", "&self.project.models(),", text
    )

    # --- Fix *self.project.tracks_mut() = updated_tracks -> .to_vec() ---
    text = re.sub(
        r"\*self\.project\.tracks_mut\(\) = updated_tracks;",
        "*self.project.tracks_mut() = updated_tracks.to_vec();",
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
