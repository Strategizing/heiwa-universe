#!/usr/bin/env bash
# Run the Rust certification lane plus the repository's full local acceptance gates.
# Pass --full to add the Lance certification used when its boundary changes.
#
# Why this exists: the rust-matrix job failed four times in a row on this
# branch, each time for a different reason, because "it passes locally" was
# being said about WEAKER commands than CI actually runs:
#
#   - bare `cargo clippy` reports warnings; CI uses `-D warnings` with a
#     specific allow-list, so every warning is a hard error there
#   - `cargo test` without `--locked` tolerates a stale Cargo.lock
#   - `cargo machete` was never run locally at all
#   - the workspace builds fine on a Mac with Homebrew protoc; no runner has it
#
# Keep the Rust commands in sync with ci.yml and certification.yml.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
repo_root="$(pwd -P)"
source "$repo_root/scripts/lib/resolve_local_python.sh"

FULL=0
if [[ "${1:-}" == "--full" ]]; then
  FULL=1
elif [[ $# -ne 0 ]]; then
  echo "usage: scripts/check_ci_local.sh [--full]" >&2
  exit 2
fi

export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_INCREMENTAL=0

if ! local_python="$(resolve_local_python "$repo_root")"; then
  echo "unable to resolve a local Python interpreter" >&2
  exit 1
fi
local_pytest="${HEIWA_PYTEST:-$local_python -m pytest}"

FAILED=()
step() {
  local name="$1"; shift
  printf '  %-34s ' "$name"
  if "$@" >/tmp/heiwa_ci_step.log 2>&1; then
    echo "OK"
  else
    echo "FAIL"
    FAILED+=("$name")
    sed -n '1,25p' /tmp/heiwa_ci_step.log | sed 's/^/      /'
  fi
}

if [[ "$FULL" == "1" ]]; then
  echo "== full-certification prerequisites =="
  printf '  %-34s ' "protoc (Lance certification)"
  if command -v protoc >/dev/null 2>&1; then echo "OK ($(protoc --version))"
  else echo "MISSING - Lance certification will not build"; FAILED+=("protoc"); fi
fi

echo
echo "== rust promotion and certification =="
step "cargo fmt --check" cargo fmt --all -- --check
step "cargo test (fast)" cargo test --workspace --exclude heiwa-desktop --locked --no-default-features
if [[ "$FULL" == "1" ]]; then
  step "cargo test (Lance)" cargo test --workspace --exclude heiwa-desktop --locked \
    --features heiwa-shell/lance
fi
step "cargo clippy -D warnings" cargo clippy --workspace --exclude heiwa-desktop --locked \
  --all-targets --no-default-features -- \
  -A clippy::too_many_arguments -A clippy::new_without_default -A clippy::unnecessary_to_owned \
  -A clippy::needless_range_loop -A clippy::approx_constant -A clippy::collapsible_if \
  -A clippy::bool_assert_comparison -A clippy::type_complexity \
  -A clippy::needless_borrows_for_generic_args -A clippy::unnecessary_unwrap -D warnings
step "cargo machete" cargo machete

echo
echo "== web + docs (ci.yml: lint, docs) =="
step "npm ci" npm ci --ignore-scripts
step "npm ci (desktop)" npm ci --ignore-scripts --prefix apps/heiwa_app/desktop
step "npm run typecheck" npm run typecheck
step "npm run lint" npm run lint

echo
echo "== python =="
step "pytest" "$local_python" -m pytest -q
step "just test-product" env \
  HEIWA_PYTHON="$local_python" \
  HEIWA_PYTEST="$local_pytest" \
  just test-product

echo
echo "== repo gates =="
step "local Python resolver" bash scripts/tests/test_local_python_resolution.sh
step "Justfile Python override" bash scripts/tests/test_just_python_override.sh
step "claim gate portability" bash scripts/tests/test_check_claims.sh
for s in check_agent_baseline check_backend_transition check_model_call_boundary \
         check_release_metadata check_runtime_baseline verify_security check_machine_security \
         check_heiwa_core_dockerfile check_workflow_pins check_public_installer; do
  step "$s" bash "scripts/$s.sh"
done

# Acceptance gates for every release whose ledger claims completion. The Work
# Fabric design requires promotion to invoke these, not just the Stop hook:
# without them a layer can drift red while its stamp still points at an old
# HEAD. Each script re-stamps itself on a clean tree, which is also what clears
# the Stop gate. A script that does not exist yet is skipped, not failed — its
# release has not reached the checkpoint that adds it.
echo
echo "== release acceptance gates =="
for s in check_l0_acceptance check_l1_acceptance check_l2_acceptance \
         check_work_fabric_a1_acceptance; do
  if [[ -f "scripts/$s.sh" ]]; then
    step "$s" bash "scripts/$s.sh"
  else
    printf '  %-34s %s\n' "$s" "SKIP (not yet written)"
  fi
done

echo
if [ ${#FAILED[@]} -eq 0 ]; then
  echo "ALL GREEN — safe to push."
else
  echo "FAILED (${#FAILED[@]}): ${FAILED[*]}"
  exit 1
fi
