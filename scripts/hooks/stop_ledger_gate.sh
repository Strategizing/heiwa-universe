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

# A stamp is fresh when the tree the gate certified is still, in every way that
# gate can see, the tree in front of you.
#
# Exact HEAD always qualifies. Beyond that, a stamp survives commits that
# cannot have invalidated it: the acceptance scripts write only at a clean
# exact HEAD, but the *reading* of an older stamp may be scoped, because a
# docs-only commit does not change whether the desktop build passes.
#
# This is not a softening. A gate that fires on every commit regardless of
# relevance is one that gets silenced, and the cheapest way to silence this one
# is to edit the ledger — the exact dishonesty it exists to prevent.
#
# Fresh requires all of:
#   1. the stamp exists;
#   2. it is HEAD, or an ancestor of HEAD (a stamp from a diverged branch is
#      not evidence about this one);
#   3. nothing under the script's declared `# acceptance-scope:` differs
#      between the stamped commit and HEAD.
#
# Deliberately commit-to-commit, not working-tree: this repository's rule is
# that a ledger states what is true at HEAD, not what is intended, so the stamp
# certifies HEAD and uncommitted work is not yet a claim. Comparing against the
# working tree would start blocking stops over work nobody has asserted.
#
# A script that declares no scope falls back to exact HEAD, so this is never
# weaker than before for a gate that has not opted in.
stamp_fresh() {
  local stamp_file="$1" script="$2"
  [[ -f "$stamp_file" ]] || return 1
  local stamped
  stamped="$(cat "$stamp_file")"
  [[ -n "$stamped" ]] || return 1
  [[ "$stamped" == "$head_sha" ]] && return 0

  local scope
  scope="$(sed -n 's/^# acceptance-scope: *//p' "$script" 2>/dev/null | head -1)"
  [[ -n "$scope" ]] || return 1

  git merge-base --is-ancestor "$stamped" "$head_sha" 2>/dev/null || return 1

  # Word-split deliberately: the scope line is a list of pathspecs.
  # shellcheck disable=SC2086
  git diff --quiet "$stamped" "$head_sha" -- $scope 2>/dev/null
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
  stamp_fresh "$stamp" "$script" && return 0
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
