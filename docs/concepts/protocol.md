# The protocol

The server and the browser talk over a **binary protocol**: typed-array buffers
for topology and positions, JSON for metadata. Topology and positions are what
there are a lot of, and they are exactly what a `Float32Array` carries with no
parsing at all; names, types and bounds are what there are few of, and they are
exactly what JSON is good at.

It lives in `kglite-visual-core` and it is **transport-agnostic by rule**:
nothing in that crate may know it is talking to a WebSocket. The encoder is a
seam, and the WebSocket, the [JSON twin](../agents.md#the-json-twin) and the
headless renderer are three consumers of it.

## The version number

Every response carries `protocol_version`. It currently reads **4**.

```json
{"protocol_version":4,"core_version":"0.1.1","tier":"compact","slot_count":98,…}
```

Version 4 added the `layout` message and request: the server computes a static
arrangement for the live view and broadcasts it, and every attached client
stops its simulation and holds the picture still. That is what
[`set_layout`](../viewer/layouts.md) and `POST /api/layout` drive.

## There is no version skew

This is the short section, and it is short because of a packaging decision.

**The frontend bundle is compiled into the binary.** `rust-embed` bakes
`frontend/dist` into the executable, and the wheel carries that same extension
module. So the server and the client it serves are always the same build —
there is no separately-deployed frontend, no CDN, no cached bundle from last
month, and no matrix of "which client version works with which server".

`protocol_version` is therefore not a negotiation. It is a **tripwire**: a
client that ever sees a number it does not expect is looking at a build that
should not exist, and says so rather than guessing.

The corollary is a build rule rather than a runtime one. A stale
`frontend/dist` inside a fresh binary looks exactly like a backend bug, so the
build refuses it: `check_bundle.py` fails on a bundle older than
`frontend/src`, and its `--resolve-binary` mode refuses a binary older than the
bundle it should embed.

## Slots and tombstones

The view is a **slot space**: an integer index per node currently in the view.
The entry screen occupies slots `0..n` — one per node type — and every
expansion appends.

Collapsing does not reissue slot numbers, so anything holding a slot stays
valid. What it leaves behind is a **tombstone**, and `slot_count` includes
them while `pointCount` does not. When tombstones accumulate the server
compacts, which renumbers everything — and the response says so, because a
compaction is the one event that invalidates a slot a caller was holding.

## Joining a session in progress

A client connecting to a view that has already been drilled into is greeted
with **the whole current view** — every live node named, holes marked,
positions from slot zero, and the static arrangement in force if there is one.

That is not the obvious implementation, and the obvious one was wrong. Every
client used to be greeted with the entry screen, whatever the shared view had
been drilled into since; the next change then arrived indexing slots that
browser had never been told about, and the points appeared with no label, no id
and nothing to click.

`window.__kglv` reports both halves of that as an honest pair: `slotCount` is
how many slots the client holds a *position* for, and `namedSlots` is how many
it holds an *identity* for. Unequal means a client is mid-resync.
