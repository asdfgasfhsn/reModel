"""Compare two remodel-play --frame-log files (`make draw-overlap-check`).

Each line is FRAME SOURCE HASH: the picture presented of the shown
machine's frame FRAME, SOURCE `game` or `ahead` (run-ahead's machine), with
`-late` when drawn with the frame before's polygons. Which frames get
drawn depends on timing, so only the frames both runs drew (with the same
source) are compared; every one drawn with its own polygons must have the
same picture, and no run may have drawn one such frame two ways. Exits
non-zero otherwise. Pictures drawn late (with the geometrizer's previous
frame's polygons) are only reported: run-ahead's machine keeps its own
previous frame across copies (`Geometrizer::copy_from` copies the engine,
not `last_frame`), so which polygons a late run-ahead picture has depends
on how the redraws before grouped their frames, i.e. on timing.

usage: python3 -I tools/frame-log-diff.py A.log B.log
"""

import sys


def load(path):
    pictures = {}
    for line in open(path):
        frame, source, picture = line.split()
        if frame != "-":
            pictures.setdefault((int(frame), source), set()).add(picture)
    return pictures


def main():
    a, b = (load(p) for p in sys.argv[1:3])
    both = sorted(k for k in a if k in b)
    late = [k for k in both if k[1].endswith("-late") and a[k] != b[k]]
    differ = [k for k in both if not k[1].endswith("-late") and a[k] != b[k]]
    twice = [k for run in (a, b) for k in run if len(run[k]) > 1 and not k[1].endswith("-late")]
    sources = sorted({k[1] for k in both})
    print(
        f"pictures      {sum(map(len, a.values()))} and {sum(map(len, b.values()))}; "
        f"{len(both)} frames drawn in both ({', '.join(f'{sum(k[1] == s for k in both)} {s}' for s in sources)})"
    )
    for k in differ[:10]:
        print(f"DIFFERS       frame {k[0]} {k[1]}: {sorted(a[k])} against {sorted(b[k])}")
    for k in late[:10]:
        print(f"late          frame {k[0]} {k[1]}: drawn with other polygons (timing; not checked)")
    for k in twice[:10]:
        print(f"DRAWN TWICE   frame {k[0]} {k[1]} differently in one run")
    ok = both and not differ and not twice
    print("pictures      " + ("the same for every frame both drew" if ok else "DIFFER"))
    sys.exit(0 if ok else 1)


main()
