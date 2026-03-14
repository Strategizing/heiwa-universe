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
TRANSPORT="${HEIWA_GATEWAY_TRANSPORT:-gateway_websocket}"

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
    echo "transport: $TRANSPORT"
    echo "---"
} >> "$LOG_FILE"

if ! command -v openclaw &>/dev/null; then
    echo "[ERR] openclaw not found in PATH" | tee -a "$LOG_FILE"
    exit 2
fi

MODE="${OPENCLAW_EXEC_MODE:-gateway}"  # gateway|auto|local
AGENT_ID="${OPENCLAW_AGENT_ID:-main}"
THINKING="${OPENCLAW_THINKING:-high}"
TIMEOUT_SEC="${OPENCLAW_TIMEOUT:-600}"
SESSION_ID="${OPENCLAW_SESSION_ID:-heiwa-worker}"
PROFILE="${OPENCLAW_PROFILE:-}"
MODEL="${HEIWA_ACTIVE_MODEL:-${OPENCLAW_MODEL:-}}"
if [[ -n "$MODEL" && "$MODEL" != */* ]]; then
    MODEL="$MODEL"
fi

{
    echo "profile: ${PROFILE:-main}"
    echo "model: ${MODEL:-default}"
} >> "$LOG_FILE"

if [[ -n "$PROFILE" ]]; then
    MAIN_AGENT_DIR="${OPENCLAW_MAIN_AGENT_DIR:-$HOME/.openclaw/agents/main/agent}"
    PROFILE_STATE_DIR="${OPENCLAW_PROFILE_STATE_DIR:-$HOME/.openclaw-$PROFILE}"
    PROFILE_AGENT_DIR="${OPENCLAW_PROFILE_AGENT_DIR:-$PROFILE_STATE_DIR/agents/main/agent}"
    if [[ "${OPENCLAW_COPY_AUTH_FROM_MAIN:-true}" == "true" ]]; then
        mkdir -p "$PROFILE_AGENT_DIR"
        if [[ -f "$MAIN_AGENT_DIR/auth-profiles.json" ]]; then
            cp "$MAIN_AGENT_DIR/auth-profiles.json" "$PROFILE_AGENT_DIR/auth-profiles.json"
        fi
    fi
    OPENCLAW_CMD=(openclaw --profile "$PROFILE")
else
    OPENCLAW_CMD=(openclaw)
fi

oc() {
    "${OPENCLAW_CMD[@]}" "$@"
}

gateway_is_healthy() {
    oc health --json >/dev/null 2>&1
}

ensure_model_selection() {
    if [[ -z "$MODEL" ]]; then
        return 0
    fi
    if [[ -z "$PROFILE" && "${OPENCLAW_ALLOW_MAIN_PROFILE_MODEL_SET:-false}" != "true" ]]; then
        echo "[WARN] model override '$MODEL' requested without OPENCLAW_PROFILE; using active OpenClaw model" | tee -a "$LOG_FILE"
        return 0
    fi
    oc models set "$MODEL" >/dev/null 2>&1 || {
        echo "[WARN] failed to pin OpenClaw profile '$PROFILE' to model '$MODEL'" | tee -a "$LOG_FILE"
        return 0
    }
}

build_cmd() {
    local base_cmd=("$@")
    "${base_cmd[@]}"
}

run_gateway() {
    ensure_model_selection
    build_cmd "${OPENCLAW_CMD[@]}" agent \
        --session-id "$SESSION_ID" \
        --agent "$AGENT_ID" \
        --thinking "$THINKING" \
        --timeout "$TIMEOUT_SEC" \
        --message "$PAYLOAD" \
        --json
}

run_local() {
    ensure_model_selection
    build_cmd "${OPENCLAW_CMD[@]}" agent \
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
