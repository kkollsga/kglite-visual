---
name: notify
description: Send a coordination/feedback note to another local project's inbox. Resolves a target repo by name anywhere under the Koding/ parent tree, composes a message file per the inbox schema, and drops it in that repo's inbox/unread/ (creating the folder if missing).
---

# notify

Deliver a message to a sibling project's inbox so its maintainer/agent picks
it up on their next `read-inbox`. Input: a **target repo** (name or path) and
the **message** (topic + body; compose from the conversation if not given).

## 1. Resolve the target repo path

The target lives somewhere under the `Koding/` parent (category folders like
`Rust/`, `Go/`, `JS/`, `Python/`, `mcp-servers/`; repos sit at depth 1–2,
sometimes deeper). Search by name (case-insensitive):

```bash
KODING="${PWD%%/Koding/*}/Koding"
find "$KODING" -maxdepth 3 -type d -iname '<name>' \
  -not -path '*/node_modules/*' -not -path '*/.git/*' \
  -not -path '*/__pycache__/*' -not -path '*/target/*' \
  -not -path '*/dist/*' -not -path '*/.venv/*' \
  -not -path '*/mcp-servers/*'
```

- **`mcp-servers/` is one externally-managed project, not a tree of repos.**
  Its subdirs (`code_review/`, `legal/`, `open_source/`, …) are **not** notify
  targets — that is why `*/mcp-servers/*` is excluded above. To reach anything
  in that ecosystem, target **`mcp-servers`** itself (the top dir is still
  matchable) → its single `inbox/unread/`. Never resolve a name to
  `mcp-servers/<subdir>/`.
- **`node_modules/`, `target/`, `dist/` and `.venv/` are excluded** because
  this project vendors a frontend and builds Rust and Python artifacts, and
  every one of those trees is full of directories with plausible project
  names. A note written into a vendored package directory is a note nobody
  ever reads.
- **`<repo>-worktrees/` is not a target.** A worktree is a checkout of a repo
  we already have; mail goes to the repo.
- **Exactly one match** → use it.
- **Several matches** → prefer a git repo (has `.git/`); if still ambiguous,
  **ask the user which path** (show the candidates).
- **No match** → widen with `-maxdepth 4`, then ask the user for the path.
- If the caller gave an absolute path directly, skip the search and use it.

Confirm the resolved path before writing if there was any ambiguity.

## 2. Ensure the inbox exists

```bash
mkdir -p "<target>/inbox/unread"
```

(Create it if the project has no inbox yet — that is expected for a first
note.)

## 3. Compose the message (the schema)

Filename: **`<YYYY-MM-DD>-from-kglite-visual-<topic-slug>.md`** (date = session
date, kebab-case topic). Body:

```markdown
# <Short title>

- **From:** kglite-visual
- **To:** <target repo>
- **Date:** <YYYY-MM-DD>
- **Type:** feedback | bug | coordination | heads-up | request
- **Re:** <optional — version, file, PR, or prior message it responds to>

<1–3 paragraphs of context: what happened / what's needed and why.>

## Ask / action requested
- <concrete, actionable item(s) — or "FYI, no action needed">

## References
- <links, file paths, commit SHAs, versions — optional>
```

## Send discipline

The outbound bar is **"changes what the recipient does"**, not "true and
relevant" — a note that merely informs does not get sent.

- **Batch per target per session.** Collect everything for a given target and
  send one note. An immediate single-purpose note needs a *blocker*, an
  *explicitly requested reply*, or a *time-sensitive fact* — otherwise it
  waits for the batch.
- **No FYI-grade notes.** Acknowledgements live in the archiving side's Status
  footer (`read-inbox` step 5), never in the target's `unread/`.
- **At most one ping per stalled thread**, and only when carrying new
  evidence.
- **Related items piggyback** on the next legitimate note instead of earning
  their own file.
- **"We adopted your conventions" is not a note.** Nor is "we released,
  nothing changes for you". Both are FYI-grade by construction and both train
  the recipient to stop reading their inbox. Wait until there is something the
  recipient can *do*.

## Writing to KGLite specifically

KGLite is this project's upstream and the most common target. Its working tree
is **read-only** from here — kglite-visual embeds the published crate and
never edits KGLite source, so a note is the only correct channel.

- **A bug** gets a reproduction: the query or file that triggers it, the
  observed and expected result, and the `kglite` version. A rendering session
  reads an engine broadly and cold; a defect found that way is usually worth
  more to KGLite than it looks.
- **A request** for an API affordance states the use case, not the design. The
  contract stays owned by KGLite, tracked in KGLite (`R8`); this side keeps a
  pointer.
- **Downstream registration** — once this project declares a `kglite` version
  floor, KGLite's ecosystem-version audit and release notifier can usefully
  know about us. Before that there is nothing to register, and the note would
  be FYI-grade.
- A version cited in a note is a **citation** if it records history
  ("verified against 0.16.13 on <date>") and a **declaration** if it states a
  requirement that holds now (`R16`). Never ask a recipient to renumber a
  citation — that falsifies their record.

## 4. Write + report

Write the file to `<target>/inbox/unread/<filename>` and report the full path.
Don't move or touch anything in our own inbox — this skill only *sends*.

## Notes

- Keep the response under 400 tokens.
- This is the send side; `read-inbox` is the receive side. Same filename
  schema (`YYYY-MM-DD-from-<sender>-<topic>.md`) so the recipient's triage just
  works.
- Sending writes into another project's working tree — if the resolved target
  was ambiguous, confirm with the user before writing.
- A local inbox note is a **local file, not a public post**. It does not need
  the verbatim-text approval that CLAUDE.md → "Public posts" requires; nothing
  here leaves the machine.
