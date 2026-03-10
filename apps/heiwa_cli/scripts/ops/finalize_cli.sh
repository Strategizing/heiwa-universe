#!/usr/bin/env bash
# apps/heiwa_cli/scripts/ops/finalize_cli.sh
set -euo pipefail

CLI_BIN="/Users/dmcgregsauce/heiwa/apps/heiwa_cli/heiwa"
ALIAS_CMD="alias heiwa='$CLI_BIN'"

echo "🚀 Finalizing Heiwa CLI Integration..."

# 1. Update Shell Profiles
for PROFILE in "$HOME/.zshrc" "$HOME/.bash_profile" "$HOME/.bashrc"; do
    if [[ -f "$PROFILE" ]]; then
        # Remove any existing heiwa alias
        sed -i '' '/alias heiwa=/d' "$PROFILE"
        # Append correct alias
        echo "$ALIAS_CMD" >> "$PROFILE"
        echo "✅ Updated $PROFILE"
    fi
done

# 2. Re-install Node Wrapper
echo "📦 Refreshing Global Node Link..."
cd /Users/dmcgregsauce/heiwa/apps/heiwa_cli
npm install -g . --quiet

# 3. Test Binary
echo "🧪 Verifying Binary..."
if "$CLI_BIN" status >/dev/null 2>&1; then
    echo "✅ Binary operational."
else
    echo "❌ Binary check failed. Please check .venv and PYTHONPATH."
fi

echo ""
echo "✨ Integration Cemented. Restart your terminal or run: source ~/.zshrc"
