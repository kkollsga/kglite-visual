---
name: clean-comments
description: Coordinator-run comment cleanup over a measured scope — the invoking agent measures comment density, briefs one sub-agent per dense file to delete zero-information comments, compress low-density ones and fix false claims (R17), then verifies the whole diff mechanically, never from worker self-reports. Deliberately smaller than a phased plan — no branch ceremony, no plan doc. Run on a subtree after a large program lands, when review keeps hitting stale comments, or on request.
---

# clean-comments

Make the comments in a measured scope **true and lean**: delete what carries
no information, compress the rest to what it carries, fix comments the code
contradicts (`R17`), and never touch what the tooling reads (`R18`).

The steady state is R17's same-change duty (CLAUDE.md → "Code health").
This skill handles residue; heavy residue after a recent cleanup means that
duty is being skipped.

## 0. Shape of the run

The invoking agent is the **coordinator**: it measures, briefs, dispatches,
verifies, reports — and does not edit comments itself. Sub-agents (**workers**)
do the edits, one file each, because de-duplication needs the whole-file read
and self-reports need an independent checker. One exception: if measurement
returns ≤ 2 files, skip the workers and apply the brief yourself — a
coordinator with one worker is ceremony. If delegation is unavailable or not
permitted, apply the brief locally and inspect the final diff separately.
Capture each selected file's current bytes before editing, including user edits.
Verification and rollback use this baseline, never blindly HEAD.

Invocation authorizes the whole run (`R12`): it ends in the report or a named
blocker, never in "workers are running".

## 1. Measure first, and be ready to stop

Measure the requested scope (or the touched files when none is specified),
not the whole repository by default. Collect candidate counts with `rg -c`
without suppressing errors: status 1 means no matches, status 2 means the
measurement failed. Store and inspect the result before sorting; a downstream
`head` exit status cannot verify the scan. Counts are a heuristic and include
lookalikes inside strings, so read selected files before classifying comments.

Compute the total across the entire selected scope before taking the **head**:
the files that jointly hold roughly half its comment lines.
**Stop rule, decided before counting (R13):** if the head is empty or trivially
small, report "already lean — nothing to do" and stop. A cleanup that runs
regardless of what measurement says is a formality with a diff attached. A
heavy head right after a recent cleanup is itself the finding: R17's
same-change duty is being skipped — say so in the report.

Count Rust/Python line comments and TypeScript/JavaScript block continuations.
Report Rust and frontend counts separately; measure the full selected scope
before choosing the head, without truncating the scan with `head -40`.

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
can introduce a shared executor defect that optimized-vs-naive comparison
cannot catch (that comparison does catch optimizer divergence);
and a repeated comment that is a **local contract** rather than a duplicate —
eight identical arena-guard preconditions were kept by four independent agents
in KGLite because collapsing them parks the protocol in one arbitrary
function.

**What reads our comments (`R18`) — hands off, or handle deliberately:**

> **Two readers (first added with P1, second with P2).** `R18` says a
> *missing* reader list stops the run; this one is maintained. There is still
> no lint-allowance checker, no comment-suppressed lint in force and no
> docstring rendered as `--help`.
>
> - **`ts-rs` mirrors `///` doc comments into generated TypeScript.** Every
>   `///` on a `#[derive(TS)]` type or field in
>   `crates/kglite-visual-core/src/messages.rs` is copied verbatim into a
>   TSDoc block in `frontend/src/generated/*.ts`. Editing or deleting one
>   changes a generated file, and `make check-generated-ts` fails until the
>   regeneration is staged — so a comment cleanup there is a code change, not
>   a comment change. Regenerate with `cargo test -p kglite-visual-core` in
>   the same commit; never hand-edit the `.ts` (its header says so).
>   This is the **published-contract mirror** shape described below, and the
>   published artifact is the frontend's compile-time contract.
>
> - **The protocol baselines are generated exact artifacts (P2).**
>   `crates/kglite-visual-core/tests/protocol_baseline.rs` generates
>   `frontend/src/generated/protocol-constants.ts` (guarded by
>   `make check-generated-ts`) and
>   `crates/kglite-visual-core/tests/baselines/framing.golden` (guarded by
>   `make check-protocol-baseline`). Neither file's "do not edit" header is
>   decorative: both are exact baselines, and both are rewritten by
>   `cargo test -p kglite-visual-core`.
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

1. **Comment-only check before formatting, against captured pre-edit bytes.**
   Rust/TypeScript: compare token streams with a lexer that distinguishes comments
   from string/raw-string contents. Prefixes alone do not prove equivalence. If a
   suitable lexer is unavailable, inspect changed spans and state the limitation.
   Python: compare ASTs after removing docstrings, then separately check changed
   docstring consumers (`__doc__` can affect runtime output). Undo behavioral
   changes using only this run's edits; preserve pre-existing work.
2. **Run the formatters for real.** `cargo fmt` and the frontend formatter can
   move code after comment removal. Inspect their delta; only formatter-induced
   code motion is allowed. Never restore a whole file from HEAD over user edits.
3. **Run the gates for applicable readers in §2.** Regenerate and re-diff touched
   TypeScript/protocol contracts and run the relevant freshness/baseline checks;
   run clippy on touched crates. Preserve published reader contracts. Undo this
   run's causative edits for unexplained artifact changes, retaining user work.

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
  meets `R15`'s bar: a concrete failure, or it is not reported. A comment-cleanup
  request does not authorize unrelated implementation; preserve blocked defects
  and their evidence explicitly.
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
