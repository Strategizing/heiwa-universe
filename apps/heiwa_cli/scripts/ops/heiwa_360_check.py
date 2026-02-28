#!/usr/bin/env python3
"""
Heiwa 360 Readiness Check (Enterprise v1.0)

Validates:
1) Monorepo structure and critical files
2) Worker executors and wrappers
3) Local service readiness (Ollama/NATS/OpenClaw gateway)
4) Railway & Edge health (DNS resolution and HTTP pulse)
5) Identity and Vault integrity
"""

from __future__ import annotations

import os
import socket
import subprocess
import sys
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import urlopen


def find_monorepo_root(start_path: Path) -> Path:
    current = start_path.resolve()
    for _ in range(5):
        if (current / "apps").exists() and (current / "packages").exists():
            return current
        current = current.parent
    return Path("/Users/dmcgregsauce/heiwa")


ROOT = find_monorepo_root(Path(__file__).resolve().parent)


def say(level: str, label: str, detail: str) -> None:
    print(f"[{level}] {label}: {detail}")


def cmd_ok(name: str) -> bool:
    import shutil
    return shutil.which(name) is not None


def tcp_open(host: str, port: int, timeout: float = 2.0) -> bool:
    try:
        with socket.create_connection((host, port), timeout=timeout):
            return True
    except Exception:
        return False


def http_health(url: str, timeout: int = 5) -> tuple[bool, str]:
    try:
        # Use HEAD if possible, fallback to GET
        with urlopen(url, timeout=timeout) as resp:
            code = getattr(resp, "status", 200)
            return (200 <= code < 300, f"HTTP {code}")
    except HTTPError as exc:
        if exc.code in {401, 403}:
            return (True, f"HTTP {exc.code} (protected)")
        return (False, f"HTTP {exc.code}")
    except Exception as exc:
        return (False, str(exc))


def main() -> int:
    print("--- HEIWA 360 CHECK ---")
    fails = 0
    warns = 0

    # 1. Environment & Vault
    vault_path = Path.home() / ".heiwa" / "vault.env"
    if vault_path.exists():
        say("OK", "vault", "~/.heiwa/vault.env present")
    else:
        say("WARN", "vault", "modular vault missing (run setup_wsl_node.sh or manual setup)")
        warns += 1

    env_files = [".env.worker.local", ".env.worker", ".env"]
    found_env = False
    for ef in env_files:
        if (ROOT / ef).exists():
            say("OK", "file", ef)
            found_env = True
            break
    if not found_env:
        say("FAIL", "file", "missing base .env")
        fails += 1

    # 2. Structural Integrity
    remaining_critical = [
        "railway.toml",
        "config/identities/profiles.json",
        "config/swarm/swarm.json",
        "apps/heiwa_hub/main.py",
        "apps/heiwa_cli/heiwa",
        "apps/heiwa_cli/scripts/agents/worker_manager.py",
        "apps/heiwa_cli/scripts/agents/wrappers/openclaw_exec.sh",
        "packages/heiwa_sdk/heiwa_sdk/db.py",
    ]
    for rel in remaining_critical:
        p = ROOT / rel
        if p.exists():
            say("OK", "file", rel)
        else:
            say("FAIL", "file", f"missing {rel}")
            fails += 1

    # 3. Dependencies & Commands
    for cmd in ("openclaw", "ollama", "railway", "wrangler", "terraform"):
        if cmd_ok(cmd):
            say("OK", "command", cmd)
        else:
            say("FAIL", "command", f"{cmd} missing")
            fails += 1

    # 4. Local Services
    if tcp_open("127.0.0.1", 11434):
        say("OK", "service", "ollama listening on 127.0.0.1:11434")
    else:
        say("WARN", "service", "ollama not listening")
        warns += 1

    if tcp_open("127.0.0.1", 4222):
        say("OK", "service", "nats listening on 127.0.0.1:4222")
    else:
        say("WARN", "service", "local nats not listening (remote mesh may be active)")
        warns += 1

    # 5. Cloud & Edge Health
    health_urls = [
        "https://heiwa.ltd",
        "https://status.heiwa.ltd",
        "https://api.heiwa.ltd/health",
        "https://auth.heiwa.ltd/health",
        "https://heiwa-cloud-hq-brain.up.railway.app/health",
    ]
    for url in health_urls:
        ok, detail = http_health(url)
        if ok:
            say("OK", "edge", f"{url} -> {detail}")
        else:
            say("WARN", "edge", f"{url} -> {detail}")
            warns += 1

    print(f"\nSummary: fails={fails} warns={warns}")
    if fails:
        print("Result: NOT READY (fix FAIL items first)")
        return 1
    if warns:
        print("Result: PARTIALLY READY (address WARN items for production stability)")
        return 0
    print("Result: READY")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
