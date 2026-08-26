#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

expected_branch="${HEIWA_BASELINE_BRANCH:-dev}"
branch_mode="${HEIWA_BRANCH_MODE:-integration}"
allow_dirty=0

usage() {
  cat >&2 <<'EOF'
Usage: scripts/check_agent_baseline.sh [--allow-dirty] [--branch <name>]

Local-only agent baseline gate. This script does not fetch, push, call gh, or
perform network health checks. Remote pre-flight remains a separate explicitly
assigned operation.

Set HEIWA_BRANCH_MODE to `experimental` for a branch descended from `dev`, or
to `post-promotion` for the brief synchronized dev/main handoff. The default is
`integration`, which requires value-bearing work on `dev` ahead of `main`.

Checks:
  - current checkout is on the expected integration branch (default: dev)
  - cached origin/<branch> ref exists and local ahead/behind is reportable
  - no tracked dirty files unless --allow-dirty is passed
  - untracked files are absent except ignored or explicit vendor/ quarantine material
  - no duplicate worktree owns the integration branch
  - repo baseline, release metadata, Dockerfile, and product-surface checks pass
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --allow-dirty)
      allow_dirty=1
      shift
      ;;
    --branch)
      if [[ $# -lt 2 ]]; then
        usage
        exit 2
      fi
      expected_branch="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage
      exit 2
      ;;
  esac
done

case "$branch_mode" in
  integration|experimental|post-promotion) ;;
  *)
    printf 'FAIL: invalid HEIWA_BRANCH_MODE: %s\n' "$branch_mode" >&2
    exit 2
    ;;
esac

fail=0

ok() {
  printf 'OK: %s\n' "$*"
}

warn() {
  printf 'WARN: %s\n' "$*" >&2
}

fail_msg() {
  printf 'FAIL: %s\n' "$*" >&2
  fail=1
}

run_gate() {
  local label="$1"
  shift
  local output
  if output="$("$@" 2>&1)"; then
    ok "$label"
  else
    fail_msg "$label"
    printf '%s\n' "$output" >&2
  fi
}

current_branch="$(git symbolic-ref --quiet --short HEAD 2>/dev/null || true)"
if [[ -z "$current_branch" ]]; then
  fail_msg "checkout is detached; expected branch $expected_branch"
elif [[ "$branch_mode" == "experimental" ]]; then
  ok "experimental branch $current_branch"
elif [[ "$current_branch" != "$expected_branch" ]]; then
  fail_msg "checkout branch is $current_branch; expected $expected_branch"
else
  ok "branch $current_branch"
fi

run_gate "branch topology ($branch_mode)" \
  bash scripts/check_branch_topology.sh --mode "$branch_mode"

head_sha="$(git rev-parse --short=8 HEAD)"
ok "local HEAD $head_sha"

remote_ref="origin/$expected_branch"
if git show-ref --verify --quiet "refs/remotes/$remote_ref"; then
  remote_sha="$(git rev-parse --short=8 "$remote_ref")"
  ok "cached $remote_ref $remote_sha (freshness not proven; no network used)"

  ahead_behind="$(git rev-list --left-right --count "$remote_ref...HEAD")"
  read -r behind ahead <<< "$ahead_behind"
  ok "cached $remote_ref...HEAD = ${behind} ${ahead}"
else
  fail_msg "cached remote ref missing: $remote_ref"
fi

tracked_dirty="$(git status --porcelain=v1 -uno)"
if [[ -n "$tracked_dirty" ]]; then
  if (( allow_dirty )); then
    warn "tracked dirty files present (--allow-dirty set)"
    printf '%s\n' "$tracked_dirty" >&2
  else
    fail_msg "tracked dirty files present"
    printf '%s\n' "$tracked_dirty" >&2
  fi
else
  ok "no tracked dirty files"
fi

vendor_untracked=0
unexpected_untracked=0
while IFS= read -r line; do
  case "$line" in
    '?? '*)
      path="${line#?? }"
      case "$path" in
        vendor|vendor/*)
          vendor_untracked=$((vendor_untracked + 1))
          ;;
        *)
          unexpected_untracked=$((unexpected_untracked + 1))
          printf 'unexpected untracked: %s\n' "$path" >&2
          ;;
      esac
      ;;
  esac
done < <(git status --porcelain=v1 -uall)

if (( unexpected_untracked > 0 )); then
  if (( allow_dirty )); then
    warn "$unexpected_untracked unexpected untracked file(s) present (--allow-dirty set)"
  else
    fail_msg "$unexpected_untracked unexpected untracked file(s)"
  fi
fi
if (( vendor_untracked > 0 )); then
  ok "vendor/ quarantine untracked entries present: $vendor_untracked (policy: do not add/remove/depend without explicit assignment)"
elif (( unexpected_untracked == 0 )); then
  ok "no untracked files"
fi

worktrees="$(git worktree list --porcelain)"
integration_worktree_count="$(printf '%s\n' "$worktrees" | awk -v branch="branch refs/heads/${expected_branch}" '$0 == branch { count++ } END { print count + 0 }')"
if [[ "$integration_worktree_count" != "1" ]]; then
  fail_msg "expected exactly one worktree on $expected_branch; found $integration_worktree_count"
else
  ok "single $expected_branch worktree"
fi

extra_worktree_count="$(printf '%s\n' "$worktrees" | awk '/^worktree / { count++ } END { print (count > 0 ? count - 1 : 0) }')"
ok "extra linked worktrees: $extra_worktree_count"

run_gate "runtime baseline pins" bash scripts/check_runtime_baseline.sh
run_gate "backend transition truth" bash scripts/check_backend_transition.sh
run_gate "model call boundary" bash scripts/check_model_call_boundary.sh
run_gate "heiwa_core Dockerfile baseline" bash scripts/check_heiwa_core_dockerfile.sh
run_gate "release metadata" bash scripts/check_release_metadata.sh

surface_output="$(bash scripts/audit_product_surface.sh 2>&1)" || {
  fail_msg "product surface audit failed"
  printf '%s\n' "$surface_output" >&2
  surface_output=""
}
if [[ -n "$surface_output" ]]; then
  unclassified_files="$(printf '%s\n' "$surface_output" | awk '$1 == "unclassified" { print $2; exit }')"
  if [[ "${unclassified_files:-}" == "0" ]]; then
    ok "product surface audit has zero unclassified tracked files"
  else
    fail_msg "product surface audit has ${unclassified_files:-unknown} unclassified tracked file(s)"
    printf '%s\n' "$surface_output" >&2
  fi
fi

if (( fail != 0 )); then
  exit 1
fi

printf 'Agent baseline gate passed. Remote health was not checked.\n'
