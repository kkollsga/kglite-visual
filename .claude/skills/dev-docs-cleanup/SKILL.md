---
name: dev-docs-cleanup
description: Reconcile the dev-docs backlog and archive completed work with a fresh retention period. Use when requested or within an authorized planning/release cleanup; preserve active work, shared references and recoverable worktrees.
---

# dev-docs cleanup

Read `dev-docs/README.md` for the repository's layout and retention policy.
Here, `bench/scripts/` is tracked; `bench/results/`, `learn-from-us.md`,
`.doctrine-synced` and `todos.md` are durable.
Durable plans, designs, benchmark scripts/results and the backlog are not
age-purged. Completed plans can be soft-archived; hard deletion is limited to
explicitly disposable, expired content. A directory name or old mtime alone
is not proof that the only copy is disposable (R4).

## 1. Inspect eligible disposable content

Use the repository's existing retention checker when it enforces these rules.
Otherwise inspect candidates before acting. `scripts/prune.py --dry-run` may
help inventory, but do not run its mutation until it enforces the eligibility
and archive-manifest checks below:


- `temp/` and `bench/out/`: require established ownership and reproducibility
  or a verified durable replacement, plus expiration under the layout policy.
- `bin/`: require a completed archive manifest and expiry measured from its
  archive timestamp, not the original document mtime (see §5).
- Legacy/unmarked files: keep and report them until their disposition is known.
- Do not follow symlinks outside the selected roots. Missing directories mean
  nothing to clean; do not create them merely to make a scan nonempty.

Purge only eligible content covered by the user's cleanup authorization.
Report deleted paths and retained exceptions. Do not run age-only blanket
`find ... -delete` commands over working records.

## 2. Read the backlog index

Read `todos.md` first, not every plan. Do not read `designs/`: it is durable
reference outside this task. Open a specific detail file only when a check
below needs it. An absent index is a condition to report, not permission to
classify the entire plans directory as abandoned.

## 3. Reconcile references and completed actions

List `dev-docs/plans/` and resolve the index's local backlinks. For an apparently
unindexed file, inspect inbound links from other durable plans as well: a
supporting document can be reachable through an indexed parent. Open only
unresolved candidates. Add a missing backlink for live work; archive a finished
or abandoned document only after confirming its disposition.

For an entry that appears completed, read its detail and corroborate completion
against code, tests or release history. Then:

- Completed thread: remove its index entry; archive its detail only if no live
  action or durable reference still needs it.
- Partially completed thread: preserve its document and trim the entry to the
  remaining work. Shared documents stay while any live section needs them.
- Ambiguous status: retain and report it; do not infer completion from age.

Use `add-todo` for new or revised entries. Preserve the evidence required to
resume before removing its last index reference.

## 4. Apply within the authorized scope

An explicit cleanup request or cleanup included in an authorized release is
sufficient authorization for the routine reversible tidy. Do not ask again.
A review-only request produces proposed changes, not mutations. Ask only for
an unresolved keep/drop decision that affects valuable work; perform other
independent authorized cleanup while it remains pending.

## 5. Archive with a real grace period

Create a unique batch under `dev-docs/bin/` (UTC timestamp plus a collision-safe
suffix). Write a manifest containing each original relative path, destination,
archive time and earliest purge time, and any durable replacement. Move without
overwriting existing files. Verify every destination and update the index,
then mark the batch complete. Preserve path structure where basenames collide.

The grace period is seven full days from archiving. A plain `mv` preserves the
old mtime and does not start that period. Incomplete batches are never purged;
retain their manifests so an interrupted run can reconcile source, destination
and index. Legacy files without a verified archive time are retained until
reviewed and explicitly archived with a fresh timestamp.

## 6. Reclaim inactive agent worktrees (release cleanup only)

Inventory `git worktree list --porcelain` and select only trees under the
repository's documented worktree directory, `../kglite-visual-worktrees/`. Check whether any task or user is
still using each tree; inactivity is not established by mtime alone. Leave
active, locked, ambiguous or otherwise unrecoverable trees intact.

Before removing an inactive tree:

1. Record the exact HEAD, branch, index/working status, base and upstream state.
   A detached HEAD or unmerged commits must have a durable recovery ref or
   verified Git bundle before removal; a branch name cannot preserve a detached
   commit. Check patch-equivalent rebases where relevant.
2. Preserve **all** uncommitted content in a unique durable recovery directory
   under `dev-docs/plans/`, with a restoration manifest: separate binary-capable
   staged and unstaged patches, untracked file contents (not just their names),
   and any valuable ignored files. Check submodules and symlinks explicitly.
   `git diff` alone misses staged and untracked work. Do not copy disposable
   build caches as recovery data. If the valuable set is uncertain, keep the
   worktree.
3. Verify recovery in a temporary checkout of the recorded HEAD: apply the
   staged patch with index state, then the unstaged patch, restore untracked
   contents, and compare status plus file contents. Keep the original until
   that check passes. Add a lean `todos.md` backlink to the recovery manifest.
4. Use `git worktree remove` without force. If it refuses a dirty tree, retain
   it and report the verified backup; do not force deletion as routine cleanup.
   Prune stale metadata only for the intended repository.

Remove the parent worktree directory only if actually empty. Cleanup can be
complete with explicitly retained active trees.

## 7. Reconcile adapters only when included in the task

Compare adapters against their declared authority, rename-aware. Classify each
divergence before regenerating: merge improvements into the authority;
regenerate stale adapters. Preserve the literal authority declaration. Run `make sync-agents`, then
`make check-skill-mirrors` (which checks both root instructions and skills). A user instruction to defer sync leaves
all consumer adapters and sync markers unchanged.

## Report

Summarize archived/purged items, backlog changes, retained worktrees and any
unresolved decision. Put lengthy disposable reports in `dev-docs/temp/`; place
recovery material and ongoing actions in durable, indexed paths.

## Steering material

The retention test is whether a fresh agent would act differently for having
read the record. Keep irreproducible graph cases and hand-labelled regression
evidence in `bench/results/`; generated `.kgl` captures belong in `bench/out/`
only when their regenerator remains usable. CLAUDE.md → "dev-docs steers the
sprint; commits are the durable record" owns the durable-record rule.
