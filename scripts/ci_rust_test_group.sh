#!/usr/bin/env bash
set -euo pipefail

runtime_packages=(
  heiwa-core
  heiwa-loop
  heiwa-orchestrator
  heiwa-provider
  heiwa-session
  heiwa_drex
  heiwa_vault
)

foundation_packages=(
  heiwa-a2a
  heiwa-install
  heiwa-memory
  heiwa-protocol
  heiwa-repl
  heiwa-resource
  heiwa-tui
  heiwa_automations
  heiwa_config
  heiwa_embed
  heiwa_evidence
  heiwa_identity
  heiwa_mcp
  heiwa_mesh
  heiwa_oauth
  heiwa_quota
  heiwa_receipts
  heiwa_work
  heiwa_worker
  heiwa_workspace
)

shell_api_targets=(
  agentic_smoke
  app_api
  model_call_executor
  operator_api
)

shell_state_targets=(
  approval_gate
  approvals_decide
  auto
  first_run
  fresh_install
  local_boot
)

shell_ops_targets=(
  apple_calendar_connector
  calendar_sync
  mail_triage
  schedule
  smoke
)

runtime_a_targets=(
  auth_session
  bootstrap_smoke
  drex_call_routing
  drex_golden
  drex_persistence
  drex_provider_routing
  drex_scoring
  loop_execution
)

runtime_b_targets=(
  ollama_detect
  openrouter_live
  operator_service
  provider_adapters
  provider_auth
  registry_test
  restart_recovery
  run_receipts
  session_attach
  transcript_migration
  worker_mesh
)

foundation_a_targets=(
  cockpit_contract
  drex_golden
  full_flow
  install_doctor
  journal
  operator_journal
  state
)

foundation_b_targets=(
  command_parse
  local_tools
  mesh_node
  policy
  runtime
  smoke
  telemetry_pane
  runs
  work_core
  work_session
  workspace_core
)

validate_groups() {
  local metadata expected actual drift
  metadata="$(cargo metadata --locked --no-deps --format-version 1)"
  expected="$({
    printf '%s\n' heiwa-shell
    printf '%s\n' "${runtime_packages[@]}"
    printf '%s\n' "${foundation_packages[@]}"
  } | LC_ALL=C sort)"
  actual="$(jq -r '.packages[].name' <<<"$metadata" | grep -v '^heiwa-desktop$' | LC_ALL=C sort)"
  drift="$(comm -3 <(printf '%s\n' "$expected") <(printf '%s\n' "$actual"))"
  if [[ -n "$drift" ]]; then
    printf '%s\n' 'Rust CI test groups do not match the non-desktop workspace:' >&2
    printf '%s\n' "$drift" >&2
    return 1
  fi

  validate_target_set "shell integration" "$metadata" \
    <(printf '%s\n' "${shell_api_targets[@]}" "${shell_state_targets[@]}" "${shell_ops_targets[@]}") \
    heiwa-shell
  validate_target_set "runtime integration" "$metadata" \
    <(printf '%s\n' "${runtime_a_targets[@]}" "${runtime_b_targets[@]}") \
    "${runtime_packages[@]}"
  validate_target_set "foundation integration" "$metadata" \
    <(printf '%s\n' "${foundation_a_targets[@]}" "${foundation_b_targets[@]}") \
    "${foundation_packages[@]}"
}

validate_target_set() {
  local label="$1"
  local metadata="$2"
  local expected_file="$3"
  shift 3
  local packages=("$@")
  local expected actual drift package

  expected="$(LC_ALL=C sort -u "$expected_file")"
  actual="$({
    for package in "${packages[@]}"; do
      jq -r --arg package "$package" '
        .packages[] |
        select(.name == $package) |
        .targets[] |
        select(.kind | index("test")) |
        .name
      ' <<<"$metadata"
    done
  } | LC_ALL=C sort -u)"
  drift="$(comm -3 <(printf '%s\n' "$expected") <(printf '%s\n' "$actual"))"
  if [[ -n "$drift" ]]; then
    printf 'Rust CI %s targets do not match Cargo metadata:\n%s\n' "$label" "$drift" >&2
    return 1
  fi
}

validate_groups

group="${1:-}"
if [[ "$group" == "--check" ]]; then
  printf '%s\n' 'Rust CI test groups cover every non-desktop workspace package exactly once.'
  exit 0
fi

case "$group" in
  shell-unit)
    packages=(heiwa-shell)
    targets=(--lib --bins)
    ;;
  shell-api)
    packages=(heiwa-shell)
    targets=()
    for target in "${shell_api_targets[@]}"; do targets+=(--test "$target"); done
    ;;
  shell-state)
    packages=(heiwa-shell)
    targets=()
    for target in "${shell_state_targets[@]}"; do targets+=(--test "$target"); done
    ;;
  shell-ops)
    packages=(heiwa-shell)
    targets=()
    for target in "${shell_ops_targets[@]}"; do targets+=(--test "$target"); done
    ;;
  runtime-unit)
    packages=("${runtime_packages[@]}")
    targets=(--lib --bins)
    ;;
  runtime-a)
    packages=("${runtime_packages[@]}")
    targets=()
    for target in "${runtime_a_targets[@]}"; do targets+=(--test "$target"); done
    ;;
  runtime-b)
    packages=("${runtime_packages[@]}")
    targets=()
    for target in "${runtime_b_targets[@]}"; do targets+=(--test "$target"); done
    ;;
  foundation-unit)
    packages=("${foundation_packages[@]}")
    targets=(--lib --bins)
    ;;
  foundation-a)
    packages=("${foundation_packages[@]}")
    targets=()
    for target in "${foundation_a_targets[@]}"; do targets+=(--test "$target"); done
    ;;
  foundation-b)
    packages=("${foundation_packages[@]}")
    targets=()
    for target in "${foundation_b_targets[@]}"; do targets+=(--test "$target"); done
    ;;
  *)
    printf 'usage: %s {shell-unit|shell-api|shell-state|shell-ops|runtime-unit|runtime-a|runtime-b|foundation-unit|foundation-a|foundation-b|--check}\n' "$0" >&2
    exit 2
    ;;
esac

package_args=()
for package in "${packages[@]}"; do
  package_args+=(--package "$package")
done

exec cargo nextest run \
  --locked \
  --no-default-features \
  "${package_args[@]}" \
  "${targets[@]}"
