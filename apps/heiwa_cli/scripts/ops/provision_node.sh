#!/usr/bin/env bash
# Prepare a node initialization package for a new Heiwa Node (Workstation/etc)
set -euo pipefail

NODE_ID="${1:-workstation}"
NODE_TYPE="${2:-heavy_compute}"

echo "🏗️  Preparing Heiwa Node Package for: $NODE_ID ($NODE_TYPE)"

# 1. Generate local env overrides
HEIWA_TOKEN=$(grep HEIWA_AUTH_TOKEN .env | cut -d'=' -f2- | tr -d '"' | tr -d "'")
if [[ -z "$HEIWA_TOKEN" ]]; then
    HEIWA_TOKEN="faab5ce53c496715150d6aa96b5082b2"
fi

HEIWA_HUB_URL=$(grep '^HEIWA_HUB_URL=' .env.worker.local 2>/dev/null | cut -d'=' -f2- || true)
if [[ -z "$HEIWA_HUB_URL" ]]; then
    HEIWA_HUB_URL="https://api.heiwa.ltd"
fi

CAPS="standard_compute"
if [[ "$NODE_TYPE" == "heavy_compute" ]]; then
    CAPS="heavy_compute,gpu_native,standard_compute"
elif [[ "$NODE_ID" == "macbook" ]]; then
    CAPS="agile_coding,workspace_interaction,standard_compute"
fi

ENV_CONTENT=$(cat <<EOF
# Heiwa Node Configuration
HEIWA_NODE_ID=$NODE_ID
HEIWA_NODE_TYPE=$NODE_TYPE
HEIWA_CAPABILITIES=$CAPS
HEIWA_AUTH_TOKEN=$HEIWA_TOKEN
HEIWA_HUB_URL=$HEIWA_HUB_URL
HEIWA_LLM_MODE=local_only
EOF
)

echo "$ENV_CONTENT" > ".env.$NODE_ID"

echo "✅ Generated .env.$NODE_ID"
echo ""
echo "✨ The $NODE_ID configuration is ready."
echo ""
if [[ "$NODE_ID" == "workstation" || "$NODE_TYPE" == "heavy_compute" ]]; then
    echo "🪟 WINDOWS/WSL SETUP (Run in PowerShell):"
    echo "irm https://raw.githubusercontent.com/Strategizing/heiwa-universe/main/infra/nodes/windows/bootstrap_heiwa.ps1 | iex"
    echo ""
    echo "👉 After running the above, copy .env.$NODE_ID to WSL: \\wsl$\Ubuntu\home\$USER\heiwa\.env.worker.local"
else
    echo "🚀 NEXT STEPS FOR THE $NODE_ID (🍎 MAC/LINUX):"
    echo "1. Install Tailscale and join the Heiwa network."
    echo "2. Clone this repo: git clone https://github.com/Strategizing/heiwa-universe.git"
    echo "3. Copy the env: cp .env.$NODE_ID .env.worker.local"
    echo "4. Run setup: ./apps/heiwa_cli/scripts/ops/install_worker_service.sh"
fi
