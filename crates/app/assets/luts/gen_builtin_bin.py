#!/usr/bin/env python3
"""Pack the built-in .cube LUTs into one compact binary.

A .cube file is text: 17x17x17 entries of three six-decimal numbers,
about 137 KB each. Four of them is 550 KB of the app binary for four
colour grades.

Packed as 16-bit values the same four LUTs are 118 KB, and the .cube
text is written out again when the cache is filled on the user's
machine -- where it costs nothing, because it is on disk and not in the
executable.

Run once after changing a .cube, then commit the result:

    python gen_builtin_bin.py
"""
import pathlib
import struct

HERE = pathlib.Path(__file__).parent
OUT = HERE / "builtin_luts.bin"

# The order the Rust side expects. It must match EMBEDDED_LUTS in
# crates/media-io/src/export_graph.rs.
NAMES = ["film_kodak", "film_fuji", "bw_contrast", "vintage_sepia"]

MAGIC = b"CPLT"


def read_cube(path):
    """Return (size, [r, g, b] triples) from a .cube file."""
    size = None
    values = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("LUT_3D_SIZE"):
            size = int(line.split()[1])
            continue
        if line.startswith(("TITLE", "DOMAIN_MIN", "DOMAIN_MAX")):
            continue
        parts = line.split()
        assert len(parts) == 3, line
        values.append(tuple(float(x) for x in parts))
    assert size is not None, path
    assert len(values) == size ** 3, (path, len(values), size)
    return size, values


def main():
    out = bytearray(MAGIC)
    out.append(len(NAMES))
    for name in NAMES:
        size, values = read_cube(HERE / (name + ".cube"))
        top = max(max(v) for v in values)
        # The output of a LUT can go above 1 (a film stock that lifts
        # the reds does), so the values are stored against the largest
        # one instead of against 1.0. The scale travels with them.
        out.append(size)
        out += struct.pack("<f", top)
        for r, g, b in values:
            for c in (r, g, b):
                assert 0.0 <= c <= top, (name, c, top)
                out += struct.pack("<H", round(c / top * 65535.0))
    OUT.write_bytes(bytes(out))
    print("wrote %s (%d bytes, %d LUTs)" % (OUT.name, len(out), len(NAMES)))
    for name in NAMES:
        cube = (HERE / (name + ".cube")).stat().st_size
        print("  %-16s %6d bytes of .cube -> %6d bytes packed"
              % (name, cube, size * size * size * 3 * 2))


if __name__ == "__main__":
    main()
