#!/usr/bin/env python3
"""Delete Cargo build output that a newer build has replaced.

Cargo never removes an old build: each change leaves the previous
libraries and executables under new hashed names. In every `deps/`
directory under the target folder (debug, release, recomp, web, dist,
wasm32 and any other profile), this keeps, for each crate, its newest
build and any build used in the last DAYS days (default 2; a file's access
time, which the filesystem updates at least daily when a build reads it),
counting every file with the build's hash (.rlib, .rmeta, .d, .so, the
executable), and deletes the rest, along with incremental caches not used
in that time. A crate is often built in more than one configuration at
once, which is why "newest only" would be wrong: Cargo would rebuild them.

Usage: clean-stale.py [TARGET_DIR] [--days N] [--dry-run]
"""

import collections
import os
import re
import shutil
import sys
import time

NAME = re.compile(r"^(?:lib)?(.+)-([0-9a-f]{16})(\..*)?$")


def size(p):
    if os.path.isdir(p) and not os.path.islink(p):
        return sum(size(os.path.join(p, f)) for f in os.listdir(p))
    try:
        return os.lstat(p).st_size
    except OSError:
        return 0


def main():
    argv = sys.argv[1:]
    days = 2.0
    if "--days" in argv:
        i = argv.index("--days")
        days = float(argv[i + 1])
        del argv[i : i + 2]
    args = [a for a in argv if not a.startswith("--")]
    dry = "--dry-run" in argv
    cutoff = time.time() - days * 86400

    def used(files):
        return max(max(os.lstat(p).st_atime, os.lstat(p).st_mtime) for p in files) >= cutoff
    target = args[0] if args else "target"
    doomed = []
    for root, dirs, _ in os.walk(target):
        for d in list(dirs):
            path = os.path.join(root, d)
            if d == "incremental":
                dirs.remove(d)
                for f in os.listdir(path):
                    q = os.path.join(path, f)
                    if os.lstat(q).st_mtime < cutoff:
                        doomed.append(q)
            elif d == "deps":
                dirs.remove(d)
                groups = collections.defaultdict(lambda: collections.defaultdict(list))
                for f in os.listdir(path):
                    m = NAME.match(f)
                    if m:
                        groups[m.group(1)][m.group(2)].append(os.path.join(path, f))
                for builds in groups.values():
                    if len(builds) < 2:
                        continue
                    newest = max(builds, key=lambda h: max(os.lstat(p).st_mtime for p in builds[h]))
                    for h, files in builds.items():
                        if h != newest and not used(files):
                            doomed.extend(files)
    total = sum(size(p) for p in doomed)
    for p in doomed:
        if dry:
            continue
        if os.path.isdir(p) and not os.path.islink(p):
            shutil.rmtree(p, ignore_errors=True)
        else:
            try:
                os.remove(p)
            except OSError:
                pass
    verb = "would free" if dry else "freed"
    print(f"clean-stale: {len(doomed)} stale files and caches in {target}, {verb} {total / 1e9:.1f} GB (uncompressed)")


if __name__ == "__main__":
    main()
