#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
checker="$repo_root/scripts/check_ci_job_deadlines.rb"
workdir="$(mktemp -d "${TMPDIR:-/tmp}/heiwa-ci-deadlines.XXXXXX")"
trap 'rm -rf "$workdir"' EXIT

log="$workdir/log"

expect_reject() {
  local fixture="$1"
  local needle="$2"
  if ruby "$checker" "$fixture" >"$log" 2>&1; then
    echo "deadline checker accepted $fixture; expected rejection for: $needle" >&2
    cat "$log" >&2
    exit 1
  fi
  if ! grep -Fq "$needle" "$log"; then
    echo "deadline checker rejected $fixture without reporting: $needle" >&2
    cat "$log" >&2
    exit 1
  fi
}

expect_accept() {
  local fixture="$1"
  if ! ruby "$checker" "$fixture" >"$log" 2>&1; then
    echo "deadline checker rejected $fixture; expected acceptance" >&2
    cat "$log" >&2
    exit 1
  fi
}

# The live workflow must satisfy its own gate.
expect_accept "$repo_root/.github/workflows/ci.yml"

# CI must run before experimental work can enter `dev`; protecting `dev` with
# contexts from a workflow that only targets `main` would deadlock every PR.
missing_dev_target="$workdir/missing-dev-target.yml"
sed 's/branches: \["dev", "main"\]/branches: ["main"]/' \
  "$repo_root/.github/workflows/ci.yml" >"$missing_dev_target"
expect_reject "$missing_dev_target" \
  'Heiwa CI pull_request targets must include dev and main'

# A job with no deadline at all is unbounded and must be rejected. Appending to
# the real workflow also proves the checker parses the shipped file.
missing="$workdir/missing-deadline.yml"
cp "$repo_root/.github/workflows/ci.yml" "$missing"
cat >>"$missing" <<'YAML'

  _unbounded_job:
    name: Missing deadline fixture
    runs-on: ubuntu-latest
    steps:
      - run: true
YAML
expect_reject "$missing" 'missing: _unbounded_job (no deadline)'

# Feedback-lane jobs stay sub-minute.
slow_feedback="$workdir/slow-feedback.yml"
cat >"$slow_feedback" <<'YAML'
name: fixture
on: workflow_dispatch
jobs:
  lint:
    name: Slow feedback fixture
    runs-on: ubuntu-latest
    timeout-minutes: 5
    steps:
      - run: true
YAML
expect_reject "$slow_feedback" 'missing: lint (5m > 1m)'

# The Rust promotion lanes get a larger cap, but it is still a hard cap.
slow_compile="$workdir/slow-compile.yml"
cat >"$slow_compile" <<'YAML'
name: fixture
on: workflow_dispatch
jobs:
  rust-tests:
    name: Rust Tests
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      - run: true
YAML
expect_reject "$slow_compile" 'missing: rust-tests (30m > 20m)'

# ...and a compile lane inside its cap is accepted even though it is not sub-minute.
ok_compile="$workdir/ok-compile.yml"
cat >"$ok_compile" <<'YAML'
name: fixture
on: workflow_dispatch
jobs:
  rust-static:
    name: Rust Static Checks
    runs-on: ubuntu-latest
    timeout-minutes: 20
    steps:
      - run: true
YAML
expect_accept "$ok_compile"

# A deadline cannot rescue a runner label nothing claims: the job never starts,
# so the clock never runs. Unproven labels are rejected outright.
unproven_runner="$workdir/unproven-runner.yml"
cat >"$unproven_runner" <<'YAML'
name: fixture
on: workflow_dispatch
jobs:
  rust-tests:
    name: Rust Tests
    runs-on: blacksmith-16vcpu-ubuntu-2404
    timeout-minutes: 20
    steps:
      - run: true
YAML
expect_reject "$unproven_runner" 'unproven: rust-tests (blacksmith-16vcpu-ubuntu-2404)'

echo "CI job deadline checker tests passed."
