"""Write the third-party notices for a Remodel build: every crate linked into
PACKAGE for TARGET, its licence, and the licence texts the crate ships.

usage: python3 -I tools/notices.py PACKAGE TARGET OUT_FILE

Only normal dependencies count (build scripts and proc macros run at build
time and aren't shipped). Crates with identical licence texts share one copy.
Crates that ship no licence file are listed with their SPDX expression only
(their licences' standard texts come with other crates); they go to stderr so
a release can be checked by hand.
"""

import collections
import glob
import hashlib
import json
import os
import subprocess
import sys

TEXT_FILES = ("LICENSE*", "LICENCE*", "COPYING*", "NOTICE*", "UNLICENSE*", "OFL*", "*-license.txt")


def linked(metadata, root_name):
    """Packages reachable from root_name through normal dependencies."""
    packages = {p["id"]: p for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    root = next(i for i, p in packages.items() if p["name"] == root_name and p["source"] is None)
    seen, todo = set(), [root]
    while todo:
        id = todo.pop()
        if id in seen:
            continue
        seen.add(id)
        for dep in nodes[id]["deps"]:
            if any(k["kind"] is None for k in dep["dep_kinds"]):
                todo.append(dep["pkg"])
    return [packages[i] for i in seen]


def texts(package):
    """(file name, contents) of each licence file the crate ships, at its top
    level or one directory down (bundled fonts and data keep theirs there)."""
    dir = os.path.dirname(package["manifest_path"])
    pats = [os.path.join(dir, d, pat) for d in ("", "*") for pat in TEXT_FILES]
    # Bundled fonts carry their licences as text files beside them.
    pats += [os.path.join(os.path.dirname(f), "*.txt") for ext in ("ttf", "otf") for f in glob.glob(os.path.join(dir, "*", f"*.{ext}"))]
    paths = sorted({p for pat in pats for p in glob.glob(pat) if os.path.isfile(p)})
    if package.get("license_file"):
        paths.append(os.path.join(dir, package["license_file"]))
    out = []
    for p in dict.fromkeys(paths):
        with open(p, encoding="utf-8", errors="replace") as f:
            out.append((os.path.relpath(p, dir), f.read().strip()))
    return out


def main():
    package, target, out_file = sys.argv[1:]
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", target],
            capture_output=True, text=True, check=True,
        ).stdout
    )
    third = sorted((p for p in linked(metadata, package) if p["source"] is not None), key=lambda p: (p["name"], p["version"]))
    groups = collections.OrderedDict()  # text hash -> (text, [crate names])
    bare = []
    lines = [
        f"Third-party software in {package} ({target})",
        "",
        "Remodel itself is MIT licensed (LICENSE). It links the crates below,",
        "each under its own licence; their licence texts follow the list. A",
        "crate that ships no text of its own uses licences whose standard texts",
        "appear with other crates.",
        "",
    ]
    for p in third:
        name = f"{p['name']} {p['version']}"
        lines.append(f"  {name:40} {p.get('license') or 'see licence text'}")
        found = texts(p)
        if not found:
            bare.append(name)
        for file, text in found:
            key = hashlib.sha256(text.encode()).hexdigest()
            groups.setdefault(key, (file, text, []))[2].append(name)
    for file, text, names in groups.values():
        lines += ["", "=" * 78, f"{file}, used by: {', '.join(names)}", "=" * 78, "", text]
    with open(out_file, "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
    print(f"notices: {len(third)} crates, {len(groups)} distinct licence texts -> {out_file}", file=sys.stderr)
    if bare:
        print(f"notices: {len(bare)} crates ship no licence file (SPDX listed only): {', '.join(bare)}", file=sys.stderr)


if __name__ == "__main__":
    main()
