#!/usr/bin/env bash
# cli/scripts/agents/wrappers/opencode_exec.sh
# Isolated wrapper for OpenCode CLI. Logs all I/O. Never called directly by agents.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${HEIWA_WORKSPACE_ROOT:-$(cd "$SCRIPT_DIR/../../.." && pwd)}"
LOG_DIR="$ROOT/runtime/logs/opencode"
mkdir -p "$LOG_DIR"

RUN_ID="$(date +%Y%m%d_%H%M%S)_$$"
LOG_FILE="$LOG_DIR/$RUN_ID.log"
PAYLOAD_FILE="$LOG_DIR/$RUN_ID.payload.txt"

# Read payload from stdin or first arg
if [[ $# -gt 0 ]]; then
    PAYLOAD="$1"
else
    PAYLOAD="$(cat)"
fi

# Log payload
echo "$PAYLOAD" > "$PAYLOAD_FILE"

{
    echo "=== OPENCODE EXEC START ==="
    echo "run_id: $RUN_ID"
    echo "timestamp: $(date -Iseconds)"
    echo "cwd: $ROOT"
    echo "payload_bytes: ${#PAYLOAD}"
    echo "---"
} >> "$LOG_FILE"

# Check if opencode is available
if ! command -v opencode &>/dev/null; then
    echo "[ERR] opencode not found in PATH" | tee -a "$LOG_FILE"
    exit 2
fi

# Execute with timeout (inherit from config or default 10 min)
TIMEOUT_SEC="${OPENCODE_TIMEOUT:-600}"

set +e
if command -v timeout &>/dev/null; then
    timeout "$TIMEOUT_SEC" opencode --non-interactive --prompt "$PAYLOAD" 2>&1 | tee -a "$LOG_FILE"
    EXIT_CODE=${PIPESTATUS[0]}
elif command -v gtimeout &>/dev/null; then
    gtimeout "$TIMEOUT_SEC" opencode --non-interactive --prompt "$PAYLOAD" 2>&1 | tee -a "$LOG_FILE"
    EXIT_CODE=${PIPESTATUS[0]}
else
    echo "[WARN] timeout command not found; running without timeout" | tee -a "$LOG_FILE"
    opencode --non-interactive --prompt "$PAYLOAD" 2>&1 | tee -a "$LOG_FILE"
    EXIT_CODE=${PIPESTATUS[0]}
fi
set -e

{
    echo "---"
    echo "exit_code: $EXIT_CODE"
    echo "=== OPENCODE EXEC END ==="
} >> "$LOG_FILE"

exit "$EXIT_CODE"
