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
# the gate. The bound's owner today is a human running `make clean-build`; an
# automatic purge is P6 work (doctrine R4 — bound and owner documented now,
# enforcement grows with the tree).
TARGET_WARN_MB ?= 20000
NODE_MODULES_WARN_MB ?= 1000

PYTHON ?= python3
CARGO ?= cargo
NPM ?= npm

.PHONY: help gate lint self-test clean-build \
        check-dev-docs check-skill-mirrors check-bans check-build-dirs \
        check-generated-ts sync-agents \
        rust-fmt rust-clippy rust-test frontend-install frontend-typecheck \
        frontend-build frontend-audit

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
gate: check-dev-docs check-skill-mirrors check-bans check-build-dirs \
      rust-fmt rust-clippy rust-test check-generated-ts \
      frontend-typecheck frontend-build frontend-audit  ## Local pre-push gate (runs what exists; reports the rest ABSENT)
	@echo ""
	@echo "gate: the following steps do not exist yet and were NOT run."
	@echo "gate: an absent step is not a pass (doctrine R10)."
	@echo "  ABSENT  pytest                     (no wheel)"
	@echo "  ABSENT  packaged-consumer check    (nothing is packaged)"
	@echo "  ABSENT  browser e2e smoke          (nothing renders yet)"
	@echo "gate: 11 checks ran, 3 absent."

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

# target/ and node_modules/ are the two paths this project writes outside git
# that nothing ever collects (a 2026-07 estate audit found 503 GB of cargo
# target dirs). Advisory by design — see TARGET_WARN_MB above.
check-build-dirs:  ## WARN when target/ or node_modules/ pass their advisory ceiling
	@for d in target frontend/node_modules; do \
	  if [ -d "$$d" ]; then \
	    mb=$$(du -sm "$$d" 2>/dev/null | cut -f1); \
	    case "$$d" in \
	      target) cap=$(TARGET_WARN_MB) ;; \
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

rust-fmt:  ## Formatting is decided by rustfmt, never in review
	@$(CARGO) fmt --all -- --check

rust-clippy:  ## Lints are errors: a warning nobody must fix is a warning nobody reads
	@$(CARGO) clippy --workspace --all-targets -- -D warnings

rust-test:  ## Workspace tests, including the kglite path-dep round trip
	@$(CARGO) test --workspace

# The generated TypeScript is written by ts-rs from the Rust message types
# during `cargo test`, so this regenerates and then asserts nothing moved. That
# catches both halves of the drift: a Rust type edited without regenerating the
# .ts, and a .ts hand-edited despite the "do not edit" header ts-rs writes.
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
	@$(CARGO) test -p kglite-visual-core --quiet >/dev/null
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
	@echo "check-generated-ts: OK"

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
self-test:  ## Prove the gate's checks can actually fail
	@$(PYTHON) scripts/check_dev_docs.py --self-test
	@$(PYTHON) scripts/check_skill_mirrors.py --self-test
	@$(PYTHON) scripts/check_bans.py --self-test
