#!/usr/bin/env python3
"""
Rule C1/C2: SQL discipline linter.
Detects obvious string-interpolated SQL patterns in Python code.
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SQL_STATEMENT = re.compile(
    r"\b(SELECT\s+.+\s+FROM|INSERT\s+INTO|UPDATE\s+\w+\s+SET|DELETE\s+FROM)\b",
    re.IGNORECASE,
)


def tracked_python_files() -> list[Path]:
    result = subprocess.run(
        ["git", "ls-files", "*.py"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    files = []
    for line in result.stdout.splitlines():
        line = line.strip()
        if line:
            files.append(ROOT / line)
    return files


def lint_file(path: Path) -> list[str]:
    issues: list[str] = []
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    for i, line in enumerate(lines, start=1):
        normalized = line.strip()
        if not normalized or normalized.startswith("#"):
            continue
        if "execute(" not in line:
            continue
        if "cursor.execute(" in line and "+" in line and SQL_STATEMENT.search(line):
            issues.append(f"{path.relative_to(ROOT)}:{i} possible SQL concatenation in execute()")
        if "f\"" in line or "f'" in line:
            if SQL_STATEMENT.search(line) and "{" in line and "}" in line:
                issues.append(f"{path.relative_to(ROOT)}:{i} possible f-string SQL interpolation")
        if SQL_STATEMENT.search(line) and ".format(" in line:
            issues.append(f"{path.relative_to(ROOT)}:{i} possible .format SQL interpolation")
        if SQL_STATEMENT.search(line) and "%" in line and "%%" not in line and "logging" not in line:
            issues.append(f"{path.relative_to(ROOT)}:{i} possible %-format SQL interpolation")
    return issues


def main() -> int:
    issues: list[str] = []
    for path in tracked_python_files():
        issues.extend(lint_file(path))

    if issues:
        print("FAIL: lint_sql")
        for issue in issues:
            print(f"- {issue}")
        return 1

    print("PASS: lint_sql")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
