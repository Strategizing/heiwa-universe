#!/usr/bin/env bash
# A caller can supply a run-owned directory; standalone gates get their own.
verification_log_dir() {
  local repo_root="$1" scope="$2"
  if [[ -n "${HEIWA_VERIFICATION_LOG_DIR:-}" ]]; then
    mkdir -p "$HEIWA_VERIFICATION_LOG_DIR"
    printf '%s\n' "$HEIWA_VERIFICATION_LOG_DIR"
  else
    mkdir -p "$repo_root/private/verification"
    mktemp -d "$repo_root/private/verification/${scope}.XXXXXX"
  fi
}
