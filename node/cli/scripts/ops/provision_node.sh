#!/usr/bin/env bash
# Prepare a node initialization package for a new Heiwa Node (Workstation/etc)
set -euo pipefail

NODE_ID="${1:-workstation}"
NODE_TYPE="${2:-heavy_compute}"

echo "🏗️  Preparing Heiwa Node Package for: $NODE_ID ($NODE_TYPE)"

# 1. Generate local env overrides
ENV_CONTENT=$(cat <<EOF
# Heiwa Node Configuration
HEIWA_NODE_ID=$NODE_ID
HEIWA_NODE_TYPE=$NODE_TYPE
HEIWA_USE_REMOTE_NATS=1
NATS_URL=$(grep NATS_URL .env.worker.local | cut -d'=' -f2-)
HEIWA_LLM_MODE=local_only
EOF
)

echo "$ENV_CONTENT" > ".env.$NODE_ID"

echo "✅ Generated .env.$NODE_ID"
echo ""
echo "🚀 NEXT STEPS FOR THE $NODE_ID:"
echo "1. Install Tailscale and join the Heiwa network."
echo "2. Clone this repo: git clone https://github.com/Strategizing/heiwa-universe.git"
echo "3. Copy the env: cp .env.$NODE_ID .env.worker.local"
echo "4. Run setup: ./node/cli/scripts/ops/install_worker_service.sh"
echo ""
echo "✨ The $NODE_ID will then automatically link to the Cloud HQ."
