#!/usr/bin/env bash
# Install persistent macOS LaunchAgents for Heiwa worker node.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
AGENTS_DIR="$HOME/Library/LaunchAgents"
mkdir -p "$AGENTS_DIR" "$ROOT/runtime/logs"

NATS_PLIST="$AGENTS_DIR/ai.heiwa.nats.plist"
WORKER_PLIST="$AGENTS_DIR/ai.heiwa.worker-manager.plist"

NATS_BIN="/opt/homebrew/opt/nats-server/bin/nats-server"
if [[ ! -x "$NATS_BIN" ]]; then
    echo "[WARN] nats-server not found at $NATS_BIN (install with: brew install nats-server)"
else
    cat >"$NATS_PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>ai.heiwa.nats</string>
  <key>ProgramArguments</key>
  <array>
    <string>$NATS_BIN</string>
    <string>-js</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>WorkingDirectory</key><string>$ROOT</string>
  <key>StandardOutPath</key><string>$ROOT/runtime/logs/nats.launchd.out.log</string>
  <key>StandardErrorPath</key><string>$ROOT/runtime/logs/nats.launchd.err.log</string>
</dict>
</plist>
PLIST

    launchctl bootout "gui/$(id -u)/ai.heiwa.nats" >/dev/null 2>&1 || true
    launchctl bootstrap "gui/$(id -u)" "$NATS_PLIST"
    launchctl kickstart -k "gui/$(id -u)/ai.heiwa.nats" || true
    echo "[OK] Installed LaunchAgent: ai.heiwa.nats"
fi

cat >"$WORKER_PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>ai.heiwa.worker-manager</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/bash</string>
    <string>$ROOT/cli/scripts/ops/worker_manager_daemon.sh</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>WorkingDirectory</key><string>$ROOT</string>
  <key>StandardOutPath</key><string>$ROOT/runtime/logs/worker-manager.launchd.out.log</string>
  <key>StandardErrorPath</key><string>$ROOT/runtime/logs/worker-manager.launchd.err.log</string>
</dict>
</plist>
PLIST

launchctl bootout "gui/$(id -u)/ai.heiwa.worker-manager" >/dev/null 2>&1 || true
launchctl bootstrap "gui/$(id -u)" "$WORKER_PLIST"
launchctl kickstart -k "gui/$(id -u)/ai.heiwa.worker-manager" || true
echo "[OK] Installed LaunchAgent: ai.heiwa.worker-manager"

echo
echo "LaunchAgent status:"
launchctl list | grep -E 'ai.heiwa.(nats|worker-manager)' || true
