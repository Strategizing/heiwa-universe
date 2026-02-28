#!/usr/bin/env bash
# Start a local worker stack (Mac/Linux):
# - Ollama daemon
# - NATS (local nats-server or docker fallback)
# - OpenClaw gateway (optional but recommended)
# - Heiwa worker_manager
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
            NATS_URL|OPENCLAW_*|PICOCLAW_*)
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

# Force local worker bus by default.
# Set HEIWA_USE_REMOTE_NATS=1 only when workers must connect to a reachable remote NATS.
if [[ "${HEIWA_USE_REMOTE_NATS:-0}" == "1" ]]; then
    echo "[INFO] HEIWA_USE_REMOTE_NATS=1 -> using configured NATS_URL=${NATS_URL:-<unset>}"
else
    if [[ -n "${NATS_URL:-}" ]]; then
        echo "[INFO] Overriding NATS_URL for local worker bus"
    fi
    export NATS_URL="${HEIWA_LOCAL_NATS_URL:-nats://127.0.0.1:4222}"
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

start_nats() {
    if is_tcp_open 127.0.0.1 4222; then
        echo "[OK] NATS already listening on :4222"
        return
    fi

    if command -v nats-server >/dev/null 2>&1; then
        echo "[INFO] Starting local nats-server ..."
        nohup nats-server -js >"$ROOT/runtime/logs/nats-server.log" 2>&1 &
    elif command -v docker >/dev/null 2>&1; then
        if docker info >/dev/null 2>&1; then
            echo "[INFO] Starting docker nats:2-alpine ..."
            if docker ps -a --format '{{.Names}}' | grep -qx 'heiwa-nats'; then
                docker start heiwa-nats >/dev/null || true
            else
                docker run -d --name heiwa-nats -p 4222:4222 nats:2-alpine -js >/dev/null || true
            fi
        else
            echo "[WARN] Docker is installed but daemon is not running; cannot start nats container"
        fi
    else
        echo "[WARN] Neither nats-server nor docker available; cannot start local NATS"
        return
    fi

    sleep 2
    if is_tcp_open 127.0.0.1 4222; then
        echo "[OK] NATS started on :4222"
    else
        echo "[WARN] NATS did not bind to :4222 yet"
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
    if ! is_tcp_open 127.0.0.1 4222; then
        echo "[WARN] NATS bus is not reachable on :4222; skipping worker_manager startup"
        return
    fi
    if pgrep -f "node/cli/scripts/agents/worker_manager.py" >/dev/null 2>&1; then
        echo "[OK] worker_manager already running"
        return
    fi
    echo "[INFO] Starting worker_manager daemon wrapper ..."
    nohup "$ROOT/node/cli/scripts/ops/worker_manager_daemon.sh" >"$ROOT/runtime/logs/worker_manager.log" 2>&1 &
    sleep 1
    if pgrep -f "node/cli/scripts/agents/worker_manager.py" >/dev/null 2>&1; then
        echo "[OK] worker_manager started"
    else
        echo "[WARN] worker_manager exited; check runtime/logs/worker_manager.log"
    fi
}

start_mini_thinker() {
    if ! is_tcp_open 127.0.0.1 4222; then
        return
    fi
    if pgrep -f "node/agents/mini_thinker.py" >/dev/null 2>&1; then
        echo "[OK] mini_thinker already running"
        return
    fi
    echo "[INFO] Starting mini_thinker ..."
    local py_bin
    py_bin=$(choose_python)
    nohup "$py_bin" "$ROOT/node/agents/mini_thinker.py" >"$ROOT/runtime/logs/mini_thinker.log" 2>&1 &
    sleep 1
    if pgrep -f "node/agents/mini_thinker.py" >/dev/null 2>&1; then
        echo "[OK] mini_thinker started"
    else
        echo "[WARN] mini_thinker exited; check runtime/logs/mini_thinker.log"
    fi
}

echo "--- HEIWA WORKER STACK START ---"
start_ollama
start_nats
start_openclaw_gateway
start_worker_manager
start_mini_thinker
echo "--- HEIWA WORKER STACK CHECK ---"
"$(choose_python)" "$ROOT/node/cli/scripts/ops/heiwa_360_check.py" || true
