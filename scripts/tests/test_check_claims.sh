#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/heiwa-check-claims.XXXXXX")"
trap 'rm -rf -- "$fixture"' EXIT

mkdir -p "$fixture/repo/scripts" "$fixture/bin" "$fixture/temp"
cp "$repo_root/scripts/check_claims.sh" "$fixture/repo/scripts/check_claims.sh"

cat >"$fixture/bin/uname" <<'SH'
#!/usr/bin/env bash
printf 'MINGW64_NT-10.0\n'
SH
chmod +x "$fixture/bin/uname"

cat >"$fixture/bin/cargo" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

if [[ "${FAKE_CARGO_FAIL:-0}" == "1" ]]; then
  printf 'synthetic claims build failure\n' >&2
  exit 7
fi

mkdir -p "$CARGO_TARGET_DIR/debug"
cat >"$CARGO_TARGET_DIR/debug/heiwa-claims.exe" <<'BIN'
#!/usr/bin/env bash
[[ "${1:-}" == "check" ]]
BIN
chmod +x "$CARGO_TARGET_DIR/debug/heiwa-claims.exe"
SH
chmod +x "$fixture/bin/cargo"

target_dir="$fixture/custom target"
output="$fixture/output"
if ! PATH="$fixture/bin:$PATH" \
  CARGO_TARGET_DIR="$target_dir" \
  TMPDIR="$fixture/temp" \
  bash "$fixture/repo/scripts/check_claims.sh" >"$output" 2>&1; then
  cat "$output" >&2
  printf 'claim gate did not resolve the Windows binary under CARGO_TARGET_DIR\n' >&2
  exit 1
fi

if PATH="$fixture/bin:$PATH" \
  CARGO_TARGET_DIR="$target_dir" \
  TMPDIR="$fixture/temp" \
  FAKE_CARGO_FAIL=1 \
  bash "$fixture/repo/scripts/check_claims.sh" >"$output" 2>&1; then
  printf 'claim gate accepted a failed cargo build\n' >&2
  exit 1
fi
grep -Fq 'synthetic claims build failure' "$output"

if find "$fixture/temp" -mindepth 1 -print -quit | grep -q .; then
  printf 'claim gate left temporary build logs behind\n' >&2
  find "$fixture/temp" -mindepth 1 -maxdepth 1 -print >&2
  exit 1
fi

printf 'Claim gate portability tests passed.\n'
