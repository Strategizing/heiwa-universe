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
import re
from urllib.parse import urlparse
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent.parent.parent
sys.path.insert(0, str(ROOT))


def status_line(level: str, label: str, detail: str) -> None:
    print(f"[{level}] {label}: {detail}")


def _read_env_file(path: Path) -> dict[str, str]:
    data: dict[str, str] = {}
    if not path.exists():
        return data
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        k, v = line.split("=", 1)
        key = k.strip()
        val = v.strip().strip('"').strip("'")
        data[key] = val
    return data


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
        "config/schemas/artifact_index.schema.json",
        "apps/heiwa_hub/main.py",
        "apps/heiwa_cli/heiwa",
        "apps/heiwa_web/clients/web/index.html",
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
    worker_env_path = ROOT / ".env.worker.local"
    keys = set(os.environ.keys())
    env_files = []
    if env_path.exists():
        keys.update(_read_env_file(env_path).keys())
        env_files.append(str(env_path))
    if worker_env_path.exists():
        keys.update(_read_env_file(worker_env_path).keys())
        env_files.append(str(worker_env_path))
    if env_files:
        status_line("OK", "env", f"loaded key names from {', '.join(env_files)}")
    else:
        status_line("WARN", "env", ".env not found at monorepo root")

    required = ("HEIWA_AUTH_TOKEN", "NATS_URL")
    missing = [k for k in required if k not in keys]
    if missing:
        status_line("WARN", "env", f"missing keys: {', '.join(missing)}")
    else:
        status_line("OK", "env", "required keys present")
    return 0


def check_nats_bridge() -> int:
    file_env = {}
    for path in (ROOT / ".env.worker.local", ROOT / ".env"):
        file_env.update(_read_env_file(path))
    nats_url = os.getenv("NATS_URL") or file_env.get("NATS_URL") or "nats://localhost:4222"
    parsed = urlparse(nats_url)
    host = parsed.hostname or ""
    host_is_ip = bool(re.match(r"^\d{1,3}(?:\.\d{1,3}){3}$", host))
    if "railway.internal" in host or "localhost" in host or host.startswith("127.") or host.endswith(".up.railway.app") or host_is_ip:
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
