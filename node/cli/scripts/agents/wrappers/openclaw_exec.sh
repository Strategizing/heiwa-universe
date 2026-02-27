#!/usr/bin/env bash
# cli/scripts/agents/wrappers/openclaw_exec.sh
# Wrapper for OpenClaw execution from worker_manager.
# Mode:
#   - gateway: requires running openclaw gateway
#   - local: embedded agent mode (requires provider creds in shell)
#   - auto: gateway if healthy, otherwise local
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${HEIWA_WORKSPACE_ROOT:-$(cd "$SCRIPT_DIR/../../.." && pwd)}"
LOG_DIR="$ROOT/runtime/logs/openclaw"
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
    echo "=== OPENCLAW EXEC START ==="
    echo "run_id: $RUN_ID"
    echo "timestamp: $(date -Iseconds)"
    echo "cwd: $ROOT"
    echo "payload_bytes: ${#PAYLOAD}"
    echo "---"
} >> "$LOG_FILE"

if ! command -v openclaw &>/dev/null; then
    echo "[ERR] openclaw not found in PATH" | tee -a "$LOG_FILE"
    exit 2
fi

MODE="${OPENCLAW_EXEC_MODE:-auto}"  # auto|gateway|local
AGENT_ID="${OPENCLAW_AGENT_ID:-main}"
THINKING="${OPENCLAW_THINKING:-high}"
TIMEOUT_SEC="${OPENCLAW_TIMEOUT:-600}"
SESSION_ID="${OPENCLAW_SESSION_ID:-heiwa-worker}"

gateway_is_healthy() {
    openclaw gateway health >/dev/null 2>&1
}

run_gateway() {
    openclaw agent \
        --session-id "$SESSION_ID" \
        --agent "$AGENT_ID" \
        --thinking "$THINKING" \
        --timeout "$TIMEOUT_SEC" \
        --message "$PAYLOAD" \
        --json
}

run_local() {
    openclaw agent \
        --local \
        --agent "$AGENT_ID" \
        --thinking "$THINKING" \
        --timeout "$TIMEOUT_SEC" \
        --message "$PAYLOAD" \
        --json
}

set +e
if [[ "$MODE" == "gateway" ]]; then
    run_gateway 2>&1 | tee -a "$LOG_FILE"
    EXIT_CODE=${PIPESTATUS[0]}
elif [[ "$MODE" == "local" ]]; then
    run_local 2>&1 | tee -a "$LOG_FILE"
    EXIT_CODE=${PIPESTATUS[0]}
else
    if gateway_is_healthy; then
        echo "[INFO] gateway health: OK (mode=gateway)" | tee -a "$LOG_FILE"
        run_gateway 2>&1 | tee -a "$LOG_FILE"
        EXIT_CODE=${PIPESTATUS[0]}
    else
        echo "[WARN] gateway unavailable (mode=local fallback)" | tee -a "$LOG_FILE"
        run_local 2>&1 | tee -a "$LOG_FILE"
        EXIT_CODE=${PIPESTATUS[0]}
    fi
fi
set -e

{
    echo "---"
    echo "mode: $MODE"
    echo "exit_code: $EXIT_CODE"
    echo "=== OPENCLAW EXEC END ==="
} >> "$LOG_FILE"

exit "$EXIT_CODE"
