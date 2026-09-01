# kglite-visual — Claude Code Conventions

Interactive, high-performance visualization for `.kgl` knowledge-graph files
produced by [KGLite](https://github.com/kkollsga/kglite) (sibling repo, same
estate). A Rust workspace plus a TypeScript/WebGL frontend, shipped as a
localhost CLI and a Python wheel.

> **Status, 2026-08-29 (P11 landed on top of P0–P10): the artifact exists,
> proves itself, reads as a graph on real data, hands that graph to an
> agent as a picture a human can read — and lets the agent navigate it
> while a human watches.** The render is structure-aware, not one force
> layout: a neighbourhood is drawn as hop rings, a community-structured
> graph as packed islands, and anything shapeless falls back to the seeded
> force pass. Where the canvas cannot hold what was asked for — a hundred
> type names in a thumbnail, a name with no free cell near it — it draws
> what fits and **says what it dropped**, in the picture and in the JSON
> line.
> Three crates (`core`, `cli`, `py`), a Vite/TypeScript frontend that
> renders the type-level meta-graph through cosmos.gl — GPU force layout by
> default, a **server-computed static layout** the user or an agent can
> switch to (hop rings, packed islands, a held-still force pass, or a
> geographic map that puts a node where it actually is, over a coastline
> drawn at whichever of three scales the frame can resolve — the simulation
> stops, dragging goes off, and the server then knows where the points
> are), and the D2 deterministic mode behind an explicit
> `?deterministic=1` switch the e2e/bench suites pass — and drills into it, a versioned binary protocol (v4)
> with an exact framing baseline, an axum server on localhost with the
> frontend embedded, **a Python wheel whose `show()` runs that same server
> lib-linked into the extension**, **`kglite-visual render` /
> `POST /api/render`** — the meta-graph, a Cypher result or a bounded
> expansion drawn as a deterministic SVG or PNG with the app's own visual
> encoding and the truncation banner in the picture — **an MCP server at
> `/mcp` on the running instance** — thirteen tools an agent uses to show,
> expand, collapse, highlight, focus, re-arrange, re-colour and **export**
> the live view, with every change broadcast to every attached browser, so the
> user's screen follows the agent in real time — a source distribution
> that installs without Node, a CHANGELOG, and a gate that builds, lints,
> tests and
> **drives** all of it — in a headless browser, and for the built wheel in a
> fresh virtualenv outside the repo. **CI is live and green**
> (`.github/workflows/{ci,release}.yml`, remote
> `github.com/kkollsga/kglite-visual`): the first-ever CI run went red on a
> real SIGTERM-race bug the warm dev machine could not see — which is what
> a first run is for — and everything since is green, including the full
> wheel matrix (seven platform wheels + sdist, first try). **Published:
> v0.1.0 is on PyPI** (2026-08-29, trusted publisher, tag verified both
> sides) — `pip install kglite-visual` is real. And when a planned thing
> becomes real, delete its
> caveat *in the same change*: an honesty label nobody retires becomes a
> lie, which is how one estate repo still described itself as "brand-new, no
> code yet" two weeks after shipping a gate and 17 test files.

The rules cited by ID (`R4`, `R11`, `R16`, …) are the estate's numbered
invariants in `../doctrine/rules/RULES.md`. Cite them; don't paraphrase them
into something weaker.

**Authority:** `CLAUDE.md` is the authority this repo's agent instructions are
regenerated from, and `.claude/skills/` is the authority for `.agents/skills/`;
`AGENTS.md` and `.agents/` are generated adapters. Edit the authority and
regenerate in the same action (`make sync-agents`) — never edit an adapter.
(This line is exempt from the naming substitution — it names the authority
literally in every copy, per doctrine `R7`/`R14`. A substituted declaration
inverts itself and tells the adapter's reader to edit the adapter, which is
what two estate repos did on the day that procedure landed.)

## Working style

- **Evidence over assertion.** For a bug, reproduce it and confirm the **root
  cause with evidence** before fixing. For a behaviour-preserving refactor,
  probe the *actual* output first — don't trust your mental model.
- **No bugs left behind.** A defect you notice mid-task gets fixed (in scope,
  as its own bisectable commit) or gets its own phase — never silently stepped
  over, and never filed to the backlog. A **bug** is fixed; a **missing
  capability** is what `plans/consider-for-future.md` is for. If you catch
  yourself writing a bug into the backlog, that is the rule firing. A defect
  that traces to the `kglite` engine, the Cypher dialect or the `.kgl` format
  is routed to KGLite via `notify` — KGLite is read-only from here.
- **Offload, don't print.** Write long output (protocol dumps, bench tables,
  build logs, big diffs) to `dev-docs/temp/` (>1-day purge) or
  `dev-docs/bench/out/` and **report the path**. Keep responses under ~400
  tokens.
- **A reported status is not the result** (`R2`). Read the exit code of the
  thing you care about, never of something downstream of it. Four shapes that
  have each failed in the *reassuring* direction elsewhere in this estate, and
  will apply here the day there is a build:
  - **A pipe reports the last stage's status.** `cargo check … | tail` reports
    `tail`'s exit code. Use `set -o pipefail` or read `$?` from the command.
  - **`git add` with one bad pathspec stages NOTHING** — the good paths in the
    same invocation are discarded with it, the only complaint is on stderr,
    and the following commit succeeds while missing the change. Read back
    `git status --porcelain` / `git diff --cached --name-only`.
  - **`grep -c` exits 1 on a count of zero**, breaking the `&&` chain of a
    command that was only ever asking "how many?".
  - **A backgrounded command's result lives in its artifact**, not in the
    "done" line it echoed. Open the log the run wrote.
  - And **"it compiles" is a weak test**: a dependency floor can compile and
    still misbehave (below `anyhow` 1.0.47, `anyhow!("{e}")` compiles and
    prints the literal `{e}`). Where a version can compile yet misbehave,
    *run* it. This project will have two such floors — `kglite` and the
    frontend's renderer — and both cross a data boundary.
- **A verification must be able to fail** (`R1`). A gate, guard or assertion
  you have not seen go red on a deliberately broken input is not yet a gate.
  Break the thing it guards, watch it fail, restore. Three ways a gate is born
  dead: substring subsumption (`assert "cmd" in block` also matches
  `cmd --self-test`), comment subsumption (the words you assert on also appear
  in the comment explaining them), and `exit` inside `$( )` (it kills only the
  subshell; the caller reads the empty output as success). A scan-based guard
  that finds zero files passes vacuously — assert the scan was non-empty.
- **When a committed claim is retracted, grep for every place it was written**
  (`R3`). The retracted claim that bought this rule lived in three files; two
  were fixed and `dev-docs/todos.md` — the file that advertises itself as
  enough to brief a fresh agent — stayed wrong for hours.

## Planned architecture

The full shape, the seams that need contracts, and the proposed bootstrap
sequence are in the working folder's architecture plan (`add-todo` /
`phased-plan` know where; it is the first entry in `dev-docs/todos.md`). The
summary that belongs in standing rules, because it decides what code is
allowed to go where:

- **`kglite-visual-core`** — embeds the `kglite` crate. Sessions, Cypher,
  snapshots, the type-level meta-graph, bounded neighborhood expansion, four
  structure-chosen layout kernels (hop-ring radial, packed islands, seeded
  force, and an equirectangular geographic map for nodes with coordinates)
  plus a final separation pass, and a **transport-agnostic binary
  protocol** (typed-array buffers for topology and positions, JSON for
  metadata). *Transport-agnostic is a rule, not a description:* nothing in
  this crate may know it is talking to a WebSocket. It has three consumers
  planned and the encoder is the seam they share.
- **`kglite-visual-cli`** — an axum server on localhost serving an embedded
  static frontend via `rust-embed`, opening a browser. The tensorboard /
  marimo pattern: one binary, no install step.
- **`kglite-visual-py`** — PyO3 + maturin wheel. `kglite_visual.show(graph_or_path)`,
  Jupyter via anywidget or an iframe, in-memory graphs handed over with
  kglite's `to_bytes()` / `from_bytes()`.
- **Frontend** — TypeScript + Vite + **cosmos.gl** (`@cosmos.gl/graph`,
  MIT, OpenJS Foundation), a WebGL GPU force-layout renderer, speaking the
  binary protocol over WebSocket. Built to static assets and embedded; not
  published to npm. **Never install any `@cosmograph/*` package** — the
  `@cosmograph` npm family (including `@cosmograph/cosmos`, the engine's
  pre-donation name) is CC-BY-NC-4.0, incompatible with shipping this MIT
  app; the two packages share version numbers, so the name is the only
  guard, and a gate check enforces it (verified 2026-08-29).
- **A Tauri desktop shell is a later, optional wrapper** — over the finished
  core and frontend. If it ever becomes a second implementation of the
  protocol or the session model, that is evidence the core boundary is wrong,
  not that the shell needs more code.

**Progressive disclosure is the product, not a UI preference.** kglite's disk
mode reaches 100M+ nodes and no browser renders that. The entry screen is the
**type-level meta-graph** — labels and relationship types with counts, always
small, whatever the graph underneath. Drill-down happens through Cypher and
bounded neighborhood expansion. **The server decides what crosses the wire**,
and the bound is enforced in core, not in the UI: a guarantee the client
implements is not a guarantee. The choke point is
`core::expand::effective_bound`, and no input reaches the renderer around
it. A change that lets an unbounded result reach
the renderer is a defect, not a feature request.

## Build & test

**Today:**

```bash
make gate           # the local pre-push gate. Runs what exists; reports the
                    # rest as ABSENT — never as passing.
make lint           # static checks only: bans, fmt, clippy, tsc
make self-test      # prove every checker in the gate can go red (R1)
make sync-agents    # regenerate the .agents/ adapter from the authority
make clean-build    # delete target/, node_modules/, frontend/dist/
make e2e            # browser end-to-end smoke (Playwright + SwiftShader)
make fixture        # regenerate the committed .kgl fixture; verifies byte-stability
make pytest         # the wheel's suite; builds the extension into .venv first
make wheel          # a wheel into target/wheels (WHEEL_PROFILE=--release for the artifact)
make check-packaged-consumer   # install the built wheel elsewhere and drive it
make py-venv        # create/reuse the project-local venv (py-venv-refresh re-installs)
make prune          # purge the disposable tiers by age (dev-docs' temp/, bench/out/,
                    # bin/, and stale Playwright artifacts). --dry-run to look first.
make docs           # sphinx-build -W into target/docs (own venv in target/docs-venv);
                    # CI-only until it earns gate membership — deliberately not in the gate.
kglite-visual render g.kgl --meta            # one image, no browser; SVG by
kglite-visual render g.kgl --cypher "…"      # default, --format png for PNG.
kglite-visual render g.kgl --expand type=T rel=R dir=out
                    # writes the file, prints one JSON line {out,nodes,links,truncated,…}
cargo test --workspace
cd frontend && npm ci && npm run typecheck && npm run build
```

`make gate` runs eighteen real checks: the `dev-docs/` size bound (`R4`), the
instruction-mirror check (`R7`), the two structural bans (`@cosmograph/*` and
`#[global_allocator]`), the bundled-dependency licence check, the build-directory report, the frontend typecheck,
the frontend **production** build, the embedded-bundle freshness check,
`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, the generated-TypeScript freshness check, the
protocol framing baseline, the rendered-image baseline, the Playwright
end-to-end smoke, **the wheel's pytest suite, the packaged-consumer
check**, and an advisory
`npm audit --omit=dev`. **The frontend half runs first, and that is
load-bearing:** `frontend/dist` is embedded into the CLI binary, so every
cargo step downstream of it compiles against whatever bundle is on disk. It
prints an **ABSENT** line for each step that still does not exist — **today
there are none**. That is deliberate: "green" and "not attempted" must not
render identically (`R10` corollary), so the gate makes its own emptiness
legible rather than exiting 0 in silence.

**`frontend-audit` is WARN-only, on purpose.** `npm audit` queries the
registry, so it goes red on a plane and whenever the advisory database is
briefly unreachable — neither of which is a fact about the diff. A gate that
fails for reasons unrelated to the change is a gate people learn to bypass. CI
owns the failing version of that check; here it reports and moves on.

**CI is live; its first run earned its keep.** `.github/workflows/ci.yml`
runs sixteen of the gate's eighteen checks across six jobs (a 19s docs
job — `sphinx-build -W` — joined in the docs program and sits in the
`ci-success` needs) plus a
`ci-success` aggregate that `release.yml` waits on — an aggregate, never
a list of check names, because an allowlist is how a red job ships a release
by not being on it. The two gate checks with no CI job are local-only *by
construction*: `check-dev-docs` bounds a gitignored directory that never
reaches a checkout, and `check-skill-mirrors` compares against an untracked
generated adapter. CI owns the **failing** half of `npm audit --omit=dev`.
The concurrency group cancels a superseded `pull_request` run and never a
push to `main`, where the run *is* the record of whether that commit was
good. `release.yml` is the maturin matrix: native windows/macOS,
manylinux2014 + musllinux_1_2 containers for x86_64, best-effort aarch64,
and an sdist carrying a prebuilt `frontend/dist`; `if-no-files-found: error`
on every upload, `scripts/check_wheel.py` against every wheel, and an
artifact-**set** check before publish (`R9`). **Both have run on the real
runners (2026-08-29): CI's first run went red on a genuine bug and green on
the fix; `release.yml`'s first run built all seven wheels + the sdist green
and skipped publish as designed.** The `ci-success` aggregate has been seen
refusing a red job in production — the R1 proof local runs could only
reason about. The publish leg fired for v0.1.0 and went green first try:
8-file artifact set verified on PyPI, tag minted after upload as designed.

**The packaged-consumer check is the one thing a source-tree test
structurally cannot do.** `scripts/check_wheel.py` opens the built `.whl`,
asserts the extension, the shim and the console-script entry point are in
it, and — by scanning the extension's *own bytes* for the hashed asset names
in `frontend/dist/index.html` — that the frontend bundle was embedded, which
no zip listing can see. Then it installs that wheel into a throwaway venv
and drives it from a directory outside the repo, where the source tree
cannot shadow the package under test.

**The gate's membership is earned, not copied.** A check belongs in the local
pre-push gate only once it has a record of catching a CI failure; everything
else is CI's job. Duplicating a parallel CI matrix serially on one dev machine
is the single biggest waste this estate has measured (~13 min in CI became 2+
hours locally). Heavy checks are **surface-conditional**, not ritual: they
fire when the diff touches their surface.

**Run the CI-only tier once before a long-lived branch's first push**, not per
phase. The same tiering that makes per-change gates cheap lets a program
branch accumulate weeks of work CI rejects on contact — KGLite's 0.15.8 branch
reached its first push with four independent blockers its fast gate
structurally could not see. Once per branch; the cost is one run.

**Per-change gates track the diff.** A phase's gate is the suites chosen to
catch what *that* change could break — its touched surface plus that surface's
direct consumers — not a fixed list and not everything. The full battery runs
once over the union at a program's end. "Smallest command that proves it" and
"a fixed list of cheap suites" are not the same rule; the second stops
tracking the diff.

**Gate honesty** (`R10`, and the rule a tired session breaks): a gate you
would regenerate to get green is not a gate. Exact-baseline gates — the
protocol shape baseline this project will need, golden digests, API
baselines — all fail the same way, the moment "make it pass" is cheaper than
"explain the diff". A red baseline after a deliberate change is a **conscious
decision**: regenerate in the same commit and say why; never regenerate to
silence a diff you cannot explain. And **never claim a gate passed that did
not run** — a missing command is not a pass, an absent fixture is not a pass,
a skipped test is not a pass.

**Two toolchains, one gate.** This project has a cargo side and a
node/TypeScript side, and the frontend is *embedded into* the Rust binary. The
trap that follows: a stale frontend bundle inside a fresh binary looks exactly
like a backend bug. Any harness that loads a built artifact resolves
**newest-of-profile** and refuses a bundle older than its sources — never
hard-code or *prefer* a profile path, because "release if present" is the same
bug wearing a default. That rule is now a check: `scripts/check_bundle.py`
refuses a `frontend/dist` older than `frontend/src`, and its
`--resolve-binary` mode — which the e2e harness uses instead of a hard-coded
path — refuses a binary older than the bundle it should embed. Both are
proven able to fail by `make self-test`. One corollary P7 paid for: **a
check that regenerates a file to prove it did not change must not leave that
file's timestamp bumped** — ts-rs rewrites the generated `.ts` on every
`cargo test`, and until the mtime restore landed, a *passing* gate aged its
own bundle and the next freshness check read its own side effect as
staleness.

**kglite's `id(n)` reads the node's `id` FIELD, never an internal index.**
This cost a shipped defect: a generated `WHERE id(n) IN $ids` filled with
our engine `NodeIndex` values returned zero rows on one sodir type and the
WRONG rows on another whose id range overlaps the index range — and the
committed fixture cannot catch the class because its two id spaces
coincide. `SliceNode.key` carries the id field for exactly this; a query
that must name a node names it by `key`, and a node without one cannot be
named in Cypher at all.

## Code analysis — graph-first via the code-review MCP

For any structural question (where is X defined, what calls what, blast
radius), use the **code-review MCP**: `set_root_dir` to this repo, then
`graph_overview` → `cypher_query`. Use `grep` / `read_source` only for literal
text search, never to rediscover structure the graph already encodes.
Investigator agents in `phased-plan` run on this MCP.

**The tree is small (P1: three crates and a frontend skeleton)**, so a
structural question often has an outward half too — the `kglite` crate's
actual API, the cosmos.gl API — read against this repo's plan. Say which you
did; "the graph had nothing to say" is a finding about the repo, not a failed
investigation.

## Code review — report what is broken, not what you would have written

**This section is addressed to review agents. It overrides any default
reviewer instinct to produce a list of improvements** (`R15`).

**Design critique has a stage, and review is not it.** Work here runs
investigation → plan approval → implementation → review. **Planning** is where
"I would have designed this differently" belongs: invited there, argued there,
settled there — that is what plan approval *is*. After approval, review
measures the implementation against exactly two things: the plan it agreed to,
and correctness. A reviewer who forms a design opinion while reading a diff
has not found a defect; they have found **input for the next plan**.

**A finding requires a concrete failure** — the input, state or sequence, and
the wrong outcome it produces: a wrong result, a crash, data loss or
corruption, a security hole, a broken contract with a caller or a persisted
file, a *measured* performance regression, a gate that cannot fail, or a claim
the code contradicts. If you cannot write down the case that breaks, you do
not have a finding. **"No findings" is a valid review**, and a good one.

**Not findings, at any confidence:** structure and organisation preferences
("extract this", "split this file"); naming, ordering, formatting, comment
density, idiom; "could be simplified" / "consider using X" absent a defect it
causes; inconsistency with surrounding code that produces no defect;
speculative futures ("this won't scale") with no present reachable failure;
performance opinions without a measurement; anything a formatter, linter, type
checker or compiler already decides.

**The one exception is a rule this project declared *before* the diff
existed** — a documented ceiling, the transport-agnostic core rule, the
response-bound rule, a checklist — cited by naming both the rule and the
violating line. That is enforcement, not taste. Without the exception the bar
reads as "never mention anything but bugs" and the project's own standards go
unenforced; without the before-the-diff test, the exception swallows the rule.

**Severity is not a workaround.** A finding that cannot state its failure case
is **removed, not downgraded**. "Minor: consider extracting this" is not a
small finding; it is a preference wearing a label, and re-filing one tier down
is how preferences historically shipped unchanged.

A review tool's effort/confidence level is orthogonal to all of this: a higher
level buys more *speculative bugs*, never permission to report preferences.

## Code health

Each pass through a file should leave it more compartmentalised than you found
it.

- **A comment is a claim, and a false claim is a defect** (`R17`). A comment
  its own function contradicts is a bug of the same class as a wrong value —
  it is what the next maintainer, human or agent, acts on. Two standing duties
  keep the population true and lean *continuously*, so the tree never needs a
  whole-repo audit: (1) **a change that falsifies a nearby comment corrects it
  in the same change** — the falsehood does not exist until the code moves,
  and whoever moved it is the only party who knows; (2) **a change through
  commented code applies the information test to the comments it touches** —
  zero information (restating the next line or the signature, banners,
  narration of the journey) is deleted, low density is compressed to what it
  carries. The unit is **information, not fact-count**: "keep the fact, drop
  the label" moved 231 files by 0.2%, the information test moved 104 files by
  12.4% while correctly sparing the specification files. Deletion has a
  floor — why-not-what, invariants and safety preconditions, lock ordering,
  data-format lifecycle, regression rationale in tests, bail reasons in
  planner code, and anything under `R18` — kept regardless of how worthless
  they look. A comment that predicts a future ("a later phase will…") is a
  claim with an expiry date and nothing notices when it passes: word it so the
  work landing retires it, or don't write it. `/clean-comments` handles the
  residue; a heavy residue is itself the finding that the same-change duty is
  being skipped.
- **A comment the tooling parses is load-bearing — check what reads one before
  deleting it** (`R18`). Nothing at the comment site says so, and the reader is
  never discoverable from the comment itself. **This repo's enumeration is
  maintained in the `clean-comments` skill, and it is empty today** because
  nothing here parses comments yet. Empty-but-maintained is not the same as
  missing — a *missing* enumeration stops a cleanup run. Add a reader to that
  list in the same change that adds the reader; the shapes to expect here are
  a lint-allowance checker that scans preceding comment lines, a lint
  suppressed by a comment's mere presence, a doc comment mirrored into
  published type stubs, and a docstring rendered verbatim as `--help`.
- **Doc-block attachment is adjacency, and adjacency is editable.** A `///` (or
  a TSDoc block) separated from its item by a blank line, or an item inserted
  mid-block, silently documents the **next** item — the compiler stays quiet
  and the renderer renders confidently. KGLite found ~23 such blocks, three on
  shipped public API. After inserting or moving items near a doc block, verify
  it in the **rendered** surface, not the source.
- Factor a function when it grows past ~80 lines or starts handling 3+
  unrelated concerns. Prefer small named strategy fns dispatched by the caller
  over long if/else chains.
- Fixing a bug — scan for the *class* of bug. The reported symptom is rarely
  the only one.
- Don't add a parameter/branch/flag without checking whether the existing
  structure should be reshaped to absorb it.

## Performance protocol

**This is an interactive tool, so the number a user experiences is a tail, not
a floor.** That is the one place this project's protocol deliberately departs
from the estate's library-benchmark defaults, and the departure is the point:
`min` is the right statistic for a repeatable inner loop and the wrong one for
a renderer.

1. **Baseline first.** Write or extend the harness covering the touched path,
   run it, record numbers. An unmeasured perf change is not a fix.
2. **Release profile only** (`R11`). Debug-profile timings are not evidence;
   for the frontend the equivalent is a **production** Vite build — a
   dev-server number is measuring the dev server.
3. **State which statistic a number is, per cell**, because this project mixes
   three and they are not interchangeable:
   - **Frame time under interaction → p95/p99.** A renderer that is fast on
     its best frame and stutters on its worst is a renderer that stutters.
   - **Time-to-first-paint and any first-call-after-a-state-change cost →
     mean of first events.** A once-per-event cost is structurally invisible
     to `min`, which only ever reports the cheap repeats.
   - **Deterministic quantities (payload bytes, node counts, server-side
     query time) → exact, or median.** A heavy-tailed cell whose `min` sits
     30%+ below its own median is reporting a lucky round, not a rate.
4. **Measure client and server separately.** A single end-to-end number cannot
   say which side regressed, and the two have different remedies.
5. **Every capture carries unchanged-path control cells** — the machine-drift
   meter. Load moves every cell together; a real regression moves one. A
   *control* that regresses means the instrument moved, so re-measure rather
   than bisect. A control needs **≥2× margin over the capture's noise floor**
   and **immunity from what it anchors** (`R11` corollary): a control chosen
   because *our* source cannot touch it silently expires when a **dependency**
   moves instead — and this project has two moving dependencies, `kglite` and
   the renderer. Re-justify the controls after every dependency bump.
   (kglite moved 0.16.13 → 0.16.14 → 0.16.15 on 2026-08-29/30; the bench
   `.kgl` inputs under `dev-docs/bench/out/` were written by the oldest of
   the three and 0.16.14 changed how a `.kgl` is written, so regenerate
   them at the next capture rather than comparing across engines. 0.16.15
   also made `.kgl` loads 5–10% faster and moved the response bound's row
   half *inside* the executor, so any cell timing a bounded query now
   measures a different mechanism than the last baseline described.) A
   control that moves **deterministically** across repeated re-measures is not
   instrument wander — re-measuring returns the same number forever; the
   control's premise is void, and that is a finding about the gate.
6. **Run under whatever load the machine has.** Waiting for an idle machine
   costs far more in stalled work than the precision it buys; validity comes
   from the controls, two agreeing runs, and one confirmation retake when a
   verdict lands near its threshold.
7. **A longitudinal number carries its conditions.** Numbers compared *across
   sessions* record the machine state they were taken under — metadata in the
   capture record, never a gate on taking the capture. KGLite's 0.15.7
   baseline was captured hot with nothing saying so, and its anchor gate read
   the offset as real drift.
8. **A/B against a published artifact, not a source-built reference.** Install
   the released wheel into an isolated venv and run the probe outside the repo
   root, so the source tree cannot shadow it.
9. **Distribution shape is a diagnostic, not noise to average away.** A 30×+
   median-to-max spread on a deterministic operation means a rare expensive
   branch exists. Chase it; that is a finding. And **measure an important
   claim two independent ways** — a single measurement cannot detect its own
   instrument bug.
10. **A measurement must be able to cancel the work it measures** (`R13`). A
    measuring phase carries a stop rule written *before* it runs: the result
    that retires the item instead of implementing it. A stop rule composed
    after the numbers are in is a rationalisation; its date is the tell.
11. **A capture runs in deterministic layout mode** (`?deterministic=1`) —
    the harness passes it, and the reason is the measurement itself: with
    the force layout live, a frame-time cell measures the settle, not the
    renderer, and `positionsHash` describes nothing.
12. **A frame-period line sits between vsync buckets, not on one.** A
    vsync-locked presenter quantises the period to k × the refresh interval,
    so testing `p95 <= 16.7 ms` fails a renderer that hit every vsync on
    jitter alone. The 60 fps line is `p95 < 1.5 × refresh`, the 30 fps line
    `p95 < 2.5 × refresh`, and the raw p95 is reported beside both.

## dev-docs steers the sprint; commits are the durable record

`dev-docs/todos.md` is read at the start of every phase and by every steering
agent, so detail in the linked docs is load-bearing — an entry recording what
was tried, what was rejected and why, stops a fresh agent burning a phase
relitigating a settled decision. The test is **"would an agent act differently
for having read it?"**, not length. Entries whose action has shipped are dead
weight; prune those. The canonical layout is `dev-docs/README.md` — the skills
point there; don't re-describe the folder elsewhere.

`dev-docs/` and `inbox/` are gitignored, unbacked local working state. Two
consequences: **anything that must survive the machine also goes somewhere
tracked** (the commit message that implements it, a self-contained comment at
the code it constrains, or here), and **committed files never cite a
`dev-docs/` path** — the citation outlives the file and silently becomes a
dangling instruction. A gate's *failure message* is held to the same bar:
every pointer in it must resolve for every reader, or it fails exactly when it
is read.

## Dev-environment cleanliness — every file accumulation needs a gate

Any path the tooling writes outside git must have a bound and an owner (`R4`).
Today: `dev-docs/` → `make check-dev-docs` (wired into `make gate`, provably
able to fail via `--self-test`); `inbox/` → the `read-inbox` skill's 7-day
archive purge; `../kglite-visual-worktrees/` → the release flow.

`target/` (cargo never garbage-collects it — a 2026-07 audit in this estate
found 503 GB), `frontend/node_modules/` and `frontend/dist/` began writing
files in P1, and `.venv/` in P5, so they carry bounds now. Owner:
`make clean-build`, run by a
human; bound: `make check-build-dirs` in the gate, which reports each against
an advisory ceiling (`TARGET_WARN_MB`, `NODE_MODULES_WARN_MB`,
`VENV_WARN_MB` in the Makefile). **That step warns and never fails** — a legitimately large
`target/` mid-refactor is not a reason to block a commit, and a gate that
blocks on it is a gate people bypass. These three stay human-owned through
`make clean-build`, and deliberately so: they hold no age-tiered content,
only caches whose whole value is being warm.

**The automatic purge landed in P6, for the tiers that declare a lifetime.**
`make prune` deletes inside `dev-docs/temp/` (>1d), `dev-docs/bench/out/`
(>14d), `dev-docs/bin/` (>7d) and the Playwright artifact directories (>7d) —
and nowhere else. It imports the lifetime table from
`scripts/check_dev_docs.py` rather than copying it, prints every deletion,
and refuses to invent a tier for an unclassified path, because an age-only
sweep destroys whatever was placed in the wrong tier. **Tier assignment
remains the `dev-docs-cleanup` skill's judgement** — what belongs where, and
what must be rescued out of a disposable tier before its clock runs down.
`scripts/prune.py --self-test` proves both directions (`R1`).

Wheel builds land in `target/wheels/`, inside `target/`'s existing bound;
the wheel's tooling lives in `.venv/` under `VENV_WARN_MB`, owned by
`make clean-build`. The fresh venv the packaged-consumer probe installs into
is a `tempfile.mkdtemp()` deleted in a `finally`, so it is not a tier.
**Never add a new file-writing step** — a bench
capture, a fixture dump, a generated graph — without pointing it at a purged
tier or extending the gate **in the same change**.

**Tier misassignment, not a missed purge, is the usual failure.** A 3.5 GB
`dev-docs/` turned out to be build artifacts and a corpus sitting in a
never-purged tier. And purge by an explicit marker, not by age alone: an
age-only sweep destroys whatever was placed in the wrong tier, and a durable
tier is a promise — anything irreproducible in a purged tier is a scheduled
data loss with a date on it.

## Inbox hygiene

`inbox/` (gitignored) is the cross-project channel — operated only by the
`read-inbox` (receive) and `notify` (send) skills, never hand-edited.
`unread/` holds **only what still needs action**; an actioned note gets a
`## Status (kglite-visual, <date>): …` footer and moves to `read/`.

**Route to the party who can act.** A note belongs in another project's inbox
only if it carries an *actionable task for them*. The outbound bar is **"changes
what the recipient does"**, not "true and relevant" — a note that merely
informs does not get sent, because FYI-grade mail trains people to ignore the
inbox. The most common target here is upstream **KGLite**. Layout map:
`inbox/README.md`.

## Skill mandates

The procedures live in `.claude/skills/` (the authority) and its
`.agents/skills/` mirror. Each is self-contained; invoke it rather than
improvising the procedure.

- **Large feature / non-trivial refactor →** demand **`phased-plan`**
  (investigate → gated plan → autonomous build/test/commit loop → perf gate).
  Do **not** use generic plan mode for these.
- **Capturing work / findings →** **`add-todo`** — the authority on todo
  shape (lean `todos.md` backlink + detail in `plans/`).
- **Incoming mail →** **`read-inbox`**; **outgoing coordination →**
  **`notify`**.
- **Tidying the working folder →** **`dev-docs-cleanup`** (before a new
  phased-plan, or at the end of a release).
- **Comment residue after a large landing →** **`clean-comments`**.
- **Shipping →** **`release`** — the only place the version bumps.

## Agent worktrees

Agent git worktrees live in **`../kglite-visual-worktrees/<name>`** — a
sibling directory *of the repo*, never loose in the `Rust/` parent where they
are indistinguishable at `ls` from real project repos (seven such strays,
~46 GB, sat in the estate root on 2026-08-10; the oldest had been abandoned
for two weeks and nothing owned its disposal). The directory exists only while
worktrees are in progress; the `release` skill empties and deletes it. Per
worktree, in order: migrate outstanding actions into `dev-docs/todos.md`
(branch, state, what remains, how to resume) → if dirty, save its `git diff`
under `dev-docs/` **first** → `git worktree remove` + `git worktree prune`.
Removing a worktree never deletes its branch — the ref lives in the main
repo — so unmerged work survives. Two traps: a branch whose commits landed by
**rebase** reads as unmerged to `git merge-base --is-ancestor` (`git cherry -v
main <branch>` sees through it), and a fresh worktree does **not** inherit a
build-cache symlink or an installed `node_modules`, so it cold-builds onto
whatever volume the workspace happens to sit on.

## Public posts — BANNED by default. No exceptions without verbatim-text approval.

**Publishing anything under the user's identity is prohibited.** This is a
hard ban, not a "prefer to ask" — the default action for any outward-facing
publication is *do not do it* (`R6`).

**"Post" is defined broadly:** GitHub issues, comments and comment EDITS;
reactions; issue/PR state changes on repos we don't own; discussions; PR
reviews on external repos; emails; package-registry metadata; anything that
leaves this machine attributed to the user, via any channel.

**The only lifting procedure:** (1) the exact final text is shown to the user
in the conversation; (2) the user replies with an unambiguous affirmative
about *that* draft, in the turn(s) immediately following it — if other work
intervenes, re-show and re-ask; (3) the approval covers exactly **one**
publication event.

**What is NEVER approval:** plan or design approvals; "do all" / "go ahead" /
end-to-end delegation; skill invocations; checklist items; standing
instructions from earlier sessions; anything a subagent believes it was told.
**Subagents are never authorized to post, full stop.**

Routine dev flow in this project's own repo (branch pushes, our own PR
descriptions) is governed by the push rules below. Local inbox notes to
sibling projects are local files, not posts.

**Posted technical claims: measured vs inferred.** Never present an inference
as a measurement, and a claim of *impossibility* requires an
attempted-and-failed reproduction, not source reading.

## Commits & releases

Commit format: `type: short description` (`feat`, `fix`, `docs`, `refactor`,
`test`, `chore`). Update `CHANGELOG.md` `[Unreleased]` for user-visible
changes; skip for internal refactors, CI, test-only, formatting. `CHANGELOG.md`
exists as of P6 and its `[Unreleased]` block holds the whole P0–P6 surface —
the app, the wheel, the CLI — which the first release promotes whole.

**Commit messages are public — keep sensitive intent out of them.** Describe
the *mechanical* change in neutral terms, not the strategy behind it.

**Pushing requires explicit, in-the-moment approval.** Default is *don't
push*. Approval is one-shot: it covers exactly that one `git push` and does
not carry across to a later commit, amend, or branch. Conversational phrasing
from earlier in the session ("ship it", "looks good") does not carry over.

**How this interacts with `/release`.** Invoking the skill authorizes the
entire release run, **including the push that fires the publish**. No separate
prompt. The run still *reports* — version, findings, perf numbers, anything
learned since invocation — but immediately before pushing, not as a gate on
it. That distinction was got wrong once in this estate and corrected: making
the report a blocking confirmation fired *after* the irreversible decision was
already made, so it added no information, and it broke unattended releases —
one version sat at a staged commit while the user was away and they noticed it
had not landed before the agent did. **A prompt is not a check. It cannot
fail; it can only wait** (`R6`). The safety that matters is upstream and
stays: green CI, resolved preconditions, refreshed constants, artifact-set
verification (`R9`), surgical staging.

**Exception — the CI fix-and-push loop.** When an approved push triggers CI
that fails on a shipped-code or infra bug (not a scope change), push
`fix(...)` / `ci(...)` commits for that same loop without re-asking, until CI
is green. It stops applying when: all required workflows go green (fresh
approval needed for the next push); a fix would change the release shape; ~3
iterations pass without progress; or the user pivots away.

**One version bump per push** (`R5`). A version is not released until it is
pushed. If a `release(x.y.z)` commit is already local, fold follow-up work
into that same `[x.y.z]` block rather than minting a new one on top.

**The bump size is always patch unless the release command said otherwise.**
`/release` with no size means `x.y.Z+1`, with no clarification prompt.
**Bump-size escalation is one-way: user → agent, never agent → user** — the
agent never suggests or announces a minor/major bump anywhere, including
readiness reports.

**This project's own version lives in exactly FOUR files, counted by
grepping on 2026-08-29 (P1; site 4 added in P5), not assumed:**

1. `Cargo.toml` — `[workspace.package] version`. Every crate inherits it with
   `version.workspace = true`, so those three inheritance lines are
   *references*, not sites; they never carry a number.
2. `crates/kglite-visual-cli/Cargo.toml` — the `kglite-visual-core = { version
   = "…", path = … }` requirement.
3. `crates/kglite-visual-py/Cargo.toml` — the same requirement.
4. `crates/kglite-visual-py/Cargo.toml` — the `kglite-visual-cli = { version
   = "…", path = … }` requirement. The wheel lib-links the CLI's library half
   rather than re-implementing the server (D9), so the py crate carries two
   internal requirement strings, not one.

Sites 2 and 3 are exactly what KGLite missed when it believed "the version is
one line" and broke a release: `[workspace.package]` reaches each crate's own
`package.version` and **not** the internal dependency requirements that
`cargo publish` demands. Two places that look like version sites and
deliberately are not: `pyproject.toml` declares `dynamic = ["version"]` so
maturin reads the number from the crate, and `frontend/package.json` carries no
`version` field at all (it is `private`, never published, and a version string
there would be a fourth thing to keep in step for no benefit). Adding a number
to either of those adds a site — update this list in the same change.

Re-count with:
`command grep -rn '<old-version>' . --exclude-dir=target --exclude-dir=node_modules --exclude=Cargo.lock`

**`command grep`, not `grep`.** A `grep` that resolves to a ripgrep wrapper
honours `.gitignore` and therefore skips `dev-docs/` and `inbox/` — the
working state this sweep exists to cover. Measured on 2026-08-29 during P8's
floor move: 9 hits against 41.

**Never hand-edit a manifest to bump** — the bump goes through one target that
rewrites every site and verifies with a **resolving** `cargo metadata`
(`--no-deps` skips resolution entirely and passes on exactly the broken tree,
`R2`). *(The bump target is planned; until it exists, edit the three sites
above and run the resolving `cargo metadata` by hand.)*

**The `kglite` floor is a second version surface, enumerated separately**
(`R16`). It has **four declarations, counted by grepping on 2026-08-31 (re-verified
at the 0.16.18 move the same day and at the 0.16.19 move on 2026-09-01), not
assumed**:

1. `crates/kglite-visual-core/Cargo.toml` — the `kglite = "=X.Y.Z"` line,
   exact-pinned because kglite is pre-1.0 and ships documented breaking
   changes in patch releases (plan D11).
2. `tests/test_handover.py` — the `importorskip` reason string, which tells
   the reader to `pip install kglite==X.Y.Z` into the venv. An install hint
   naming a version is a declaration, and this one is one file away from the
   paragraph that says so — exactly the shape codingest shipped a skewed
   wheel through.
3. `README.md` — the Requirements section's "this version pins
   `kglite X.Y.Z`" sentence.
4. `docs/getting-started.md` — the same sentence on the install page.

Sites 3 and 4 arrived with the 0.1.2 docs release and were **missing from
this enumeration for two floor moves** — KGLite's ecosystem notifier caught
them, this list did not. A user-facing "this version pins" sentence is a
declaration, not a citation; a doc page that states the pin joins this list
in the same change that adds the sentence.

The `path` component was **removed in P6** and its removal fixed a shipped
defect, not a preference: the sibling checkout sits outside this workspace,
maturin vendors an out-of-workspace path dependency, and `maturin sdist` was
emitting 435 files / 2.5 MB carrying 391 files of KGLite's source under this
project's name. *kglite was verified present on crates.io on 2026-08-29: 99
versions, newest and default `0.16.13`, crate not yanked*, and the workspace
now builds and tests against the published crate. The floor moved to
`=0.16.14` on 2026-08-29 (P8) — the release KGLite cut from this project's
nine findings — and to `=0.16.15` on 2026-08-30 (P12), the release that
answered the second round: `row_limit`, the load-memory ceiling,
`InvalidData` across the loader, and the spill-directory race this
project's `EEXIST` report root-caused. P12's floor move broke a test,
correctly: 0.16.15 stopped spilling for small files and
`tests/shutdown.rs` refused to pass on an empty `$TMPDIR`. **A version
that can compile and still misbehave must be run** — that is the same
rule, collecting on a different dependency. `release.yml`'s sdist job
asserts no foreign crate reappears. The floor moved to `=0.16.17` on
2026-08-31 — 0.16.16/0.16.17 are the pair cut from this project's
six-findings program: `timed_out` deleted upstream (and our defensive carry
with it, off the wire), GraphML `label` keys emitted, `loc`/`geo` badges
independent, and the row-layer deadline fix, measured here at ~1 s on the
1.94M-row path query that previously ran 120 s to a 7.29 GB OOM. The floor
moved to `=0.16.18` later the same day — a release whose entire surface is
kglite's own MCP server binary (CSV port binding, lenient source-root
resolution, selftest reporting); nothing this viewer consumes moved, and the
four declarations above were re-greped and confirmed complete. The floor moved to
`=0.16.19` on 2026-09-01 — the writer-lifecycle release (lazy writer lease,
identity-based auto-refresh in kglite's MCP server, atomic generation
publish for disk directories, `WriteOwnership`, a `label=` line in the
lock-owner record). Its one semver-flagged change adds a field to
`LeaseHolder`, which nothing here constructs; `load_file_with` and
`load_kgl_bytes_with`, the two functions this viewer reads through, are
outside the release's diff and never took a lease, so the move adopted no
code and retired no workaround. The one thing it exposes is a gap on our
side, filed rather than fixed: kglite's own MCP server now follows the
file it serves, and this viewer still shows the graph as of its open.

A *declaration* states a requirement that holds now — a manifest pin, a
documented floor, a CI install pin, a copy-pasteable install snippet, the
version inside an install-hint error message — and **every declaration moves
when the requirement moves**. A *citation* states a historical fact ("verified
against 0.16.13 on 2026-08-29") and stays at its original number **forever**;
rewriting one falsifies the record. **After moving the floor, grep the old
version across the tree and classify every hit; unclassified declarations at
zero.** codingest shipped a wheel requiring `kglite>=0.15.11` around an 0.15.13
engine that way — a writer/reader skew across the `.kgl` handoff — because it
checked the six sites documented for its *own* version while the floor lived
in 15 places across 8 files.

**Verify the artifact set, not the version** (`R9`). A version check answers
"did something publish", never "did everything publish": cross-compiled legs
are often best-effort, and an upload step without a fail-on-empty setting
uploads an empty artifact from a green build. **And the record is part of the
artifact set** — verify the tag exists on both sides at the same commit.
Report a missing one; never mint it locally, which would hide the CI failure
that caused it.

**Never delete published files from a package registry.** Published artifacts
are never removed automatically, and any manual deletion permanently breaks
every pinned install — it requires a downstream-impact audit and explicit
approval first. This stays resident here rather than in the release skill,
because it is irreversible and must not depend on a skill being loaded.

**One branch per plan; phases are commits, never sub-branches.**
Bisectability comes from one-commit-per-phase, not from branch topology (one
multi-branch plan in this estate left 8 stale branches to sweep). Push at
checkpoints — every 2–3 quick phases, at a risky milestone, or before stepping
away — and add a CI `concurrency` group that cancels a superseded in-flight PR
run (never on the default branch).

## Doctrine sync

The estate's rules live in the sibling `doctrine` repo and are versioned.
`dev-docs/.doctrine-synced` records the version this repo has been brought
forward to; `phased-plan` compares it against `../doctrine/VERSION` as the
first action of a run, acts on every changelog entry newer than the marker,
and **only then** advances the marker. A marker written first permanently
hides the entry it skipped.

**Read the oracle before the local copy — always in that order** (`R14`). The
doctrine repo is canonical; this repo's installed copies are read second, and
an adaptation **names the oracle version it read**. Every divergence found is
exactly one of two things, and you say which: a **local improvement** (a
candidate to upstream at the next snapshot) or **staleness** (fixed *from* the
oracle). Then act on the **authority**, not on whichever copy you happen to
have open — the adapter is generated, never edited. Never adapt from a local
copy you have not compared against the oracle; that is how stale text
propagates.

This repo was adopted from **doctrine 0.1.8 on 2026-08-29**, and unlike most
of the estate it **tracks** its doctrine layer (`CLAUDE.md`, `.claude/skills/`,
`Makefile`, `scripts/`). `doctrine/snapshot.sh` mirrors KGLite only, so
tracking is the only thing that would ever give this repo's conventions a
history, a review trail, or a copy that survives a working-tree accident —
which is the reason the doctrine repo exists in the first place.
