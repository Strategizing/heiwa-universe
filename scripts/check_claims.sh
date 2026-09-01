#!/usr/bin/env bash
set -euo pipefail

# acceptance-scope: claims crates/heiwa_claims Cargo.toml Cargo.lock scripts/check_claims.sh
#
# Executable claim registry gate.
#
# Fails when any claim in claims/*.toml does not currently meet the state its
# consumers require. That is the whole contract: Heiwa may not advertise a
# capability whose proof does not hold at this exact source state.
#
# This gate replaces nothing yet. The roadmap acceptance scripts and their
# `.claude/*-accept-sha` stamps keep working; the registry is the
# provider-neutral generalization they migrate into once the receipt taxonomy
# lands. Running both for a while is deliberate — a claim mechanism that has
# never disagreed with the mechanism it replaces has not been tested.
#
# Local-only. No network. The `cargo-test` verifiers build, so a cold tree is
# slow on first run and cached afterwards.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

build_log="$(mktemp "${TMPDIR:-/tmp}/heiwa-claims-build.XXXXXX")"
trap 'rm -f -- "$build_log"' EXIT

if ! cargo build -q -p heiwa_claims --bin heiwa-claims 2>"$build_log"; then
  printf 'FAIL: heiwa-claims did not build\n' >&2
  cat "$build_log" >&2
  exit 1
fi

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) exe_suffix=".exe" ;;
  *) exe_suffix="" ;;
esac
target_dir="${CARGO_TARGET_DIR:-target}"
bin="$target_dir/debug/heiwa-claims$exe_suffix"
if [[ ! -x "$bin" ]]; then
  printf 'FAIL: %s missing after build\n' "$bin" >&2
  exit 1
fi

if "$bin" check; then
  printf '\nClaim registry gate passed.\n'
  exit 0
fi

cat >&2 <<'HELP'

Every claim above marked MISS is one of:

  planned      the subject does not exist yet — build it or drop the claim
  implemented  the subject exists but nothing proves the claim — run:
                 cargo run -q -p heiwa_claims --bin heiwa-claims -- verify <claim_id>
  degraded     the proof no longer holds — read the reason, fix, then reverify
  retired      the subject is gone — retire the claim in claims/*.toml

`verify` refuses a scope with uncommitted changes. Evidence names a commit, so
it has to be evidence about that commit: commit first, then verify, then commit
the evidence record.
HELP
exit 1
