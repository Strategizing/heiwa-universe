#!/usr/bin/env bash
# Start a local worker stack (Mac/Linux):
# - Ollama daemon
# - OpenClaw gateway (optional but recommended)
# - Heiwa worker_manager (hub WebSocket transport)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
cd "$ROOT"

mkdir -p "$ROOT/runtime/logs"

load_env_file() {
    local file="$1"
    is_allowed_key() {
        if [[ "$1" == HEIWA_* ]]; then
            return 0
        fi
        case "$1" in
            OPENCLAW_*|PICOCLAW_*|GOOGLE_API_KEY|GROQ_API_KEY|CEREBRAS_API_KEY|OPENROUTER_API_KEY|MISTRAL_API_KEY|TOGETHER_API_KEY)
                return 0
                ;;
            *)
                return 1
                ;;
        esac
    }
    while IFS= read -r raw || [[ -n "$raw" ]]; do
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

if [[ -f "$HOME/.heiwa/vault.env" ]]; then
    load_env_file "$HOME/.heiwa/vault.env"
fi

if [[ -f "$ROOT/.env.worker.local" ]]; then
    load_env_file "$ROOT/.env.worker.local"
elif [[ -f "$ROOT/.env.worker" ]]; then
    load_env_file "$ROOT/.env.worker"
elif [[ -f "$ROOT/.env" ]]; then
    load_env_file "$ROOT/.env"
fi

choose_python() {
    if [[ -x "$ROOT/.venv313/bin/python" ]]; then
        echo "$ROOT/.venv313/bin/python"
        return
    fi
    if [[ -x "$ROOT/.venv/bin/python" ]]; then
        echo "$ROOT/.venv/bin/python"
        return
    fi
    command -v python3 >/dev/null 2>&1 && echo "python3" && return
    echo "python"
}

is_tcp_open() {
    local host="$1"
    local port="$2"
    (echo >"/dev/tcp/${host}/${port}") >/dev/null 2>&1
}

start_ollama() {
    if is_tcp_open 127.0.0.1 11434; then
        echo "[OK] Ollama already listening on :11434"
        return
    fi
    echo "[INFO] Starting ollama serve ..."
    nohup ollama serve >"$ROOT/runtime/logs/ollama.serve.log" 2>&1 &
    sleep 2
    if is_tcp_open 127.0.0.1 11434; then
        echo "[OK] Ollama started"
    else
        echo "[WARN] Ollama did not bind to :11434 yet"
    fi
}

start_openclaw_gateway() {
    if ! command -v openclaw >/dev/null 2>&1; then
        echo "[WARN] openclaw not installed; skipping gateway"
        return
    fi

    if openclaw status 2>/dev/null | grep -q 'Gateway.*reachable'; then
        echo "[OK] OpenClaw gateway already healthy"
        return
    fi

    echo "[INFO] Starting openclaw gateway ..."
    nohup openclaw gateway run --allow-unconfigured >"$ROOT/runtime/logs/openclaw.gateway.log" 2>&1 &
    sleep 2
    if openclaw status 2>/dev/null | grep -q 'Gateway.*reachable'; then
        echo "[OK] OpenClaw gateway started"
    else
        echo "[WARN] OpenClaw gateway still unhealthy (openclaw wrapper can fallback to --local mode)"
    fi
}

start_worker_manager() {
    if pgrep -f "apps/heiwa_cli/scripts/agents/worker_manager.py" >/dev/null 2>&1; then
        echo "[OK] worker_manager already running"
        return
    fi
    echo "[INFO] Starting worker_manager daemon wrapper ..."
    nohup "$ROOT/apps/heiwa_cli/scripts/ops/worker_manager_daemon.sh" >"$ROOT/runtime/logs/worker_manager.log" 2>&1 &
    sleep 1
    if pgrep -f "apps/heiwa_cli/scripts/agents/worker_manager.py" >/dev/null 2>&1; then
        echo "[OK] worker_manager started"
    else
        echo "[WARN] worker_manager exited; check runtime/logs/worker_manager.log"
    fi
}

echo "--- HEIWA WORKER STACK START ---"
start_ollama
start_openclaw_gateway
start_worker_manager
echo "--- HEIWA WORKER STACK CHECK ---"
"$(choose_python)" "$ROOT/apps/heiwa_cli/scripts/ops/heiwa_360_check.py" || true
