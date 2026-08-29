#!/usr/bin/env python3
"""Guard the two-toolchain seam: the frontend bundle embedded in the binary.

This project builds a TypeScript bundle with one toolchain and bakes it into a
Rust binary with another. The trap that follows is stated in CLAUDE.md: *a
stale frontend bundle inside a fresh binary looks exactly like a backend bug*.
Nothing in either toolchain notices — `cargo build` is happy with whatever
bytes were in `frontend/dist`, and `vite build` has no idea a binary exists.

Two checks, and the second is the one CLAUDE.md's newest-of-profile rule is
about:

``--freshness``
    ``frontend/dist`` must be newer than every source that produces it.

``--resolve-binary NAME``
    Print the path of the newest ``kglite-visual`` binary across build
    profiles, and REFUSE it if it is older than the bundle it should contain.
    Never hard-code a profile and never *prefer* one: "release if present" is
    the same bug wearing a default, and it is how a harness ends up testing a
    week-old artifact while reporting on today's change.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# What `vite build` reads. A file added here that is not in this list can go
# stale without the check noticing, so the list is deliberately wide: config
# and manifest included, not just `src/`.
SOURCE_PATHS = [
    Path("frontend/src"),
    Path("frontend/index.html"),
    Path("frontend/vite.config.ts"),
    Path("frontend/tsconfig.json"),
    Path("frontend/package.json"),
]
DIST = Path("frontend/dist")


def newest(root: Path, base: Path) -> tuple[float, Path | None, int]:
    """Newest mtime under ``root``, the file holding it, and how many were seen."""
    target = base / root
    if target.is_file():
        return target.stat().st_mtime, target, 1
    if not target.is_dir():
        return 0.0, None, 0
    best, holder, seen = 0.0, None, 0
    for path in target.rglob("*"):
        if not path.is_file():
            continue
        seen += 1
        mtime = path.stat().st_mtime
        if mtime > best:
            best, holder = mtime, path
    return best, holder, seen


def newest_of(paths: list[Path], base: Path) -> tuple[float, Path | None, int]:
    best, holder, seen = 0.0, None, 0
    for path in paths:
        mtime, file, count = newest(path, base)
        seen += count
        if mtime > best:
            best, holder = mtime, file
    return best, holder, seen


def check_freshness(base: Path) -> int:
    source_mtime, source_file, source_count = newest_of(SOURCE_PATHS, base)
    # A scan that found nothing would pass vacuously (doctrine R1): assert the
    # scan itself was non-empty before believing its verdict.
    if source_count == 0:
        print(
            "check-bundle: FAIL — no frontend sources found; this check would "
            "pass on an empty tree",
            file=sys.stderr,
        )
        return 1

    dist_mtime, dist_file, dist_count = newest(DIST, base)
    if dist_count == 0:
        print(
            f"check-bundle: FAIL — {DIST}/ is missing or empty. "
            "Run `make frontend-build`.",
            file=sys.stderr,
        )
        return 1

    if dist_mtime < source_mtime:
        print(
            f"check-bundle: FAIL — the embedded bundle is older than its sources.\n"
            f"  newest source: {source_file}\n"
            f"  newest bundle: {dist_file}\n"
            "  A stale bundle inside a fresh binary reads exactly like a backend "
            "bug. Run `make frontend-build`.",
            file=sys.stderr,
        )
        return 1

    print(f"check-bundle: OK — {dist_count} bundled file(s), newer than {source_count} source(s)")
    return 0


def resolve_binary(name: str, base: Path) -> int:
    profiles = base / "target"
    candidates = []
    if profiles.is_dir():
        for profile in sorted(profiles.iterdir()):
            binary = profile / name
            if binary.is_file() and os.access(binary, os.X_OK):
                candidates.append(binary)
    if not candidates:
        print(
            f"check-bundle: FAIL — no `{name}` binary under target/. "
            "Run `cargo build` (or `cargo build --release`).",
            file=sys.stderr,
        )
        return 1

    chosen = max(candidates, key=lambda p: p.stat().st_mtime)
    dist_mtime, dist_file, dist_count = newest(DIST, base)
    if dist_count and chosen.stat().st_mtime < dist_mtime:
        print(
            f"check-bundle: FAIL — {chosen} predates the bundle it should embed "
            f"({dist_file}).\n"
            "  Rebuild the binary before driving it, or it will serve an older "
            "frontend than the one you just built.",
            file=sys.stderr,
        )
        return 1

    print(chosen)
    return 0


def self_test() -> int:
    """Prove both checks can go red (doctrine R1)."""
    failures = []

    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp)
        # 1. A tree with sources but no dist must FAIL.
        (base / "frontend/src").mkdir(parents=True)
        (base / "frontend/src/main.ts").write_text("//\n")
        if check_freshness(base) == 0:
            failures.append("a missing frontend/dist was accepted")

        # 2. A dist older than its sources must FAIL.
        (base / "frontend/dist").mkdir(parents=True)
        (base / "frontend/dist/index.html").write_text("<!doctype html>\n")
        os.utime(base / "frontend/dist/index.html", (1000, 1000))
        os.utime(base / "frontend/src/main.ts", (2000, 2000))
        if check_freshness(base) == 0:
            failures.append("a bundle older than its sources was accepted")

        # 3. The same tree with a fresh dist must PASS, or the check is a
        #    permanent red rather than a check.
        os.utime(base / "frontend/dist/index.html", (3000, 3000))
        if check_freshness(base) != 0:
            failures.append("a fresh bundle was rejected")

        # 4. No binary at all must FAIL.
        if resolve_binary("kglite-visual", base) == 0:
            failures.append("a missing binary was accepted")

        # 5. A binary older than the bundle must FAIL.
        (base / "target/debug").mkdir(parents=True)
        binary = base / "target/debug/kglite-visual"
        binary.write_text("#!/bin/sh\n")
        binary.chmod(0o755)
        os.utime(binary, (2000, 2000))
        if resolve_binary("kglite-visual", base) == 0:
            failures.append("a binary older than the bundle was accepted")

        # 6. Newest-of-profile: a fresh release build must win over an older
        #    debug one even though debug is listed first.
        (base / "target/release").mkdir(parents=True)
        release = base / "target/release/kglite-visual"
        release.write_text("#!/bin/sh\n")
        release.chmod(0o755)
        os.utime(release, (4000, 4000))
        chosen = subprocess.run(
            [sys.executable, __file__, "--resolve-binary", "kglite-visual", "--base", str(base)],
            capture_output=True,
            text=True,
            check=False,
        )
        if chosen.stdout.strip() != str(release):
            failures.append(
                f"newest-of-profile picked {chosen.stdout.strip()!r}, not the newer release build"
            )

    if failures:
        for failure in failures:
            print(f"check-bundle --self-test: FAIL — {failure}", file=sys.stderr)
        return 1
    print("check-bundle --self-test: OK — both checks can fail")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freshness", action="store_true")
    parser.add_argument("--resolve-binary", metavar="NAME")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--base", default=str(REPO), help=argparse.SUPPRESS)
    args = parser.parse_args()

    base = Path(args.base)
    if args.self_test:
        return self_test()
    if args.resolve_binary:
        return resolve_binary(args.resolve_binary, base)
    if args.freshness:
        return check_freshness(base)
    parser.error("choose --freshness, --resolve-binary or --self-test")
    return 2


if __name__ == "__main__":
    sys.exit(main())
