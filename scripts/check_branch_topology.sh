#!/usr/bin/env bash
set -euo pipefail

mode="integration"
integration_branch="${HEIWA_INTEGRATION_BRANCH:-dev}"
production_ref="${HEIWA_PRODUCTION_REF:-refs/remotes/origin/main}"

usage() {
  cat >&2 <<'EOF'
Usage: scripts/check_branch_topology.sh [--mode integration|experimental|post-promotion]

Local-only branch topology gate. It never fetches. By default it compares the
local integration branch `dev` with the cached production ref `origin/main`.

Modes:
  integration     require dev to be ahead of and not behind origin/main
  experimental    require a non-dev/main branch descended from current dev
  post-promotion  permit dev to be synchronized with, but never behind, main
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode)
      [[ $# -ge 2 ]] || {
        usage
        exit 2
      }
      mode="$2"
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

case "$mode" in
  integration|experimental|post-promotion) ;;
  *)
    usage
    exit 2
    ;;
esac

repo_root="$(git rev-parse --show-toplevel 2>/dev/null)" || {
  printf 'FAIL: not inside a git repository\n' >&2
  exit 1
}
cd "$repo_root"

current_branch="$(git symbolic-ref --quiet --short HEAD 2>/dev/null || true)"
if [[ -z "$current_branch" ]]; then
  printf 'FAIL: checkout is detached\n' >&2
  exit 1
fi

if ! git show-ref --verify --quiet "refs/heads/$integration_branch"; then
  printf 'FAIL: local integration branch is missing: %s\n' "$integration_branch" >&2
  exit 1
fi
if ! git show-ref --verify --quiet "$production_ref"; then
  printf 'FAIL: cached production ref is missing: %s\n' "$production_ref" >&2
  exit 1
fi

read -r behind ahead < <(
  git rev-list --left-right --count "$production_ref...refs/heads/$integration_branch"
)

if (( behind > 0 )); then
  printf 'FAIL: %s is behind origin/main by %s commit(s)\n' \
    "$integration_branch" "$behind" >&2
  exit 1
fi

case "$mode" in
  experimental)
    if [[ "$current_branch" == "$integration_branch" || "$current_branch" == "main" ]]; then
      printf 'FAIL: experimental work must not run on %s\n' "$current_branch" >&2
      exit 1
    fi
    if ! git merge-base --is-ancestor "$integration_branch" "$current_branch"; then
      printf 'FAIL: %s does not descend from %s\n' \
        "$current_branch" "$integration_branch" >&2
      exit 1
    fi
    printf 'OK: %s descends from %s\n' "$current_branch" "$integration_branch"
    ;;
  integration)
    if [[ "$current_branch" != "$integration_branch" ]]; then
      printf 'FAIL: checkout branch is %s; expected %s\n' \
        "$current_branch" "$integration_branch" >&2
      exit 1
    fi
    if (( ahead == 0 )); then
      printf 'FAIL: %s has no value-bearing commit ahead of origin/main\n' \
        "$integration_branch" >&2
      exit 1
    fi
    printf 'OK: %s is %s commit(s) ahead of origin/main\n' \
      "$integration_branch" "$ahead"
    ;;
  post-promotion)
    if [[ "$current_branch" != "$integration_branch" ]]; then
      printf 'FAIL: checkout branch is %s; expected %s\n' \
        "$current_branch" "$integration_branch" >&2
      exit 1
    fi
    if (( ahead == 0 )); then
      printf 'OK: %s is synchronized with origin/main\n' "$integration_branch"
    else
      printf 'OK: %s is %s commit(s) ahead of origin/main\n' \
        "$integration_branch" "$ahead"
    fi
    ;;
esac
