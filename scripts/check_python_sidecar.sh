#!/usr/bin/env bash
# The same frozen sidecar environment and checks run locally and in PR CI.
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root/runtime/python"
uv run --locked --all-extras --python 3.14 python -m pytest -q
uv run --locked --all-extras --python 3.14 ruff check src tests
