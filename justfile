set shell := ["bash", "-cu"]

# Transitional product graph: Rust + TypeScript + Shell is the target stack.
# Python recipes remain here as regression coverage during migration.
python := env_var_or_default("HEIWA_PYTHON", justfile_directory() + "/.venv/bin/python")
pytest := env_var_or_default("HEIWA_PYTEST", justfile_directory() + "/.venv/bin/python -m pytest")

default:
    @echo "Product graph recipes:"
    @echo "  test-trading   Run incubator trading tests"
    @echo "  check-web      Validate transitional web surface"
    @echo "  check-docs     Build MkDocs docs strictly"
    @echo "  fmt-docs       Format authored markdown (root + docs/) via deno fmt"
    @echo "  check-fmt-docs Check markdown formatting without writing"
    @echo "  check-machine-security Inspect/fix owner-local machine security posture"
    @echo "  verify-security Run dependency/security/type/product-surface gate"
    @echo "  rotate-security Run weekly security rotation and write ~/.heiwa evidence"
    @echo "  test-product   Run product test recipes"
    @echo "  check-product  Run product verification recipes"
    @echo "  verify-product Run product tests and checks"
    @echo "  deploy-product Push dev, PR to main, auto-merge on green CI"
    @echo "  drex-evals     Run DREX routing golden eval suite (E3)"

test-trading:
    cd apps/heiwa_trading && PYTHONPATH=src {{python}} -m pytest tests -q

check-web:
    {{python}} apps/heiwa_app/scripts/check_static_surface.py

check-docs:
    {{python}} -m mkdocs build --strict

# Scope lives in deno.json: authored docs only (root *.md + docs/); ops/, legacy/, .worktrees/ excluded
fmt-docs:
    deno fmt

check-fmt-docs:
    deno fmt --check

check-machine-security:
    bash scripts/check_machine_security.sh --fix

verify-security:
    bash scripts/verify_security.sh

rotate-security:
    bash scripts/weekly_security_rotate.sh

test-product: test-trading

check-product: check-web check-docs

verify-product: test-product check-product

# dev and main are branch-protected (enforce_admins + required checks).
# Experimental work reaches dev through its own PR before this production step.
# Requires explicit remote authorization and gh auth.
deploy-product:
    git fetch --prune origin dev main
    test "$(git branch --show-current)" = dev
    test "$(git rev-parse HEAD)" = "$(git rev-parse origin/dev)"
    bash scripts/check_branch_topology.sh --mode integration
    gh pr view dev --json url >/dev/null 2>&1 || gh pr create --base main --head dev --fill
    gh pr merge dev --merge --auto

# E3 DREX routing golden eval suite: L1 intent classification + L2 routing
# decisions. Hermetic (no providers/network/STDB). Fails on route regressions.
drex-evals:
    cargo test -p heiwa-protocol -p heiwa-core --test drex_golden -- --nocapture
