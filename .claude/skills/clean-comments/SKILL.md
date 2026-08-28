---
name: clean-comments
description: Coordinator-run comment cleanup over a measured scope — the invoking agent measures comment density, briefs one sub-agent per dense file to delete zero-information comments, compress low-density ones and fix false claims (R17), then verifies the whole diff mechanically, never from worker self-reports. Deliberately smaller than a phased plan — no branch ceremony, no plan doc. Run on a subtree after a large program lands, when review keeps hitting stale comments, or on request.
---

# clean-comments

Make the comments in a measured scope **true and lean**: delete what carries
no information, compress the rest to what it carries, fix comments the code
contradicts (`R17`), and never touch what the tooling reads (`R18`).

> **This repo has no code (2026-08-29), so this skill has nothing to run on
> yet.** It is installed now because the failure it repairs is *accumulated*,
> and the cheapest time to have the rule is before the first comment is
> written. The steady state is `R17`'s same-change duty (CLAUDE.md → "Code
> health"): a change that falsifies a nearby comment corrects it in the same
> change, and a change through commented code applies the information test to
> what it touches. **This skill is for the residue, and a heavy residue is
> itself the finding** that the same-change duty is being skipped — that is
> the headline of the report, not the line counts.

## 0. Shape of the run

The invoking agent is the **coordinator**: it measures, briefs, dispatches,
verifies, reports — and does not edit comments itself. Sub-agents (**workers**)
do the edits, one file each, because de-duplication needs the whole-file read
and self-reports need an independent checker. One exception: if measurement
returns ≤ 2 files, skip the workers and apply the brief yourself — a
coordinator with one worker is ceremony.

Invocation authorizes the whole run (`R12`): it ends in the report or a named
blocker, never in "workers are running".

## 1. Measure first, and be ready to stop

Count comment lines per file over the scope (default: the whole repo; a
subtree argument narrows it). This project has two comment syntaxes and they
must both be counted or the head is wrong:

```bash
# Rust + Python
rg -c --no-messages '^\s*(//|#)' -t rust -t py <scope> | sort -t: -k2 -rn | head -40
# TypeScript / JavaScript, including block-comment continuation lines
rg -c --no-messages '^\s*(//|/\*|\*)' -t ts -t js <scope> | sort -t: -k2 -rn | head -40
```

Take the **head**: the files that jointly hold ~half the scope's comment
lines. **Stop rule, decided before counting (`R13`):** if the head is empty or
trivially small, report "already lean — nothing to do" and stop. A cleanup
that runs regardless of what measurement says is a formality with a diff
attached.

Expect the two languages to split differently and **report them separately** —
a generated-ish protocol encoder and a hand-written renderer adapter do not
have the same right answer, and a single head list ranks them against each
other for no reason.

## 2. Assemble the worker brief (once, fixed)

**The two tests, per comment paragraph** — *does this add a fact the reader
cannot get from the code or an earlier paragraph?*

- Zero information → **delete**: restates the next line or the signature,
  generic banner, self-referential bookkeeping, dead scaffolding.
- Low density → **compress** to the information carried: repetition across
  paragraphs, throat-clearing, narration of the journey, over-explained
  mechanics, four variations of one example, hedging.

**"Keep the fact, drop the label" is not the test** and must not be
substituted for it: it preserves volume by construction — 231 files, −0.2%,
versus −12.4% on 104 files for the information test.

**The floor — never delete:** why-not-what; invariants, safety preconditions,
lock ordering; **data-format lifecycle** (how an older `.kgl` is detected and
refused, and which `kglite` version wrote it — this project is a *reader* of a
format it does not own, and that is exactly where a deleted comment costs a
user a legible error); **protocol-version handling** on either side of the
wire, for the same reason; regression rationale in tests — the reason the test
is not deletable; bail reasons in any planner-like code, where deleting one
invites a wrong-results regression a comparison-based corpus *cannot* catch;
and a repeated comment that is a **local contract** rather than a duplicate —
eight identical arena-guard preconditions were kept by four independent agents
in KGLite because collapsing them parks the protocol in one arbitrary
function.

**What reads our comments (`R18`) — hands off, or handle deliberately:**

> **Nothing. This enumeration is empty as of 2026-08-29, and empty is a
> maintained state, not a missing one.** `R18` says a *missing* reader list
> stops the run; an explicitly-empty one that is dated does not. This repo has
> no lint-allowance checker, no comment-suppressed lint in force, no generated
> header, no published type stubs and no docstring rendered as `--help`.
>
> **Add a reader here in the same change that adds the reader.** The shapes to
> expect, each of which cost someone in this estate:
> - a **justification checker** — a lint-allowance gate that scans preceding
>   comment lines for a reason. KGLite's accepted any `//`-prefixed line ≥ 12
>   characters *including a `///` doc comment*, so on one function a
>   signature-restating doc was the only thing keeping the gate green. Worse,
>   its identities were keyed by **proximity** (searching the next 2000
>   characters for an item), so deleting ~45 comment lines silently re-keyed an
>   allowance. Key such identities to the **nearest following item**, and until
>   one is fixed, re-run it after any comment deletion.
> - a **presence-suppressed lint** — clippy's `collapsible_if` /
>   `collapsible_else_if` are suppressed by a comment's *mere existence* inside
>   the block; deleting it turns a comment cleanup into a `-D warnings`
>   failure. ESLint has equivalents.
> - a **published-contract mirror** — a doc comment copied verbatim into a
>   shipped artifact (type stubs, a generated header, `--help` text). Editing
>   the comment edits a published artifact and answers to release discipline,
>   not to comment hygiene.

**De-duplication: within-file only.** Keep the fullest statement at the
most-read location, point the others at it. That, not deletion, is the lever
on dense files — one KGLite header lost 15% with its ordering proof intact. A
fact repeated *across* files is flagged to the coordinator, never collapsed by
a worker; that decision needs cross-file sight. **This project will have one
fact stated on both sides of the wire** — the protocol's shape — and the
correct move there is a pointer from one side to the other, not a deletion.

## 3. Calibrate — one worker, and this gate can fail (`R1`)

Dispatch one worker on one representative head file. Read its **full diff**
against the brief. If it holds, fan out. If not, fix the brief and calibrate
again on a different file; after two failed calibrations, stop and surface the
diffs to the user. This gate failed for real on 2026-08-23 — the first
doctrine passed 231 files while moving volume 0.2%, and only a human read of
the calibration diff caught it.

## 4. Fan out

One worker per remaining head file, in parallel batches. Each dispatch is the
brief plus the file path plus this contract:

- Read the entire file before editing anything.
- Comment and doc lines only. Apply the two tests per paragraph; respect the
  floor and the reader list.
- Fix false comments (code-contradicted claims, expired "a later phase
  will…").
- Re-attach stranded doc blocks — a `///` or TSDoc block split from its item
  by a blank line, or an item inserted mid-block, documents the **next** item;
  the compiler stays quiet and the renderer renders confidently. Check doc
  fences balance, tracking fence **width** (a narrower fence inside a wider one
  is literal content; a parity count calls four unbalanced fences even — the
  detector was wrong twice before it was right).
- Return a structured result: lines deleted / lines compressed (from → to),
  false comments fixed, cross-file duplicates flagged, code defects noticed
  (not fixed), anything left untouched and why.

A worker that fails is retried once, then its file is reported unprocessed
(`R12`). **Never hand a worker a bulk fixer script** — one that matched every
fence opener rather than only malformed ones re-indented two well-formed
blocks in user-facing docs. Hand-fix or revert.

## 5. Verify mechanically — the diff, never the self-reports

Worker summaries were wrong twice in one day on the audit that bought this
skill (one claimed it left published C-ABI docs alone while two of its
compressions were inside one). In this order:

1. **Comment-only diff check, before any formatter.** Every changed line, both
   sides of the diff, is a comment line or blank. Rust/TS: audit that each
   changed line starts with a comment marker (`//`, `/*`, `*`, `///`).
   Python: parse both revisions (`git show HEAD:<file>` vs worktree), strip
   docstrings, compare ASTs — equal, or the file is reverted and reported.
2. **Run the formatters for real, not `--check`.** Comment removal *moves
   code*: `cargo fmt` collapses a block whose only content was a comment, and
   removing a separator reorders `use` statements; Prettier does the analogous
   thing to a TS block. Code motion introduced *by the formatter* is the only
   non-comment change allowed in the final diff.
3. **Re-run every gate that reads comments** (§2's list — empty today, so this
   step is currently a no-op and **must be reported as such**, not silently
   skipped). Re-diff any generated contract artifact the touched surfaces feed.
   An unexplained artifact diff reverts the file that caused it.

## 6. Report

- **Deletion and compression separately, per file — never one percentage.**
  The rate splits hard by file character: mechanical emitters lose 25–38%,
  specification files 2–11%, and **both are correct**. A −2% on a dense spec
  file is a success, not a shirk; an agent tuned on line count fails on
  exactly the files whose comments matter most.
- **Report Rust and TypeScript separately.** Different idioms, different doc
  conventions, different right answers.
- Findings fixed in-run are part of the diff; anything larger — code defects
  workers noticed, cross-file de-dup decisions, a reader-list gap — goes
  through `add-todo` under its entry rules. Anything reported as a finding
  meets `R15`'s bar: a concrete failure, or it is not reported.
- **Budget for findings, not just deletions.** Reading comments against code
  is effective static analysis: the audit that bought this skill surfaced ~60
  code-contradicted comments, three undocumented public API surfaces and two
  gate couplings — none visible to any existing gate.
- Offload the long form to `dev-docs/temp/clean-comments-report.md` and give
  the path; keep the inline summary lean.

## Relationship to phased-plan

Not part of one, on purpose. A first-ever whole-tree audit at campaign scale —
hundreds of head files, release integration — wraps this in a phased plan; for
everything else this skill is complete on its own.
