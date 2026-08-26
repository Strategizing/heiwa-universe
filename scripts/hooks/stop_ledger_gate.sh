#!/usr/bin/env bash
set -euo pipefail

# Claude Code Stop gate for release ledgers.
#
# Blocks ending the session while a ledger declares a release complete (every
# row `done`) but that release's acceptance gate has not passed at the current
# HEAD. The acceptance scripts write their stamp on success.
#
# A stop with work still in progress (any row todo/doing/blocked/pending) is
# always allowed — this gate only fires on unverified completion claims.
#
# A claimed-complete release whose acceptance script does not exist yet is
# blocked on purpose: the script is part of the release, so its absence is an
# unverified claim, not an exemption.
#
# Fast by design: awk + grep + git rev-parse only, no builds.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

head_sha="$(git rev-parse HEAD 2>/dev/null || echo none)"

# A section claims completion when it has at least one `done` row and no row
# still carrying an open status. `pending` is included because the Work Fabric
# ledger uses it where the roadmap ledger uses `todo`.
section_claims_complete() {
  local ledger="$1" start="$2" end="$3"
  local section done_rows open_rows
  section="$(awk "/^## ${start}/,/^## ${end}/" "$ledger")"
  done_rows="$(printf '%s' "$section" | grep -c '| done |' || true)"
  open_rows="$(printf '%s' "$section" | grep -Ec '\| (todo|doing|pending|blocked[^|]*) \|' || true)"
  [[ "$done_rows" -gt 0 && "$open_rows" -eq 0 ]]
}

stamp_fresh() {
  local stamp_file="$1"
  [[ -f "$stamp_file" ]] && [[ "$(cat "$stamp_file")" == "$head_sha" ]]
}

block() {
  # Stop-hook JSON: decision block + reason re-engages the model.
  printf '{"decision":"block","reason":"%s"}\n' "$1"
  exit 0
}

# check_release <label> <ledger> <section-start> <section-end> <stamp> <script>
#
# Section boundaries are exact prefixes of the `## ` headings that bracket the
# rows. A loose range would let one release's rows satisfy another's claim.
check_release() {
  local label="$1" ledger="$2" start="$3" end="$4" stamp="$5" script="$6"
  [[ -f "$ledger" ]] || return 0
  section_claims_complete "$ledger" "$start" "$end" || return 0
  stamp_fresh "$stamp" && return 0
  if [[ ! -f "$script" ]]; then
    block "Ledger $ledger declares $label complete but its acceptance gate $script does not exist. Write and pass it, or set the rows back to their honest status."
  fi
  block "Ledger $ledger declares $label complete but $script has not passed at HEAD ($head_sha). Run it; on failure fix and rerun, or set the ledger rows back to their honest status."
}

roadmap_ledger="docs/superpowers/ledgers/2026-08-14-L0-L1-task-ledger.md"
check_release "L0" "$roadmap_ledger" "L0 " "L1 " \
  ".claude/l0-accept-sha" "scripts/check_l0_acceptance.sh"
check_release "L1" "$roadmap_ledger" "L1 — " "L2 " \
  ".claude/l1-accept-sha" "scripts/check_l1_acceptance.sh"
check_release "L2" "$roadmap_ledger" "L2 " "L1 review" \
  ".claude/l2-accept-sha" "scripts/check_l2_acceptance.sh"

# Work Fabric A1 is one checkpoint across A1-a, A1-b, the cohesion repair, and
# A1-c. The range ends at `Deferred with reason`, which is prose, not rows.
work_fabric_ledger="docs/superpowers/ledgers/2026-08-22-work-fabric-task-ledger.md"
check_release "Work Fabric A1" "$work_fabric_ledger" "Release A1-a " "Deferred with reason" \
  ".claude/work-fabric-a1-accept-sha" "scripts/check_work_fabric_a1_acceptance.sh"

exit 0
