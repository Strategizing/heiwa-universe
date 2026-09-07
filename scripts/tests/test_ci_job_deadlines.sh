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

# The live workflows must satisfy their own gates, including the publication
# jobs that used to escape the CI-only runner check.
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
name: Heiwa CI
on:
  pull_request:
    branches: [dev, main]
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
name: Heiwa CI
on:
  pull_request:
    branches: [dev, main]
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
name: Heiwa CI
on:
  pull_request:
    branches: [dev, main]
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
name: Heiwa CI
on:
  pull_request:
    branches: [dev, main]
jobs:
  rust-tests:
    name: Rust Tests
    runs-on: blacksmith-16vcpu-ubuntu-2404
    timeout-minutes: 20
    steps:
      - run: true
YAML
expect_reject "$unproven_runner" 'unproven: rust-tests (blacksmith-16vcpu-ubuntu-2404)'

# Historical adoption is not current health: the retired smaller runner must
# be rejected too, including when buried in a publishing matrix include.
retired_runner="$workdir/retired-runner.yml"
sed 's/blacksmith-16vcpu/blacksmith-4vcpu/' "$unproven_runner" >"$retired_runner"
expect_reject "$retired_runner" 'unproven: rust-tests (blacksmith-4vcpu-ubuntu-2404)'

matrix_runner="$workdir/matrix-runner.yml"
cat >"$matrix_runner" <<'YAML'
name: Publication fixture
on: workflow_dispatch
jobs:
  publish:
    runs-on: ${{ matrix.runner }}
    timeout-minutes: 60
    strategy:
      matrix:
        include:
          - runner: ubuntu-latest
          - runner: macos-26
          - runner: windows-latest
    steps:
      - run: true
YAML
expect_accept "$matrix_runner"
sed 's/runner: windows-latest/runner: blacksmith-4vcpu-windows-2025/' \
  "$matrix_runner" >"$retired_runner"
expect_reject "$retired_runner" 'unproven: publish (blacksmith-4vcpu-windows-2025)'

axis_runner="$workdir/axis-runner.yml"
cat >"$axis_runner" <<'YAML'
name: Certification fixture
on: workflow_dispatch
jobs:
  certify:
    runs-on: ${{ matrix.os }}
    timeout-minutes: 12
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-2025]
        include:
          - os: macos-15
    steps:
      - run: true
YAML
expect_accept "$axis_runner"
sed 's/os: macos-15/os: blacksmith-6vcpu-macos-15/' \
  "$axis_runner" >"$retired_runner"
expect_reject "$retired_runner" 'unproven: certify (blacksmith-6vcpu-macos-15)'

# An include row without a runner may create a new matrix combination with
# no selector. Require explicit selectors rather than guessing inheritance.
sed 's/os: macos-15/name: unbound-row/' "$axis_runner" >"$retired_runner"
expect_reject "$retired_runner" 'certify (unresolved matrix.os)'
sed 's/os: macos-15/os: blacksmith-6vcpu-macos-15/' \
  "$axis_runner" >"$retired_runner"

# Missing runners, groups, conjunctions and unresolved dynamic selectors must
# fail closed instead of passing as an empty label list.
missing_runner="$workdir/missing-runner.yml"
sed '/runs-on:/d' "$matrix_runner" >"$missing_runner"
expect_reject "$missing_runner" 'publish (missing or unsupported runs-on)'
sed 's/runs-on: .*/runs-on: [ubuntu-latest, windows-latest]/' \
  "$matrix_runner" >"$missing_runner"
expect_reject "$missing_runner" 'publish (missing or unsupported runs-on)'
sed 's/runs-on: .*/runs-on: {group: private-runners}/' \
  "$matrix_runner" >"$missing_runner"
expect_reject "$missing_runner" 'publish (missing or unsupported runs-on)'
sed 's/matrix.runner/needs.setup.outputs.runner/' \
  "$matrix_runner" >"$missing_runner"
expect_reject "$missing_runner" 'publish (unresolved runner expression'
sed 's/runner: windows-latest/name: no-runner/' \
  "$matrix_runner" >"$missing_runner"
expect_reject "$missing_runner" 'publish (unresolved matrix.runner)'
sed 's/runner: windows-latest/runner: ${{ inputs.runner }}/' \
  "$matrix_runner" >"$missing_runner"
expect_reject "$missing_runner" 'publish (unresolved matrix.runner)'
sed 's/runs-on: .*/runs-on: [ubuntu-latest]/' \
  "$matrix_runner" >"$missing_runner"
expect_accept "$missing_runner"

# Local reusable jobs own no runner; their workflow jobs are checked during
# normal all-workflow discovery.
reusable="$workdir/reusable.yml"
cat >"$reusable" <<'YAML'
name: Release fixture
on: workflow_dispatch
jobs:
  verify-install:
    uses: ./.github/workflows/public-install-smoke.yml
YAML
expect_accept "$reusable"
sed 's|./.github/workflows/public-install-smoke.yml|third-party/runners/.github/workflows/build.yml@0123456789012345678901234567890123456789|' \
  "$reusable" >"$missing_runner"
expect_reject "$missing_runner" 'verify-install (runner policy only inspects local reusable workflows)'

# Swapping labels alone cannot repair a publishing job that still invokes
# the retired provider's remote Docker builder.
retired_builder="$workdir/retired-builder.yml"
cat >"$retired_builder" <<'YAML'
name: Container fixture
on: workflow_dispatch
jobs:
  container:
    runs-on: ubuntu-latest
    steps:
      - uses: useblacksmith/setup-docker-builder@9309da73a81f66976a6d750572e221508b1e2682
YAML
expect_reject "$retired_builder" 'retired runner action: container (useblacksmith/setup-docker-builder'

# Dynamic runner input is allowed only in the isolated manual canary, never
# in another workflow or after adding a push / workflow_call trigger.
canary="$workdir/runner-canary.yml"
cp "$repo_root/.github/workflows/runner-canary.yml" "$canary"
expect_accept "$canary"
sed 's/name: Runner Canary/name: Publication fixture/' "$canary" >"$missing_runner"
expect_reject "$missing_runner" 'probe (unresolved runner expression'
for trigger in push workflow_call; do
  sed "s/^  workflow_dispatch:/  $trigger:\\n  workflow_dispatch:/" \
    "$repo_root/.github/workflows/runner-canary.yml" >"$canary"
  expect_reject "$canary" 'probe (unresolved runner expression'
done
sed 's/timeout-minutes: 5/timeout-minutes: 6/' \
  "$repo_root/.github/workflows/runner-canary.yml" >"$canary"
expect_reject "$canary" 'probe (unresolved runner expression'

# Default invocation must discover new publication workflows, not silently
# keep validating ci.yml alone.
fixture_repo="$workdir/repo"
mkdir -p "$fixture_repo/.github/workflows"
cp "$repo_root/.github/workflows/ci.yml" "$fixture_repo/.github/workflows/ci.yml"
cp "$retired_runner" "$fixture_repo/.github/workflows/future-publish.yaml"
if (cd "$fixture_repo" && ruby "$checker") >"$log" 2>&1; then
  echo "default checker skipped a publishing workflow with a retired runner" >&2
  exit 1
fi
grep -Fq 'future-publish.yaml' "$log"
grep -Fq 'blacksmith-6vcpu-macos-15' "$log"

(cd "$repo_root" && ruby "$checker")
echo "CI deadline and workflow runner checker tests passed."
