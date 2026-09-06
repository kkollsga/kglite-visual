# Saved views and history

Open **Saved views & history** to save a named exploration or restore a previous
one. A view includes the loaded nodes and exact relationships, visual filters,
appearance, captions, selected identities, layout settings and frozen calculations. Saving a query
is separate: restoring a view does not execute query text.

The header names the active saved view and shows when the current exploration
differs from it. Local selection changes also affect that indication. If another
client changes the view while a save is finishing, the captured version may be
saved successfully while the current view remains marked as changed.

## Durable and session-only views

For a file-backed graph, saving verifies the source snapshot and the identities
of its members. Regular files use a SHA-256 fingerprint; published disk-graph
generations use their generation identity. Restore requires the same verified
source. Changed or missing files are refused without replacing the live view.

Null or ambiguous keys cannot identify a node reliably across launches. Such
explorations use **session-only** saved views. Bytes, in-memory graph objects
and older disk directories without a published generation identity also use
session-only storage. Naming an in-memory graph after a file does not make it
file-backed. Session-only views are lost when that server closes.

Durable views share one local catalog across viewer processes. Session-only
views belong to their running server. Each catalog permits at most **20 views
and 40 MiB**, with a **4 MiB** ceiling per view. A full catalog refuses another
save; it does not remove an existing named view. Replacement is explicit.
Unreadable or newer unsupported saved formats are refused rather than
overwritten. Durable writes coordinate across processes and replace the
catalog atomically.

Format version 2 preserves canonical field identities and calculation input
metadata. Supported version 1 views containing source fields migrate on read;
a newer format or an incomplete version 2 record is refused. Frozen results
are restored as saved, including their original input revision, without
recomputation.

## Layout and selection

Static layouts retain positions by source identity. A live force layout stores
its recipe and is **recomputed** on restore. Framing can fit the restored view
or focus named references; it does not preserve an exact GPU camera position.

Hidden but loaded selected records can be saved. A selected record that has
been unloaded must be loaded again or cleared before saving it as part of an
exact view. Restored shared selection is visible to other clients. Clearing
shared selection is labelled as a shared action; local selections in other
clients remain theirs.

## Shared history

History records acknowledged graph actions with their before/after revisions,
including actions from other clients. It retains at most **20 checkpoints and
16 MiB**, reports when older checkpoints are no longer available, and keeps
named saved views separate from this rolling limit. Focus, hover and camera
movement do not fill the history.

Restoring a checkpoint is a new shared action. A stale recovery request is
refused if another client has changed the view; refresh the history before
choosing again. Failed recovery preserves the current membership, filters and
appearance. The inspector’s local exploration trail is a separate record of
this browser’s actions, not an undo stack.
