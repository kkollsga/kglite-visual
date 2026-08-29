---
name: phased-plan
description: Run a large feature or refactor as a gated, phased project. Starts with an investigation phase (investigator agents on the code-review MCP map scale and impacted paths) — NOT standard plan mode — then builds a custom gated phased plan, creates a branch + draft PR for CI tracking, and executes each phase autonomously (code → test → lint → commit → push) until done. Ships only via the release skill.
---

# Phased plan

For any large feature or non-trivial refactor. **Demand this skill** when the
user kicks off such work. Do **not** use standard plan mode
(`EnterPlanMode` / `ExitPlanMode`) — this skill builds its own gated phased
plan instead of the harness's generic plan.

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

## Doctrine sync — first action of the run, before Phase −1

The estate's rules live in the sibling `doctrine` repo and are versioned.
**Pull them forward before planning anything**, so a plan is never built on
doctrine this repo has already been told is superseded.

1. Read **`../doctrine/VERSION`** and **`dev-docs/.doctrine-synced`**. If the
   marker is absent, create it with the current version and note in the
   report-out that this was a first sync.
2. **Versions equal → done.** That is the normal case and it costs **one file
   read**; it is never worth skipping to save time, and "we're probably
   current" is not the check.
3. **Doctrine ahead → read `../doctrine/CHANGELOG.md` forward from the
   marker** and act on every entry newer than it. Each item carries exactly
   one action class:
   - **`[skills-update]`** — merge the change into this repo's declared
     **authority** (see the Authority line at the top of the conventions
     file) and regenerate the adapter from it in the same action; verify with
     `make check-skill-mirrors`. Never hand-port into an adapter — that is
     what `R7` measures.
   - **`[local-sweep]`** — run the check command the entry states. If it comes
     back clean, say so and move on. **If it fails, the sweep becomes Phase 0
     work of *this* plan** — scoped, listed and visible in the plan doc, never
     a silent side-task folded into an unrelated phase.
   - **`[info]`** — nothing to do.
4. **Write the new version to `dev-docs/.doctrine-synced` only after those
   actions completed.** A marker written first permanently hides the entry it
   skipped: the next run compares against it and sees nothing.

**kglite-visual is a pure consumer of doctrine**, unlike KGLite, which is the
source those entries are generated from. So: read the oracle first, name the
version you read, and never "fix" the oracle from here (`R14`). A divergence
you find between doctrine's reference copies and this repo's installed copies
is either a **local improvement** (a candidate to upstream) or **staleness**
(fixed *from* the oracle) — say which, then act on the authority rather than
on whichever copy you happen to have open.

## Phase −1 — Start fresh (recommend cleanup first)

Before investigating, **recommend the user run the `dev-docs-cleanup` skill**
so we start from a tidy `dev-docs/` and a current `todos.md`. Relevant
carried-over todos can then be folded into this plan — **only with the user's
go-ahead.** If they decline, proceed without it.

## Phase 0 — Investigation (get a feel for scale before committing to a plan)

- **Do not enter plan mode.** Investigate first, plan second.
- **Read-only until approval.** The main loop makes **zero edits** during
  Phase 0 and Phase 1 — no branch, no PR, no code, no file writes. All
  investigation goes through **read-only `Explore` agents**; nothing touches
  the working tree until the user approves the plan in Phase 1.
- Kick off **investigator agents** equipped with the **code-review MCP**
  (Cypher over the code graph + ripgrep). Fan them out in parallel — one per
  subsystem / suspected blast-radius area. **Scale the count to blast
  radius:** 1–2 for a medium change, more only for a genuinely large one.
  Have them report: structure of the affected area, impacted paths / callers,
  hidden couplings, existing test coverage, a rough size estimate — **and any
  structural or design objection they hold.** Phase 0/1 is the only stage that
  will hear it: an investigator that thinks a boundary is wrong says so *now*,
  because review will not accept it later.
- **While this repo is small or empty, the investigation looks outward.** The
  code graph has nothing to map, and that is a fact to report, not a failed
  investigation. Investigate instead: the actual `kglite` API surface (the
  crate's docs and its Python stub, which is the stated source of truth for
  its API), the actual cosmos.gl API, and the architecture plan read against
  both. An investigator that reports "the plan assumes an API that does not
  exist in the version we would pin" has done the most valuable thing this
  phase can do.
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
- No phase touches version manifests or CHANGELOG promotion — shipping is the
  `release` skill's job.
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
- Present the plan, then **invite revision: ask the user to revise or approve,
  and loop on their feedback until they approve.**
- **This is the stage where design critique belongs — raise it now or hold
  it.** "I would have designed this differently", "this should be split", "use
  X instead of Y", "that boundary is in the wrong place": all **in scope
  here**, from the user, from an investigator agent, and from you. Argue it,
  settle it, write the outcome into the plan. It is in scope *only* here. Once
  approved, the diff is measured against **this plan** and against
  correctness — never against a design someone preferred afterwards (CLAUDE.md
  → "Code review — report what is broken"). A design objection arriving at
  review time is late; it becomes input to the *next* plan.
- **Hard stop — wait for an explicit go-ahead.** Do not create the branch,
  open the PR, or write any code until the user says proceed. A simple proceed
  is enough. Until then, stay read-only.
- Once approved, **do not pause between phases.**

## Phase 2 — Branch + draft PR (the CI tracking handle)

- Create a feature branch: `feat/<slug>` or `refactor/<slug>` (never work the
  project directly on `main`).
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
  cold-builds onto whatever volume the workspace sits on.
- **Run the CI-only tier once before that first push**, not per phase
  (CLAUDE.md → "Build & test"). A long-lived branch can accumulate weeks of
  work that CI rejects on contact.
- Push the branch and **open a draft PR against `main`**. This is what makes
  CI run on the branch while nothing publishes.
- Put the phased plan into the **PR description as a checklist** (one box per
  phase).

> **PARTIALLY ABSENT: CI exists as a file and has never run, and there is
> still no remote (2026-08-29, after P6).** `.github/workflows/ci.yml` and
> `build_wheels.yml` are committed and pass `actionlint`, but no Actions run
> has ever happened and no PR has ever existed. Until the user creates the
> remote and the first run goes green, this phase is **not applicable, and
> saying so is the correct output** — never report a green PR check that does
> not exist (`R10` corollary). Work on `main` locally, keep the phase commits
> bisectable anyway, and note in the report-out that CI confirmation is
> outstanding. Delete this block in the same change that lands the first green
> CI run — not in the one that creates the remote, because a remote with a
> never-executed workflow is exactly the state this block describes.

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
   commit per phase.
5. **Push at checkpoints, not per commit.** Every branch push starts a full CI
   run; batch every 2–3 quick phases, at a risky milestone worth CI
   confirmation, or before stepping away — and always once at plan
   completion. Tick completed phases' checkboxes in the PR description when
   you push.
6. **Retire any `todos.md` action this phase completed** — at phase-commit
   time, not as a separate pass:
   - **Fully done** → remove the backlink line and move its supporting
     `plans/<doc>.md` to `dev-docs/bin/` (7-day grace).
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

**Bugs that surface mid-plan — no bugs left behind. Fixing is the default; the
backlog is for missing capability, never for a known defect.** The first
question a surfaced defect asks is *fix it now*, and the answer is yes unless
fixing it is genuinely impossible inside this plan. "Out of scope" is a reason
to give the fix its **own** phase, not a reason to file the bug and walk past
it.

First, **classify — bug or missing capability**, because only one may be
filed:

- A **bug** is a defect in behaviour that exists: a wrong result, a crash,
  data loss or corruption, a broken contract, a *measured* regression, a gate
  that cannot fail, a claim the code contradicts. A bug is **fixed**, never
  backlogged.
- A **missing capability** is a feature that was never built. *That* is what
  `plans/consider-for-future.md` is for.

Then fix, by where the bug lives:

- **In scope** — reproduce, confirm the root cause, fix it as its **own
  bisectable phase** (`Phase Nb`) with its own test and commit. Don't fold a
  behaviour change into a mechanical-refactor commit.
- **Out of scope** — still fix by default, as its own `Phase Nb`. Out-of-scope
  changes the *commit boundary*, not the decision to fix. File to
  `consider-for-future.md` only when fixing now is genuinely blocked, and then
  it is a *surfaced* bug with a `todos.md` backlink and a cheap regression
  assertion pinning it. Say **why** in the report-out.
- **Upstream** — a defect that traces to `kglite`, the Cypher dialect or the
  `.kgl` format is **not ours to fix**: KGLite is read-only from here. Pin it
  locally with a regression assertion if we can, route it via `notify` with a
  reproduction, and record it in the report-out.
- **Suspected perf bug** — an unmeasured perf change is not a fix, so it earns
  a measurement *before* the fix counts — but that measurement runs **in this
  plan**, not deferred.

Either way, record it in the **report-out** — a discovered bug never vanishes.

## Phase 4 — Perf gate (only if the plan touched perf-sensitive paths)

If any phase touched the protocol encoder, the expansion/query path, the
layout, or the renderer's data path, run new + existing measurements **exactly
per CLAUDE.md "Performance protocol"** — the release-profile requirement, the
per-cell statistic (p95/p99 for frame time, mean-of-first-events for
time-to-first-paint, exact/median for deterministic quantities), the control
cells and their ≥2× margin, the two agreeing runs, and the
threshold-adjacent retake — before declaring done. Record the numbers **and
the machine state they were taken under**. Fix regressions now, not in a
follow-up.

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
- **Bugs surfaced** during execution and each one's disposition — *fixed in
  Phase Nb* by default; *routed upstream* with the note path; *filed to
  backlog* only for a bug fixing-now was genuinely blocked, stating **why**
  (not "out of scope" — that is a location, not a reason). **Mandatory even
  if empty** ("no bugs surfaced").
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
