#!/usr/bin/env bash
# Make the Heiwa CLI global on macOS/Linux
set -euo pipefail

CLI_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CLI_BIN="$CLI_DIR/heiwa"
BIN_TARGET="/usr/local/bin/heiwa"

echo "🚀 Setting up global Heiwa CLI..."

# 1. Ensure the binary is executable
chmod +x "$CLI_BIN"

# 2. Attempt to symlink to /usr/local/bin
if [[ -w "/usr/local/bin" ]]; then
    ln -sf "$CLI_BIN" "$BIN_TARGET"
    echo "✅ Symlinked to $BIN_TARGET"
else
    echo "⚠️  Cannot write to /usr/local/bin. Attempting sudo..."
    sudo ln -sf "$CLI_BIN" "$BIN_TARGET"
    echo "✅ Symlinked to $BIN_TARGET (with sudo)"
fi

# 3. Add alias to shell profile as fallback
SHELL_TYPE=$(basename "$SHELL")
PROFILE_FILE=""

if [[ "$SHELL_TYPE" == "zsh" ]]; then
    PROFILE_FILE="$HOME/.zshrc"
elif [[ "$SHELL_TYPE" == "bash" ]]; then
    if [[ "$OSTYPE" == "darwin"* ]]; then
        PROFILE_FILE="$HOME/.bash_profile"
    else
        PROFILE_FILE="$HOME/.bashrc"
    fi
fi

if [[ -n "$PROFILE_FILE" ]]; then
    if ! grep -q "alias heiwa=" "$PROFILE_FILE"; then
        echo "alias heiwa='$CLI_BIN'" >> "$PROFILE_FILE"
        echo "✅ Added alias to $PROFILE_FILE"
    fi
fi

echo ""
echo "✨ Setup Complete! Restart your terminal or run 'source $PROFILE_FILE'."
echo "👉 You can now type 'heiwa' from anywhere."
