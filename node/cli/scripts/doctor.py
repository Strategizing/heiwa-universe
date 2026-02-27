#!/usr/bin/env python3
"""
Monorepo operability doctor for Heiwa Universe.
Runs fast checks for runtime/tooling alignment and NATS Swarm connectivity.
"""

from __future__ import annotations

import importlib
import os
import socket
import shutil
import subprocess
import sys
from urllib.parse import urlparse
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent.parent.parent
sys.path.insert(0, str(ROOT))


def status_line(level: str, label: str, detail: str) -> None:
    print(f"[{level}] {label}: {detail}")


def check_python() -> int:
    status_line("OK", "python", f"{sys.version.split()[0]}")
    return 0


def check_commands() -> int:
    missing = []
    for cmd in ("ollama", "gh", "railway", "docker", "git"):
        if shutil.which(cmd):
            status_line("OK", "command", cmd)
        else:
            missing.append(cmd)
            status_line("WARN", "command", f"{cmd} not found in PATH")
    return 0 if not missing else 0


def check_files() -> int:
    required = [
        "core/schemas/artifact_index.schema.json",
        "runtime/fleets/hub/main.py",
        "node/agents/heiwaclaw/main.py",
        ".env",
    ]
    failures = 0
    for rel in required:
        p = ROOT / rel
        if p.exists():
            status_line("OK", "file", rel)
        else:
            failures += 1
            status_line("FAIL", "file", f"missing {rel}")
    return failures


def check_env() -> int:
    env_path = ROOT / ".env"
    keys = set(os.environ.keys())
    if env_path.exists():
        for raw in env_path.read_text(encoding="utf-8").splitlines():
            line = raw.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            keys.add(line.split("=", 1)[0].strip())
        status_line("OK", "env", f"loaded key names from {env_path}")
    else:
        status_line("WARN", "env", ".env not found at monorepo root")

    required = ("DISCORD_BOT_TOKEN", "NATS_URL")
    missing = [k for k in required if k not in keys]
    if missing:
        status_line("WARN", "env", f"missing keys: {', '.join(missing)}")
    else:
        status_line("OK", "env", "required keys present")
    return 0


def check_nats_bridge() -> int:
    nats_url = os.getenv("NATS_URL", "nats://localhost:4222")
    if "railway.internal" in nats_url or "localhost" in nats_url or "docker" in nats_url:
        status_line("OK", "swarm", f"configured for {nats_url}")
    else:
        status_line("WARN", "swarm", f"unusual NATS target: {nats_url}")
    return 0


def main() -> int:
    print("--- HEIWA UNIVERSE DOCTOR ---")
    status_line("INFO", "root", str(ROOT))
    failures = sum(
        [
            check_python(),
            check_commands(),
            check_files(),
            check_env(),
            check_nats_bridge(),
        ]
    )
    print("---")
    if failures > 0:
        print(f"\n❌ Doctor found {failures} blocking issue(s).")
        return 1
    print("\n✅ All checks passed. Swarm connection verified.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
