#!/usr/bin/env bash
set -euo pipefail

# acceptance-scope: apps crates Cargo.toml Cargo.lock scripts/check_l1_acceptance.sh
#
# Wider than the files this reads directly: it builds heiwa-shell and tests
# heiwa-provider, both of which compile most of the workspace, so a change in
# any crate can invalidate the result.

# L1 acceptance gate — roadmap 2026-08-14, layer L1 (BYOK provider tier).
#
# Deterministic checks:
#   1. heiwa-provider unit tests pass (direct-API adapters included)
#   2. direct-API adapter modules exist for the three CLI-dependent families
#   3. fresh-install harness passes: the shipped `heiwa` binary, run with an
#      emptied PATH and no system bin dirs, no reachable local runtime, and
#      one API key, completes a turn against a loopback mock, prints the
#      model's text, sends the prompt once, and registers no CLI account.
#      Covers the Anthropic wire format only
#   4. zero-provider state is usable: no accounts yields actionable guidance,
#      not a crash (asserted inside the harness)
#   5. adapter selection is not duplicated in the shell binary
#
# Local-only; the mock provider server binds loopback. No external network.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

fail=0
ok() { printf 'OK: %s\n' "$*"; }
fail_msg() { printf 'FAIL: %s\n' "$*" >&2; fail=1; }

# ── 2. Adapter modules exist ────────────────────────────────────────────────
for adapter in anthropic_api openai_api gemini_api; do
  if [[ -f "crates/heiwa_provider/src/providers/${adapter}.rs" ]]; then
    ok "direct-API adapter present: ${adapter}"
  else
    fail_msg "direct-API adapter missing: crates/heiwa_provider/src/providers/${adapter}.rs"
  fi
done

# ── 1. Provider crate tests ─────────────────────────────────────────────────
if cargo test -p heiwa-provider --quiet >/tmp/l1_provider_tests.log 2>&1; then
  ok "heiwa-provider tests"
else
  fail_msg "heiwa-provider tests (see /tmp/l1_provider_tests.log)"
fi

# ── 3+4. Fresh-install harness ──────────────────────────────────────────────
# The harness spawns the built `heiwa` binary with an emptied PATH and empty
# HEIWA_BIN_DIRS, a temp state root holding one API-key account, the key in
# the environment, and both the provider and the local runtime pointed at
# loopback. It asserts the model's text reaches stdout, the request carried
# the key exactly once, and no provider-CLI account was registered. It also
# asserts the zero-account guidance path.
if [[ -f "apps/heiwa_shell/tests/fresh_install.rs" ]]; then
  ok "fresh-install harness present"
  # The harness drives the binary, so it must exist before the test runs.
  if cargo build -p heiwa-shell --bin heiwa --quiet >/tmp/l1_build.log 2>&1; then
    if cargo test -p heiwa-shell --test fresh_install --quiet >/tmp/l1_fresh_install.log 2>&1; then
      ok "fresh-install harness passes (no CLI, no keychain, one API key → full turn)"
    else
      fail_msg "fresh-install harness failed (see /tmp/l1_fresh_install.log)"
    fi
  else
    fail_msg "heiwa binary did not build (see /tmp/l1_build.log)"
  fi
else
  fail_msg "fresh-install harness missing: apps/heiwa_shell/tests/fresh_install.rs"
fi

# ── 5. Adapter selection is not duplicated in the shell ─────────────────────
# A second provider-alias table in the shell is what broke L1 the first time:
# it never mapped vendor names onto route names, so every direct-API model was
# filtered out before routing and a valid API key reported "no working
# adapters". File existence proves nothing here — the property is that the
# shell has no alias table of its own.
if [[ ! -f "crates/heiwa_provider/src/routing.rs" || ! -f "crates/heiwa_provider/src/health.rs" ]]; then
  fail_msg "expected crates/heiwa_provider/src/{routing,health}.rs"
elif grep -nE '^[[:space:]]*(pub[[:space:]]+)?(fn[[:space:]]+canonical_provider_id|const[[:space:]]+SUPPORTED_ADAPTER_PROVIDERS)' \
     apps/heiwa_shell/src/main.rs >/tmp/l1_shell_alias.log 2>&1; then
  fail_msg "the shell defines its own provider alias table (see /tmp/l1_shell_alias.log); route through heiwa_provider::routing"
elif ! grep -q 'heiwa_provider::routing::' apps/heiwa_shell/src/main.rs; then
  fail_msg "the shell does not use heiwa_provider::routing for adapter selection"
else
  ok "adapter selection is shared: the shell has no provider alias table of its own"
fi

if (( fail != 0 )); then
  printf 'L1 acceptance gate FAILED.\n' >&2
  exit 1
fi
# Stamp HEAD only when HEAD is what actually passed.
if git diff --quiet && git diff --cached --quiet; then
  mkdir -p .claude && git rev-parse HEAD > .claude/l1-accept-sha
  printf 'L1 acceptance gate passed (stamp written for HEAD).\n'
else
  printf 'L1 acceptance gate passed. Tree is dirty, so no HEAD stamp was written.\n'
fi
