#!/usr/bin/env python3
"""png-diff.py A B [PANELS KEEP]: how two PNG pictures of the same size differ.

Prints "PIXELS DIFFERING MAX" (the number of pixels, of all, where any
channel differs, and the largest difference in a channel). With PANELS
and KEEP, the pictures are PANELS panels side by side of which only the
first KEEP are compared (gpu-check.png: software | GPU | differences). For the
8-bit, non-interlaced RGB or RGBA pictures reModel writes. Standard
library only.
"""
import struct
import sys
import zlib


def decode(path):
    with open(path, "rb") as f:
        data = f.read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        sys.exit(f"{path}: not a PNG")
    pos, idat, w = 8, b"", 0
    while pos < len(data):
        n, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        if kind == b"IHDR":
            w, h, depth, colour, _, _, interlace = struct.unpack(">IIBBBBB", body)
            if depth != 8 or colour not in (2, 6) or interlace:
                sys.exit(f"{path}: not 8-bit non-interlaced RGB(A)")
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


def main():
    a, b = decode(sys.argv[1]), decode(sys.argv[2])
    if a[:3] != b[:3]:
        print(f"{a[0] * a[1]} {a[0] * a[1]} 255")
        return
    w, h, bpp, da = a
    db = b[3]
    cols = w
    if len(sys.argv) > 4:
        cols = w // int(sys.argv[3]) * int(sys.argv[4])
    differ, top = 0, 0
    for y in range(h):
        ra = da[y * w * bpp:(y * w + cols) * bpp]
        rb = db[y * w * bpp:(y * w + cols) * bpp]
        if ra == rb:
            continue
        for i in range(0, len(ra), bpp):
            m = max(abs(ra[i + k] - rb[i + k]) for k in range(bpp))
            if m:
                differ += 1
                top = max(top, m)
    print(f"{cols * h} {differ} {top}")


main()
