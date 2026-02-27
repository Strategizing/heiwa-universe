#!/bin/bash
# fleets/local-field-op/connect.sh

# Resolve absolute paths
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
# Assuming structure: root/fleets/local-field-op/connect.sh -> ROOT_DIR is 2 levels up
ROOT_DIR="$(dirname "$(dirname "$SCRIPT_DIR")")"

AGENT_CONFIG="$SCRIPT_DIR/agent.yaml"
RUNTIME_SCRIPT="$ROOT_DIR/cli/scripts/agent_runtime.py"

# CONFIG
GATEWAY="http://heiwa-core:8080"
MCP_FS_PATH="$ROOT_DIR/tools/mcp-filesystem/server.py"

echo "🔌 Connecting Heiwa Engineer to Cloud Brain..."

# Check for MagicDNS (is Cloud online?)
# We use a loop to wait for Tailscale to route
# Note: ping might fail if ICMP is blocked, but valid DNS resolution is a good proxy.
echo "Waiting for heiwa-core to appear on mesh..."
until ping -c 1 -W 2 heiwa-core &> /dev/null; do
    echo "  ... ping failed, retrying in 5s ..."
    sleep 5
done
echo "Target acquired."

# Launch REAL Client with MCP Tool
openclaw connect \
    --gateway "$GATEWAY" \
    --agent "$AGENT_CONFIG" \
    --mcp "filesystem:$MCP_FS_PATH"
