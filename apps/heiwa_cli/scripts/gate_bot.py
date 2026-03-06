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
        ROOT / "apps/heiwa_hub/agents/messenger.py",
        ROOT / "apps/heiwa_hub/main.py",
        ROOT / "apps/heiwa_hub/actions/smoke_test_discord.py",
        ROOT / "packages/heiwa_sdk/heiwa_sdk/config.py",
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
    sources = {
        ROOT / "apps/heiwa_hub/agents/messenger.py": ("DISCORD_BOT_TOKEN", "DISCORD_TOKEN"),
        ROOT / "apps/heiwa_hub/main.py": ("HEIWA_ENABLE_MESSENGER", "DISCORD_BOT_TOKEN"),
        ROOT / "packages/heiwa_sdk/heiwa_sdk/config.py": ("DISCORD_BOT_TOKEN", "DISCORD_GUILD_ID"),
    }
    for path, keys in sources.items():
        if not path.exists():
            issues.append(f"missing file: {path.relative_to(ROOT)}")
            continue
        text = path.read_text(encoding="utf-8")
        for key in keys:
            if key not in text:
                issues.append(f"{path.relative_to(ROOT)} missing env reference: {key}")
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
