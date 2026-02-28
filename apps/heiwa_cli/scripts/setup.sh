#!/bin/bash
# cli/scripts/setup.sh
# HEIWA LIMITED: Environment Initialization (The Muscle Prep)

set -e

echo "=== HEIWA LIMITED: ENVIRONMENT CHECK ==="

# Function to check and install brew packages
check_brew_pkg() {
    PKG=$1
    if ! command -v $PKG &> /dev/null; then
        echo "[MISSING] $PKG not found. Installing..."
        brew install $PKG
    else
        echo "[OK] $PKG is installed."
    fi
}

# 1. Check Homebrew
if ! command -v brew &> /dev/null; then
    echo "[CRITICAL] Homebrew not found. Please install Homebrew first."
    exit 1
fi

# 2. Install/Verify Core Tools
check_brew_pkg gh
check_brew_pkg tailscale
check_brew_pkg railway

# 3. Authentication Checks
echo "--- Checking Authentication ---"

if gh auth status &> /dev/null; then
    echo "[OK] GitHub: Authenticated"
else
    echo "[WARN] GitHub: Not authenticated. Run 'gh auth login'"
fi

if railway whoami &> /dev/null; then
    echo "[OK] Railway: Authenticated"
else
    echo "[WARN] Railway: Not authenticated. Run 'railway login'"
fi

# 4. Tailscale Check
# Note: On macOS, we prefer the CLI from brew for scripting, but check for the GUI app too.
if pgrep -f "Tailscale" &> /dev/null; then
    echo "[OK] Tailscale: Daemon/App is running."
else
    echo "[WARN] Tailscale: Not running. Ensure the App is open or the daemon is started."
fi

echo "=== PREP COMPLETE ==="
