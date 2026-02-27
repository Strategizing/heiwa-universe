#!/bin/bash
# [ANTIGRAVITY] Heiwa Muscle Node Installer
# Deployment Scaffolding for 24/7 Persistence.

set -e

echo "--- HEIWA LIMITED: MUSCLE DEPLOYMENT ---"

# 1. Environment Validation
if [ -z "$HEIWA_AUTH_TOKEN" ]; then
    echo "Error: HEIWA_AUTH_TOKEN is not set in your current shell."
    echo "Please run: export HEIWA_AUTH_TOKEN=your_token_here"
    exit 1
fi

HUB_URL="${HEIWA_HUB_BASE_URL:-http://127.0.0.1:8000}"
NODE_ID="${HEIWA_NODE_ID:-muscle-$(hostname | tr '[:upper:]' '[:lower:]' | cut -d. -f1)}"
INSTALL_DIR="$(pwd)"
PYTHON_BIN="$(which python3)"

if [ -z "$PYTHON_BIN" ]; then
    echo "Error: python3 not found in PATH."
    exit 1
fi

OS_TYPE="$(uname -s)"
echo "[INFO] OS: $OS_TYPE"
echo "[INFO] Node ID: $NODE_ID"
echo "[INFO] Hub URL: $HUB_URL"

# 2. Service Generation
if [ "$OS_TYPE" == "Linux" ]; then
    SERVICE_PATH="/etc/systemd/system/heiwa-muscle.service"
    echo "[INFO] Generating systemd service: $SERVICE_PATH"
    
    sudo bash -c "cat <<EOF > $SERVICE_PATH
[Unit]
Description=Heiwa Limited Muscle Node
After=network.target

[Service]
Type=simple
User=${SUDO_USER:-$(whoami)}
WorkingDirectory=$INSTALL_DIR
Environment=HEIWA_AUTH_TOKEN=$HEIWA_AUTH_TOKEN
Environment=HEIWA_HUB_BASE_URL=$HUB_URL
Environment=HEIWA_NODE_ID=$NODE_ID
ExecStart=$PYTHON_BIN $INSTALL_DIR/fleets/nodes/muscle/muscle_node.py
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
EOF"

    echo "[INFO] Activating service..."
    sudo systemctl daemon-reload
    sudo systemctl enable heiwa-muscle
    echo "[SUCCESS] Service installed. Run 'sudo systemctl start heiwa-muscle' to ignite."

    # --- SSH Server Configuration ---
    echo ">>> Checking OpenSSH Server status..."

    # Check if sshd is installed
    if ! command -v sshd >/dev/null; then
        echo ">>> SSH Server not found. Installing..."
        sudo apt-get update
        sudo apt-get install -y openssh-server
    else
        echo ">>> SSH Server is already installed."
    fi

    # Ensure service is running and enabled
    echo ">>> Enabling and starting SSH service..."
    sudo systemctl enable ssh
    sudo systemctl start ssh

    # Verify status
    if systemctl is-active --quiet ssh; then
        echo ">>> SSH service is ACTIVE."
    else
        echo ">>> ERROR: SSH service failed to start."
        exit 1
    fi

    # Optional: UFW allow if firewall is active
    if sudo ufw status | grep -q "Status: active"; then
        echo ">>> Allowing SSH through UFW..."
        sudo ufw allow ssh
    fi

    echo ">>> Muscle is ready for remote connection."


elif [ "$OS_TYPE" == "Darwin" ]; then
    PLIST_PATH="$HOME/Library/LaunchAgents/ltd.heiwa.muscle.plist"
    echo "[INFO] Generating launchd plist: $PLIST_PATH"
    
    cat <<EOF > $PLIST_PATH
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ltd.heiwa.muscle</string>
    <key>ProgramArguments</key>
    <array>
        <string>$PYTHON_BIN</string>
        <string>$INSTALL_DIR/fleets/nodes/muscle/muscle_node.py</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>WorkingDirectory</key>
    <string>$INSTALL_DIR</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>HEIWA_AUTH_TOKEN</key>
        <string>$HEIWA_AUTH_TOKEN</string>
        <key>HEIWA_HUB_BASE_URL</key>
        <string>$HUB_URL</string>
        <key>HEIWA_NODE_ID</key>
        <string>$NODE_ID</string>
    </dict>
    <key>StandardOutPath</key>
    <string>$INSTALL_DIR/logs/muscle.log</string>
    <key>StandardErrorPath</key>
    <string>$INSTALL_DIR/logs/muscle.err</string>
</dict>
</plist>
EOF

    echo "[INFO] Activating launchd job..."
    launchctl load "$PLIST_PATH"
    echo "[SUCCESS] Service installed and loaded. Use 'launchctl start ltd.heiwa.muscle' if not running."

else
    echo "Error: Unsupported OS type $OS_TYPE"
    exit 1
fi

echo "----------------------------------------"
echo "Deployment Scaffold Complete."
