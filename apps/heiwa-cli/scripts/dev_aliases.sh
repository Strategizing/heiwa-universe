#!/bin/bash
# Heiwa Limited - Dev Workflow Aliases
# Source this file: source ~/heiwa-limited/cli/scripts/dev_aliases.sh

# 1. NATS Management
alias nats-up='nats-server -V'  # Start NATS locally (verbose)
alias nats-mon='nats server report jetstream' # Check stream status

# 2. Fleet Execution (The new structure)
# Core Orchestrator
alias h-core='python3 -m fleets.hub.main'

# Field Ops (Specific Nodes)
alias h-node='python3 -m fleets.nodes.muscle.heiwa_node'

# 3. Quick Utils
alias h-clean='find . -type d -name "__pycache__" -not -path "./_archive/*" -exec rm -r {} + 2>/dev/null; echo "🧹 Cache cleared."'
alias h-id='cat identity.json'
