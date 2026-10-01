import pathlib

path = pathlib.Path(r"F:\Dev\projects\CapRust\crates\ui\src\panels\clip_properties.rs")
raw = path.read_bytes()
crlf = b"\r\n" in raw
s = raw.decode("utf-8")
if crlf:
    s = s.replace("\r\n", "\n")

# 1. Import: add text
old = "use crate::theme::tokens::space;"
new = "use crate::theme::tokens::{space, text};"
assert old in s, "imports"
s = s.replace(old, new, 1)

# 2. Whisper model / language labels
old = "ui.label(egui::RichText::new(model_id).monospace().size(11.0));"
new = "ui.label(egui::RichText::new(model_id).monospace().size(text::S));"
assert old in s, "model_id"
s = s.replace(old, new, 1)

old = "ui.label(egui::RichText::new(language).monospace().size(11.0));"
new = "ui.label(egui::RichText::new(language).monospace().size(text::S));"
assert old in s, "language"
s = s.replace(old, new, 1)

# 3. ALL grid spacing occurrences
old = ".spacing([6.0, 4.0])"
count = s.count(old)
assert count >= 1, f"no grid spacing hits"
print(f"  grid spacing hits: {count}")
s = s.replace(old, ".spacing([space::S, space::XS])")

# 4. ComboBox width comments — first hit of each width
widths = [
    (".width(180.0)", "// model / language combo; component-specific"),
    (".width(140.0)", "// effect/param combo; component-specific"),
    (".width(120.0)", "// speed combo; component-specific"),
    (".width(200.0)", "// font-family combo; component-specific"),
]
for anchor, note in widths:
    lines = s.split("\n")
    replaced = 0
    for i, line in enumerate(lines):
        if anchor in line:
            # skip if a // already sits to the right of this anchor
            tail = line.split(anchor, 1)[1]
            if "//" in tail:
                continue
            lines[i] = line.rstrip() + " " + note
            replaced += 1
            break
    s = "\n".join(lines)
    if replaced == 0:
        print(f"  WARN: no bare hit for {anchor}; skipping")

# 5. max_height on keyframe scroll area
old = "        .max_height(180.0)"
new = "        .max_height(180.0) // keyframe list cap; component-specific"
assert old in s, "max_height"
s = s.replace(old, new, 1)

# sanity
assert ".size(11.0)" not in s, "leftover size(11.0)"
assert ".spacing([6.0, 4.0])" not in s, "leftover grid spacing"

out = s.replace("\n", "\r\n") if crlf else s
path.write_bytes(out.encode("utf-8"))
print("OK: clip_properties.rs patched (" + ("CRLF" if crlf else "LF") + ")")
