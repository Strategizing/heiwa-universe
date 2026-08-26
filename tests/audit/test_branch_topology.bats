#!/usr/bin/env bats

setup() {
    REPO_ROOT="$(git rev-parse --show-toplevel)"
    CHECK="$REPO_ROOT/scripts/check_branch_topology.sh"
    FIXTURE="$BATS_TEST_TMPDIR/repo"

    git init -q -b main "$FIXTURE"
    git -C "$FIXTURE" config user.name "Heiwa Test"
    git -C "$FIXTURE" config user.email "test@heiwa.invalid"
    git -C "$FIXTURE" commit -q --allow-empty -m "initial"
    git -C "$FIXTURE" branch dev
    git -C "$FIXTURE" update-ref refs/remotes/origin/main "$(git -C "$FIXTURE" rev-parse main)"
}

run_check() {
    cd "$FIXTURE"
    "$CHECK" "$@"
}

@test "integration rejects dev when production is ahead" {
    git -C "$FIXTURE" commit -q --allow-empty -m "production-only"
    git -C "$FIXTURE" update-ref refs/remotes/origin/main "$(git -C "$FIXTURE" rev-parse main)"
    git -C "$FIXTURE" checkout -q dev

    run run_check --mode integration

    [ "$status" -ne 0 ]
    [[ "$output" == *"dev is behind origin/main by 1 commit(s)"* ]]
}

@test "integration rejects equality instead of inventing a synthetic commit" {
    git -C "$FIXTURE" checkout -q dev

    run run_check --mode integration

    [ "$status" -ne 0 ]
    [[ "$output" == *"dev has no value-bearing commit ahead of origin/main"* ]]
}

@test "integration accepts dev with a real commit ahead of production" {
    git -C "$FIXTURE" checkout -q dev
    git -C "$FIXTURE" commit -q --allow-empty -m "verified integration value"

    run run_check --mode integration

    [ "$status" -eq 0 ]
    [[ "$output" == *"dev is 1 commit(s) ahead of origin/main"* ]]
}

@test "post-promotion mode permits the brief synchronized handoff" {
    git -C "$FIXTURE" checkout -q dev

    run run_check --mode post-promotion

    [ "$status" -eq 0 ]
    [[ "$output" == *"dev is synchronized with origin/main"* ]]
}

@test "experimental accepts a non-integration branch descended from dev" {
    git -C "$FIXTURE" checkout -q dev
    git -C "$FIXTURE" commit -q --allow-empty -m "verified integration value"
    git -C "$FIXTURE" checkout -q -b codex/experiment

    run run_check --mode experimental

    [ "$status" -eq 0 ]
    [[ "$output" == *"codex/experiment descends from dev"* ]]
}

@test "experimental rejects a branch that did not start from current dev" {
    git -C "$FIXTURE" checkout -q dev
    git -C "$FIXTURE" commit -q --allow-empty -m "verified integration value"
    git -C "$FIXTURE" checkout -q -b codex/experiment main

    run run_check --mode experimental

    [ "$status" -ne 0 ]
    [[ "$output" == *"codex/experiment does not descend from dev"* ]]
}

@test "agent baseline accepts an explicitly declared experimental checkout" {
    if ! git -C "$REPO_ROOT" symbolic-ref --quiet --short HEAD >/dev/null; then
        skip "live baseline integration requires a named experimental checkout"
    fi

    run env HEIWA_BRANCH_MODE=experimental \
        "$REPO_ROOT/scripts/check_agent_baseline.sh" --allow-dirty

    [ "$status" -eq 0 ]
    [[ "$output" == *"branch topology (experimental)"* ]]
}

@test "agent baseline rejects an unknown branch mode clearly" {
    run env HEIWA_BRANCH_MODE=unknown \
        "$REPO_ROOT/scripts/check_agent_baseline.sh" --allow-dirty

    [ "$status" -eq 2 ]
    [[ "$output" == *"invalid HEIWA_BRANCH_MODE: unknown"* ]]
}
