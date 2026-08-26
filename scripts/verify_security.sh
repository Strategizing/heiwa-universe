#!/usr/bin/env bash
set -u -o pipefail

SKIP_SECRET_SCAN=0
if [[ "${1:-}" == "--skip-secret-scan" ]]; then
  SKIP_SECRET_SCAN=1
elif [[ $# -ne 0 ]]; then
  echo "usage: scripts/verify_security.sh [--skip-secret-scan]" >&2
  exit 2
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
PYTHON_AUDIT_PYTHON="${PYTHON_AUDIT_PYTHON:-3.14}"

failures=0
warnings=0

section() {
  printf '\n== %s ==\n' "$1"
}

pass() {
  printf 'PASS %s\n' "$1"
}

warn() {
  warnings=$((warnings + 1))
  printf 'WARN %s\n' "$1"
}

fail() {
  failures=$((failures + 1))
  printf 'FAIL %s\n' "$1"
}

# Dependency audits are independent and read-only. Run them concurrently so
# the security gate costs the duration of its slowest ecosystem instead of the
# sum of every network and type-check operation. Logs are replayed in launch
# order to keep CI output deterministic and easy to audit.
parallel_labels=()
parallel_logs=()
parallel_pids=()

run_required_async() {
  local label="$1"
  shift
  local index="${#parallel_pids[@]}"
  local log="$TMPDIR/parallel-$index.log"

  (
    section "$label"
    "$@"
  ) >"$log" 2>&1 &

  parallel_labels+=("$label")
  parallel_logs+=("$log")
  parallel_pids+=("$!")
}

run_required() {
  local label="$1"
  shift
  local status=0

  section "$label"
  "$@" || status=$?
  if [[ "$status" -eq 0 ]]; then
    pass "$label"
  else
    fail "$label (exit $status)"
  fi
}

wait_required_async() {
  local index status
  for index in "${!parallel_pids[@]}"; do
    status=0
    wait "${parallel_pids[$index]}" || status=$?
    cat "${parallel_logs[$index]}"
    if [[ "$status" -eq 0 ]]; then
      pass "${parallel_labels[$index]}"
    else
      fail "${parallel_labels[$index]} (exit $status)"
    fi
  done
}

require_command() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    fail "missing command: $cmd"
    return 1
  fi
}

audit_python_project() {
  local label="$1"
  local project_dir="$2"
  local output_file="$TMPDIR/${label//[^A-Za-z0-9_.-]/_}.requirements.txt"

  require_command uv || return 1

  (
    cd "$project_dir" &&
      uv export \
        --frozen \
        --all-extras \
        --format requirements.txt \
        --no-hashes \
        --no-emit-project \
        --no-emit-local \
        --output-file "$output_file" >/dev/null &&
      uv tool run --python "$PYTHON_AUDIT_PYTHON" pip-audit \
        --cache-dir "$TMPDIR/pip-audit-cache-$label" \
        -r "$output_file"
  )
}

cd "$ROOT" || exit 1

section "security gate metadata"
printf 'root: %s\n' "$ROOT"
printf 'python-audit-python: %s\n' "$PYTHON_AUDIT_PYTHON"
printf 'git: %s\n' "$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
printf 'dirty: %s\n' "$(test -z "$(git status --porcelain 2>/dev/null)" && echo false || echo true)"

if command -v cargo-audit >/dev/null 2>&1; then
  # Calling the installed subcommand directly avoids rustup auto-syncing the
  # repository toolchain before the audit can run on hosted runners.
  run_required_async "cargo audit" cargo-audit audit
else
  run_required_async "cargo audit" cargo audit
fi
# The cockpit is a workspace member of the root package.json, so the root audit
# above already covers its dependencies. It has no lockfile of its own -- a
# workspace member with one is the orphan e70a1fe4 removed -- and both
# `npm audit` and `npm ci` scoped to that directory fail without one.
# apps/heiwa_app/desktop is not a workspace and is audited separately.
run_required_async "root npm audit" npm audit
run_required_async "desktop npm audit" npm --prefix apps/heiwa_app/desktop audit
run_required_async "root TypeScript typecheck" npm run typecheck --silent
run_required_async "desktop TypeScript typecheck" bash -c 'cd apps/heiwa_app/desktop && npm run typecheck --silent'
run_required_async "Deno herd/prototype typecheck" deno check scripts/herd.ts prototypes/heiwa-desk/main.ts prototypes/heiwa-desk/herdr.ts
run_required_async "root Python dependency audit" audit_python_project root "$ROOT"
run_required_async "runtime Python dependency audit" audit_python_project runtime-python "$ROOT/runtime/python"
run_required_async "product surface audit" bash scripts/audit_product_surface.sh
wait_required_async

if [[ "$SKIP_SECRET_SCAN" == "1" ]]; then
  section "gitleaks secret scan"
  pass "gitleaks secret scan already completed by the caller"
elif command -v gitleaks >/dev/null 2>&1; then
  run_required "gitleaks secret scan (redacted, blocking; reviewed baseline in .gitleaksignore)" \
    gitleaks detect --no-banner --redact --source "$ROOT"
else
  section "gitleaks secret scan"
  fail "gitleaks not installed; secret scanning is mandatory (brew install gitleaks)"
fi

section "security gate summary"
printf 'failures: %s\n' "$failures"
printf 'warnings: %s\n' "$warnings"

if [ "$failures" -ne 0 ]; then
  exit 1
fi
