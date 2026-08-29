---
name: run-app
description: Launch, drive, inspect, and stop the kglite-visual app — the agent-operability contract in executable form. Use when a change needs verifying in the real running app (not just tests), when asked to run or screenshot the app, or before claiming any user-visible behaviour works.
---

# run-app

How an agent operates the real app. The contract these steps rely on shipped
in P1/P2 (launch contract, JSON twin, `window.__kglv`); if a step below
doesn't match reality, that is a defect in the app or this skill — fix it,
don't improvise around it (`R17`: this file is a claim).

## 1. Build + resolve the binary (never hard-code a path)

```bash
cd frontend && npm run build && cd ..     # embed-input first: dist is compiled in
cargo build -p kglite-visual-cli
BIN=$(python3 scripts/check_bundle.py --resolve-binary)   # newest-of-profile,
                                                          # refuses stale bundle
```

`check_bundle.py --resolve-binary` fails rather than hand you a binary older
than the bundle it should embed — a stale bundle inside a fresh binary looks
exactly like a backend bug (CLAUDE.md → "Two toolchains, one gate").

## 2. Launch (agent mode)

```bash
"$BIN" crates/kglite-visual-core/tests/fixtures/meta.kgl --no-open --port 0 &
```

- Exactly **one line on stdout**, JSON: `{"url","port","pid","graph"}`.
  Parse it; never scrape stderr, never race a hardcoded port.
- All diagnostics are on stderr.
- `--no-open` is mandatory for agents/CI (no browser spawn); `--port 0`
  means OS-assigned. Server binds 127.0.0.1 only.
- Error path: bad file → exit 1, **empty stdout**, one stderr line.

## 3. Inspect without a browser — the JSON twin

```bash
curl -s http://127.0.0.1:$PORT/api/session     # protocol_version, tier, counts
curl -s http://127.0.0.1:$PORT/api/meta-graph  # slots, edges, positions, bounds
curl -s http://127.0.0.1:$PORT/api/describe    # schema tiers, per-type detail
```

Same structs as the binary WebSocket protocol — divergence between the twin
and the wire is a bug, not a nuance. Truncated responses carry
`{returned, total, truncated}`; report those numbers, don't hide them.

## 4. Drive the real frontend

- Scripted: `make e2e` (Playwright, headless Chromium with
  `--use-gl=angle --use-angle=swiftshader --enable-unsafe-swiftshader`).
  Readiness is `window.__kglv.ready === true` — **never a fixed sleep**
  (cosmos.gl v3 is async-init and draws zero frames when static).
- Interactive (Claude in Chrome / a headed browser): open the reported URL,
  then read `window.__kglv` — `{protocolVersion, tier, pointCount,
  linkCount, ready, simRunning, lastMessageSeq, positionsHash,
  deviceFeatures, error}`. Assert on state; screenshots are artifacts.
  `error` non-null explains any `ready:false`.

## 5. Stop

Kill the `pid` from the stdout JSON line. Verify the port is released before
relaunching on a fixed port.

## Report shape

Paste observed output — the stdout line, curl payloads (trimmed), `__kglv`
dump — not summaries of it. "The tests pass" without "I ran it and here is
what it printed" is an incomplete report (`R2`).
