#!/usr/bin/env bash

# resolve_local_python REPOSITORY_ROOT
# Prints the interpreter used by local acceptance gates. Repository-managed
# interpreters are always absolute so nested recipes can change directories.
resolve_local_python() {
  local repository_root="${1:-}"

  if [[ -z "$repository_root" ]]; then
    echo "resolve_local_python: missing repository root" >&2
    return 2
  fi

  if [[ -n "${HEIWA_PYTHON:-}" ]]; then
    printf '%s\n' "$HEIWA_PYTHON"
  elif [[ -x "$repository_root/.venv/bin/python" ]]; then
    printf '%s\n' "$repository_root/.venv/bin/python"
  else
    command -v python3
  fi
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  resolve_local_python "${1:-}"
fi
