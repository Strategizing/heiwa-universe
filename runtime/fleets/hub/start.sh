#!/usr/bin/env bash
# Compatibility wrapper for older fleet/runtime entrypoints.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
TARGET="$ROOT/apps/heiwa_hub/start.sh"

if [[ ! -f "$TARGET" ]]; then
    echo "[HEIWA] Missing canonical hub entrypoint: $TARGET" >&2
    exit 1
fi

echo "[HEIWA] Delegating to canonical hub entrypoint: $TARGET"
exec /bin/bash "$TARGET"
