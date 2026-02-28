#!/usr/bin/env bash
# Setup script for a Heiwa Node running in WSL2 (Ubuntu)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
cd "$ROOT"

echo "🛠️  Starting Heiwa WSL Node Setup..."

# 1. Update and install dependencies
sudo apt-get update
sudo apt-get install -y python3-pip python3-venv nodejs npm git tailscale screen

# 2. Setup Python environment
if [[ ! -d ".venv" ]]; then
    echo "🐍 Creating virtual environment..."
    python3 -m venv .venv
fi
.venv/bin/pip install --upgrade pip
.venv/bin/pip install -r requirements.txt nats-py python-dotenv

# 3. Setup OpenClaw
if ! command -v openclaw &>/dev/null; then
    echo "🦞 Installing OpenClaw..."
    sudo npm install -g @openclaw/cli || echo "⚠️  Global npm install failed, try local install later."
fi

# 3.5 Setup Ollama & Models
if ! command -v ollama &>/dev/null; then
    echo "🧠 Installing Ollama..."
    curl -fsSL https://ollama.com/install.sh | sh
fi

echo "📥 Pulling Heavy Reasoner models (this may take a while)..."
# Start ollama temporarily to pull
nohup ollama serve >/dev/null 2>&1 &
sleep 5
ollama pull deepseek-r1:14b
ollama pull qwen2.5-coder:7b

# 3.7 Initialize Modular Vault
VAULT_DIR="$HOME/.heiwa"
VAULT_FILE="$VAULT_DIR/vault.env"
mkdir -p "$VAULT_DIR"
if [[ ! -f "$VAULT_FILE" ]]; then
    echo "🔐 Initializing secure vault..."
    touch "$VAULT_FILE"
    chmod 600 "$VAULT_FILE"
    
    # Pre-populate from provided env if available
    if [[ -f "$ROOT/.env.worker.local" ]]; then
        PROVIDED_TOKEN=$(grep HEIWA_AUTH_TOKEN "$ROOT/.env.worker.local" | cut -d'=' -f2-)
        if [[ -n "$PROVIDED_TOKEN" ]]; then
            echo "HEIWA_AUTH_TOKEN=$PROVIDED_TOKEN" >> "$VAULT_FILE"
        fi
    fi
    echo "GITHUB_PAT=PLACEHOLDER_TOKEN_PLEASE_REPLACE" >> "$VAULT_FILE"
fi

# 4. Persistence via systemd (if enabled)
if [[ -d /run/systemd/system ]]; then
    echo "🔄 Configuring systemd service..."
    SERVICE_FILE="/etc/systemd/system/heiwa-worker.service"
    sudo bash -c "cat <<EOF > $SERVICE_FILE
[Unit]
Description=Heiwa Worker Daemon
After=network.target

[Service]
Type=simple
User=$USER
WorkingDirectory=$ROOT
ExecStart=$ROOT/node/cli/scripts/ops/worker_manager_daemon.sh
Restart=always
Environment=PATH=$ROOT/.venv/bin:/usr/local/bin:/usr/bin:/bin
Environment=PYTHONPATH=$ROOT/runtime

[Install]
WantedBy=multi-user.target
EOF"
    sudo systemctl daemon-reload
    sudo systemctl enable heiwa-worker
    echo "✅ systemd service 'heiwa-worker' created and enabled."
else
    echo "⚠️  systemd not detected in WSL. Use './node/cli/scripts/ops/start_worker_stack.sh' manually."
fi

echo "✨ WSL Node Setup Complete."
echo "👉 Now run: cp .env.workstation .env.worker.local"
echo "👉 Then start: ./node/cli/scripts/ops/start_worker_stack.sh"
