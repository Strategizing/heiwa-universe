#!/usr/bin/env python3
"""
Gate A2: configuration sanity checks.
Validates JSON/TOML and performs lightweight YAML text checks.
"""

from __future__ import annotations

import json
import subprocess
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]


def git_files(*patterns: str) -> list[Path]:
    files: list[Path] = []
    for pattern in patterns:
        result = subprocess.run(
            ["git", "ls-files", pattern],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        for line in result.stdout.splitlines():
            line = line.strip()
            if line:
                files.append(ROOT / line)
    return files


def lint_json() -> list[str]:
    issues: list[str] = []
    for path in git_files("*.json", "**/*.json"):
        try:
            json.loads(path.read_text(encoding="utf-8"))
        except Exception as exc:
            issues.append(f"{path.relative_to(ROOT)} invalid JSON: {exc}")
    return issues


def lint_toml() -> list[str]:
    issues: list[str] = []
    for path in [ROOT / "pyproject.toml", ROOT / "railway.toml"]:
        if not path.exists():
            continue
        try:
            tomllib.loads(path.read_text(encoding="utf-8"))
        except Exception as exc:
            issues.append(f"{path.relative_to(ROOT)} invalid TOML: {exc}")
    return issues


def lint_yaml_text() -> list[str]:
    issues: list[str] = []
    yaml_files = git_files("*.yaml", "*.yml", "**/*.yaml", "**/*.yml")
    for path in yaml_files:
        text = path.read_text(encoding="utf-8")
        if "\t" in text:
            issues.append(f"{path.relative_to(ROOT)} contains tab characters")
        if text.strip() == "":
            issues.append(f"{path.relative_to(ROOT)} is empty")

    config_candidates = [
        ROOT / "config/agents.yaml",
        ROOT / "runtime/fleets/hub/agent.yaml",
    ]
    if not any(path.exists() for path in config_candidates):
        issues.append("missing runtime config manifest (config/agents.yaml or runtime/fleets/hub/agent.yaml)")

    return issues


def main() -> int:
    issues = []
    issues.extend(lint_json())
    issues.extend(lint_toml())
    issues.extend(lint_yaml_text())

    if issues:
        print("FAIL: lint_config")
        for issue in issues:
            print(f"- {issue}")
        return 1

    print("PASS: lint_config")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
