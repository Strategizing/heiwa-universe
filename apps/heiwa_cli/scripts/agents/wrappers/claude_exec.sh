#!/usr/bin/env bash
# Direct Claude Code wrapper for Heiwa OAuth-backed strategy/review routes.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${HEIWA_WORKSPACE_ROOT:-$(cd "$SCRIPT_DIR/../../.." && pwd)}"
LOG_DIR="$ROOT/runtime/logs/claude"
mkdir -p "$LOG_DIR"

RUN_ID="$(date +%Y%m%d_%H%M%S)_$$"
LOG_FILE="$LOG_DIR/$RUN_ID.log"
PAYLOAD_FILE="$LOG_DIR/$RUN_ID.payload.txt"

if [[ $# -gt 0 ]]; then
    PAYLOAD="$1"
else
    PAYLOAD="$(cat)"
fi
echo "$PAYLOAD" > "$PAYLOAD_FILE"

{
    echo "=== CLAUDE EXEC START ==="
    echo "run_id: $RUN_ID"
    echo "timestamp: $(date -Iseconds)"
    echo "cwd: $ROOT"
    echo "payload_bytes: ${#PAYLOAD}"
    echo "---"
} >> "$LOG_FILE"

if ! command -v claude &>/dev/null; then
    echo "[ERR] claude not found in PATH" | tee -a "$LOG_FILE"
    exit 2
fi

MODEL="${HEIWA_ACTIVE_MODEL:-${CLAUDE_MODEL:-}}"
if [[ -n "$MODEL" && "$MODEL" == */* ]]; then
    MODEL="${MODEL#*/}"
fi
MODEL="${MODEL//./-}"
TIMEOUT_SEC="${CLAUDE_TIMEOUT:-900}"

CMD=(claude -p "$PAYLOAD" --output-format text --permission-mode plan --add-dir "$ROOT")
if [[ -n "$MODEL" ]]; then
    CMD+=(--model "$MODEL")
fi

set +e
if command -v timeout &>/dev/null; then
    timeout "$TIMEOUT_SEC" "${CMD[@]}" 2>&1 | tee -a "$LOG_FILE"
    EXIT_CODE=${PIPESTATUS[0]}
elif command -v gtimeout &>/dev/null; then
    gtimeout "$TIMEOUT_SEC" "${CMD[@]}" 2>&1 | tee -a "$LOG_FILE"
    EXIT_CODE=${PIPESTATUS[0]}
else
    echo "[WARN] timeout command not found; running without timeout" | tee -a "$LOG_FILE"
    "${CMD[@]}" 2>&1 | tee -a "$LOG_FILE"
    EXIT_CODE=${PIPESTATUS[0]}
fi
set -e

{
    echo "---"
    echo "model: ${MODEL:-default}"
    echo "exit_code: $EXIT_CODE"
    echo "=== CLAUDE EXEC END ==="
} >> "$LOG_FILE"

exit "$EXIT_CODE"
