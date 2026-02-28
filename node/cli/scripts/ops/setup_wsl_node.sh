#!/usr/bin/env bash
# Setup script for a Heiwa Node running in WSL2 (Ubuntu)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
cd "$ROOT"

echo "🛠️  Starting Heiwa WSL Node Setup..."

# 1. Purge Snap Bloat (Verified optimization)
if command -v snap &>/dev/null; then
    echo "🧹 Purging snapd and associated bloat..."
    sudo apt-get purge -y snapd
    sudo rm -rf /snap /var/snap /var/lib/snapd
fi

# 1.5 Update and install basic dependencies
sudo apt-get update
sudo apt-get install -y python3-pip python3-venv nodejs git tailscale screen

# 2. Update and install core dependencies
sudo apt-get update
sudo apt-get install -y curl ca-certificates gnupg git screen tailscale

# 2.5 Setup Node.js 22.x (Explicit versioning)
if ! command -v node &>/dev/null || [[ $(node -v | cut -d'v' -f2 | cut -d'.' -f1) -lt 22 ]]; then
    echo "📦 Installing Node.js 22.x via NodeSource..."
    sudo mkdir -p /etc/apt/keyrings
    curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key | sudo gpg --dearmor -o /etc/apt/keyrings/nodesource.gpg
    NODE_MAJOR=22
    echo "deb [signed-by=/etc/apt/keyrings/nodesource.gpg] https://deb.nodesource.com/node_$NODE_MAJOR.x nodistro main" | sudo tee /etc/apt/sources.list.d/nodesource.list
    sudo apt-get update
    sudo apt-get install nodejs -y
fi

# 3. Setup Python environment
if [[ ! -d ".venv" ]]; then
    echo "🐍 Creating virtual environment..."
    python3 -m venv .venv
fi
.venv/bin/pip install --upgrade pip
.venv/bin/pip install -r requirements.txt nats-py python-dotenv

# 3.5 Setup OpenClaw (Handle potential 404)
if ! command -v openclaw &>/dev/null; then
    echo "🦞 Attempting OpenClaw installation..."
    # Note: Package may be private or in a different registry
    sudo npm install -g @openclaw/cli || echo "⚠️  OpenClaw CLI install failed. Proceeding with local wrappers."
fi

# 3.6 Setup Ollama & Models (Native Install)
if ! command -v ollama &>/dev/null; then
    echo "🧠 Installing Ollama natively..."
    curl -fsSL https://ollama.com/install.sh | sh
fi

echo "📥 Pulling Heavy Reasoner models..."
# Ensure ollama is running to pull
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
