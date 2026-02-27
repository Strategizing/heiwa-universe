#!/usr/bin/env python3
"""
Heiwa 360 Readiness Check

Validates:
1) Source/deploy boundary files
2) Worker executors and wrappers
3) Local service readiness (Ollama/NATS/OpenClaw gateway)
4) Railway control-plane health
5) Env drift signals that break local-vs-cloud separation
"""

from __future__ import annotations

import os
import socket
import subprocess
import sys
from pathlib import Path
from urllib.request import urlopen


ROOT = Path(__file__).resolve().parents[2]


def say(level: str, label: str, detail: str) -> None:
    print(f"[{level}] {label}: {detail}")


def cmd_ok(name: str) -> bool:
    return subprocess.call(["/usr/bin/env", "bash", "-lc", f"command -v {name} >/dev/null"]) == 0


def tcp_open(host: str, port: int, timeout: float = 2.0) -> bool:
    try:
        with socket.create_connection((host, port), timeout=timeout):
            return True
    except Exception:
        return False


def http_health(url: str, timeout: int = 5) -> tuple[bool, str]:
    try:
        with urlopen(url, timeout=timeout) as resp:  # nosec B310
            code = getattr(resp, "status", 200)
            return (200 <= code < 300, f"HTTP {code}")
    except Exception as exc:  # noqa: BLE001
        return (False, str(exc))


def read_env_file(path: Path) -> dict[str, str]:
    out: dict[str, str] = {}
    if not path.exists():
        return out
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        k, v = line.split("=", 1)
        out[k.strip()] = v.strip().strip('"').strip("'")
    return out


def main() -> int:
    print("--- HEIWA 360 CHECK ---")
    fails = 0
    warns = 0

    critical_files = [
        ".railwayignore",
        ".dockerignore",
        "railway.toml",
        "cli/scripts/agents/worker_manager.py",
        "cli/scripts/agents/wrappers/codex_exec.sh",
        "cli/scripts/agents/wrappers/openclaw_exec.sh",
        "cli/scripts/agents/wrappers/picoclaw_exec.py",
        "cli/scripts/agents/wrappers/ollama_exec.py",
        "config/env/.env.railway.example",
        "config/env/.env.worker.mac.example",
        "config/env/.env.worker.pc.example",
    ]
    for rel in critical_files:
        p = ROOT / rel
        if p.exists():
            say("OK", "file", rel)
        else:
            say("FAIL", "file", f"missing {rel}")
            fails += 1

    for cmd in ("codex", "openclaw", "ollama", "railway"):
        if cmd_ok(cmd):
            say("OK", "command", cmd)
        else:
            say("FAIL", "command", f"{cmd} missing")
            fails += 1

    for cmd in ("docker", "nats-server", "picoclaw"):
        if cmd_ok(cmd):
            say("OK", "command", cmd)
        else:
            say("WARN", "command", f"{cmd} missing (optional depending on topology)")
            warns += 1

    if tcp_open("127.0.0.1", 11434):
        say("OK", "service", "ollama listening on 127.0.0.1:11434")
    else:
        say("WARN", "service", "ollama not listening on 127.0.0.1:11434")
        warns += 1

    if tcp_open("127.0.0.1", 4222):
        say("OK", "service", "nats listening on 127.0.0.1:4222")
    else:
        say("WARN", "service", "nats not listening on 127.0.0.1:4222")
        warns += 1

    # OpenClaw gateway can still be bypassed with --local mode.
    gateway_rc = subprocess.call(
        [
            "/usr/bin/env",
            "bash",
            "-lc",
            "openclaw status 2>/dev/null | grep -q 'Gateway.*reachable'",
        ]
    )
    if gateway_rc == 0:
        say("OK", "service", "openclaw gateway reachable")
    else:
        say("WARN", "service", "openclaw gateway not reachable (wrapper may fallback to --local)")
        warns += 1

    env_file = ROOT / ".env.worker"
    if not env_file.exists():
        env_file = ROOT / ".env"
    env = read_env_file(env_file)
    if env:
        nats_url = env.get("NATS_URL", "")
        db_url = env.get("DATABASE_URL", "")
        if "railway.internal" in nats_url:
            say("WARN", "env", "NATS_URL uses railway.internal; local workers usually cannot reach this directly")
            warns += 1
        if "${{" in db_url:
            say("WARN", "env", "DATABASE_URL contains unresolved template placeholders")
            warns += 1
    else:
        say("WARN", "env", f"{env_file.name} not found or contains no parseable values")
        warns += 1

    # Railway control-plane health check (best-effort).
    health_urls = [
        "https://heiwa-cloud-hq-brain.up.railway.app/health",
        "https://hub-api-brain.up.railway.app/health",
    ]
    for url in health_urls:
        ok, detail = http_health(url)
        if ok:
            say("OK", "railway", f"{url} -> {detail}")
        else:
            say("WARN", "railway", f"{url} -> {detail}")
            warns += 1

    print(f"\nSummary: fails={fails} warns={warns}")
    if fails:
        print("Result: NOT READY (fix FAIL items first)")
        return 1
    if warns:
        print("Result: PARTIALLY READY (address WARN items for persistent production)")
        return 0
    print("Result: READY")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
