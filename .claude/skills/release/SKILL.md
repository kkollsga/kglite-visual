---
name: release
description: Cut a kglite-visual release — goal-check against the phased plan, gate, bump every version site, promote the CHANGELOG, commit, push, poll CI to green (fixing and repeating on failure), verify the published artifact set and the tag, notify affected downstreams, then tidy dev-docs. Runs to completion autonomously — invoking it IS the approval.
---

# Release

> **Nothing has ever been released from this repo, and most of the pipeline
> this skill describes does not exist yet (2026-08-29).** There is no git
> history, no remote, no CI, no manifest, no CHANGELOG and no registry
> presence. **Do not run this skill to "see what happens".** The first release
> is preceded by a `phased-plan` that *builds* the pipeline; this file is the
> procedure that pipeline must satisfy, written down first so the pipeline is
> built to fit a known-good shape rather than discovered afterwards.
>
> Every step below marked **(ABSENT)** names a thing that does not exist.
> **An absent step is never a pass** (`R10` corollary): report it as absent,
> and never continue past it as though it had run. Delete each ABSENT marker
> in the same change that makes the step real.

## THE AUTONOMY CONTRACT — read this before anything else

**`/release` is the final release approval. It is not a request for a plan, a
status report, or a checkpoint. It authorizes one continuous run that ends
only in a published version or a named blocker** (`R6`, `R12`).

The failure this contract exists to prevent, observed three times in one week
elsewhere in this estate: `/release` was invoked, every check passed, and the
release did not complete — because the run reported progress and ended the
turn instead of continuing. One version sat at a staged commit while the user
was away; another stalled twice, once at "waiting for CI" and once after a CI
fix. **In every case the pipeline was healthy and the human had already given
the only approval needed.** The mechanism is not laziness and it will recur:
long operations have natural pause points, each *feels* like a reasonable
place to check in, and each is indistinguishable from the inside from
genuinely needing input.

Four rules, and they override any instinct to check in:

1. **Never end a turn between the release commit and the publish push.** They
   are one action. A commit without its push is the defect, not a safe
   midpoint.
2. **Waiting for CI is not a stopping point.** Poll it. A poll that takes
   twenty minutes is twenty minutes of polling, not a reason to hand control
   back. Background the poller or block on it — either is fine. Ending the
   turn is not.
3. **A red CI is a task, not a verdict.** Diagnose, fix, push, re-poll,
   repeat. Explicitly authorized; needs no re-approval. Bound it by
   **iterations without progress** (~3), not by attempts.
4. **Report at the END.** One report, after publish is verified. Intermediate
   narration is optional and must never replace the next action. If you find
   yourself writing "next I will…", do that thing instead.

**The run is complete when the published artifact set and the tag are
verified — or when you have surfaced a specific blocker you cannot fix.** "CI
is running", "the commit is ready" and "waiting for X" are not endings.

**Stop only for:** an unfinished open PR at step 0, a genuine scope change (a
new version shape, a feature gap, a removal of declared functionality), a CI
failure that survives ~3 fix attempts without progress, or a
destructive/irreversible action outside the release itself.

## Step 0 — land every open PR, or stop for the user's decision

A release ships `main` complete: **every open PR is merged before the goal
check** — none rides past a release silently. For each open PR:

- **Finished** — ready for review (not draft), CI green on its head, no
  conflicts against `main` → ff-merge it.
- **Not finished** — draft, red or incomplete CI, merge conflicts, or visibly
  partial work → **stop and put it to the user as a decision before any
  release work begins.** Name the PR, its exact state, and the options:
  *finish it as part of this run*, *merge it as-is*, or *defer it to the next
  release*. This is a sanctioned stop under the autonomy contract (an
  unfinished PR is a release-scope decision only the user can make), and it
  sits at step 0 precisely so the run never stalls on it later.

A deferred PR is named in the final report with the user's decision recorded,
not just "deferred". *(The prior rule elsewhere in this estate silently
skipped drafts, and a release shipped past a draft fix branch the user learned
about only from the final report. Skipping is a scope decision, so it belongs
to the user, up front.)* **(ABSENT — no remote, no PRs.)**

## Preconditions

- **No release already staged.** `git log origin/main..HEAD --oneline | grep
  -E "^\w+ release\("` — if it returns a commit, **keep that version** and
  fold the new work into the same `[x.y.z]` block. One version bump per push
  (`R5`).
- On `main` (or a fold-into-main branch). If there is **unrelated uncommitted
  work**, don't block on it and don't sweep it in: **stage every release file
  explicitly by path** (`git add <file> …`, never `git add -A` or `.`), then
  verify with `git status --porcelain` that only release files are staged. One
  bad pathspec stages **nothing** and the following commit still succeeds
  (`R2`) — read the staged list back, never the silence.

## Steps

1. **Goal check — did we achieve what we set out to do?** If this release
   ships a `phased-plan` project, read its plan doc and the PR checklist and
   confirm every planned phase actually shipped. List any phase that was
   **dropped, deferred or only partially done**, and surface the gaps before
   bumping. Each gap is a conscious choice: finish it now, or carry it to
   `dev-docs/todos.md` — don't let it vanish silently.

2. **Reuse the final local evidence; do not rebuild for ceremony.** Confirm
   the phased plan's gate and targeted tests passed on the current HEAD. If
   HEAD is unchanged, do not rerun them. If evidence is missing or stale,
   rerun only the affected suites plus `make gate`. Never serialize CI locally
   — the green PR checks in step 3 are authoritative. Never run a
   release-profile build as a correctness test.

3. **Test review before the CI push — the release's tests are reviewed like
   its code.** Walk the release diff test-first:
   - **Harvest what the phases generated.** Program phases and worktrees
     produce harnesses under `dev-docs/bench/scripts/` and suites CI may never
     run. Anything durable that CI never runs is a decision, not a default:
     audit what landed marked, ignored or env-gated and deserves a CI leg, and
     which scratch harness that proved a defect should be promoted into the
     real suite or explicitly retired.
   - **Overlap: trim or split, by what fails better.** Two tests asserting one
     contract → keep the one at the right layer. Split a fat test whose
     coupled concerns fail independently — a red should name its defect, not a
     bundle.
   - **Gaps: every substantive diff area names its catcher.** For each area
     the release changes, name the existing test that goes red if it
     regresses. Where none answers, add one now.
   - **Non-vacuity is the bar for everything added or promoted** (`R1`): seen
     red first. A *promoted* test re-proves red in its new home — a green
     relocation can be a silently dead test.

4. **Full branch CI before release work — poll it, do not pause on it.** The
   completed HEAD must be committed, pushed and green before any version bump
   or release-artifact build. `/release` authorizes that push. If CI is
   already green for the exact HEAD, do not repush.

   **This is the single most common place a run stalls.** Poll to a conclusion
   and continue in the same run. Check **per-job, never the rollup** — an
   in-progress job reports an empty conclusion that a naive filter reads as
   failure, and an aggregate success can be green while a job inside it is
   not. Read `.outcome`, not `.conclusion`, on anything carrying
   `continue-on-error`; and check for a **job-level** flag, which makes the
   whole job unable to fail and every gate inside it decorative (`R1`).
   **(ABSENT — no CI.)**

5. **Bump version — always patch, unless the invocation said otherwise.**
   `x.y.Z` → `x.y.Z+1`, no clarification prompt and no judgement call. A minor
   or major happens only when the release command itself specified it.
   **Bump-size escalation is one-way: user → agent, never agent → user** — the
   agent never suggests, recommends or announces a minor/major bump anywhere,
   including readiness reports; an agent-announced number the user did not
   repeat back is void, and proceeding past it adopts the patch default.

   **Then apply it with the bump target — never hand-edit a manifest.**
   **(ABSENT — no manifests, no bump target, and the version-site count is
   unknown.)** The count is *established by counting*, not assumed: KGLite
   believed "the version is one line", and the internal dependency
   requirements that `cargo publish` demands turned it into five files and
   broke a release. Whatever target gets built must rewrite every site and
   verify with a **resolving** `cargo metadata` — `--no-deps` skips resolution
   entirely and passes on exactly the broken tree (`R2`).

   **The `kglite` floor is a separate surface** (`R16`). If this release moves
   it: grep the old version across the tree and **classify every hit** — a
   *declaration* (manifest pin, documented floor, CI install pin, install
   snippet, the version inside an install-hint error message) moves; a
   *citation* ("verified against 0.16.13 on 2026-08-29") stays at its number
   forever, and rewriting one falsifies the record. Unclassified declarations
   at zero. codingest shipped a wheel requiring `kglite>=0.15.11` around an
   0.15.13 engine that way, having checked the six sites documented for its
   *own* version while the floor lived in 15 places across 8 files.

6. **Refresh captured constants and baselines. (ABSENT.)** When this project
   has exact committed baselines — the protocol shape, a public API surface,
   the type stubs, the perf baseline — refreshing them is
   **artifact/data generation, not another test gate**. Rules that will apply
   the day they exist:
   - **No step in the refresh is best-effort.** A step that cannot do its job
     exits non-zero with the fix printed. A missing artifact, a wrong-version
     tool, a failed capture: all abort. Do the remediation and re-run; never
     work around it. (One estate release found the wrong tool version on
     `PATH`, printed a no-op, and continued — only a human reading the output
     stopped stale baselines from shipping.)
   - **A baseline you would regenerate to get green is not a baseline**
     (`R10`). A red baseline after a deliberate change is a conscious
     decision: regenerate in the same commit and say why.
   - **Every performance number is release-profile** (production build for the
     frontend), captured under **whatever load the machine has**, with its
     control cells and **the machine state recorded** — it is a longitudinal
     record read several releases later, and an unrecorded hot capture is
     indistinguishable from real drift (`R11`).
   - **Add an anchor comparison against several releases back.** A gate that
     recaptures its own baseline each release structurally cannot see slow
     drift; 10% per release passes a 20% threshold forever. Recapturing must
     not clear the anchor check — only recovering the performance does.

7. **Promote CHANGELOG** `[Unreleased]` → `[x.y.z]`, leaving an empty
   `## [Unreleased]` on top. **(ABSENT — no CHANGELOG.)**

8. **Preflight, then commit.** A preflight is a **checker, not a driver**: it
   reports whether the tree is ready and refuses if not, prints the command
   for each unmet precondition, and has no `--fix`. It must assert at minimum:
   every version site agrees, the workspace *resolves*, the workspace version
   and the top CHANGELOG section agree, the tree is formatted, and
   `origin/main` is an ancestor of HEAD. **(ABSENT.)** Do not grow it into a
   driver — a tool that quietly performs the steps it checks is how gates stop
   gating. When green, commit as the final phase: `release(x.y.z): …` (version
   bump + CHANGELOG promotion + refreshed constants in **one** commit).

9. **Push — invoking `/release` is the authorization, including the publish
   push.** No separate prompt.

   **Report immediately before pushing — do not block on it.** State the exact
   version, the semver findings, the perf numbers, and anything the run turned
   up that the user did not know when they typed `/release`. Then push. A
   report is not a gate: making it one fired *after* the irreversible decision
   was already made and stalled unattended runs (`R6`). The safety that
   matters is upstream and stays: green CI, the preflight preconditions,
   refreshed constants, artifact-set verification, surgical staging.

   **ff mechanic — push the branch HEAD straight to `main`, don't `checkout
   main`.** With unrelated WIP in the tree, a local `git checkout main` drags
   it across. Instead: confirm fast-forward
   (`git merge-base --is-ancestor origin/main HEAD`), then
   `git push origin HEAD:<branch>` and `git push origin HEAD:main`. The
   working tree never moves.

10. **Poll CI until green.** Do not hand-roll a naive poll. Three failure
    modes any poller must encode, each of which has produced a false verdict
    in this estate: query **by branch** with a client-side head-SHA filter (a
    by-commit query reported zero runs for a full hour while all of them were
    green on that SHA); **require the expected run count to be present** before
    concluding anything (a zero-incomplete loop exits instantly green on an
    empty array); and report **`conclusion`, not `status`**. A timeout is a
    non-zero exit, never a pass. **(ABSENT.)**
    - **CI fix-and-push loop — authorized, and it is a loop.** Diagnose, push
      `fix(...)` / `ci(...)`, poll again, repeat. Green means **continue to
      the next step in the same run**, not report and stop. Stop and surface
      after ~3 iterations without progress, or on any change to the release
      shape. Infra flakes (registry 429s, a runner timeout) are **re-run, not
      code-fixed** — never change code to route around a flake.

11. **Verify the published artifact SET, and verify the release was
    *recorded*** (`R9`). **(ABSENT.)**
    - **A version check answers "did something publish", never "did everything
      publish".** Cross-compiled legs are often best-effort, and an upload step
      without a fail-on-empty setting uploads an *empty* artifact from a green
      build — so a partial set ships and nothing says so. Compare the artifact
      count and platform tags against the previous release. For this project
      the set is at least: the crates, and the wheel for **every** platform tag
      the workflow claims to build.
    - Conversely, an empty version read out of a manifest by a pipeline
      (`grep … | cut` reports cut's status, always 0) yields a green run that
      publishes *nothing* — a silent non-release. Assert the extracted version
      is well-formed before it drives any publish decision.
    - **The record is part of the artifact set.** Verify the tag exists
      locally, exists on the remote, and points at the same commit in both.
      One estate release published correctly to two registries while the local
      clone had no tag for two days and every registry query answered
      correctly. Worse, when the tag is created only inside the wheel-publish
      job, a wheels failure beside a successful crates publish ships crates
      with no tag and no release page — and a version check calls it green.
      **Report a missing tag; never mint it locally**, which would hide the CI
      failure that caused it.

12. **Notify affected downstreams.** Write a release note into `inbox/unread/`
    of each *affected* sibling only — one whose declared range excludes the
    new version, which pins a superseded exact version, which states a
    superseded version in published prose, or which references this release's
    breaking set. Everything else gets nothing: a "we released, nothing
    changes for you" note trains people to ignore the inbox. Numbered here
    rather than left in a reference section, because an agent walking a
    checklist stops at the last step — which is how downstreams went
    unnotified elsewhere. **(No downstreams today. kglite-visual is a leaf: it
    consumes KGLite and nothing consumes it.)**

13. **Delete the released branch** — local + remote. Once publish is verified
    the feature branch is fully merged and its PR shows merged, so it is pure
    clutter. `git branch -f main origin/main`, then `git switch main` (a
    zero-diff switch when `main == HEAD`, so working-tree WIP is preserved),
    then `git branch -d <branch>` (it refuses if somehow unmerged — don't
    `-D` past that) and delete the remote branch. Confirm the PR shows merged;
    if it shows open, the commits didn't land — investigate, don't force-close.

14. **Tidy dev-docs — perform directly, no prompt** (the `/release` invocation
    is the authorization). Follow the **`dev-docs-cleanup`** logic, which is
    `todos.md`-driven: auto-purge the time-boxed dirs, then read **only
    `todos.md`** — archive the now-shipped plan to `dev-docs/bin/` and prune
    its entry, trim other completed entries (reading a backlinked doc only to
    confirm it shipped). Carry the step-1 gaps into `todos.md`. Don't read
    `designs/` or sweep through `plans/`.

    **Then process `../kglite-visual-worktrees/` per that skill's §6** — it
    exists only while worktrees are in progress, so the release empties it: per
    worktree, migrate outstanding actions into `todos.md`; a **dirty** tree
    gets its `git diff` saved under `dev-docs/` **first** — never remove
    uncommitted work without that diff and a todos entry — then
    `git worktree remove` + `prune`; finally delete the emptied directory.
    Removing a worktree does not delete its branch, so unmerged work survives.
    Anything ambiguous, or dirty *and* touched within the week, is left in
    place and reported.

    **Adapter resync:** perform `dev-docs-cleanup` §7 — never blind-sync a
    divergent pair; an improvement is merged into the authority first and the
    adapter regenerated from it (`R7`). `make check-skill-mirrors` is the end
    state this must leave green.

15. **Snapshot / conformance.** Run `../doctrine/conform.sh kglite-visual` and
    report any rule violation — reported, never fatal to the release. Note
    that `../doctrine/snapshot.sh` mirrors **KGLite only**: this repo's
    doctrine layer has history because it is *tracked here*, which is why
    `CLAUDE.md`, `.claude/skills/`, `Makefile` and `scripts/` are committed
    files rather than gitignored working state.

16. **Prune the dev environment.** Every file accumulation needs a gate
    (`R4`): the cargo target dir (cargo never garbage-collects it — a 503 GB
    one was found in this estate), `node_modules`, the frontend build output,
    wheel builds, and tool caches. **(ABSENT — no prune target yet; it is an
    `R4` obligation the moment any of those first writes a file.)** Then leave
    the working tree in the canonical debug/dev state, not with the release
    build installed.

## Notes

- Keep responses under 400 tokens; write long diffs/logs to a file and report
  the path.
- **Never delete published files from a package registry.** Published
  artifacts are never removed automatically, and any manual deletion
  permanently breaks every pinned install — it requires a downstream-impact
  audit and explicit approval first. This is also resident in CLAUDE.md,
  because it is irreversible and must not depend on this skill being loaded.
- `/release` authorizes a release. **It authorizes nothing else** — an issue,
  a comment, an email or anything else attributed to the maintainer still
  needs its own in-the-moment, verbatim-text approval (CLAUDE.md → "Public
  posts").
