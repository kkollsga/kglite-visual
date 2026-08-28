---
name: dev-docs-cleanup
description: Tidy the gitignored dev-docs/ working folder — auto-purge time-boxed dirs, then run a todos.md-driven tidy (read only todos.md; reconcile misplaced plans/ files and stale/completed actions, reading a specific backlinked doc only when a check points at it), soft-deleting finished docs to dev-docs/bin/ and pruning their todos entries. Run before a new phased-plan to start fresh, or at the end of a release. Never reads design docs.
---

# dev-docs cleanup

`dev-docs/` accumulates plans, intermediate files and scratch (it is
gitignored working state). Over time it goes stale and cluttered. This skill
tidies it: nothing is hard-deleted from a durable tier — stale files are
soft-deleted to `dev-docs/bin/` with a 7-day grace, and any open actions are
preserved in `dev-docs/todos.md`.

**Layout is `dev-docs/README.md` (the canonical map).** Durable dirs —
`plans/`, `designs/`, `bench/scripts/`, `bench/results/`, `learn-from-us.md`,
`.doctrine-synced` and `todos.md` — are never auto-purged; only `temp/`,
`bench/out/` and `bin/` are time-boxed.

## 1. Auto-purge generated dirs (always first)

At skill start, hard-delete the three time-boxed locations. **Never touch
`bench/scripts/` or `bench/results/`** — harnesses and the result history are
durable, and `bench/scripts/` is additionally *tracked in git*; only
`bench/out/` is disposable.

```bash
mkdir -p dev-docs/temp dev-docs/bin dev-docs/bench/out
find dev-docs/temp      -type f -mmin  +1440 -print -delete   # ephemeral handoff, >1 day
find dev-docs/bench/out -type f -mtime +14   -print -delete   # heavy generated artifacts, >14 days
find dev-docs/bin       -type f -mtime +7    -print -delete   # soft-deleted docs, >7 days
```

Report what was purged (path list, or "nothing aged out").

**Before deleting, glance at what is about to go.** An age-only purge destroys
whatever was placed in the wrong tier (`R4` corollary), and this project will
generate exactly the confusable pair: regenerable protocol captures and
`.kgl` fixtures (correctly disposable) sitting next to a hand-curated "graphs
the renderer must not choke on" record (irreproducible, and misfiled if it is
in `out/`). Anything irreproducible in a purged tier is a scheduled data loss
with a date on it — move it to `bench/results/` instead of deleting it, and
say so in the report.

## 2. Read `todos.md` — the only file read by default

`todos.md` is the index of open threads; its backlinks point to the durable
docs under `plans/`. **Read only `todos.md` to start.** Do NOT read through
`plans/`, and **never read `designs/`** — design-reference docs are not
todos-driven (they are durable reference, out of scope for cleanup). Open a
specific doc *only* when a check below points you at it. This keeps the
cleanup cheap: one file read, plus at most the few docs the checks flag.

## 3. Two checks, both driven by `todos.md`

**a) Misplaced files in `plans/`.** `ls dev-docs/plans/`. Every file there
should be backlinked from `todos.md`. For any `plans/` file with **no
backlink**, read *that file only* and decide:

- a live thread missing from the index → add a one-line `todos.md` backlink; or
- finished / abandoned → soft-delete to `bin/`.

When genuinely unsure, surface it to the user. (`designs/`, `bench/`,
`learn-from-us.md`, `.doctrine-synced`, the `README` and `todos.md` itself are
exempt — this check is `plans/`-only.)

**b) Stale / completed actions in `todos.md`.** Scan the entries — especially
at end-of-release, where shipped work leaves completed items behind. For any
entry that reads as done/outdated, **read its backlinked doc to confirm**,
then:

- shipped / doc fully complete → move the doc to `bin/` and **remove the
  entry** (code + CHANGELOG + git are the record);
- partially done → trim the entry to only what is left;
- superseded / abandoned → drop it (keep a one-line "Closed / dead" note only
  if it is worth not rediscovering).

Don't read a backlinked doc unless its entry looks stale — a healthy entry
needs no read.

**c) The retired-caveat check, while this repo still has one.** The
`Bootstrap` section of `todos.md` carries an entry listing every file that
claims something is "planned" or "does not exist yet". If any of those claims
has become false, that is a stale entry in the strongest sense — the artifact
is now lying to its reader. Fix the artifact, not just the todo. Delete this
check with the `Bootstrap` section, once the section empties.

## 4. Surface the plan

Report a short summary: what purged, misplaced `plans/` files found (+ the
decision per each), stale `todos.md` entries to prune, and docs to soft-delete
to `bin/`.

- **Run standalone** (e.g. before a phased-plan): wait for the user's
  go-ahead before moving files or editing `todos.md`. A simple proceed is
  enough.
- **Run inside an authorized flow** (e.g. `/release`): perform the tidy
  directly — no prompt — then report what was done. The flow's invocation is
  the authorization.

## 5. Soft-delete processed files

On go-ahead, move processed stale files into `dev-docs/bin/` (preserves them
for 7 days in case something was lifted wrongly). Never delete the active
plans, `todos.md`, `learn-from-us.md`, `.doctrine-synced`, or anything the user
chose to keep.

## 6. Process `../kglite-visual-worktrees/` (release-end)

Agent worktrees live in `../kglite-visual-worktrees/<name>` — one sibling
directory of the repo, never loose in `Rust/` (CLAUDE.md → "Agent worktrees").
That directory exists **only while worktrees are in progress**, so this step
empties it. Run it **after** the tidy above: the entries it writes are for the
*next* sprint and must not be exposed to §3(b)'s staleness sweep in the same
run.

`git worktree list`, then for each worktree under `kglite-visual-worktrees/`:

1. **Capture state first** — `git -C <wt> status --porcelain`, its branch, and
   whether that branch is merged into `main`
   (`git merge-base --is-ancestor <branch> main`). A branch whose commits
   landed by *rebase* reads as unmerged; `git cherry -v main <branch>` tells
   you whether the patches are already upstream (`-` means already there).
2. **Dirty tree → save the work before anything else.** Write
   `git -C <wt> diff` and `status` to
   `dev-docs/worktree-harvest-<name>.diff`. **A worktree with uncommitted work
   is never removed without that diff saved and a `todos.md` entry pointing at
   it.**
3. **Migrate outstanding actions into `todos.md`** — branch name, what it
   contains, what remains, and how to resume. Removing the worktree does *not*
   delete the branch: the ref lives in the main repo, so unmerged work
   survives. Follow `add-todo`'s entry shape.
4. `git worktree remove <wt>` then `git worktree prune`.

Finally **delete the now-empty `kglite-visual-worktrees/` directory**. If
anything is ambiguous — or a worktree looks like *active* user work (modified
in the last week **and** dirty) — leave it and report it instead of removing
it.

**Absent until Phase 0 lands:** this repo is not a git repo yet, so
`git worktree list` will fail rather than return an empty list. That is
**ABSENT, not clean** — say so and skip the step; do not report it as "no
worktrees to process".

## 7. Resync the agent-instruction adapters

**Adapter resync — diff each adapter against its declared authority,
rename-aware.** Identical: done. Divergent: classify each hunk before touching
either side — an *improvement* is merged into the **authority** first and the
adapter regenerated from it; *staleness* is simply regenerated away. Never run
a blind sync on a divergent pair: blind sync deletes improvements (~20 lines
in one estate repo, 2026-08-10), and no sync preserves stale doctrine the
other harness will follow (`R7`, with `R14`'s improvement-or-staleness
adjudication).

The **authority-declaration line is exempt from the rename substitution** — it
names the authority literally in every copy. A substituted declaration inverts
itself and tells the adapter's reader to edit the adapter; two estate repos
hit that on the day the procedure landed.

In this repo:

```bash
make sync-agents          # regenerate the adapter from the authority
make check-skill-mirrors  # the end state this step must leave green (in `make gate`)
```

`sync-agents` regenerates; it never merges. If the adapter has diverged
because someone edited it, that edit is *lost* by regeneration — so classify
first and merge any improvement into the authority *before* running it.

## Output discipline

Keep the response under 400 tokens. If the stale-doc review is long, write the
full report to `dev-docs/temp/cleanup-report.md` and report that path; surface
only the new-todos list and the keep/drop confirmations inline.

## Relationship to phased-plan

`phased-plan` recommends running this skill first, so a new project starts
from a tidy `dev-docs/` and a current `todos.md`. Relevant carried-over todos
can then be folded into the new plan — only with the user's go-ahead.

## dev-docs is the sprint's steering material

The prune decisions above run on one test — **"would an agent picking this up
act differently for having read it?"** An entry whose action has shipped is
dead weight; one that would change what the next agent does stays, however
long it is. Full rule, and the two consequences of `dev-docs/` being
gitignored: CLAUDE.md → "dev-docs steers the sprint; commits are the durable
record".
