#!/usr/bin/env bash
# Launch wrapper for persistent worker_manager service.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

# launchd default PATH is minimal; add user/global bins required by wrappers.
export PATH="$HOME/.npm-global/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"

load_env_file() {
    local file="$1"
    is_allowed_key() {
        case "$1" in
            HEIWA_WORKSPACE_ROOT|HEIWA_LLM_MODE|HEIWA_EXECUTOR_CONCURRENCY|HEIWA_WORKER_WARM_TTL_SEC|HEIWA_ALLOWED_OUTBOUND_TARGETS|HEIWA_LOCAL_NATS_URL|HEIWA_USE_REMOTE_NATS|NATS_URL|HEIWA_OLLAMA_LOCAL_URL|OPENCLAW_EXEC_MODE|OPENCLAW_AGENT_ID|OPENCLAW_THINKING|OPENCLAW_TIMEOUT|PICOCLAW_BIN|PICOCLAW_DEFAULT_COMMAND|PICOCLAW_TIMEOUT)
                return 0
                ;;
            *)
                return 1
                ;;
        esac
    }
    while IFS= read -r raw; do
        local line
        line="$(echo "$raw" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
        [[ -z "$line" || "${line:0:1}" == "#" ]] && continue
        [[ "$line" != *"="* ]] && continue
        local key="${line%%=*}"
        local val="${line#*=}"
        key="$(echo "$key" | tr -d ' ')"
        if ! is_allowed_key "$key"; then
            continue
        fi
        if [[ "$val" == *'${{'* ]]; then
            continue
        fi
        val="${val%\"}"
        val="${val#\"}"
        val="${val%\'}"
        val="${val#\'}"
        export "$key=$val"
    done <"$file"
}

if [[ -f "$ROOT/.env.worker" ]]; then
    load_env_file "$ROOT/.env.worker"
elif [[ -f "$ROOT/.env" ]]; then
    load_env_file "$ROOT/.env"
fi

export HEIWA_WORKSPACE_ROOT="${HEIWA_WORKSPACE_ROOT:-$ROOT}"
export HEIWA_LLM_MODE="${HEIWA_LLM_MODE:-local_only}"

# Force local worker bus by default.
# Set HEIWA_USE_REMOTE_NATS=1 only when workers should use a reachable remote NATS.
if [[ "${HEIWA_USE_REMOTE_NATS:-0}" == "1" ]]; then
    export NATS_URL="${NATS_URL:-nats://127.0.0.1:4222}"
else
    export NATS_URL="${HEIWA_LOCAL_NATS_URL:-nats://127.0.0.1:4222}"
fi

if [[ -x "$ROOT/.venv313/bin/python" ]]; then
    PYTHON_BIN="$ROOT/.venv313/bin/python"
elif [[ -x "$ROOT/.venv/bin/python" ]]; then
    PYTHON_BIN="$ROOT/.venv/bin/python"
else
    PYTHON_BIN="$(command -v python3 || command -v python)"
fi

exec "$PYTHON_BIN" "$ROOT/cli/scripts/agents/worker_manager.py"
