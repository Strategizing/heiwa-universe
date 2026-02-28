#!/usr/bin/env bash
# apps/heiwa_cli/scripts/agents/wrappers/antigravity_exec.sh
# Wrapper for Antigravity execution.
set -euo pipefail

ROOT="${HEIWA_WORKSPACE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)}"
LOG_DIR="$ROOT/runtime/logs/antigravity"
mkdir -p "$LOG_DIR"

RUN_ID="$(date +%Y%m%d_%H%M%S)_$$"
LOG_FILE="$LOG_DIR/$RUN_ID.log"

PAYLOAD="$1"

if ! command -v antigravity &>/dev/null; then
    echo "[ERR] antigravity not found in PATH" | tee -a "$LOG_FILE"
    exit 2
fi

# Antigravity often needs explicit model selection
MODEL="${ANTIGRAVITY_MODEL:-gpt-4o}"

echo "=== ANTIGRAVITY EXEC START ===" >> "$LOG_FILE"
antigravity chat "$PAYLOAD" --model "$MODEL" --non-interactive 2>&1 | tee -a "$LOG_FILE"
EXIT_CODE=${PIPESTATUS[0]}
echo "=== ANTIGRAVITY EXEC END (code: $EXIT_CODE) ===" >> "$LOG_FILE"

exit "$EXIT_CODE"
