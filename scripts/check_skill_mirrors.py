#!/usr/bin/env python3
"""Assert the two agent-instruction trees have not drifted apart (doctrine R7).

This repo keeps agent instructions twice: ``CLAUDE.md`` + ``.claude/skills/``
(the tracked **authority**) and ``AGENTS.md`` + ``.agents/skills/`` (gitignored
**generated adapters**). A stale adapter does not merely lag — it teaches a
procedure the live copy warns against. In this estate one adapter was 194 lines
behind and still instructed its reader that the version bump is "one line …
there is no per-manifest bump", the exact belief that broke a release, sitting
in the file the other harness would follow.

The only legitimate difference between the two sides is that each names its own
conventions file. So this normalises ``AGENTS.md`` -> ``CLAUDE.md`` and the root
title, then requires the rest to be identical.

**Two things this check deliberately cannot see, named so nobody trusts it
further than it goes:**

1. **An inverted authority declaration.** The ``**Authority:**`` paragraph is
   exempt from the rename substitution — it names the authority *literally* in
   every copy. If someone substitutes it anyway, the adapter's copy tells its
   reader to edit the adapter, and because both sides normalise to the same
   string the mirror check stays green. Two repos in this estate hit that on
   the day the procedure landed. ``--sync`` is what prevents it here: it copies
   that paragraph verbatim rather than substituting it.
2. **Whether the authority itself is right.** Two identical files can be
   identically wrong. Equivalence is the property this checks; correctness is
   what review and the oracle-first rule (R14) are for.

Exit codes: 0 in sync, 1 drifted, 2 the checker could not do its job.
"""

from __future__ import annotations

import argparse
import re
import shutil
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Each root file names itself in its H1. That, and a reference naming its own
# conventions file, are the only differences the pair is allowed to have.
ROOT_TITLE = re.compile(
    r"^# (?P<repo>.+?) — (?:Claude Code|Codex) Conventions[ \t]*$", re.M
)

# The authority declaration is copied verbatim into the adapter, never
# substituted — see the module docstring, failure mode 1.
AUTHORITY_START = "**Authority:**"


def normalise(text: str) -> str:
    """Erase the one difference the two skill trees are allowed to have."""
    return text.replace("AGENTS.md", "CLAUDE.md")


def normalise_root(text: str) -> str:
    """Erase the differences the two root conventions files are allowed to have."""
    return ROOT_TITLE.sub(r"# \g<repo> — Conventions", text.replace("AGENTS.md", "CLAUDE.md"))


def _authority_span(lines: list[str]) -> tuple[int, int] | None:
    """The [start, end) line span of the ``**Authority:**`` paragraph, if any."""
    for i, line in enumerate(lines):
        if line.startswith(AUTHORITY_START):
            j = i
            while j < len(lines) and lines[j].strip():
                j += 1
            return i, j
    return None


def render_adapter(text: str, *, is_root: bool) -> str:
    """Generate the adapter's copy from the authority's text."""
    lines = text.split("\n")
    span = _authority_span(lines) if is_root else None
    out: list[str] = []
    for i, line in enumerate(lines):
        if span and span[0] <= i < span[1]:
            out.append(line)  # verbatim — the exempt declaration
            continue
        out.append(line.replace("CLAUDE.md", "AGENTS.md"))
    rendered = "\n".join(out)
    if is_root:
        rendered = re.sub(
            r"^# (?P<repo>.+?) — Claude Code Conventions[ \t]*$",
            r"# \g<repo> — Codex Conventions",
            rendered,
            flags=re.M,
        )
    return rendered


def first_difference(a: list[str], b: list[str]) -> tuple[int, str, str]:
    for i, (x, y) in enumerate(zip(a, b), start=1):
        if x != y:
            return i, x, y
    n = min(len(a), len(b))
    return (
        n + 1,
        a[n] if len(a) > n else "<end of file>",
        b[n] if len(b) > n else "<end of file>",
    )


def _excerpt(line: str, limit: int = 96) -> str:
    line = line.rstrip()
    return line if len(line) <= limit else line[: limit - 1] + "…"


def check(root: Path, summary: list[str]) -> tuple[int, list[str]]:
    """Return (exit code, problems)."""
    claude_dir = root / ".claude" / "skills"
    agents_dir = root / ".agents" / "skills"
    root_claude = root / "CLAUDE.md"
    root_agents = root / "AGENTS.md"
    problems: list[str] = []

    # --- root pair -------------------------------------------------------
    if not (root_agents.is_file() or agents_dir.exists()):
        summary.append("no adapter installed (no AGENTS.md, no .agents/) — nothing to compare")
    else:
        missing = [p.name for p in (root_claude, root_agents) if not p.is_file()]
        if missing:
            problems.append(
                f"root conventions: the adapter is installed but {', '.join(missing)} is"
                " missing at the repo root — the pair must exist for the mirror check to"
                " mean anything"
            )
        else:
            c = normalise_root(root_claude.read_text(encoding="utf-8"))
            a = normalise_root(root_agents.read_text(encoding="utf-8"))
            if a == c:
                summary.append("root CLAUDE.md/AGENTS.md identical bar the rename")
            else:
                c_lines, a_lines = c.splitlines(), a.splitlines()
                lineno, cl, al = first_difference(c_lines, a_lines)
                drift = sum(1 for x, y in zip(c_lines, a_lines) if x != y) + abs(
                    len(c_lines) - len(a_lines)
                )
                problems += [
                    f"CLAUDE.md vs AGENTS.md: differ beyond the rename (~{drift} line(s));"
                    f" first at line {lineno}:",
                    f"    CLAUDE.md:{lineno}: {_excerpt(cl)}",
                    f"    AGENTS.md:{lineno}: {_excerpt(al)}",
                    "    -> CLAUDE.md is the authority; merge any improvement there first,"
                    " then `make sync-agents`",
                ]

    # --- skill trees -----------------------------------------------------
    if not agents_dir.exists():
        summary.append("no .agents/skills tree — no skill trees to compare")
        return (1 if problems else 0), problems
    if not claude_dir.exists():
        problems.append("skill mirrors: .agents/skills exists but .claude/skills does not")
        return 1, problems

    claude_files = {p.relative_to(claude_dir) for p in claude_dir.rglob("SKILL.md")}
    agents_files = {p.relative_to(agents_dir) for p in agents_dir.rglob("SKILL.md")}

    # A scan that finds nothing passes vacuously (R1 corollary).
    if not claude_files:
        problems.append(
            "skill mirrors: found no SKILL.md under .claude/skills — the scan is broken"
        )
        return 2, problems

    for missing in sorted(claude_files - agents_files):
        problems.append(f"{missing}: present in .claude/skills, absent from .agents/skills")
    for extra in sorted(agents_files - claude_files):
        problems.append(f"{extra}: present in .agents/skills, absent from .claude/skills")

    for rel in sorted(claude_files & agents_files):
        c = normalise((claude_dir / rel).read_text(encoding="utf-8"))
        a = normalise((agents_dir / rel).read_text(encoding="utf-8"))
        if a != c:
            a_lines, c_lines = a.splitlines(), c.splitlines()
            # Count added/removed lines too — a pure append zips to zero
            # differing lines and would report "~0 lines", which reads as a
            # non-difference on the one output a tired reader actually scans.
            drift = sum(1 for x, y in zip(a_lines, c_lines) if x != y) + abs(
                len(a_lines) - len(c_lines)
            )
            problems.append(
                f"{rel}: trees differ beyond the rename (~{drift} lines) — one side is"
                " stale; classify it as improvement or staleness (R14), merge into the"
                " authority, then `make sync-agents`"
            )

    if not problems:
        summary.append(f"{len(claude_files)} skill(s) identical across .claude and .agents")
    return (1 if problems else 0), problems


def sync(root: Path) -> int:
    """Regenerate the adapter from the authority. Never merges."""
    claude_dir = root / ".claude" / "skills"
    agents_dir = root / ".agents" / "skills"
    root_claude = root / "CLAUDE.md"

    if not claude_dir.is_dir() or not root_claude.is_file():
        print(
            "sync-agents: the authority (CLAUDE.md + .claude/skills/) is missing."
            " Refusing to write an adapter over nothing.",
            file=sys.stderr,
        )
        return 2

    sources = sorted(claude_dir.rglob("SKILL.md"))
    if not sources:
        print(
            "sync-agents: found no SKILL.md under .claude/skills — refusing to empty"
            " the adapter tree from an empty scan.",
            file=sys.stderr,
        )
        return 2

    (root / "AGENTS.md").write_text(
        render_adapter(root_claude.read_text(encoding="utf-8"), is_root=True),
        encoding="utf-8",
    )

    # rmtree then rebuild, so a skill deleted from the authority disappears from
    # the adapter too. Guarded by the empty-scan refusal above.
    if agents_dir.exists():
        shutil.rmtree(agents_dir)
    for src in sources:
        rel = src.relative_to(claude_dir)
        dst = agents_dir / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        dst.write_text(
            render_adapter(src.read_text(encoding="utf-8"), is_root=False),
            encoding="utf-8",
        )

    print(f"sync-agents: regenerated AGENTS.md + {len(sources)} skill(s) from the authority")
    return 0


def self_test() -> int:
    """Prove the check fires, and prove it does not fire on a correct mirror (R1).

    A check that never fires and a check that always fires are equally useless,
    so both directions are observed. Five observations on temp fixtures.
    """
    failures: list[str] = []

    def verdict(root: Path) -> int:
        return check(root, [])[0]

    def make_fixture(tmp: Path) -> Path:
        root = tmp / "repo"
        (root / ".claude" / "skills" / "demo").mkdir(parents=True)
        (root / "CLAUDE.md").write_text(
            "# demo — Claude Code Conventions\n\n"
            f"{AUTHORITY_START} `CLAUDE.md` is the authority; `AGENTS.md` is the adapter.\n\n"
            "Body referring to CLAUDE.md → \"Working style\".\n",
            encoding="utf-8",
        )
        (root / ".claude" / "skills" / "demo" / "SKILL.md").write_text(
            "---\nname: demo\n---\n\nSee CLAUDE.md → \"Build & test\".\n", encoding="utf-8"
        )
        sync(root)
        return root

    tmp = Path(tempfile.mkdtemp(prefix="check-skill-mirrors-selftest-"))
    try:
        # 1. A freshly generated pair is in sync, and the substitution alone
        #    must NOT be reported — otherwise every repo fails forever.
        root = make_fixture(tmp)
        if verdict(root) != 0:
            failures.append("fired on a correctly-generated mirror")

        # 2. The generated adapter really did substitute, so observation 1 was
        #    not passing because both sides are byte-identical.
        adapter = (root / ".agents" / "skills" / "demo" / "SKILL.md").read_text()
        if "AGENTS.md" not in adapter:
            failures.append("sync did not substitute the conventions filename")

        # 3. The authority declaration is copied VERBATIM, not substituted —
        #    the inversion trap the module docstring names.
        root_adapter = (root / "AGENTS.md").read_text()
        if f"{AUTHORITY_START} `CLAUDE.md` is the authority" not in root_adapter:
            failures.append("sync substituted the exempt authority declaration")

        # 4. Content drift in a skill is caught.
        drifted = tmp / "drift"
        shutil.copytree(root, drifted)
        p = drifted / ".agents" / "skills" / "demo" / "SKILL.md"
        p.write_text(p.read_text() + "\nA line the authority does not have.\n")
        if verdict(drifted) != 1:
            failures.append("did not fire on a drifted skill")

        # 5. A skill missing from the adapter is caught.
        missing = tmp / "missing"
        shutil.copytree(root, missing)
        (missing / ".agents" / "skills" / "demo" / "SKILL.md").unlink()
        if verdict(missing) != 1:
            failures.append("did not fire on a skill missing from the adapter")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    if failures:
        print("check-skill-mirrors --self-test: FAILED", file=sys.stderr)
        for f in failures:
            print(f"  {f}", file=sys.stderr)
        return 1
    print("check-skill-mirrors --self-test: 5 observations, all as expected")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--sync", action="store_true", help="regenerate the adapter from the authority")
    ap.add_argument("--self-test", action="store_true", help="prove the check can fail (R1)")
    args = ap.parse_args()

    if args.self_test:
        return self_test()
    if args.sync:
        return sync(ROOT)

    summary: list[str] = []
    code, problems = check(ROOT, summary)
    if problems:
        print("instruction mirrors: DRIFTED", file=sys.stderr)
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        return code or 1
    print("instruction mirrors: " + "; ".join(summary))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
