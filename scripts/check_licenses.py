#!/usr/bin/env python3
"""Every npm package whose bytes ship must be permissively licensed AND carry
its licence text.

**The gap this closes.** ``check_bans.py`` refuses one package *family* by
name, because ``@cosmograph/*`` and ``@cosmos.gl/graph`` are the same engine
under two licences and share version numbers. A name ban cannot see the other
shape of the same problem: a package that declares a permissive SPDX id in its
``package.json`` and ships no licence text at all. The declaration is metadata
anyone can type; the text is the grant, and MIT, BSD and Apache-2.0 all require
it to be *distributed with* the software. This app distributes them — Vite
bundles them into ``frontend/dist``, rust-embed bakes that into the CLI binary,
and the wheel ships the binary's library half — so a missing grant is a
licensing defect in an artifact we publish, not a tidiness complaint about
somebody's tarball. It was found on this check's first run against the real
tree (``seedrandom`` 3.0.5, MIT, no LICENSE file).

**Scope: the frontend's production npm dependencies, and that is a deliberate
floor rather than the whole artifact.** Those are the third-party bytes the
build inlines into a file we publish. Two exclusions, each with a reason:

* *devDependencies* — Playwright, TypeScript, Vite and their trees never reach
  ``dist/``. The lockfile's own ``dev`` marker is what separates them, so this
  check and npm agree on the boundary by construction rather than by a list
  maintained here.
* *cargo dependencies* — also shipped, also unchecked here. Answering the same
  question for them needs ``cargo metadata`` plus the registry source cache,
  which is a different mechanism and a different failure mode (an offline
  machine has the lockfile but not the sources, so the check would be
  unrunnable rather than red). It is a real gap and it is named here so nobody
  reads this step as covering the binary.

**A licence file is not the only place licence text can live.** ``seedrandom``
ships its MIT grant in ``README.md`` under a "LICENSE (MIT)" heading and in the
header comment of ``seedrandom.js`` — the text is distributed, just not in a
file named for it. Refusing that would be enforcing a filename convention
rather than the licence, so a package with no licence *file* is searched for
the grant itself, at its own root, and reported as ``text in <file>``. A
package with the grant nowhere is what fails, and there is no exception list
for one to rot into.

Exit codes: 0 clean, 1 a dependency failed, 2 the checker could not do its job
(no lockfile, or ``node_modules`` not installed — neither is a pass).
"""

from __future__ import annotations

import argparse
import contextlib
import io
import json
import re
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The SPDX ids this project is willing to redistribute inside an MIT app.
# Deliberately short: every addition is a licence somebody has to have read.
PERMISSIVE = frozenset(
    {
        "0bsd",
        "apache-2.0",
        "bsd-2-clause",
        "bsd-3-clause",
        "isc",
        "mit",
        "mit-0",
        "unlicense",
        "zlib",
    }
)

# A file whose name says it is the licence.
LICENSE_FILE_RE = re.compile(r"^(licen[sc]e|copying)(\.|$)", re.IGNORECASE)

# An empty or stub LICENSE is not licence text. Every grant below is well over
# this; the shortest real one in the tree (ISC) is ~750 bytes.
MIN_LICENSE_FILE_BYTES = 200

# The grant sentences themselves, for the fallback scan. Each is the opening of
# a licence body rather than a name, so a README that merely writes "MIT" in a
# badge does not satisfy the check.
GRANT_PHRASES = (
    "permission is hereby granted",  # MIT, ISC
    "permission to use, copy, modify",  # ISC (older wording)
    "redistribution and use in source and binary forms",  # BSD
    "licensed under the apache license",  # Apache-2.0 header
    "apache license\nversion 2.0",  # Apache-2.0 body
    "this software is provided 'as-is'",  # Zlib
    'this software is provided "as-is"',  # Zlib
    "mozilla public license",  # MPL, as an OR branch
    "this is free and unencumbered software",  # Unlicense
)

# Where the fallback scan looks: the package's own root, nothing recursive. A
# grant buried in a test fixture three directories down is not distributed
# *with* the software in any sense a reader would find it.
SCANNABLE_SUFFIXES = frozenset({"", ".md", ".txt", ".js", ".mjs", ".cjs", ".ts"})
MAX_SCAN_BYTES = 1_000_000


# ---------------------------------------------------------------------------
# SPDX expressions
# ---------------------------------------------------------------------------


def spdx_is_permissive(expression: str) -> bool:
    """Evaluate an SPDX licence expression against :data:`PERMISSIVE`.

    ``OR`` means the redistributor picks, so one permissive branch is enough —
    which is how ``dompurify``'s ``(MPL-2.0 OR Apache-2.0)`` passes. ``AND``
    means every branch binds, so all of them must be acceptable. ``WITH``
    attaches an exception to a licence; the licence is what decides.
    """
    tokens = re.findall(r"\(|\)|[^\s()]+", expression or "")
    if not tokens:
        return False
    position = 0

    def peek() -> str | None:
        return tokens[position] if position < len(tokens) else None

    def take() -> str:
        nonlocal position
        token = tokens[position]
        position += 1
        return token

    def primary() -> bool:
        token = take()
        if token == "(":
            value = expr()
            if peek() == ")":
                take()
            return value
        # `A WITH B` — the exception narrows A; A is what we are judging.
        while peek() is not None and peek().upper() == "WITH":
            take()
            if peek() is not None:
                take()
        return token.rstrip("+").lower() in PERMISSIVE

    def expr() -> bool:
        value = primary()
        while peek() is not None and peek().upper() in ("AND", "OR"):
            operator = take().upper()
            right = primary()
            value = (value and right) if operator == "AND" else (value or right)
        return value

    return expr()


# ---------------------------------------------------------------------------
# Licence text
# ---------------------------------------------------------------------------


def _has_grant(text: str) -> bool:
    lowered = text.lower()
    return any(phrase in lowered for phrase in GRANT_PHRASES)


def find_license_text(package_dir: Path) -> str | None:
    """Where this package's licence grant is, or ``None`` if it is nowhere.

    A named licence file wins and is accepted on size alone — reading every
    grant's wording to validate a file called ``LICENSE`` would be enforcing
    taste. The fallback below is the strict half: it looks for the grant text
    itself.
    """
    try:
        entries = sorted(package_dir.iterdir())
    except OSError:
        return None

    for entry in entries:
        if not entry.is_file() or not LICENSE_FILE_RE.match(entry.name):
            continue
        try:
            if entry.stat().st_size >= MIN_LICENSE_FILE_BYTES:
                return entry.name
        except OSError:
            continue

    for entry in entries:
        if not entry.is_file() or entry.suffix.lower() not in SCANNABLE_SUFFIXES:
            continue
        try:
            if entry.stat().st_size > MAX_SCAN_BYTES:
                continue
            text = entry.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        if _has_grant(text):
            return f"text in {entry.name}"
    return None


# ---------------------------------------------------------------------------
# The check
# ---------------------------------------------------------------------------


def production_packages(lock: dict) -> dict[str, dict]:
    """The lockfile's non-dev entries, keyed by their ``node_modules/...`` path.

    npm's own marker, not a re-derivation of the dependency graph: the two
    cannot disagree about what ships, and a re-derivation is exactly where a
    subtle miss would hide.
    """
    packages = lock.get("packages")
    if not isinstance(packages, dict):
        return {}
    return {
        path: meta
        for path, meta in packages.items()
        if path and isinstance(meta, dict) and not meta.get("dev") and not meta.get("devOptional")
    }


def declared_license(frontend: Path, path: str, meta: dict) -> str | None:
    """The package's SPDX declaration — installed copy first, lockfile second.

    The installed ``package.json`` is what was actually unpacked; the lockfile
    entry is a copy npm made of it. They agree today, and where they could not,
    the shipped bytes are the ones that matter.
    """
    installed = frontend / path / "package.json"
    if installed.is_file():
        try:
            value = json.loads(installed.read_text(encoding="utf-8")).get("license")
            if isinstance(value, str) and value.strip():
                return value
        except (OSError, json.JSONDecodeError):
            pass
    value = meta.get("license")
    return value if isinstance(value, str) and value.strip() else None


def check(root: Path) -> int:
    frontend = root / "frontend"
    lockfile = frontend / "package-lock.json"
    if not lockfile.is_file():
        print(
            f"check-licenses: CANNOT RUN — no lockfile at {lockfile}; "
            "nothing was scanned, which is not a pass",
            file=sys.stderr,
        )
        return 2
    try:
        lock = json.loads(lockfile.read_text(encoding="utf-8"))
    except json.JSONDecodeError as err:
        print(f"check-licenses: CANNOT RUN — {lockfile} is not valid JSON: {err}", file=sys.stderr)
        return 2

    packages = production_packages(lock)
    if not packages:
        # A scan that finds nothing to scan passes vacuously (R1). A rename of
        # the lockfile's `packages` key, or a tree with its one runtime
        # dependency removed, is the realistic way this happens.
        print(
            "check-licenses: FAIL — the lockfile lists no production dependencies; "
            "the check scanned nothing, which is not a pass",
            file=sys.stderr,
        )
        return 1

    missing_install = [path for path in packages if not (frontend / path).is_dir()]
    if missing_install:
        print(
            "check-licenses: CANNOT RUN — these packages are in the lockfile but not "
            "installed, so their licence text cannot be inspected:",
            file=sys.stderr,
        )
        for path in sorted(missing_install)[:10]:
            print(f"  {path}", file=sys.stderr)
        print("  Run `npm ci` in frontend/ first.", file=sys.stderr)
        return 2

    problems: list[str] = []
    for path, meta in sorted(packages.items()):
        name = path.split("node_modules/")[-1]
        license_id = declared_license(frontend, path, meta)
        if license_id is None:
            problems.append(f"{name}: declares no license at all")
        elif not spdx_is_permissive(license_id):
            problems.append(f"{name}: license '{license_id}' is not on the permissive allowlist")
        if find_license_text(frontend / path) is None:
            problems.append(
                f"{name}: declares '{license_id}' but ships no licence text — "
                "no LICENSE/COPYING file and no grant in any file at its root"
            )

    if problems:
        print("check-licenses: FAIL — bundled dependencies", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        print(
            "  These packages are inlined into frontend/dist, which rust-embed bakes\n"
            "  into the published binary and the wheel. A permissive licence that\n"
            "  ships no grant text is not satisfied by distributing the code alone.",
            file=sys.stderr,
        )
        return 1

    print(f"check-licenses: OK — {len(packages)} production dependencies, all permissive with text")
    return 0


# ---------------------------------------------------------------------------
# R1
# ---------------------------------------------------------------------------


def _write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


MIT_TEXT = (
    "MIT License\n\nCopyright (c) 2026 Someone\n\n"
    "Permission is hereby granted, free of charge, to any person obtaining a copy "
    "of this software and associated documentation files (the \"Software\"), to deal "
    "in the Software without restriction, including without limitation the rights "
    "to use, copy, modify, merge, publish, distribute, sublicense, and/or sell "
    "copies of the Software.\n"
)


def self_test() -> int:
    """Prove the check fails on each violation it claims to catch (R1)."""
    failures: list[str] = []
    observations = 0

    def quiet_check(root: Path) -> int:
        sink = io.StringIO()
        with contextlib.redirect_stdout(sink), contextlib.redirect_stderr(sink):
            return check(root)

    def observe(label: str, got: int, want: int) -> None:
        nonlocal observations
        observations += 1
        if got != want:
            failures.append(f"{label}: expected exit {want}, got {got}")

    def build(root: Path, entries: dict[str, dict]) -> None:
        """Write a lockfile plus the node_modules tree it describes.

        `entries` maps a package name to `{license, dev, files}`; `files` is a
        name -> content map written into the package directory.
        """
        packages = {"": {"name": "fixture"}}
        for name, spec in entries.items():
            path = f"node_modules/{name}"
            entry: dict = {"version": "1.0.0"}
            if spec.get("license") is not None:
                entry["license"] = spec["license"]
            if spec.get("dev"):
                entry["dev"] = True
            packages[path] = entry
            manifest: dict = {"name": name, "version": "1.0.0"}
            if spec.get("license") is not None:
                manifest["license"] = spec["license"]
            _write(root / "frontend" / path / "package.json", json.dumps(manifest))
            for filename, content in spec.get("files", {}).items():
                _write(root / "frontend" / path / filename, content)
        _write(
            root / "frontend" / "package-lock.json",
            json.dumps({"lockfileVersion": 3, "packages": packages}, indent=2),
        )

    good = {"license": "MIT", "files": {"LICENSE": MIT_TEXT}}

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)

        build(root, {"clean-dep": good})
        observe("a permissive dependency with a LICENSE file passes", quiet_check(root), 0)

        # The E13 case: a permissive declaration and no text anywhere.
        build(root, {"clean-dep": good, "textless": {"license": "Apache-2.0", "files": {}}})
        observe("a declared licence with no text fails", quiet_check(root), 1)

        # A LICENSE file that exists and says nothing.
        build(root, {"stub": {"license": "MIT", "files": {"LICENSE": "MIT\n"}}})
        observe("an empty LICENSE stub fails", quiet_check(root), 1)

        # The grant in the README rather than in a file named for it — the
        # seedrandom shape. Accepted, because the text is what is distributed.
        build(
            root,
            {"readme-only": {"license": "MIT", "files": {"README.md": "# x\n\n" + MIT_TEXT}}},
        )
        observe("a grant in the README passes", quiet_check(root), 0)

        # A README that only *names* the licence is not the grant.
        build(root, {"badge": {"license": "MIT", "files": {"README.md": "# x\n\nMIT licensed.\n"}}})
        observe("a README that only names MIT fails", quiet_check(root), 1)

        build(root, {"nonfree": {"license": "CC-BY-NC-4.0", "files": {"LICENSE": MIT_TEXT}}})
        observe("a non-permissive licence fails", quiet_check(root), 1)

        build(root, {"unlicensed": {"license": None, "files": {"LICENSE": MIT_TEXT}}})
        observe("a package declaring no licence fails", quiet_check(root), 1)

        # Scope proof: the same violation in a devDependency is not this
        # check's business, and must not be reported as one.
        build(
            root,
            {
                "clean-dep": good,
                "toolchain": {"license": "CC-BY-NC-4.0", "dev": True, "files": {}},
            },
        )
        observe("a dev-only violation is out of scope", quiet_check(root), 0)

        # SPDX expressions: dompurify's real one, and an AND that must not pass
        # on its permissive half alone.
        build(
            root,
            {"either": {"license": "(MPL-2.0 OR Apache-2.0)", "files": {"LICENSE": MIT_TEXT}}},
        )
        observe("an OR expression with one permissive branch passes", quiet_check(root), 0)
        build(root, {"both": {"license": "MIT AND CC-BY-NC-4.0", "files": {"LICENSE": MIT_TEXT}}})
        observe("an AND expression with a non-free branch fails", quiet_check(root), 1)

        # Vacuous-pass guards.
        build(root, {})
        observe("a lockfile with no production packages fails", quiet_check(root), 1)

        build(root, {"clean-dep": good})
        (root / "frontend" / "package-lock.json").unlink()
        observe("no lockfile cannot run", quiet_check(root), 2)

        build(root, {"clean-dep": good})
        for entry in (root / "frontend" / "node_modules" / "clean-dep").iterdir():
            entry.unlink()
        (root / "frontend" / "node_modules" / "clean-dep").rmdir()
        observe("an uninstalled dependency cannot run", quiet_check(root), 2)

    if failures:
        print("check-licenses --self-test: FAILED", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1
    print(f"check-licenses --self-test: {observations} observations, all as expected")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="prove the check can fail on a deliberately broken fixture (R1)",
    )
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    return check(ROOT)


if __name__ == "__main__":
    raise SystemExit(main())
