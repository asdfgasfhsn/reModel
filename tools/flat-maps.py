#!/usr/bin/env python3
"""flat-maps.py SRC DEST: a copy of texture pack SRC whose normal maps are all flat.

Every `*.n.png` in SRC is written to DEST as a flat map (128, 128, 255) of
the same size; every other file is linked, not copied (materials.txt, the
generator's record, is left out). A pack of flat maps must draw with
`--materials` exactly as without its maps (`make materials-identity`,
docs/texture-packs.md). Standard library only.
"""
import os
import struct
import sys
import zlib


def size(path):
    with open(path, "rb") as f:
        head = f.read(24)
    if head[:8] != b"\x89PNG\r\n\x1a\n" or head[12:16] != b"IHDR":
        sys.exit(f"{path}: not a PNG")
    return struct.unpack(">II", head[16:24])


def flat_png(w, h):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    row = b"\x00" + b"\x80\x80\xff" * w
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(row * h, 9))
        + chunk(b"IEND", b"")
    )


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    src, dest = (os.path.abspath(a) for a in sys.argv[1:])
    os.makedirs(dest, exist_ok=True)
    maps = 0
    for name in sorted(os.listdir(src)):
        s, d = os.path.join(src, name), os.path.join(dest, name)
        if os.path.lexists(d):
            os.remove(d)
        if name.endswith(".n.png"):
            with open(d, "wb") as f:
                f.write(flat_png(*size(s)))
            maps += 1
        elif name != "materials.txt":
            os.symlink(s, d)
    print(f"{maps} normal maps made flat in {dest}")
    if maps == 0:
        sys.exit("no normal maps (*.n.png) in the pack: nothing to check")


if __name__ == "__main__":
    main()
