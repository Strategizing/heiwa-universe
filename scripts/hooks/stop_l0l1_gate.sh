#!/usr/bin/env bash
set -euo pipefail

# Compatibility entrypoint, preserved by the Work Fabric design
# (docs/superpowers/specs/2026-08-22-heiwa-work-fabric-design.md, "Acceptance
# Gates and SHA-Bound Stamps"): the hook was generalized to cover every release
# ledger, not just the roadmap's L0-L2 layers.
#
# Anything still wired to this path keeps working. New configuration should
# point at scripts/hooks/stop_ledger_gate.sh directly.

exec bash "$(dirname "${BASH_SOURCE[0]}")/stop_ledger_gate.sh" "$@"
