#!/bin/bash
# fleets/hub/start.sh
# Unified entrypoint for Railway deployment.

echo "[HEIWA] Initializing Cloud HQ..."
HEIWA_ENABLE_TAILSCALE="${HEIWA_ENABLE_TAILSCALE:-true}"

# 0. Tailscale is optional for Cloud HQ boot. If unavailable, continue in
# non-mesh mode so the HTTP/WebSocket control plane still comes up.
if [[ "$HEIWA_ENABLE_TAILSCALE" != "true" ]]; then
    echo "[HEIWA] Tailscale disabled for this Cloud HQ boot (HEIWA_ENABLE_TAILSCALE=false)."
else
    # Install Tailscale (Railpack fallback)
    if ! command -v tailscaled &> /dev/null; then
        echo "[HEIWA] Tailscale not found. Attempting runtime install..."
        if command -v timeout &> /dev/null; then
            timeout 30 sh -c 'curl -fsSL https://tailscale.com/install.sh | sh' || true
        elif command -v gtimeout &> /dev/null; then
            gtimeout 30 sh -c 'curl -fsSL https://tailscale.com/install.sh | sh' || true
        else
            sh -c 'curl -fsSL https://tailscale.com/install.sh | sh' || true
        fi
    fi
fi

# 1. Start the daemon in userspace mode (Critical for Railway when mesh is enabled)
export PATH=$PATH:/usr/local/bin:/root/.local/bin
TAILSCALE_READY=false

if [[ "$HEIWA_ENABLE_TAILSCALE" == "true" ]] && command -v tailscaled &> /dev/null && command -v tailscale &> /dev/null; then
    echo "[HEIWA] Starting tailscaled (Userspace Mode)..."
    tailscaled --tun=userspace-networking --socket=/tmp/tailscaled.sock &
    TAILSCALE_READY=true
else
    echo "[HEIWA] Tailscale binaries unavailable. Proceeding without mesh."
fi

# 2. Wait for the daemon to wake up
AUTH_KEY="${TAILSCALE_AUTH_KEY:-${RAILWAY_TAILSCALE_AUTH_KEY:-}}"
TS_HOSTNAME="${TAILSCALE_HOSTNAME:-heiwa-core}"
TS_TIMEOUT_SEC="${TAILSCALE_UP_TIMEOUT_SEC:-12}"
MAX_RETRIES="${TAILSCALE_MAX_RETRIES:-5}"

run_tailscale_up() {
    if command -v timeout &>/dev/null; then
        timeout "$TS_TIMEOUT_SEC" tailscale --socket=/tmp/tailscaled.sock up --authkey="$AUTH_KEY" --hostname="$TS_HOSTNAME"
    elif command -v gtimeout &>/dev/null; then
        gtimeout "$TS_TIMEOUT_SEC" tailscale --socket=/tmp/tailscaled.sock up --authkey="$AUTH_KEY" --hostname="$TS_HOSTNAME"
    else
        tailscale --socket=/tmp/tailscaled.sock up --authkey="$AUTH_KEY" --hostname="$TS_HOSTNAME"
    fi
}

if [[ "$TAILSCALE_READY" == "true" ]]; then
    echo "[HEIWA] Waiting for Tailscale daemon..."
    if [[ -z "$AUTH_KEY" ]]; then
        echo "[HEIWA] No TAILSCALE_AUTH_KEY/RAILWAY_TAILSCALE_AUTH_KEY provided. Skipping mesh join."
    else
        for ((ATTEMPT=1; ATTEMPT<=MAX_RETRIES; ATTEMPT++)); do
            if run_tailscale_up; then
                echo "[HEIWA] Tailscale handshake succeeded."
                break
            fi

            if [[ "$ATTEMPT" -ge "$MAX_RETRIES" ]]; then
                echo "[HEIWA] Tailscale handshake failed after $MAX_RETRIES attempts. Proceeding without mesh..."
                break
            fi

            echo "[HEIWA] Waiting for Tailscale handshake... (Attempt $ATTEMPT/$MAX_RETRIES)"
            sleep 2
        done
    fi

    TS_IP=$(tailscale --socket=/tmp/tailscaled.sock ip -4 2>/dev/null)
    if [ -n "$TS_IP" ]; then
        echo "[HEIWA] Tailscale is UP. IP: $TS_IP"
    else
        echo "[HEIWA] Tailscale is DOWN or unavailable."
    fi
else
    echo "[HEIWA] Tailscale startup skipped."
fi

# 3. Start the Ollama Micro-Brain (optional)
HEIWA_ENABLE_OLLAMA="${HEIWA_ENABLE_OLLAMA:-false}"
HEIWA_OLLAMA_PREFETCH_MODEL="${HEIWA_OLLAMA_PREFETCH_MODEL:-false}"

if [[ "$HEIWA_ENABLE_OLLAMA" == "true" ]]; then
    if command -v ollama &>/dev/null; then
        echo "[HEIWA] Starting Local Micro-Brain (Ollama)..."
        OLLAMA_HOST=0.0.0.0 ollama serve > /tmp/ollama.log 2>&1 &

        echo "[HEIWA] Waiting for Ollama to become responsive..."
        for i in {1..15}; do
            if curl -s http://127.0.0.1:11434/api/tags > /dev/null; then
                echo "[HEIWA] Ollama is UP."
                break
            fi
            sleep 2
        done

        if [[ "$HEIWA_OLLAMA_PREFETCH_MODEL" == "true" ]]; then
            echo "[HEIWA] Ensuring phi3:mini model is available locally..."
            ollama pull phi3:mini &
        else
            echo "[HEIWA] Skipping Ollama model prefetch (HEIWA_OLLAMA_PREFETCH_MODEL=false)."
        fi
    else
        echo "[HEIWA] HEIWA_ENABLE_OLLAMA=true but ollama binary is not installed. Continuing without local micro-brain."
    fi
else
    echo "[HEIWA] Ollama disabled for this Cloud HQ boot (HEIWA_ENABLE_OLLAMA=false)."
fi

# 4. Launch the Core Collective (Spine + Messenger)
# Bootstrap a cloud-safe default net policy if no runtime policy is mounted.
HEIWA_HOME_DIR="${HEIWA_HOME:-/root/.heiwa}"
HEIWA_NET_POLICY_TARGET="$HEIWA_HOME_DIR/policy/internet/net_policy_v2.json"
HEIWA_NET_POLICY_BOOTSTRAP_PATH="${HEIWA_NET_POLICY_BOOTSTRAP_PATH:-/app/policies/net_policy_v2.cloud_hq.json}"
if [[ ! -f "$HEIWA_NET_POLICY_TARGET" && -f "$HEIWA_NET_POLICY_BOOTSTRAP_PATH" ]]; then
    mkdir -p "$(dirname "$HEIWA_NET_POLICY_TARGET")"
    cp "$HEIWA_NET_POLICY_BOOTSTRAP_PATH" "$HEIWA_NET_POLICY_TARGET"
    echo "[HEIWA] Bootstrapped net policy to $HEIWA_NET_POLICY_TARGET"
fi

echo "[HEIWA] Launching Core Collective..."
cd /app || exit 1
export PYTHONPATH=$PYTHONPATH:/app/packages/heiwa_sdk:/app/packages/heiwa_protocol:/app/packages/heiwa_identity:/app/packages/heiwa_ui:/app/apps
exec python -m apps.heiwa_hub.main
