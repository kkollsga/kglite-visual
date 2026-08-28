#!/usr/bin/env python3
"""Two structural bans whose violation is silent, remote, or both.

**The renderer-family ban.** ``@cosmos.gl/graph`` (MIT, OpenJS Foundation) and
the ``@cosmograph/*`` family (CC-BY-NC-4.0) are the same engine under two
licences, and they *share version numbers* — the package name is the only
thing that tells them apart. Nothing about ``npm install @cosmograph/cosmos``
looks wrong, the app keeps working, and the repo silently stops being
MIT-shippable. So the manifest and the lockfile are scanned: the lockfile
matters as much, because a transitive pull would never touch package.json.

**The global-allocator ban.** ``kglite-visual-py`` must declare no
``#[global_allocator]`` (plan D9). KGLite's wheel installs mimalloc; a notebook
importing both wheels loads two extension modules into one interpreter, and two
allocators there is a SIGSEGV shape KGLite already runs a canary for. The crash
lands in the *user's* process, days later, with no line pointing here — which
is exactly the class of rule a code review cannot be trusted to hold.

Both scans assert they found something to scan. A guard that walks an empty
directory and reports success is not a guard (R1: a scan-based check that finds
zero files passes vacuously), and a crate rename or a moved manifest is the
realistic way that happens.

Rust line comments are stripped before the allocator scan, so the comments in
that crate explaining *why* it has no allocator do not trip the check that
enforces it — comment subsumption in reverse, and the self-test pins both
directions.

Exit codes: 0 clean, 1 a ban was violated, 2 the checker could not do its job.
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

BANNED_NPM_SCOPE = "@cosmograph/"
MANIFEST_NAMES = ("package.json", "package-lock.json")

# The attribute, tolerant of the whitespace rustfmt would never produce but a
# hand edit might: `# [global_allocator]` is legal Rust.
ALLOCATOR_RE = re.compile(r"#\s*\[\s*global_allocator\s*\]")


def _strip_rust_line_comments(source: str) -> str:
    """Drop ``//`` comments, leaving code positions intact.

    Deliberately not a full Rust lexer: a ``//`` inside a string literal is
    treated as a comment start, which can only ever *hide* an occurrence in a
    string. An attribute inside a string literal is not a global allocator, so
    the direction of that inaccuracy is safe. Block comments are left alone —
    ``/* #[global_allocator] */`` still trips the check, and failing loud on a
    commented-out allocator is the right side to err on.
    """
    out = []
    for line in source.splitlines():
        idx = line.find("//")
        out.append(line if idx < 0 else line[:idx])
    return "\n".join(out)


def check_npm_ban(frontend: Path, root: Path) -> list[str]:
    """Report every banned-scope occurrence in the frontend's manifests.

    Any occurrence anywhere in the file counts, prose fields included — a
    ``description`` naming the banned scope trips this, as it did on the first
    run. That is the correct side to err on for a licence guard, so the ban's
    *rationale* lives in this file and in CLAUDE.md, never inside the manifests
    it polices.
    """
    problems: list[str] = []
    scanned = 0
    for name in MANIFEST_NAMES:
        path = frontend / name
        if not path.is_file():
            continue
        scanned += 1
        for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if BANNED_NPM_SCOPE in line:
                problems.append(f"{path.relative_to(root)}:{lineno}: {line.strip()}")

    if scanned == 0:
        problems.append(
            f"no npm manifest found under {frontend} — the ban scanned nothing, "
            "which is not a pass"
        )
    return problems


def check_allocator_ban(crate_src: Path, root: Path) -> list[str]:
    """Report every ``#[global_allocator]`` in the PyO3 crate's sources."""
    problems: list[str] = []
    sources = sorted(crate_src.rglob("*.rs"))
    if not sources:
        return [
            f"no .rs sources found under {crate_src} — the ban scanned nothing, "
            "which is not a pass"
        ]

    for path in sources:
        code = _strip_rust_line_comments(path.read_text(encoding="utf-8"))
        for lineno, line in enumerate(code.splitlines(), 1):
            if ALLOCATOR_RE.search(line):
                problems.append(f"{path.relative_to(root)}:{lineno}: {line.strip()}")
    return problems


def check(root: Path) -> int:
    frontend = root / "frontend"
    crate_src = root / "crates" / "kglite-visual-py" / "src"

    npm = check_npm_ban(frontend, root)
    alloc = check_allocator_ban(crate_src, root)

    if npm:
        print("check-bans: FAIL — banned npm scope @cosmograph/*", file=sys.stderr)
        for p in npm:
            print(f"  {p}", file=sys.stderr)
        print(
            "  The @cosmograph family is CC-BY-NC-4.0 and cannot ship in this MIT app.\n"
            "  Use @cosmos.gl/graph — same engine, MIT, OpenJS Foundation.",
            file=sys.stderr,
        )
    if alloc:
        print("check-bans: FAIL — #[global_allocator] in the PyO3 crate", file=sys.stderr)
        for p in alloc:
            print(f"  {p}", file=sys.stderr)
        print(
            "  Two global allocators in one interpreter (this wheel + kglite's) is a\n"
            "  known SIGSEGV. This crate declares none, by design.",
            file=sys.stderr,
        )

    if npm or alloc:
        return 1
    print("check-bans: OK — no @cosmograph/* dependency, no global allocator")
    return 0


def _write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def self_test() -> int:
    """Prove both bans fail on a broken tree and pass on a clean one (R1)."""
    failures: list[str] = []
    observations = 0

    def quiet_check(root: Path) -> int:
        """Run the real check with its reporting muted.

        The self-test's own verdict is the output that matters here; eight
        checks' worth of FAIL banners would bury it.
        """
        sink = io.StringIO()
        with contextlib.redirect_stdout(sink), contextlib.redirect_stderr(sink):
            return check(root)

    def observe(label: str, got: int, want: int) -> None:
        nonlocal observations
        observations += 1
        if got != want:
            failures.append(f"{label}: expected exit {want}, got {got}")

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        clean_pkg = json.dumps({"dependencies": {"@cosmos.gl/graph": "3.4.1"}}, indent=2)
        clean_rs = "// no allocator here\nfn main() {}\n"

        _write(root / "frontend" / "package.json", clean_pkg)
        _write(root / "frontend" / "package-lock.json", clean_pkg)
        _write(root / "crates" / "kglite-visual-py" / "src" / "lib.rs", clean_rs)
        observe("clean tree passes", quiet_check(root), 0)

        # A banned dependency in the manifest.
        _write(
            root / "frontend" / "package.json",
            json.dumps({"dependencies": {"@cosmograph/cosmos": "3.4.1"}}, indent=2),
        )
        observe("banned scope in package.json fails", quiet_check(root), 1)
        _write(root / "frontend" / "package.json", clean_pkg)

        # A transitive pull that never touched package.json.
        _write(
            root / "frontend" / "package-lock.json",
            json.dumps({"packages": {"node_modules/@cosmograph/cosmos": {}}}, indent=2),
        )
        observe("banned scope in the lockfile fails", quiet_check(root), 1)
        _write(root / "frontend" / "package-lock.json", clean_pkg)

        # A real allocator declaration.
        _write(
            root / "crates" / "kglite-visual-py" / "src" / "lib.rs",
            "#[global_allocator]\nstatic A: X = X;\n",
        )
        observe("global allocator fails", quiet_check(root), 1)

        # ...and a comment that merely names it must NOT fail: otherwise the
        # rule cannot be documented at the site it governs.
        _write(
            root / "crates" / "kglite-visual-py" / "src" / "lib.rs",
            "// never declare #[global_allocator] here\nfn main() {}\n",
        )
        observe("a comment naming the attribute still passes", quiet_check(root), 0)

        # Vacuous-pass guards: nothing to scan is not a pass.
        (root / "frontend" / "package.json").unlink()
        (root / "frontend" / "package-lock.json").unlink()
        observe("no npm manifest fails", quiet_check(root), 1)
        _write(root / "frontend" / "package.json", clean_pkg)

        (root / "crates" / "kglite-visual-py" / "src" / "lib.rs").unlink()
        observe("no rust sources fails", quiet_check(root), 1)

    if failures:
        print("check-bans --self-test: FAILED", file=sys.stderr)
        for f in failures:
            print(f"  {f}", file=sys.stderr)
        return 1
    print(f"check-bans --self-test: {observations} observations, all as expected")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--self-test",
        action="store_true",
        help="prove the check can fail on a deliberately broken fixture (R1)",
    )
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    return check(ROOT)


if __name__ == "__main__":
    raise SystemExit(main())
