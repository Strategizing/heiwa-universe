#!/usr/bin/env python3
"""
Gate A1: cold build sanity.
Performs syntax and structure checks without external dependencies.
"""

from __future__ import annotations

import py_compile
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


def tracked_python_files() -> list[Path]:
    try:
        result = subprocess.run(
            ["git", "ls-files", "*.py"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=True,
        )
        files = [ROOT / line.strip() for line in result.stdout.splitlines() if line.strip()]
        return files
    except Exception:
        return sorted(ROOT.rglob("*.py"))


def compile_all(paths: list[Path]) -> list[str]:
    errors: list[str] = []
    for path in paths:
        try:
            py_compile.compile(str(path), doraise=True)
        except Exception as exc:
            errors.append(f"{path}: {exc}")
    return errors


def check_required_paths() -> list[str]:
    required = [
        ROOT / "apps/heiwa_hub/main.py",
        ROOT / "apps/heiwa_hub/config.py",
        ROOT / "apps/heiwa_cli/scripts/agents/sentinel.py",
        ROOT / "requirements.txt",
    ]
    missing = [str(p.relative_to(ROOT)) for p in required if not p.exists()]
    return missing


def check_wrapper_flags() -> list[str]:
    issues: list[str] = []
    codex_wrapper = ROOT / "apps/heiwa_cli/scripts/agents/wrappers/codex_exec.sh"
    if codex_wrapper.exists():
        text = codex_wrapper.read_text(encoding="utf-8")
        if "--approval-mode" in text:
            issues.append("apps/heiwa_cli/scripts/agents/wrappers/codex_exec.sh uses deprecated --approval-mode flag")
    return issues


def main() -> int:
    failures: list[str] = []
    py_files = tracked_python_files()
    if not py_files:
        print("FAIL: no Python files found")
        return 1

    failures.extend(compile_all(py_files))
    missing = check_required_paths()
    failures.extend([f"missing required path: {m}" for m in missing])
    failures.extend(check_wrapper_flags())

    if failures:
        print("FAIL: gate_build")
        for item in failures:
            print(f"- {item}")
        return 1

    print("PASS: gate_build")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())