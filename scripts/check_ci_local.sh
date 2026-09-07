#!/usr/bin/env bash
# Stable local verification entry point; check inventory and evidence live together.
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$repo_root/scripts/lib/resolve_local_python.sh"
local_python="$(resolve_local_python "$repo_root")"
export HEIWA_PYTHON="$local_python"
exec "$local_python" "$repo_root/scripts/check_ci_local.py" "$@"
