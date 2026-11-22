#!/usr/bin/env python3
"""pack-tint.py: a synthetic texture pack, and a look at which half of a picture a pack changed.

  pack-tint.py tint SRC DEST   a copy of the texture dump or pack SRC with every PNG's
                               colours inverted (alpha kept) and every other file linked:
                               a pack that replaces what the game draws with something
                               plainly different, made at test time, never committed
                               (`make link-pack-test`)
  pack-tint.py halves A B      for two side-by-side pictures of the same size: whether the
                               left half and the right half of each are the same
                               in A and B, as `left same|differs right same|differs`
                               (a margin of 8 pixels each side of the middle is left out)

For the 8-bit, non-interlaced RGB or RGBA pictures reModel writes. Standard library only.
"""
import os
import struct
import sys
import zlib


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))


def decode(path):
    with open(path, "rb") as f:
        data = f.read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        sys.exit(f"{path}: not a PNG")
    pos, idat = 8, b""
    while pos < len(data):
        n, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        if kind == b"IHDR":
            w, h, depth, colour, _, _, interlace = struct.unpack(">IIBBBBB", body)
            if depth != 8 or colour not in (2, 6) or interlace:
                return None
            bpp = 3 if colour == 2 else 4
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    raw = zlib.decompress(idat)
    stride = w * bpp
    out = bytearray(stride * h)
    prev = bytearray(stride)
    for y in range(h):
        ft = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            b = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            if ft == 1:
                line[x] = (line[x] + a) & 255
            elif ft == 2:
                line[x] = (line[x] + b) & 255
            elif ft == 3:
                line[x] = (line[x] + (a + b) // 2) & 255
            elif ft == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        out[y * stride:(y + 1) * stride] = line
        prev = line
    return w, h, bpp, bytes(out)


def encode(w, h, bpp, pix):
    stride = w * bpp
    raw = b"".join(b"\x00" + pix[y * stride:(y + 1) * stride] for y in range(h))
    head = struct.pack(">IIBBBBB", w, h, 8, 2 if bpp == 3 else 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", head) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


def tint(src, dest):
    made = 0
    for root, _, files in os.walk(src):
        out = os.path.join(dest, os.path.relpath(root, src))
        os.makedirs(out, exist_ok=True)
        for name in files:
            a, b = os.path.join(root, name), os.path.join(out, name)
            img = decode(a) if name.endswith(".png") else None
            if img is None:
                if not os.path.lexists(b):
                    os.symlink(os.path.abspath(a), b)
                continue
            w, h, bpp, pix = img
            pix = bytearray(pix)
            for i in range(0, len(pix), bpp):
                for k in range(3):
                    pix[i + k] = 255 - pix[i + k]
            with open(b, "wb") as f:
                f.write(encode(w, h, bpp, bytes(pix)))
            made += 1
    print(f"tinted {made} images into {dest}")


def halves(pa, pb):
    a, b = decode(pa), decode(pb)
    if a is None or b is None or a[:3] != b[:3]:
        sys.exit("the pictures are not the same size and format")
    w, h, bpp, da = a
    db = b[3]
    mid, margin = w // 2, 8
    same = {}
    for name, (x0, x1) in (("left", (0, mid - margin)), ("right", (mid + margin, w))):
        same[name] = all(
            da[(y * w + x0) * bpp:(y * w + x1) * bpp] == db[(y * w + x0) * bpp:(y * w + x1) * bpp] for y in range(h)
        )
    print(f"left {'same' if same['left'] else 'differs'} right {'same' if same['right'] else 'differs'}")


def main():
    if len(sys.argv) == 4 and sys.argv[1] == "tint":
        tint(sys.argv[2], sys.argv[3])
    elif len(sys.argv) == 4 and sys.argv[1] == "halves":
        halves(sys.argv[2], sys.argv[3])
    else:
        sys.exit(__doc__)


main()
