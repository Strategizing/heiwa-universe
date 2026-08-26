#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$repo_root/scripts/lib/resolve_local_python.sh"

fixture="$(mktemp -d /tmp/heiwa-python-resolution.XXXXXX)"
trap 'rm -rf -- "$fixture"' EXIT
mkdir -p "$fixture/.venv/bin"
ln -s "$(command -v python3)" "$fixture/.venv/bin/python"

unset HEIWA_PYTHON
selected="$(resolve_local_python "$fixture")"

if [[ "$selected" != "$fixture/.venv/bin/python" ]]; then
  printf 'expected absolute repository Python, got: %s\n' "$selected" >&2
  exit 1
fi

override="$(HEIWA_PYTHON=/tmp/heiwa-python resolve_local_python "$fixture")"
if [[ "$override" != "/tmp/heiwa-python" ]]; then
  printf 'expected explicit Python override, got: %s\n' "$override" >&2
  exit 1
fi

printf 'local Python resolution uses an absolute repository path\n'
