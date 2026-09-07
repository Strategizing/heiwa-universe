#!/usr/bin/env bash
set -euo pipefail

# acceptance-scope: apps crates Cargo.toml Cargo.lock scripts/check_l0_acceptance.sh scripts/lib/verification_logs.sh
#
# Broad on purpose. Checks 1-4 only read apps/heiwa_app/desktop, but check 5
# scans every runtime source under apps/ and crates/ for home-path resolution
# outside the ConfigRoot resolver, and check 6 scans desktop and cockpit
# sources for raw hex. Narrowing this to the desktop would let a ConfigRoot
# violation land in a crate while the stamp still read fresh.

# L0 acceptance gate — roadmap 2026-08-14, layer L0 (UI foundation + N-user config root).
#
# Deterministic checks:
#   1. desktop typecheck + production build succeed
#   2. desktop vitest suite passes (operator seam + surface render tests)
#   3. operator seam test files are byte-identical to the pre-migration baseline
#   4. all ten surfaces exist as component modules and a shell render test exists
#   5. no home-path resolution outside the ConfigRoot resolver (runtime code)
#   6. design tokens: no raw hex colors outside the theme layer
#
# This script is local-only and does not use the network.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
source "$repo_root/scripts/lib/verification_logs.sh"
umask 077
log_dir="$(verification_log_dir "$repo_root" "l0")"

desktop="apps/heiwa_app/desktop"
fail=0

ok() { printf 'OK: %s\n' "$*"; }
fail_msg() { printf 'FAIL: %s\n' "$*" >&2; fail=1; }

# ── 1+2. Desktop typecheck, build, tests ────────────────────────────────────
if (cd "$desktop" && npm run --silent typecheck >"$log_dir/l0_typecheck.log" 2>&1); then
  ok "desktop typecheck"
else
  fail_msg "desktop typecheck (see $log_dir/l0_typecheck.log)"
fi

if (cd "$desktop" && npm run --silent build >"$log_dir/l0_build.log" 2>&1); then
  ok "desktop production build"
else
  fail_msg "desktop production build (see $log_dir/l0_build.log)"
fi

if (cd "$desktop" && npm test --silent >"$log_dir/l0_vitest.log" 2>&1); then
  ok "desktop vitest suite"
else
  fail_msg "desktop vitest suite (see $log_dir/l0_vitest.log)"
fi

# ── 3. Operator seam preserved: test files byte-identical to baseline ───────
# The seam is the implementation as much as its tests: pinning only the
# tests would let store.ts be rewritten under a passing suite.
declare -A seam_baseline=(
  ["$desktop/src/operator/store.test.ts"]="7f68b72bc113940349648ef505bc49b52ecd11d21410b046b05fee06b8e6b2a0"
  ["$desktop/src/operator/client.test.ts"]="a162fe8e094baf8f497504c9e99761ad069b8e5c614321efea4a34ab0ebb8470"
  ["$desktop/src/operator/store.ts"]="e2ca87af2c7e975b38b7f6eafb90d0ae4f8b5d44b5cf1b12bad5bd33607ae793"
  ["$desktop/src/operator/client.ts"]="0986fd4366d2e1cb3d876c36ecf00dc68db82b0b340d5cae59e0d53e3509cd17"
  ["$desktop/src/operator/types.ts"]="a01a076c800ccccbfe5d1bedd3cfd08e72c2a68261c8aa7502c1df3296cac663"
)
for file in "${!seam_baseline[@]}"; do
  if [[ ! -f "$file" ]]; then
    fail_msg "seam test missing: $file"
    continue
  fi
  actual="$(shasum -a 256 "$file" | awk '{print $1}')"
  if [[ "$actual" == "${seam_baseline[$file]}" ]]; then
    ok "seam unmodified: ${file#"$desktop"/}"
  else
    fail_msg "operator seam modified since baseline: $file (the seam must be preserved; if a change was deliberately approved, update the baseline hash in this script in the same commit)"
  fi
done

# ── 4. Ten surfaces as component modules ────────────────────────────────────
surfaces=(home ai windows calendar mail finance social workers browser files)
missing_surfaces=0
for surface in "${surfaces[@]}"; do
  if ! ls "$desktop/src/surfaces/$surface"/*.tsx >/dev/null 2>&1; then
    fail_msg "surface module missing: src/surfaces/$surface/"
    missing_surfaces=1
  fi
done
[[ $missing_surfaces -eq 0 ]] && ok "all ten surface modules present"

if ls "$desktop/src/shell"/*.test.tsx >/dev/null 2>&1 || ls "$desktop/src/surfaces"/*.test.tsx >/dev/null 2>&1 || ls "$desktop/src"/app.test.tsx >/dev/null 2>&1; then
  ok "shell/surface render test present"
else
  fail_msg "no shell/surface render test found (need a vitest that mounts the shell and renders every surface)"
fi

# ── 5. No home-path resolution outside ConfigRoot (runtime code) ────────────
# The invariant is about path *construction*, not about the string "~/.heiwa"
# appearing in help text: only crates/heiwa_config may read HOME/USERPROFILE,
# call dirs::home_dir(), or join a ".heiwa" root. Prose in println!/json! that
# documents the default location is not a violation.
#
# Test modules are skipped by brace-depth tracking rather than by exiting at
# the first `#[cfg(test)]`: files here put test modules in the middle as well
# as at the end, and an early exit blinded the scan to thousands of lines of
# production code (including every call site in cmd/app.rs).
strip_tests() {
  # A top-level `#[cfg(test)]` module starts at column 0 and closes with a
  # `}` at column 0, so skip between those. Brace counting looked more
  # precise but miscounts braces inside string literals and format specifiers,
  # which silently ended the skip early. Anything after the module's close is
  # scanned again, so a file with production code between two test modules is
  # fully covered.
  awk '
    !in_test && /^#\[cfg\(test\)\]/ { in_test = 1; next }
    in_test && /^\}/ { in_test = 0; next }
    in_test { next }
    { print FILENAME ":" FNR ":" $0 }
  ' "$1"
}

# Patterns that construct a state root rather than consuming ConfigRoot.
# Includes the HEIWA_* env vars themselves: reading those outside the
# resolver is how the tree grew eight competing resolvers in the first place.
resolver_pattern='env::var(_os)?\("(HOME|USERPROFILE|HOMEPATH|HEIWA_HOME|HEIWA_STATE_DIR|HEIWA_EVIDENCE_DIR)"\)'
resolver_pattern+='|dirs::home_dir|home::home_dir|directories::UserDirs'
resolver_pattern+='|join\("\.heiwa"\)|push\("\.heiwa"\)|PathBuf::from\("\.heiwa"\)|/\.heiwa/?"'

resolver_violations=""
while IFS= read -r file; do
  [[ "$file" == crates/heiwa_config/src/lib.rs ]] && continue
  hits="$(strip_tests "$file" | grep -E "$resolver_pattern" || true)"
  [[ -n "$hits" ]] && resolver_violations+="$hits"$'\n'
done < <(find apps/heiwa_core/src apps/heiwa_shell/src apps/heiwa_orchestrator/src crates \
  apps/heiwa_app/desktop/src-tauri/src \
  -name '*.rs' -not -path '*/tests/*' -not -name '*_test.rs' 2>/dev/null | sort)

resolver_violations="$(printf '%s' "$resolver_violations" | grep -v '^$' || true)"
if [[ -z "$resolver_violations" ]]; then
  ok "no independent home/state-root resolution outside ConfigRoot"
else
  count="$(printf '%s\n' "$resolver_violations" | wc -l | tr -d ' ')"
  fail_msg "$count independent home/state-root resolution(s) outside ConfigRoot:"
  printf '%s\n' "$resolver_violations" | head -40 >&2
fi

# The maintainer's name in any form, not a fixed list of three literals: a
# bare "Devon" in a mail-draft template is the same defect as a
# "devon-canonical" user id, and only the second was caught before.
identity_violations="$(grep -rniE --include='*.rs' --include='*.ts' --include='*.tsx' \
  '\bdevon\b|dmcgreg' \
  apps/heiwa_core/src apps/heiwa_shell/src apps/heiwa_orchestrator/src crates \
  "$desktop/src" apps/heiwa_app/clients/cockpit/src "$desktop/src-tauri/src" \
  2>/dev/null \
  | grep -v -e 'tests/' -e '_test\.' -e '\.test\.' \
  | grep -v 'contains("devon")' \
  || true)"
if [[ -z "$identity_violations" ]]; then
  ok "no hardcoded operator identity in runtime code"
else
  count="$(printf '%s\n' "$identity_violations" | wc -l | tr -d ' ')"
  fail_msg "$count hardcoded operator identity reference(s):"
  printf '%s\n' "$identity_violations" | head -20 >&2
fi

# ── 6. Token discipline: no raw hex colors outside theme layer ──────────────
if [[ -d "$desktop/src/theme" ]]; then
  # Every way to write a color, in stylesheets and in inline JSX styles —
  # a hex check alone lets rgb()/hsl()/oklch() and `style={{color:"#f00"}}`
  # straight through. `transparent`/`currentColor`/`inherit` are keywords,
  # not palette values, so they are allowed everywhere.
  color_violations="$(grep -rnE --include='*.css' --include='*.tsx' \
    '#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(|\boklch\(|\bcolor-mix\(' \
    "$desktop/src" \
    | grep -v "^$desktop/src/theme/" || true)"
  if [[ -z "$color_violations" ]]; then
    ok "styles consume tokens only (no raw color values outside theme/)"
  else
    count="$(printf '%s\n' "$color_violations" | wc -l | tr -d ' ')"
    fail_msg "$count raw color value(s) outside the theme layer:"
    printf '%s\n' "$color_violations" | head -20 >&2
  fi
else
  fail_msg "theme layer missing: $desktop/src/theme/"
fi

if (( fail != 0 )); then
  printf 'L0 acceptance gate FAILED.\n' >&2
  exit 1
fi
# Stamp HEAD only when HEAD is what actually passed. With a dirty tree the
# gate ran against uncommitted work, and attesting the commit would make the
# stamp a claim about code that was never tested.
if git diff --quiet && git diff --cached --quiet; then
  mkdir -p .claude && git rev-parse HEAD > .claude/l0-accept-sha
  printf 'L0 acceptance gate passed (stamp written for HEAD).\n'
else
  printf 'L0 acceptance gate passed. Tree is dirty, so no HEAD stamp was written.\n'
fi
