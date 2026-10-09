#!/usr/bin/env python3
"""Generate built-in 3D LUT (.cube) files for CapRust.

Produces four small (17x17x17) film-emulation LUTs in
crates/app/assets/luts/. Run once, commit the output.
"""
import pathlib
import math

OUT = pathlib.Path(__file__).parent / "luts"
OUT.mkdir(parents=True, exist_ok=True)

SIZE = 17

def write_cube(name, transform):
    """Write a .cube file applying transform(r,g,b) -> (r,g,b)."""
    lines = [
        f'TITLE "{name}"',
        f"LUT_3D_SIZE {SIZE}",
        "DOMAIN_MIN 0.0 0.0 0.0",
        "DOMAIN_MAX 1.0 1.0 1.0",
    ]
    for b in range(SIZE):
        for g in range(SIZE):
            for r in range(SIZE):
                ri = r / (SIZE - 1)
                gi = g / (SIZE - 1)
                bi = b / (SIZE - 1)
                ro, go, bo = transform(ri, gi, bi)
                lines.append(f"{ro:.6f} {go:.6f} {bo:.6f}")
    (OUT / f"{name}.cube").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {name}.cube ({len(lines)} lines)")


def film_kodak(r, g, b):
    """Warm film stock: lifted reds, slightly compressed highlights."""
    def lift(c):
        return c * 0.95 + 0.05 * (1 - c)
    def comp(c):
        return 1 - (1 - c) ** 1.1
    return lift(comp(r)) * 1.08, lift(comp(g)) * 1.0, lift(comp(b)) * 0.92


def film_fuji(r, g, b):
    """Cool film stock: shifted blues, softer mids."""
    def soft(c):
        return c ** 0.9
    return soft(r) * 0.96, soft(g) * 1.0, soft(b) * 1.1


def bw_contrast(r, g, b):
    """B&W with boosted contrast: luminance mapped through a steep curve."""
    lum = 0.2126 * r + 0.7152 * g + 0.0722 * b
    lum = (lum ** 1.4)
    return lum, lum, lum


def vintage_sepia(r, g, b):
    """Sepia tone: luminance shifted toward warm browns."""
    lum = 0.2126 * r + 0.7152 * g + 0.0722 * b
    return lum * 1.1, lum * 0.9, lum * 0.7


write_cube("film_kodak", film_kodak)
write_cube("film_fuji", film_fuji)
write_cube("bw_contrast", bw_contrast)
write_cube("vintage_sepia", vintage_sepia)
print(f"done -> {OUT}")
