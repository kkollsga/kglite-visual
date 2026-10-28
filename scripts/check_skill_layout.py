#!/usr/bin/env python3
"""Assert the agent-skills layout (doctrine R7, as restated in 0.1.13).

``AGENTS.md`` is the one instructions file and ``.agents/skills/`` the one real
skills folder; ``.claude/skills`` is a symlink to ``../.agents/skills`` so Claude
Code finds the same files. Nothing is regenerated or compared: a second real
tree is the failure this check exists to catch, because it is what drifts.

The skills folders are gitignored (workstation-local), so this check reads the
checkout it runs in and is not part of CI.

Exit codes: 0 layout correct, 1 layout wrong, 2 the checker could not do its job.
"""

from __future__ import annotations

import argparse
import shutil
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def check(root: Path) -> list[str]:
    problems: list[str] = []
    agents_md = root / "AGENTS.md"
    agents_skills = root / ".agents" / "skills"
    claude_skills = root / ".claude" / "skills"

    if (root / "CLAUDE.md").exists():
        problems.append("CLAUDE.md exists: AGENTS.md is the only instructions file")
    if not agents_md.is_file() or agents_md.stat().st_size == 0:
        problems.append("AGENTS.md is missing or empty")
    if not agents_skills.is_dir() or agents_skills.is_symlink():
        problems.append(".agents/skills must be the real skills directory")
    elif not any(agents_skills.rglob("SKILL.md")):
        problems.append("no SKILL.md under .agents/skills — the scan is broken")
    if not claude_skills.is_symlink():
        problems.append(
            ".claude/skills must be a symlink to ../.agents/skills"
            " (`ln -s ../.agents/skills .claude/skills`)"
        )
    elif claude_skills.resolve() != agents_skills.resolve():
        problems.append(f".claude/skills points at {claude_skills.resolve()}, not .agents/skills")
    return problems


def self_test() -> int:
    """A correct layout passes; each way of leaving it fails (R1)."""
    failures: list[str] = []

    def fixture(tmp: Path, name: str) -> Path:
        root = tmp / name
        (root / ".agents" / "skills" / "demo").mkdir(parents=True)
        (root / ".agents" / "skills" / "demo" / "SKILL.md").write_text("---\nname: demo\n---\n")
        (root / ".claude").mkdir()
        (root / ".claude" / "skills").symlink_to("../.agents/skills")
        (root / "AGENTS.md").write_text("# demo\n")
        return root

    tmp = Path(tempfile.mkdtemp(prefix="check-skill-layout-selftest-"))
    try:
        good = fixture(tmp, "good")
        if check(good):
            failures.append("fired on a correct layout")

        real = fixture(tmp, "real-tree")
        (real / ".claude" / "skills").unlink()
        shutil.copytree(real / ".agents" / "skills", real / ".claude" / "skills")
        if not check(real):
            failures.append("did not fire on a second real skills tree")

        wrong = fixture(tmp, "wrong-target")
        (wrong / "other").mkdir()
        (wrong / ".claude" / "skills").unlink()
        (wrong / ".claude" / "skills").symlink_to("../other")
        if not check(wrong):
            failures.append("did not fire on a symlink to the wrong folder")

        stale = fixture(tmp, "claude-md")
        (stale / "CLAUDE.md").write_text("# stale\n")
        if not check(stale):
            failures.append("did not fire on a leftover CLAUDE.md")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    if failures:
        print("check-skill-layout --self-test: FAILED", file=sys.stderr)
        for f in failures:
            print(f"  {f}", file=sys.stderr)
        return 1
    print("check-skill-layout --self-test: 4 observations, all as expected")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--self-test", action="store_true", help="prove the check can fail (R1)")
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    problems = check(ROOT)
    if problems:
        print("skill layout: WRONG", file=sys.stderr)
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        return 1
    print("skill layout: OK — AGENTS.md, .agents/skills real, .claude/skills linked")
    return 0


if __name__ == "__main__":
    sys.exit(main())
