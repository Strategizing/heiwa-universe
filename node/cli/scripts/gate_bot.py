#!/usr/bin/env python3
"""
Gate: Bot service static checks.
Ensures bot-related modules are syntactically valid and expected env hooks exist.
"""

from __future__ import annotations

import py_compile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]


def compile_targets() -> list[str]:
    errors: list[str] = []
    targets = [
        ROOT / "runtime/fleets/hub/discord_portal.py",
        ROOT / "runtime/fleets/hub/agents/messenger.py",
        ROOT / "runtime/fleets/hub/config.py",
    ]
    for path in targets:
        if not path.exists():
            errors.append(f"missing file: {path.relative_to(ROOT)}")
            continue
        try:
            py_compile.compile(str(path), doraise=True)
        except Exception as exc:
            errors.append(f"{path.relative_to(ROOT)}: {exc}")
    return errors


def check_env_hooks() -> list[str]:
    issues: list[str] = []
    config_path = ROOT / "runtime/fleets/hub/config.py"
    if not config_path.exists():
        return ["missing file: runtime/fleets/hub/config.py"]
    text = config_path.read_text(encoding="utf-8")
    for key in ("DISCORD_BOT_TOKEN", "DISCORD_GUILD_ID"):
        if key not in text:
            issues.append(f"runtime/fleets/hub/config.py missing env reference: {key}")
    return issues


def main() -> int:
    issues = []
    issues.extend(compile_targets())
    issues.extend(check_env_hooks())
    if issues:
        print("FAIL: gate_bot")
        for issue in issues:
            print(f"- {issue}")
        return 1
    print("PASS: gate_bot")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
