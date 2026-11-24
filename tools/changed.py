#!/usr/bin/env python3
"""What a change can affect, for `make check-changed` and `make bench-changed`.

The change is everything that differs from BASE (default origin/main):
commits not pushed yet, edits not committed, and new files. From the
files it works out:

  crates    the workspace crates touched, plus every crate that depends on
            them (cargo metadata), as `-p NAME` arguments; `all` when a
            change can reach every crate (Cargo.lock, the toolchain, a file
            outside crates/ that isn't docs or tooling); empty when no crate
            is affected.
  web       `yes` when remodel-web is among them (web-check).
  lock      `yes` when tools/lock changed (lock-test).
  bench     `yes` when the benchmark state hashes can move: a change to the
            emulator (remodel-model2 and what it is built from), the CLI
            that runs the benchmarks, the renderer or the recompiler, or the
            benchmark files themselves. The launcher (remodel-play), the
            browser build, link networking, the analyser and docs can't
            change a state hash.

usage: tools/changed.py crates|web|lock|bench|why [BASE]
"""

import json
import os
import subprocess
import sys

# Paths outside crates/ that no test, lint or benchmark reads.
QUIET = ("docs/", "site/", ".github/", "assets/posters/README.md", "out/")
QUIET_FILES = ("Makefile", "LICENSE", ".gitignore", "CHANGELOG.md", "README.md", "CLAUDE.md")
# Tooling that only the named gate step reads.
TOOLS_LOCK = ("tools/lock", "tools/lock-test.sh")
TOOLS_BENCH = ("tools/release.sh", "tools/expected-hashes.txt", "tools/expected-variant-hashes.txt")
# The crates a benchmark's state hash is made by, besides remodel-model2's
# own dependencies.
BENCH_ROOTS = ("remodel-model2", "remodel", "remodel-gpu", "remodel-recomp")


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout.split("\n")


def changed_files(base):
    files = set(git("diff", "--name-only", base))
    files |= set(git("ls-files", "--others", "--exclude-standard"))
    files.discard("")
    return sorted(files)


def workspace():
    cargo = os.environ.get("CARGO", "cargo")
    out = subprocess.run(
        [cargo, "metadata", "--format-version", "1", "--no-deps"], capture_output=True, text=True, check=True
    ).stdout
    meta = json.loads(out)
    names = {p["name"] for p in meta["packages"]}
    by_dir, deps = {}, {}
    for p in meta["packages"]:
        rel = os.path.relpath(os.path.dirname(p["manifest_path"]))
        by_dir[rel + "/"] = p["name"]
        deps[p["name"]] = {d["name"] for d in p["dependencies"] if d["name"] in names}
    return by_dir, deps


def closure(start, edges):
    seen, todo = set(start), list(start)
    while todo:
        for n in edges.get(todo.pop(), ()):
            if n not in seen:
                seen.add(n)
                todo.append(n)
    return seen


def analyse(base):
    files = changed_files(base)
    by_dir, deps = workspace()
    touched, everything, reasons = set(), False, []
    lock = bench_files = False
    for f in files:
        crate = next((n for d, n in by_dir.items() if f.startswith(d)), None)
        if crate:
            touched.add(crate)
        elif f.startswith(TOOLS_LOCK):
            lock = True
        elif f in TOOLS_BENCH:
            bench_files = True
            reasons.append(f"bench: {f}")
        elif f.startswith(QUIET) or f in QUIET_FILES or f.startswith("tools/") or f.endswith(".md"):
            pass
        else:
            everything = True
            reasons.append(f"all crates: {f}")
    users = {}
    for n, ds in deps.items():
        for d in ds:
            users.setdefault(d, set()).add(n)
    affected = set(deps) if everything else closure(touched, users)
    bench_set = closure(BENCH_ROOTS, deps)
    bench_hit = sorted(touched & bench_set)
    bench = everything or bench_files or bool(bench_hit)
    if bench_hit:
        reasons.append("bench: " + ", ".join(bench_hit))
    return {
        "files": files,
        "touched": sorted(touched),
        "affected": sorted(affected),
        "all": everything,
        "web": "remodel-web" in affected,
        "lock": lock,
        "bench": bench,
        "reasons": reasons,
    }


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    what, base = sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else "origin/main"
    a = analyse(base)
    if what == "crates":
        print("all" if a["all"] else " ".join(f"-p {n}" for n in a["affected"]))
    elif what in ("web", "lock", "bench"):
        print("yes" if a[what] else "no")
    elif what == "why":
        print(f"changed since {base}: {len(a['files'])} files")
        print("crates touched:  " + (" ".join(a["touched"]) or "none"))
        print("crates to test:  " + ("all" if a["all"] else " ".join(a["affected"]) or "none"))
        print(f"web-check: {'yes' if a['web'] else 'no'}   lock-test: {'yes' if a['lock'] else 'no'}   "
              f"benchmarks: {'yes' if a['bench'] else 'no'}")
        for r in a["reasons"]:
            print("  " + r)
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
