#!/usr/bin/env bash
set -euo pipefail

# acceptance-scope: apps crates Cargo.toml Cargo.lock scripts/check_l2_acceptance.sh scripts/lib/verification_logs.sh
#
# Same reason as L1, plus the readiness-decider scan, which walks all of
# apps/ and crates/ looking for a second place that decides onboarding.

# L2 acceptance gate — roadmap 2026-08-14, layer L2 (onboarding and per-user
# identity).
#
# The roadmap's Verification section defines criteria for L0, L1, L3 and L4
# and none for L2. This gate is that criterion, chosen to match L2's own
# wording — "first run establishes a local user identity, a configuration
# root, and at least one provider account, entirely inside the application":
#
#   1. heiwa_identity unit tests pass (identity record + onboarding projection)
#   2. the first-run harness passes: the shipped binary walks an empty state
#      root to a completed turn using only what it told the user to do
#   3. identity is local and per-installation — no code path mints one under
#      the process working directory, and none reads it from the repository
#   4. onboarding state has one implementation, so the CLI and the desktop
#      cannot disagree about whether a user is set up
#
# Local-only; the harness binds loopback. No external network.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
source "$repo_root/scripts/lib/verification_logs.sh"
umask 077
log_dir="$(verification_log_dir "$repo_root" "l2")"

fail=0
ok() { printf 'OK: %s\n' "$*"; }
fail_msg() { printf 'FAIL: %s\n' "$*" >&2; fail=1; }

# ── 1. Identity + onboarding unit tests ─────────────────────────────────────
if cargo test -p heiwa_identity --quiet >"$log_dir/l2_identity_tests.log" 2>&1; then
  ok "heiwa_identity tests"
else
  fail_msg "heiwa_identity tests (see $log_dir/l2_identity_tests.log)"
fi

# ── 2. First-run harness ────────────────────────────────────────────────────
if [[ ! -f "apps/heiwa_shell/tests/first_run.rs" ]]; then
  fail_msg "first-run harness missing: apps/heiwa_shell/tests/first_run.rs"
elif ! cargo build -p heiwa-shell --bin heiwa --quiet >"$log_dir/l2_build.log" 2>&1; then
  fail_msg "heiwa binary did not build (see $log_dir/l2_build.log)"
elif cargo test -p heiwa-shell --test first_run --quiet >"$log_dir/l2_first_run.log" 2>&1; then
  ok "first-run harness passes (empty root → identity → provider → turn)"
else
  fail_msg "first-run harness failed (see $log_dir/l2_first_run.log)"
fi

# ── 3. Identity is per-user, not per-repository ─────────────────────────────
# The Python package the roadmap named as the starting point resolves identity
# from the monorepo root — the single-seat pattern L0 removed. The Rust crate
# must not reacquire it: no repo-relative discovery, and no lenient resolver
# that would fall back to the process working directory.
if grep -nE 'monorepo|CARGO_MANIFEST_DIR|current_dir|HeiwaPaths::resolve\(\)' \
   crates/heiwa_identity/src/*.rs >"$log_dir/l2_identity_paths.log" 2>&1; then
  fail_msg "heiwa_identity resolves a path outside the strict per-user root (see $log_dir/l2_identity_paths.log)"
else
  ok "identity resolves only through the strict per-user root"
fi

# ── 4. One onboarding implementation ────────────────────────────────────────
# Every surface must read the same projection. A second place that decides
# "is this user set up" is how a first-run screen and a CLI come to disagree.
readiness_deciders=$(grep -rlE 'fn .*(is_onboarded|onboarding_complete|needs_onboarding)' \
  apps/ crates/ --include='*.rs' 2>/dev/null | grep -v 'crates/heiwa_identity/' || true)
if [[ -n "$readiness_deciders" ]]; then
  fail_msg "onboarding readiness is decided outside heiwa_identity::onboarding: $readiness_deciders"
elif ! grep -q 'heiwa_identity::onboarding' apps/heiwa_shell/src/main.rs; then
  fail_msg "the shell does not consume heiwa_identity::onboarding"
else
  ok "onboarding readiness has a single implementation"
fi

if (( fail != 0 )); then
  printf 'L2 acceptance gate FAILED.\n' >&2
  exit 1
fi
# Stamp HEAD only when HEAD is what actually passed.
if git diff --quiet && git diff --cached --quiet; then
  mkdir -p .claude && git rev-parse HEAD > .claude/l2-accept-sha
  printf 'L2 acceptance gate passed (stamp written for HEAD).\n'
else
  printf 'L2 acceptance gate passed. Tree is dirty, so no HEAD stamp was written.\n'
fi
