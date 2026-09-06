---
name: phased-plan
description: Run a large feature or refactor as a gated, phased project. Starts with an investigation phase (investigator agents on the code-review MCP map scale and impacted paths) — NOT standard plan mode — then builds a custom gated phased plan, creates a branch + draft PR for CI tracking, and executes each phase autonomously (code → test → lint → commit → push) until done. Ships only via the release skill.
---

# Phased plan

Use for substantial work that benefits from separately testable phases. Respect
the user's chosen workflow and existing scope approval; a small fix or review
alone does not require this process. This skill builds its own phased plan.
Use available harness tools; named agent roles are not required API names.

## Working dir: `dev-docs/` (gitignored)

All plans, scratch and intermediates live under **`dev-docs/`**. **The
canonical layout + lifecycle is `dev-docs/README.md` — read it; this is only
the phased-plan-relevant subset:**

- This project's plan → **`dev-docs/plans/<slug>.md`** (durable).
- Design choices/trade-offs you weigh → **`dev-docs/designs/`** (durable).
- Open threads → a lean one-line backlink in **`dev-docs/todos.md`** (detail in
  the linked durable doc, never inline; `add-todo` owns the shape).
- **Offload large output to `dev-docs/temp/` and report the path** (>1-day
  purge) instead of printing it.
- Measurements: harnesses → **`dev-docs/bench/scripts/`** (tracked in git),
  regression rows → **`bench/results/results.csv`**, heavy generated artifacts
  → **`bench/out/`** (>14-day purge; never write artifacts next to the
  script).

## Doctrine sync — before planning

Follow `../doctrine/learn-from-us.md` → **Doctrine sync procedure** against
`dev-docs/.doctrine-synced`, or reuse completed evidence from this session.
Compare numeric semver tuples; a missing marker requires an initial audit.
Deferred actions do not count as completed. Advance atomically only through
contiguous fully completed entries, after authority merges, sweeps and mirror
verification (`make sync-agents`, `make check-skill-mirrors`).

kglite-visual is a consumer: read and name the oracle version; never edit the
oracle from here (R14). Versioned corrections are source doctrine even where
reference snapshots normally originate in KGLite. Preserve local improvements
while merging corrections into `.claude/skills/` and `CLAUDE.md` (R7).

## Phase −1 — Check existing work

Read `dev-docs/todos.md` for relevant ongoing work and avoid duplicates. Run
`dev-docs-cleanup` when stale entries impede this task and cleanup is authorized;
otherwise proceed. Reuse existing permission to incorporate relevant actions.

## Phase 0 — Investigation (get a feel for scale before committing to a plan)

- Investigate before choosing implementation. Before scope approval, source
  changes, branches and PRs wait; a reviewable plan and bounded scratch probes
  in the session scratch area or documented dev-docs tier are permitted.
- Use the code-review MCP for structural questions; verify active root and graph
  freshness. If unavailable or incomplete, report that limit and inspect source.
  Delegate independent subsystem investigations when the harness permits it;
  otherwise investigate locally. Bound workers to read-only source and scratch
  fixtures. Report affected structure, callers, hidden couplings, test coverage,
  scale and design objections. Phase 0/1 settles design before implementation.

- **Investigate outward dependencies too.** Check the actual pinned `kglite`
  API (including its Python stub where applicable), the cosmos.gl API, and the
  architecture plan against both. If the local graph has no relevant structure,
  report that limit. An API assumed by the plan but absent from the pinned
  version is a concrete planning finding.
- **Probe behaviour before preserving it.** For a behaviour-preserving
  refactor, write a throwaway scratch script that exercises the paths you are
  about to move and capture their *actual* outputs — don't trust your mental
  model. In one KGLite refactor this surfaced three latent bugs the structural
  investigation missed; catching them before planning beats discovering them
  mid-execution.
- **Confirm your intended safety net catches *this class* of change.** An
  existing harness can be the wrong net. For this project the recurring trap
  is a net that only sees one side of the wire: a server-side test of the
  protocol encoder cannot catch a renderer that misreads the buffer, and a
  frontend test against a hand-written fixture cannot catch an encoder that
  stopped producing that shape. Decide the net in Phase 0, not after writing
  the wrong one.
- **Phase 0's cost attributions are hypotheses, and the record must say so.**
  "The time is in X" written before measuring is a lead, not a finding. When a
  later measurement phase falsifies one, the plan doc records the
  falsification **next to the original claim** — never quietly re-word the
  claim to match the result.
- Synthesize into a scale read: small/medium/large, risk hot spots, what could
  invalidate a naive plan.

## Phase 1 — Build the gated phased plan

- Write the plan to **`dev-docs/plans/<slug>.md`** (the durable copy; the PR
  description in Phase 2 mirrors it as a checklist).
- Break the work into numbered phases. Each phase must be independently
  **buildable, testable, committable** (bisectable).
- For each phase spell out: the change, the tests that prove it, the green
  gate. The gate is the suites **chosen to catch what that phase could
  break** — its touched surface plus that surface's direct consumers — not a
  fixed list and not everything; the full battery runs once at the end.
- **A measurement phase carries a stop rule that can retire the work**
  (`R13`). Write, *before* measuring, the result that closes the item
  **instead of** implementing it ("if the GPU layout holds interactive frame
  times at the largest slice the response bound permits, the Rust-side layout
  is not built"). A measurement phase whose only possible outcome is "proceed"
  was never a decision point. **Checkable: the stop rule is in the approved
  plan, dated before the measuring phase ran.** One composed after the numbers
  are in is a rationalisation of the outcome.
- No phase bumps package versions or promotes the release CHANGELOG block.
  Dependency/feature manifest changes within the approved scope are permitted;
  shipping is the `release` skill's job.
- **Challenge the plan once before presenting it.** (a) List the factual
  claims it rests on — paths, call sites, API behaviour, cost attributions —
  and verify each against the code (or the upstream API), recording the
  evidence in the plan doc. Phase 0's attributions are hypotheses until
  re-checked *as written into the plan*, where a stale one now reads as
  settled. (b) Run one pre-mortem: "this plan shipped and failed — why?", 2–3
  concrete scenarios. A scenario that names a real failure changes a phase,
  adds a test, or becomes a stop rule; one that cannot is a design preference
  — argue it in the approval loop, unlabelled. **No severity tiers**:
  severity labels are how preferences get laundered, and planning needs only
  the binary *changes the plan* or *argued and settled*.
- Present a concrete plan for scope/design decisions not yet authorized and
  invite revision. Record approval already supplied; do not ask for it again.
- **This is the stage where design critique belongs — raise it now or hold
  it.** "I would have designed this differently", "this should be split", "use
  X instead of Y", "that boundary is in the wrong place": all **in scope
  here**, from the user, from an investigator agent, and from you. Argue it,
  settle it, write the outcome into the plan. It is in scope *only* here. Once
  approved, the diff is measured against **this plan** and against
  correctness — never against a design someone preferred afterwards (CLAUDE.md
  → "Code review — report what is broken"). A design objection arriving at
  review time is late; it becomes input to the *next* plan.
- Begin implementation once scope is authorized. A required pending decision
  remains pending until answered; elapsed time is not approval.

- Once approved, **do not pause between phases.**

## Phase 2 — Branch + draft PR (the CI tracking handle)

- Create the user-requested branch or follow the harness naming convention
  (`codex/<slug>` in Codex); do not implement directly on `main`.
- **Exactly one branch + one draft PR per plan. Phases are commits, never
  sub-branches** — no per-phase or per-workstream branches merged back later
  (one such plan left 8 stale branches in this estate). When the plan ships,
  the release skill deletes the branch, local + remote.
- **If a phase needs an isolated tree, it goes under
  `../kglite-visual-worktrees/<name>`** — one sibling directory holding every
  worktree, never a loose `../kglite-visual-<name>` scattered beside the real
  projects in `Rust/` (that habit left 7 worktrees totalling ~46 GB in the
  estate root). A fresh worktree inherits neither a build-cache symlink nor an
  installed `node_modules`; set both up before its first build or it
  cold-builds onto whatever volume the workspace sits on. Removal follows
  `dev-docs-cleanup` §6, including verified staged/untracked recovery and
  preservation of detached commits; do not substitute a plain `git diff`.
- **Run the CI-only tier once before that first push**, not per phase
  (CLAUDE.md → "Build & test"). A long-lived branch can accumulate weeks of
  work that CI rejects on contact.
- Push the branch and **open a draft PR against `main`**. This is what makes
  CI run on the branch while nothing publishes.
- Put the phased plan into the **PR description as a checklist** (one box per
  phase).

## Phase 3 — Execute each phase (the autonomous loop)

For every phase, in order:

1. Implement the phase's code + its tests.
2. **Local green gate before committing:** run `make gate`, then the targeted
   suites chosen to catch what this phase could break — the touched surface
   and that surface's direct consumers. Not a fixed list, and not the full
   battery: that runs once over the plan's union at the Final branch gate.
   Build the smallest touched surface — a package-scoped `cargo test -p …
   <filter>` for a Rust-only change, the frontend's own test command for a
   frontend-only change. Do not reproduce CI locally.
   - **"Green" means you saw the command's own exit status, never a
     pipeline's** (CLAUDE.md → "A reported status is not the result").
   - **A step that does not exist is ABSENT, not green.** `make gate` prints
     an ABSENT line for each planned step; carry those lines into the phase's
     report rather than letting them read as passes.
   - **This project has two toolchains and one embedded artifact.** A phase
     that changes the protocol touches *both* sides, and a phase that changes
     the frontend must rebuild the bundle before any test that loads the
     binary means anything — a stale bundle inside a fresh binary looks
     exactly like a backend bug.
   - **A NEW GATE IS NOT TRUSTED UNTIL YOU HAVE SEEN IT FAIL** (`R1`). If the
     phase adds or changes a check — a test, a CI step, an assertion in a
     script — break the thing it guards, confirm it goes red, then restore.
     Reading a gate cannot tell you whether it works. Three ways a gate is
     born dead: **substring subsumption** (`assert "cmd" in block` also
     matches `cmd --self-test` — compare whole stripped lines), **comment
     subsumption** (the words you assert on also appear in the comment
     explaining them — strip comments before matching), and **`exit` inside
     `$( )`** (it kills only the subshell; the caller reads the empty output
     as 0). Also: a scan-based guard that finds zero files passes vacuously —
     assert the scan was non-empty. **Verify the probe, not just the result**
     — a mutation that silently edited the wrong text makes a working gate
     look broken.
   - **Cleanliness:** any new file-writing step (a bench capture, a fixture
     dump, a generated graph) targets a purged tier or the session scratchpad,
     or extends the cleanup gate **in the same phase** (`R4`).
   - **If you review the phase's own diff, review it for failures only** —
     "does this break something", plus "does it do what the plan said". The
     not-findings list and the severity-label rule are CLAUDE.md → "Code
     review"; that argument was open in Phase 1 and is closed now, so a design
     opinion here is input to the next plan, not a reason to rework this
     phase.
3. Update `CHANGELOG.md` `[Unreleased]` for user-visible changes (not the
   version block).
4. **Commit** the phase (`feat(...)` / `refactor(...)` / `fix(...)`), one
   commit per phase. **Then run `make prune-target`** (doctrine 0.1.9): a
   bound checked only at milestones is not a bound — one heavy day of
   feature-variant builds grew an estate repo's `target/` by ~95 GiB and
   killed an agent's shell with ENOSPC before the release-time prune ever
   ran. The gate makes it a free no-op on a lean tree; a mid-plan cold
   rebuild is cheaper than a mid-phase ENOSPC. Know the meter lies low:
   `du` under-reports what a clean actually frees (measured here: du said
   12.0 GB, the clean removed 14.8 GiB).
5. **Push at checkpoints, not per commit.** Every branch push starts a full CI
   run; batch every 2–3 quick phases, at a risky milestone worth CI
   confirmation, or before stepping away — and always once at plan
   completion. Tick completed phases' checkboxes in the PR description when
   you push.
6. **Retire any `todos.md` action this phase completed** — at phase-commit
   time, not as a separate pass:
   - **Fully done** → remove the backlink only after preserving required evidence;
     archive unused detail via `dev-docs-cleanup` §5, with a unique completed
     manifest and a fresh seven-day grace period. A plain move retains old mtime.
   - **Partially done** → leave the doc; trim the entry to what is left.
   - **Shared doc** (one `plans/` file backing several todos, e.g.
     `consider-for-future.md`) → remove only the closed entry; move the doc to
     `bin/` *only* once no live backlink points at it.
   `dev-docs/` is gitignored, so this is local bookkeeping alongside the
   commit, not part of the git change. Note each retirement in the report-out.
7. Continue into the next phase. If a phase's CI comes back red, fold the fix
   into the loop before the project merges — don't leave the PR red.

If a targeted check is silent for roughly three minutes, inspect its exact
process, CPU and output-artifact timestamp once. A compiler asleep at 0% CPU
with no artifact progress gets one final 60-second window, then stop that exact
process tree. Stop immediately for an unexpected dependency sync or an
unrelated feature tree. Diagnose the command first; do not keep polling or
restart under another profile.

Stop mid-plan only for a genuine blocker (unfixable test, architectural
surprise invalidating a later phase). Surface it; don't push through.

**Bugs found during the plan:** reproduce the failure, confirm the root cause
and scan for its class. Fix defects within the authorized scope in a separate
bisectable change where needed. A materially broader fix, separate investigation
or blocked defect is preserved through `add-todo` under **Bugs (surfaced, not yet
fixed)** with reproduction and reason. Do not force unrelated implementation to
finish this plan. Keep missing capabilities and unmeasured hypotheses distinct.

An engine, Cypher or `.kgl` format defect belongs to KGLite, whose source remains
read-only from here. Preserve our reproduction; use `notify` for an authorized
send or record a pending draft. State the disposition in the report.

Use the repository's configured build-cache paths, including any shared target
symlink. Check free space before builds; do not override `CARGO_TARGET_DIR` or
`SCCACHE_DIR` based on an assumption about disk speed.

Before Python correctness tests, rebuild the debug extension if any linked
Rust engine/core/CLI code changed or a performance run left a release extension
installed. A current debug extension can be reused. Rebuild the production
frontend before embedding it in either CLI or Python artifacts.

## Phase 4 — Perf gate (only if the plan touched perf-sensitive paths)

If any phase touched the protocol encoder, the expansion/query path, the
layout, or the renderer's data path, run new + existing measurements **exactly
per CLAUDE.md "Performance protocol"** — the release-profile requirement, the
per-cell statistic (p95/p99 for frame time, mean-of-first-events for
time-to-first-paint, exact/median for deterministic quantities), the control
cells and their ≥2× margin, the two agreeing runs, and the
threshold-adjacent retake — before declaring done. Record the numbers **and
the machine state they were taken under**. Compare against published artifacts
from an isolated environment, never a source-built ancestor. Fix regressions
within scope; explicitly preserve any blocked fix.

For plans that never touched those paths, skip this phase and say so.

## Final branch gate — required before Report out / release

After the last phase, run `make gate`, the union of the plan's targeted tests,
and — **once, here, over that union** — the full battery the per-phase gates
deliberately skipped. This is the completion union, and it is what lets a
phase gate be narrow. Then run only the surface-conditional extras the diff
requires. Commit any fixes, push the completed branch once, and let the full
PR CI on that exact HEAD perform the broad matrix in parallel. Do not begin
release work while CI is pending or red.

**Any review at this gate is failures-only, against the plan**: did every
phase do what the plan said, and does anything now break — concrete failing
input, state or consequence named. A design objection surfacing here does not
block the branch — record it as input for the next plan
(`plans/consider-for-future.md` + a `todos.md` backlink) and ship.

## Report out (when the plan completes, before Ship)

Keep it under the 400-token rule and link the plan doc for detail:

- **Phases** done (one line each) + the PR link / final commit shas.
- **Bugs surfaced** and each disposition: fixed in a named phase; sent upstream
  with the note path; or preserved with reproduction and a concrete scope/blocker
  reason. Distinguish pending drafts from sent notes. Mandatory even if empty.

- **Perf gate** result (per-cell statistic + verdict: flat / regression /
  improved), or "not applicable — no perf-sensitive path touched".
- **Gate steps that were ABSENT** rather than green.
- **`todos.md` changes**: actions retired, carried-over items added.
- **Plan deviations** (inserted phases, re-scopes) and why.

## Phase 5 — Ship (only on request)

When the user asks to ship, run the **`release`** skill. The release skill
starts only after the completed branch's full CI is green. This skill never
bumps a version and never pushes `main`.

## Notes

- Keep responses under 400 tokens; write long diffs/logs to a file, report the
  path.
- Branch pushes during the loop are routine (no publish). Only the `main` push
  at release time is the approval-gated one.
- **If context is genuinely running out, hand over at a clean boundary — never
  start a large phase tired.** A handover is: finish the phase in flight, gate
  it, commit, push, and write the state into `dev-docs/plans/<slug>.md` so the
  next agent resumes at a phase start. This is not a licence to pause between
  phases (that stays forbidden) — it is the rule for *where* an unavoidable
  break lands, and mid-phase is the one place it must not.

## dev-docs is the sprint's steering material

The full rule — why detail in the linked docs is load-bearing, the **"would an
agent picking this up act differently for having read it?"** test that decides
what to write and what to prune, and the two consequences of `dev-docs/` being
gitignored (durable decisions also go somewhere tracked; never cite a
`dev-docs/` path from committed files) — is CLAUDE.md → "dev-docs steers the
sprint; commits are the durable record".
