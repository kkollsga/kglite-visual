# Contributing

The repository is
[kkollsga/kglite-visual](https://github.com/kkollsga/kglite-visual). Issues and
pull requests are welcome; this page is the short version of how to run the
thing.

## Build

Two toolchains. The frontend is **embedded into** the Rust binary, so the
frontend half builds first — every cargo step downstream of it compiles against
whatever bundle is on disk.

```bash
cd frontend && npm ci && npm run build && cd ..
cargo build -p kglite-visual-cli
```

Never hard-code a path to the binary. `scripts/check_bundle.py
--resolve-binary kglite-visual` resolves newest-of-profile and **refuses a
binary older than the bundle it should embed** — a stale bundle inside a fresh
binary looks exactly like a backend bug.

For the wheel:

```bash
make py-develop     # build the extension into the project venv
make wheel          # a wheel into target/wheels
```

## The gate

```bash
make gate
```

The local pre-push gate. It runs eighteen real checks and prints an **ABSENT**
line for any step that does not exist yet — today there are none. "Green" and
"not attempted" must not render identically.

```bash
make lint        # static checks only: bans, fmt, clippy, tsc
make self-test   # prove every checker in the gate can actually go red
make e2e         # browser end-to-end smoke (Playwright + SwiftShader)
make pytest      # the wheel's suite
make docs        # this documentation, built with -W --keep-going
make help        # every target, with a line on why it exists
```

Membership in the gate is **earned**: a check belongs there once it has a record
of catching a CI failure, and everything else is CI's job.

CI runs sixteen of the gate's eighteen checks across five jobs, plus a docs job
and a `ci-success` aggregate. The two with no CI job are local-only by
construction — one bounds a gitignored directory that never reaches a checkout,
and one compares against an untracked generated file.

## Running the end-to-end suite

```bash
make e2e
```

Playwright, headless Chromium with `--use-gl=angle --use-angle=swiftshader
--enable-unsafe-swiftshader`. The renderer is WebGL2 and headless Chromium has
no GPU, so ANGLE over SwiftShader is what makes this stack render at all.

The suite launches the real binary itself — there is deliberately no Playwright
`webServer`, because the launch contract (one JSON line on stdout with the
resolved port) is part of what is under test.

Readiness is `window.__kglv.ready === true`, **never a fixed sleep**: cosmos.gl
v3 is async-init and draws zero frames when static. Specs assert on
`window.__kglv` state; screenshots are artifacts, not assertions.

Point `KGLITE_VISUAL_CONFIG_DIR` at a temporary directory in any harness, or it
reads and writes the developer's own [saved queries](concepts/storage.md).

## Conventions

The full engineering doctrine lives in `CLAUDE.md` at the repo root — build and
test policy, commit and release rules, performance protocol, the code-health
bar. Two rules that shape almost every diff:

- **Documentation is a claim.** A comment, a docstring or a page that says
  something the code does not do is a defect of the same kind as a wrong
  branch.
- **Baselines are never regenerated to get green.** The protocol framing
  baseline and the golden render baseline are exact. A red baseline after a
  deliberate change is a conscious decision: regenerate in the same commit and
  say why.

`CHANGELOG.md` records what a **user** can see change — the viewer, the wheel,
the CLI. Internal refactors, CI plumbing and test-only work do not appear
there.

## Two things not to do

**Never install any `@cosmograph/*` package.** The `@cosmograph` npm family —
including `@cosmograph/cosmos`, the renderer's pre-donation name — is
CC-BY-NC-4.0 and incompatible with shipping this MIT app. The two packages
share version numbers, so the name is the only guard. A gate check enforces it.

**Do not add a `#[global_allocator]` to the Python crate.** A second global
allocator in a loaded extension module segfaults in somebody else's notebook.
Also a gate check.
