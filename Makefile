# kglite-visual — local entry points.
#
# Small named targets, each with a comment saying *why* it exists. Today the
# gate runs two real checks and honestly reports the rest as ABSENT: "green"
# and "not attempted" must never render identically (doctrine R10 corollary).
# Every ABSENT line below is deleted in the same change that makes its step
# real — an honesty label nobody retires becomes a lie.

# Size ceiling for the gitignored dev-docs/ working folder. dev-docs/ never
# reaches CI, so a local gate is the only thing that can ever see it growing
# (doctrine R4: every file accumulation has a bound and an owner). 200 MB is a
# starting value for a repo whose bench/out/ will hold generated .kgl fixtures
# and protocol captures; raise it deliberately, with a reason, not because a
# run went red.
DEV_DOCS_MAX_MB ?= 200

PYTHON ?= python3

.PHONY: help gate lint check-dev-docs check-skill-mirrors sync-agents self-test

help:  ## List the targets
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | sort | \
	  awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-22s\033[0m %s\n", $$1, $$2}'

# The local pre-push gate. Membership is EARNED: a check belongs here once it
# has a record of catching a CI failure, and everything else is CI's job.
# Duplicating a parallel CI matrix serially on one machine is the single
# biggest waste this estate has measured.
gate: check-dev-docs check-skill-mirrors  ## Local pre-push gate (runs what exists; reports the rest ABSENT)
	@echo ""
	@echo "gate: the following steps do not exist yet and were NOT run."
	@echo "gate: an absent step is not a pass (doctrine R10)."
	@echo "  ABSENT  cargo fmt --check          (no Cargo.toml)"
	@echo "  ABSENT  cargo clippy -D warnings   (no workspace)"
	@echo "  ABSENT  cargo test                 (no crates)"
	@echo "  ABSENT  frontend typecheck/lint/test/build   (no frontend)"
	@echo "  ABSENT  pytest                     (no wheel)"
	@echo "  ABSENT  packaged-consumer check    (nothing is packaged)"
	@echo "gate: 2 checks ran, 6 absent."

lint: check-dev-docs check-skill-mirrors  ## Doc/config checks only, today
	@echo "lint: source linting is ABSENT — there is no source."

# The R4 bound. Reports, never deletes: deciding whether something is
# reproducible, and which tier it belongs in, is a judgement the script must
# not make. Purging is the dev-docs-cleanup skill's job.
check-dev-docs:  ## FAIL if dev-docs/ exceeds DEV_DOCS_MAX_MB; warn on overdue tiers
	@$(PYTHON) scripts/check_dev_docs.py --max-mb $(DEV_DOCS_MAX_MB)

# The R7 mirror. Two harness adapters that disagree do not merely lag — the
# stale one teaches a procedure the live one warns against.
check-skill-mirrors:  ## Assert .agents/ is an unmodified adapter of the authority
	@$(PYTHON) scripts/check_skill_mirrors.py

# Regenerate, never merge. If the adapter diverged because someone edited it,
# that edit is LOST here — classify the divergence and merge any improvement
# into the authority FIRST (doctrine R7 + R14).
sync-agents:  ## Regenerate .agents/ + AGENTS.md from the CLAUDE.md authority
	@$(PYTHON) scripts/check_skill_mirrors.py --sync

# A verification that has never been seen failing is not yet a verification
# (doctrine R1). Both checkers carry a self-test; run it after touching either.
self-test:  ## Prove the gate's checks can actually fail
	@$(PYTHON) scripts/check_dev_docs.py --self-test
	@$(PYTHON) scripts/check_skill_mirrors.py --self-test
