#!/usr/bin/env python3
"""Bound the gitignored ``dev-docs/`` working folder (doctrine R4).

Every path the tooling writes outside git must have a documented lifetime and
something that enforces or reports it. ``dev-docs/`` is gitignored, so it never
reaches CI and a local gate is the only thing that can ever see it grow. A
3.5 GB one was found in this estate in 2026-07, and it turned out to be **tier
misassignment** rather than a missed purge: build artifacts and a corpus were
sitting in a never-purged tier.

So this reports two different things, and only one of them fails:

* **Total size over the ceiling → FAIL.** The bound is mechanical, not a
  memory aid.
* **Files past their tier's purge lifetime → WARN.** They are the
  ``dev-docs-cleanup`` skill's job, and an overdue file is not by itself a
  reason to block a push.

It never deletes. Deciding whether something is reproducible, and which tier it
belongs in, is a judgement a script must not make — an age-only purge destroys
whatever was placed in the wrong tier (R4 corollary).

Exit codes: 0 within bounds, 1 over the ceiling, 2 the checker could not do its
job. The third is deliberate: a missing folder or an empty scan is **not** a
pass (R10 corollary — "green" and "not attempted" must not render identically).
"""

from __future__ import annotations

import argparse
import shutil
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEV_DOCS = ROOT / "dev-docs"

# Tier -> purge lifetime in days, mirroring dev-docs/README.md. That file is the
# canonical layout map; this table is the machine-readable half of it, and the
# two are changed together. A tier absent here is durable and never overdue.
PURGE_DAYS = {
    "temp": 1,
    "bench/out": 14,
    "bin": 7,
}

MB = 1024 * 1024


def _tier_of(rel: Path) -> str:
    """The tier a path belongs to, longest-prefix-first.

    ``bench/out`` must be tested before ``bench``, or every generated artifact
    would be classified as durable bench material and never reported overdue.
    """
    parts = rel.parts
    for depth in (2, 1):
        if len(parts) >= depth:
            candidate = "/".join(parts[:depth])
            if candidate in PURGE_DAYS:
                return candidate
    return parts[0] if parts else "."


def scan(root: Path) -> tuple[dict[str, int], list[tuple[Path, str, float]], int]:
    """Return (bytes per tier, overdue files, number of files seen)."""
    per_tier: dict[str, int] = {}
    overdue: list[tuple[Path, str, float]] = []
    seen = 0
    now = time.time()

    for path in root.rglob("*"):
        if path.is_symlink() or not path.is_file():
            continue
        seen += 1
        rel = path.relative_to(root)
        tier = _tier_of(rel)
        try:
            stat = path.stat()
        except OSError:
            continue
        per_tier[tier] = per_tier.get(tier, 0) + stat.st_size
        limit_days = PURGE_DAYS.get(tier)
        if limit_days is not None:
            age_days = (now - stat.st_mtime) / 86400.0
            if age_days > limit_days:
                overdue.append((rel, tier, age_days))

    return per_tier, overdue, seen


def check(root: Path, max_mb: int, quiet: bool = False) -> int:
    if not root.is_dir():
        if not quiet:
            print(
                f"check-dev-docs: {root} does not exist. That is not a pass — the"
                " working folder is where this project's plans and todos live.",
                file=sys.stderr,
            )
        return 2

    per_tier, overdue, seen = scan(root)

    # A scan that finds zero files passes vacuously (R1 corollary). dev-docs/
    # always holds at least README.md and todos.md, so an empty scan means the
    # scan is broken or pointed somewhere else — say so instead of reporting
    # a comfortable zero.
    if seen == 0:
        if not quiet:
            print(
                f"check-dev-docs: scanned {root} and found no files at all."
                " The scan is broken or aimed at the wrong path; refusing to"
                " report a verdict from it.",
                file=sys.stderr,
            )
        return 2

    total = sum(per_tier.values())
    over = total > max_mb * MB

    if not quiet:
        biggest = sorted(per_tier.items(), key=lambda kv: kv[1], reverse=True)[:5]
        summary = ", ".join(f"{tier} {size / MB:.1f}MB" for tier, size in biggest)
        print(
            f"check-dev-docs: {total / MB:.1f}MB across {seen} files "
            f"(ceiling {max_mb}MB) — largest: {summary}"
        )
        if overdue:
            print(
                f"check-dev-docs: WARN {len(overdue)} file(s) past their tier's"
                " purge lifetime — run the dev-docs-cleanup skill:"
            )
            for rel, tier, age in sorted(overdue, key=lambda t: -t[2])[:10]:
                print(f"    {rel}  ({tier}, {age:.1f}d old, limit {PURGE_DAYS[tier]}d)")

    if over:
        if not quiet:
            print(
                f"check-dev-docs: FAIL — {total / MB:.1f}MB exceeds the {max_mb}MB"
                " ceiling. Run the dev-docs-cleanup skill, move anything"
                " irreproducible out of a purged tier, and raise DEV_DOCS_MAX_MB"
                " in the Makefile only as a deliberate decision with a reason.",
                file=sys.stderr,
            )
        return 1
    return 0


def self_test() -> int:
    """Prove the check can fail, and that it does not fail vacuously (R1).

    Reading a gate cannot tell you whether it works. Four observations, each
    reproducible by whoever reads this later:

      1. an over-ceiling tree exits 1,
      2. the same tree under a larger ceiling exits 0 (so the failure was the
         size, not the fixture),
      3. an absent folder exits 2, not 0,
      4. an empty folder exits 2, not 0 — the vacuous-scan case.
    """
    failures: list[str] = []
    tmp = Path(tempfile.mkdtemp(prefix="check-dev-docs-selftest-"))
    try:
        fixture = tmp / "dev-docs"
        (fixture / "temp").mkdir(parents=True)
        (fixture / "README.md").write_text("fixture\n")
        # 3 MB of payload, so a 1 MB ceiling must fail and a 100 MB one must not.
        (fixture / "temp" / "payload.bin").write_bytes(b"\0" * (3 * MB))

        if check(fixture, max_mb=1, quiet=True) != 1:
            failures.append("did not fail on a tree over the ceiling")
        if check(fixture, max_mb=100, quiet=True) != 0:
            failures.append("failed on a tree under the ceiling (fixture is wrong)")

        if check(tmp / "does-not-exist", max_mb=100, quiet=True) != 2:
            failures.append("reported a pass for an absent dev-docs/")

        empty = tmp / "empty"
        empty.mkdir()
        if check(empty, max_mb=100, quiet=True) != 2:
            failures.append("reported a pass for an empty scan (vacuous)")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    if failures:
        print("check-dev-docs --self-test: FAILED", file=sys.stderr)
        for f in failures:
            print(f"  {f}", file=sys.stderr)
        return 1
    print("check-dev-docs --self-test: 4 observations, all as expected")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--max-mb", type=int, default=200, help="size ceiling in MB")
    ap.add_argument(
        "--self-test",
        action="store_true",
        help="prove the check can fail on a deliberately broken fixture (R1)",
    )
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    return check(DEV_DOCS, args.max_mb)


if __name__ == "__main__":
    raise SystemExit(main())
