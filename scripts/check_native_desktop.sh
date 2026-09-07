#!/usr/bin/env bash
# Explicit host target avoids the Heiwa/heiwa filename collision on macOS.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
desktop_target="$(rustc -vV | sed -n 's/^host: //p')"
if [[ ! "$desktop_target" =~ ^[a-zA-Z0-9_-]+$ ]]; then
  echo "Unable to resolve the native Rust target" >&2
  exit 1
fi
cargo test -p heiwa-desktop --locked --target "$desktop_target"
cargo clippy -p heiwa-desktop --locked --all-targets --target "$desktop_target" -- -D warnings
cargo build --release -p heiwa-desktop --locked --target "$desktop_target"
