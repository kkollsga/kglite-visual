# kglite-visual — local entry points.
#
# Small named targets, each with a comment saying *why* it exists. Every ABSENT
# line below is deleted in the same change that makes its step real: "green" and
# "not attempted" must never render identically (doctrine R10 corollary), and an
# honesty label nobody retires becomes a lie.

# Size ceiling for the gitignored dev-docs/ working folder. dev-docs/ never
# reaches CI, so a local gate is the only thing that can ever see it growing
# (doctrine R4: every file accumulation has a bound and an owner). 200 MB is a
# starting value for a repo whose bench/out/ will hold generated .kgl fixtures
# and protocol captures; raise it deliberately, with a reason, not because a
# run went red.
DEV_DOCS_MAX_MB ?= 200

# Advisory ceilings for the two build directories cargo and npm never garbage
# collect. WARN, not FAIL: a legitimately large target/ mid-refactor is not a
# reason to block a commit, and a gate that blocks on it trains people to skip
# the gate. These three stay human-owned through `make clean-build`, and
# deliberately so: they hold no age-tiered content, only a cache whose whole
# value is being warm, so an automatic sweep would cost rebuild time and free
# nothing that will not be refilled. The accumulations that DO declare a
# lifetime — dev-docs' disposable tiers and the Playwright artifacts — are
# purged by `make prune` (doctrine R4).
TARGET_WARN_MB ?= 20000
NODE_MODULES_WARN_MB ?= 1000
# The project-local venv the wheel's test loop lives in. Same tier, same owner
# (`make clean-build`), same advisory report. It holds maturin, pytest and an
# editable install of the extension; ~200 MB is a fresh one with a debug .so.
VENV_WARN_MB ?= 800

PYTHON ?= python3
CARGO ?= cargo
NPM ?= npm

# ts-rs rewrites every `#[ts(export)]` type into frontend/src/generated/ on
# every `cargo test`, unchanged content included — and `check_bundle.py
# --freshness` counts those files as sources of the embedded bundle. So a gate
# run that merely *proved* the generated TypeScript had not moved left it
# newer than the bundle, and the bundle the gate had just built read as stale
# to everything downstream: the bench harness refused to start immediately
# after a green gate for exactly this reason.
#
# Every step that runs `cargo test` therefore brackets it with these two. The
# stamp is the newest generated file's timestamp from BEFORE the run, so a
# genuinely new generated type keeps its own (correctly newer) timestamp and
# still trips the freshness check — the restore only undoes the bump a no-op
# regeneration caused. Content drift is caught by the git comparisons in
# check-generated-ts, which mtimes have nothing to do with.
GENERATED_TS_STAMP = target/.generated-ts.stamp
SAVE_GENERATED_MTIMES = mkdir -p target; \
  newest="$$(ls -t frontend/src/generated/*.ts 2>/dev/null | head -1)"; \
  if [ -n "$$newest" ]; then touch -r "$$newest" $(GENERATED_TS_STAMP); \
  else rm -f $(GENERATED_TS_STAMP); fi
RESTORE_GENERATED_MTIMES = if [ -f $(GENERATED_TS_STAMP) ]; then \
  find frontend/src/generated -name '*.ts' \
    -exec touch -r $(GENERATED_TS_STAMP) {} +; \
  fi

# One project-local virtualenv, created on demand and reused thereafter, so
# `make pytest` means the same thing on every machine and in CI. Created rather
# than skipped: an ABSENT pytest step and a passing one must not depend on
# whether the developer happened to have a venv (doctrine R10).
VENV ?= .venv
VENV_STAMP = $(VENV)/.kglv-tooling
# Wheels land in target/, which already has a bound and an owner. A new
# top-level dist/ would be a new accumulation tier needing both (doctrine R4).
WHEEL_DIR = target/wheels
# `maturin build` is a debug build by default, and the gate wants it that way:
# the embedded-bundle question this check exists to ask is answered identically
# by either profile (rust-embed's `debug-embed` is always on), and a release
# build on every gate run is minutes of LTO for no extra evidence. The release
# artifact is `make wheel WHEEL_PROFILE=--release`, which is what P6's CI runs.
WHEEL_PROFILE ?=

.PHONY: help gate lint self-test clean-build prune \
        check-dev-docs check-skill-mirrors check-bans check-build-dirs \
        check-generated-ts check-protocol-baseline check-bundle sync-agents \
        rust-fmt rust-clippy rust-test cli-build fixture e2e \
        frontend-install frontend-typecheck frontend-build frontend-audit \
        py-venv py-venv-refresh py-develop pytest wheel check-packaged-consumer

help:  ## List the targets
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | sort | \
	  awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-22s\033[0m %s\n", $$1, $$2}'

# The local pre-push gate. Membership is EARNED: a check belongs here once it
# has a record of catching a CI failure, and everything else is CI's job.
# Duplicating a parallel CI matrix serially on one machine is the single
# biggest waste this estate has measured.
#
# Order is deliberate — the cheap doc/config checks and the two structural bans
# run before anything that compiles, so a licence violation or a stray global
# allocator is reported in under a second instead of after a cargo build.
# The frontend half runs FIRST, and that is load-bearing rather than
# aesthetic: `frontend/dist` is baked into the CLI binary by rust-embed, so
# every cargo step downstream of it compiles against whatever bundle is on
# disk. Building the bundle, checking it is newer than its sources, and only
# then compiling the binary is the order that makes the stale-bundle trap
# (CLAUDE.md → "Two toolchains, one gate") unreachable instead of merely
# documented.
gate: check-dev-docs check-skill-mirrors check-bans check-build-dirs \
      frontend-typecheck frontend-build check-bundle \
      rust-fmt rust-clippy rust-test check-generated-ts check-protocol-baseline \
      check-render-baseline \
      e2e pytest check-packaged-consumer frontend-audit  ## Local pre-push gate
	@echo ""
	@echo "gate: 17 checks ran, 0 absent."

lint: check-dev-docs check-skill-mirrors check-bans rust-fmt rust-clippy frontend-typecheck  ## Static checks only — no test execution

# The R4 bound. Reports, never deletes: deciding whether something is
# reproducible, and which tier it belongs in, is a judgement the script must
# not make. Purging is the dev-docs-cleanup skill's job.
check-dev-docs:  ## FAIL if dev-docs/ exceeds DEV_DOCS_MAX_MB; warn on overdue tiers
	@$(PYTHON) scripts/check_dev_docs.py --max-mb $(DEV_DOCS_MAX_MB)

# The R7 mirror. Two harness adapters that disagree do not merely lag — the
# stale one teaches a procedure the live one warns against.
check-skill-mirrors:  ## Assert .agents/ is an unmodified adapter of the authority
	@$(PYTHON) scripts/check_skill_mirrors.py

# Two bans whose violation is silent: a CC-BY-NC renderer that works perfectly
# while making the repo unshippable, and a second global allocator that
# segfaults in someone else's notebook. Both scans assert they scanned
# something, so a moved crate cannot turn either into a vacuous pass.
check-bans:  ## FAIL on an @cosmograph/* dependency or a #[global_allocator] in the py crate
	@$(PYTHON) scripts/check_bans.py

# The three paths this project writes outside git that nothing ever collects
# (a 2026-07 estate audit found 503 GB of cargo target dirs). `.venv/` joined
# them in P5 with the wheel's test loop; `target/wheels/` and the fresh venv the
# packaged-consumer probe builds need no separate entry — the first is inside
# target/, and the second is a tempdir the probe deletes in a `finally`.
# Advisory by design — see TARGET_WARN_MB above.
check-build-dirs:  ## WARN when target/, node_modules/ or .venv/ pass their ceiling
	@for d in target frontend/node_modules $(VENV); do \
	  if [ -d "$$d" ]; then \
	    mb=$$(du -sm "$$d" 2>/dev/null | cut -f1); \
	    case "$$d" in \
	      target) cap=$(TARGET_WARN_MB) ;; \
	      $(VENV)) cap=$(VENV_WARN_MB) ;; \
	      *)      cap=$(NODE_MODULES_WARN_MB) ;; \
	    esac; \
	    if [ "$$mb" -gt "$$cap" ]; then \
	      echo "check-build-dirs: WARN $$d is $${mb} MB (advisory ceiling $${cap} MB) — run 'make clean-build'"; \
	    fi; \
	  fi; \
	done
	@echo "check-build-dirs: OK"

clean-build:  ## Delete the regenerable build directories (owner of the R4 bound above)
	@$(CARGO) clean
	@rm -rf frontend/node_modules frontend/dist
	@rm -rf $(VENV) .pytest_cache

# The other half of check-dev-docs. That check REPORTS files past their tier's
# purge lifetime and deletes nothing, because tier assignment is a judgement a
# script must not make — which left the bound's only enforcement a human
# reading a WARN. This deletes, but strictly inside tiers declared disposable
# in advance (dev-docs/README.md's table, imported rather than copied) and
# strictly past the lifetime those declarations state. The dev-docs-cleanup
# skill stays the owner of the judgement: what belongs in which tier, and what
# must be rescued out of a disposable one before its clock runs down.
# `--dry-run` lists without deleting; `scripts/prune.py --self-test` proves it
# both purges and spares (R1).
prune:  ## Purge dev-docs' disposable tiers and stale Playwright artifacts by age
	@$(PYTHON) scripts/prune.py

rust-fmt:  ## Formatting is decided by rustfmt, never in review
	@$(CARGO) fmt --all -- --check

rust-clippy:  ## Lints are errors: a warning nobody must fix is a warning nobody reads
	@$(CARGO) clippy --workspace --all-targets -- -D warnings

rust-test:  ## Workspace tests, including the kglite .kgl round trip
	@$(SAVE_GENERATED_MTIMES)
	@$(CARGO) test --workspace
	@$(RESTORE_GENERATED_MTIMES)

# The generated TypeScript is written by ts-rs from the Rust message types
# during `cargo test`, so this regenerates and then asserts nothing moved. That
# catches both halves of the drift: a Rust type edited without regenerating the
# .ts, and a CHANGED generated type — whether from an unregenerated Rust
# edit or a hand-edit that was staged. (An *unstaged* hand-edit is
# overwritten by the regeneration above rather than reported, which is the
# right outcome and not a catch.)
#
# Three assertions, because each misses what the others catch:
#   1. the directory is non-empty     — an export that silently wrote nothing
#      would otherwise pass vacuously (R1);
#   2. no untracked file in it        — a NEW generated type that was never
#      `git add`ed is invisible to `git diff`;
#   3. worktree matches the index     — a CHANGED generated type. Compared
#      against the index rather than HEAD so a correctly-staged regeneration
#      passes, including in a first commit.
check-generated-ts:  ## FAIL if frontend/src/generated/ is stale or hand-edited
	@$(SAVE_GENERATED_MTIMES)
	@$(CARGO) test -p kglite-visual-core --quiet
	@if [ -z "$$(ls -A frontend/src/generated 2>/dev/null)" ]; then \
	  echo "check-generated-ts: FAIL — frontend/src/generated/ is empty; ts-rs exported nothing" >&2; \
	  exit 1; \
	fi
	@untracked="$$(git ls-files --others --exclude-standard -- frontend/src/generated)"; \
	if [ -n "$$untracked" ]; then \
	  echo "check-generated-ts: FAIL — generated TypeScript is not tracked, so drift is invisible:" >&2; \
	  echo "$$untracked" >&2; \
	  exit 1; \
	fi
	@if ! git diff --quiet -- frontend/src/generated; then \
	  echo "check-generated-ts: FAIL — generated TypeScript is stale or hand-edited:" >&2; \
	  git diff --stat -- frontend/src/generated >&2; \
	  echo "  Regenerate with 'cargo test -p kglite-visual-core' and stage the result." >&2; \
	  exit 1; \
	fi
	@$(RESTORE_GENERATED_MTIMES)
	@echo "check-generated-ts: OK"

# The two-toolchain seam. `--freshness` fails when the embedded bundle is
# older than the sources that produce it; the same script's --resolve-binary
# mode is what the e2e harness uses to pick a binary, so neither half can
# quietly settle for a stale artifact.
check-bundle:  ## FAIL if frontend/dist is older than frontend/src
	@$(PYTHON) scripts/check_bundle.py --freshness

# The framing baseline is EXACT (test-plan L2). Same shape as
# check-generated-ts: `cargo test` rewrites it, and this asserts nothing moved.
# A red here after a deliberate protocol change is regenerated in the same
# commit, with the version bumped and the reason stated — never to silence a
# diff nobody can explain (CLAUDE.md → "Gate honesty").
check-protocol-baseline:  ## FAIL if the committed protocol framing baseline moved
	@$(SAVE_GENERATED_MTIMES)
	@$(CARGO) test -p kglite-visual-core --quiet
	@$(RESTORE_GENERATED_MTIMES)
	@if [ -z "$$(ls -A crates/kglite-visual-core/tests/baselines 2>/dev/null)" ]; then \
	  echo "check-protocol-baseline: FAIL — the baseline directory is empty; the generator wrote nothing" >&2; \
	  exit 1; \
	fi
	@untracked="$$(git ls-files --others --exclude-standard -- crates/kglite-visual-core/tests/baselines)"; \
	if [ -n "$$untracked" ]; then \
	  echo "check-protocol-baseline: FAIL — a baseline file is untracked, so drift is invisible:" >&2; \
	  echo "$$untracked" >&2; \
	  exit 1; \
	fi
	@if ! git diff --quiet -- crates/kglite-visual-core/tests/baselines; then \
	  echo "check-protocol-baseline: FAIL — the wire format changed:" >&2; \
	  git diff -- crates/kglite-visual-core/tests/baselines >&2; \
	  echo "  If that was deliberate: bump PROTOCOL_VERSION, regenerate, and say why in the commit." >&2; \
	  exit 1; \
	fi
	@echo "check-protocol-baseline: OK"

# The render's exact baseline (plan D13). Its own step rather than a widening of
# check-protocol-baseline, and the reason is the failure message: that check
# tells the reader to bump PROTOCOL_VERSION, which is the wrong instruction for
# a moved circle. Same shape otherwise — `cargo test` regenerates, this asserts
# nothing moved — and the same rule applies: a red here after a deliberate
# encoding, layout or emitter change is regenerated in the same commit with the
# reason stated, never to silence a diff nobody can explain (CLAUDE.md → "Gate
# honesty").
check-render-baseline:  ## FAIL if the committed golden SVG renders moved
	@$(SAVE_GENERATED_MTIMES)
	@$(CARGO) test -p kglite-visual-core --test render_golden --quiet
	@$(RESTORE_GENERATED_MTIMES)
	@if [ -z "$$(ls -A crates/kglite-visual-core/tests/goldens 2>/dev/null)" ]; then \
	  echo "check-render-baseline: FAIL — the goldens directory is empty; the generator wrote nothing" >&2; \
	  exit 1; \
	fi
	@untracked="$$(git ls-files --others --exclude-standard -- crates/kglite-visual-core/tests/goldens)"; \
	if [ -n "$$untracked" ]; then \
	  echo "check-render-baseline: FAIL — a golden is untracked, so drift is invisible:" >&2; \
	  echo "$$untracked" >&2; \
	  exit 1; \
	fi
	@if ! git diff --quiet -- crates/kglite-visual-core/tests/goldens; then \
	  echo "check-render-baseline: FAIL — the rendered image changed:" >&2; \
	  git diff --stat -- crates/kglite-visual-core/tests/goldens >&2; \
	  echo "  If that was deliberate: regenerate with 'cargo test -p kglite-visual-core', stage it, and say why." >&2; \
	  exit 1; \
	fi
	@echo "check-render-baseline: OK"

cli-build:  ## Build the CLI binary the e2e harness drives
	@$(CARGO) build -p kglite-visual-cli

# L3. Launches the real binary, parses its stdout contract, drives headless
# Chromium over SwiftShader and asserts on window.__kglv. Depends on cli-build
# because a harness that silently tests last week's binary is worse than no
# harness.
e2e: cli-build  ## Browser end-to-end smoke (Playwright + SwiftShader)
	@cd frontend && npx playwright test

# Regenerate the committed fixture, then prove it is byte-stable. The second
# half is the point: a fixture that changed on every regeneration could not be
# an exact baseline, and nothing else in the tree would notice.
fixture:  ## Regenerate crates/kglite-visual-core/tests/fixtures/ (seeded, byte-stable)
	@$(CARGO) run -q -p kglite-visual-core --example make_fixture
	@cp crates/kglite-visual-core/tests/fixtures/meta.kgl /tmp/kglv-fixture-once.kgl
	@cp crates/kglite-visual-core/tests/fixtures/spill.kgl /tmp/kglv-spill-once.kgl
	@cp crates/kglite-visual-core/tests/fixtures/meta.positions.json /tmp/kglv-positions-once.json
	@$(CARGO) run -q -p kglite-visual-core --example make_fixture
	@cmp /tmp/kglv-fixture-once.kgl crates/kglite-visual-core/tests/fixtures/meta.kgl \
	  || { echo "fixture: FAIL — regeneration is not byte-stable" >&2; exit 1; }
	@cmp /tmp/kglv-spill-once.kgl crates/kglite-visual-core/tests/fixtures/spill.kgl \
	  || { echo "fixture: FAIL — the spill fixture is not byte-stable" >&2; exit 1; }
	@cmp /tmp/kglv-positions-once.json crates/kglite-visual-core/tests/fixtures/meta.positions.json \
	  || { echo "fixture: FAIL — the positions baseline is not byte-stable" >&2; exit 1; }
	@rm -f /tmp/kglv-fixture-once.kgl /tmp/kglv-spill-once.kgl /tmp/kglv-positions-once.json
	@echo "fixture: OK — regenerated twice, byte-identical"

# ---- the Python wheel -------------------------------------------------
#
# One project-local venv, created once and reused. The stamp file has no
# prerequisites on purpose: re-resolving pip requirements on every gate run
# would put the network on the critical path of a check that has nothing to do
# with the network. `make py-venv-refresh` is the deliberate way to move the
# tooling.
$(VENV_STAMP):
	@test -x $(VENV)/bin/python || $(PYTHON) -m venv $(VENV)
	@$(VENV)/bin/python -m pip install -q --upgrade pip
	@$(VENV)/bin/python -m pip install -q maturin pytest
	@touch $@

py-venv: $(VENV_STAMP)  ## Create (or reuse) the project-local venv the wheel is tested in

py-venv-refresh:  ## Re-install the venv's tooling from the network
	@rm -f $(VENV_STAMP)
	@$(MAKE) py-venv

# `maturin develop` compiles the extension into the venv. pytest depends on it
# because a suite that silently tested last week's .so is worse than no suite —
# the same trap as a stale frontend bundle inside a fresh binary, one toolchain
# over.
py-develop: py-venv  ## Build the extension into the venv (editable install)
	@VIRTUAL_ENV=$(CURDIR)/$(VENV) $(VENV)/bin/maturin develop -q -m crates/kglite-visual-py/Cargo.toml

# L5. The launch contract from Python, the to_bytes() handover, the error
# paths, the notebook rendering, and the three shutdown routes — two of which
# are subprocess tests with a wall-clock timeout, because the failure they
# guard is a hang and a hang is invisible to any assertion after it.
pytest: py-develop  ## The wheel's test suite (test-plan L5)
	@$(VENV)/bin/python -m pytest

wheel: py-venv  ## Build a wheel into target/wheels (WHEEL_PROFILE=--release for the artifact)
	@mkdir -p $(WHEEL_DIR)
	@VIRTUAL_ENV=$(CURDIR)/$(VENV) $(VENV)/bin/maturin build $(WHEEL_PROFILE) \
	  -m crates/kglite-visual-py/Cargo.toml -o $(WHEEL_DIR)

# The one check that can see what a source-tree test cannot: the *artifact*
# missing something the sources have. The frontend bundle lives inside the
# extension module, so a wheel built against an empty frontend/dist imports,
# serves, and answers / with an error page — and nothing upstream notices.
# Installs into a throwaway venv and probes from outside the repo root, so the
# source tree cannot shadow the package being tested.
check-packaged-consumer: wheel  ## Install the built wheel elsewhere and drive it
	@$(PYTHON) scripts/check_wheel.py

frontend-install:  ## Install frontend deps from the committed lockfile
	@cd frontend && $(NPM) ci

frontend-typecheck:  ## tsc over the app and the generated protocol types
	@cd frontend && $(NPM) run --silent typecheck

frontend-build:  ## PRODUCTION build — the only bundle a perf number may describe (R11)
	@cd frontend && $(NPM) run --silent build

# WARN-only, deliberately. `npm audit` queries the registry, so it fails on a
# plane, on a flaky network, and whenever the advisory database is briefly
# unreachable — none of which are facts about this diff. A gate that goes red
# for reasons unrelated to the change is a gate people learn to bypass. CI,
# which always has a network, owns the failing version of this check.
frontend-audit:  ## Report production-dependency advisories (never fails the gate)
	@cd frontend && $(NPM) audit --omit=dev || \
	  echo "frontend-audit: WARN advisories or no network — see above; CI owns the failing version"

# Regenerate, never merge. If the adapter diverged because someone edited it,
# that edit is LOST here — classify the divergence and merge any improvement
# into the authority FIRST (doctrine R7 + R14).
sync-agents:  ## Regenerate .agents/ + AGENTS.md from the CLAUDE.md authority
	@$(PYTHON) scripts/check_skill_mirrors.py --sync

# A verification that has never been seen failing is not yet a verification
# (doctrine R1). Every checker carries a self-test; run it after touching one.
self-test: wheel  ## Prove the gate's checks can actually fail
	@$(PYTHON) scripts/check_dev_docs.py --self-test
	@$(PYTHON) scripts/check_skill_mirrors.py --self-test
	@$(PYTHON) scripts/check_bans.py --self-test
	@$(PYTHON) scripts/check_bundle.py --self-test
	@$(PYTHON) scripts/check_wheel.py --self-test
	@$(PYTHON) scripts/prune.py --self-test

# Size-gated target prune (doctrine 0.1.9): a bound checked only at
# milestones is not a bound. Free no-op on a lean tree; a mid-plan cold
# rebuild is cheaper than a mid-phase ENOSPC. The phased-plan loop runs
# this after every phase commit; the release runs it before its heaviest
# build. Ceiling shared with the advisory check (TARGET_WARN_MB).
prune-target:  ## cargo clean iff target/ exceeds TARGET_WARN_MB
	@if [ -d target ]; then \
	  mb=$$(du -sm target 2>/dev/null | cut -f1); \
	  if [ "$$mb" -gt $(TARGET_WARN_MB) ]; then \
	    echo "prune-target: target/ is $${mb} MB (> $(TARGET_WARN_MB)) — running cargo clean"; \
	    $(CARGO) clean; \
	  else \
	    echo "prune-target: OK — target/ is $${mb} MB (ceiling $(TARGET_WARN_MB))"; \
	  fi; \
	else echo "prune-target: OK — no target/"; fi
