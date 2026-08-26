#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

python_value="$(
  HEIWA_PYTHON=/tmp/heiwa-python \
    just --justfile "$repo_root/Justfile" --evaluate python
)"
pytest_value="$(
  HEIWA_PYTEST='/tmp/heiwa-python -m pytest' \
    just --justfile "$repo_root/Justfile" --evaluate pytest
)"

if [[ "$python_value" != "/tmp/heiwa-python" ]]; then
  printf 'Justfile ignored HEIWA_PYTHON: %s\n' "$python_value" >&2
  exit 1
fi
if [[ "$pytest_value" != "/tmp/heiwa-python -m pytest" ]]; then
  printf 'Justfile ignored HEIWA_PYTEST: %s\n' "$pytest_value" >&2
  exit 1
fi

printf 'Justfile Python overrides are worktree-portable.\n'
