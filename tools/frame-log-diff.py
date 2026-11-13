"""Compare two remodel-play --frame-log files (`make draw-overlap-check`).

Each line is FRAME SOURCE HASH: the picture presented of the shown
machine's frame FRAME, SOURCE `game` or `ahead` (run-ahead's machine), with
`-late` when drawn with the frame before's polygons. Which frames get
drawn depends on timing, so only the frames both runs drew (with the same
source) are compared; every one must have the same picture, and no run may
have drawn one frame two ways. Exits non-zero otherwise.

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
    differ = [k for k in both if a[k] != b[k]]
    twice = [k for run in (a, b) for k in run if len(run[k]) > 1]
    sources = sorted({k[1] for k in both})
    print(
        f"pictures      {sum(map(len, a.values()))} and {sum(map(len, b.values()))}; "
        f"{len(both)} frames drawn in both ({', '.join(f'{sum(k[1] == s for k in both)} {s}' for s in sources)})"
    )
    for k in differ[:10]:
        print(f"DIFFERS       frame {k[0]} {k[1]}: {sorted(a[k])} against {sorted(b[k])}")
    for k in twice[:10]:
        print(f"DRAWN TWICE   frame {k[0]} {k[1]} differently in one run")
    ok = both and not differ and not twice
    print("pictures      " + ("the same for every frame both drew" if ok else "DIFFER"))
    sys.exit(0 if ok else 1)


main()
