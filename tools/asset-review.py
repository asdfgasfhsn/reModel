"""Turn reModel capture sessions into an art review set: distinct 3D textures
and distinct screens, with contact sheets.

usage: python3 -I tools/asset-review.py ASSETS_DIR

ASSETS_DIR (written by `make assets`) holds:
  raw/<session>/      texture dumps from `remodel run --dump-textures`
  frames-<session>/   snapshots from `remodel run ... SNAP`
It gains:
  textures/ + textures.txt   one PNG per distinct texture; colour variants of
                             the same texels collapse to the most-used one
  screens/                   distinct snapshots (near-black and look-alikes
                             at thumbnail size dropped)
  sheet-<size>-NN.png        texture contact sheets (large, medium, small)
  screens-sheet-NN.png       screen contact sheets

Needs ImageMagick (`magick`, `montage`). Everything here is derived from the
game's ROMs: keep it under out/, never commit it.
"""

import collections
import os
import shutil
import subprocess
import sys

PER_SHEET = 36


def magick_format(paths, ops, fmt):
    """Run `magick PATHS OPS -format FMT info:` in batches; one line per path."""
    lines = []
    for i in range(0, len(paths), 200):
        batch = paths[i : i + 200]
        r = subprocess.run(["magick"] + batch + ops + ["-format", fmt, "info:"], capture_output=True, text=True, check=True)
        lines += r.stdout.splitlines()
    return lines


def sheets(paths, prefix, title, geometry, background):
    for page in range(0, len(paths), PER_SHEET):
        n = page // PER_SHEET + 1
        subprocess.run(
            ["montage"] + paths[page : page + PER_SHEET]
            + ["-tile", "6x", "-geometry", geometry, "-background", "#303030", *background, "-pointsize", "11",
               "-fill", "white", "-title", f"{title}, page {n}", "-set", "label", "%t", f"{prefix}-{n:02d}.png"],
            check=True,
        )


def textures(root):
    raw = os.path.join(root, "raw")
    files, uses = [], collections.Counter()
    for session in sorted(os.listdir(raw)):
        d = os.path.join(raw, session)
        if not os.path.isdir(d):
            continue
        idx = os.path.join(d, "index.txt")
        if os.path.exists(idx):
            for line in open(idx):
                if line.startswith("tex_"):
                    parts = line.split("\t")
                    uses[(session, parts[0])] += next((int(p[5:]) for p in parts if p.startswith("uses=")), 1)
        files += [(session, os.path.join(d, f)) for f in sorted(os.listdir(d)) if f.startswith("tex_") and f.endswith(".png")]
    if not files:
        return "no texture dumps"

    # Greyscale, contrast-normalised 16x16 signature: colour variants match.
    sigs = magick_format(
        [p for _, p in files], ["-alpha", "off", "-colorspace", "gray", "-normalize", "-resize", "16x16!", "-depth", "3"], "%#\n"
    )
    groups = collections.defaultdict(list)
    for (session, path), h in zip(files, sigs):
        size = os.path.basename(path).rsplit("_", 1)[1][:-4]
        groups[(size, h)].append((session, path))

    out = os.path.join(root, "textures")
    shutil.rmtree(out, ignore_errors=True)
    os.makedirs(out)
    by_size = collections.defaultdict(list)
    with open(os.path.join(root, "textures.txt"), "w") as f:
        f.write("# name\tsize\tcolour variants\tsessions\n")
        for (size, _), members in sorted(groups.items()):
            best = max(members, key=lambda m: uses[(m[0], os.path.basename(m[1])[:-4])])
            name = os.path.basename(best[1])
            shutil.copy(best[1], os.path.join(out, name))
            sessions = ",".join(sorted({m[0] for m in members}))
            f.write(f"{name[:-4]}\t{size}\t{len({os.path.basename(m[1]) for m in members})}\t{sessions}\n")
            w, h = map(int, size.split("x"))
            by_size["large" if max(w, h) >= 256 else "medium" if max(w, h) >= 128 else "small"].append(os.path.join(out, name))
    for group, paths in sorted(by_size.items()):
        sheets(sorted(paths), os.path.join(root, f"sheet-{group}"), f"3D textures: {group}", "200x200+6+6",
               ["-texture", "pattern:checkerboard"])
    return f"{len(files)} dumps -> {len(groups)} distinct textures (" + ", ".join(
        f"{k} {len(v)}" for k, v in sorted(by_size.items())) + ")"


def screens(root):
    out = os.path.join(root, "screens")
    shutil.rmtree(out, ignore_errors=True)
    os.makedirs(out)
    seen, kept = set(), []
    for d in sorted(x for x in os.listdir(root) if x.startswith("frames-")):
        session = d[len("frames-"):]
        paths = sorted(os.path.join(root, d, f) for f in os.listdir(os.path.join(root, d)) if f.endswith(".png"))
        # Mean brightness and a 12x9 colour signature at 2 bits per channel.
        for path, line in zip(paths, magick_format(paths, ["-resize", "12x9!", "-depth", "2"], "%[fx:mean] %#\n")):
            mean, h = line.split()
            if float(mean) < 0.04 or h in seen:
                continue
            seen.add(h)
            dest = os.path.join(out, f"{session}-{os.path.basename(path)}")
            shutil.copy(path, dest)
            kept.append(dest)
    if not kept:
        return "no snapshots"
    sheets(kept, os.path.join(root, "screens-sheet"), "Screens", "248x192+4+4", [])
    return f"{len(kept)} distinct screens"


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    root = sys.argv[1]
    for f in os.listdir(root):
        if f.startswith(("sheet-", "screens-sheet-")):
            os.remove(os.path.join(root, f))
    print("textures:", textures(root))
    print("screens: ", screens(root))


main()
