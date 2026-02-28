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
from urllib.error import HTTPError
from urllib.request import urlopen


ROOT = Path(__file__).resolve().parents[4]


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
        with urlopen(url, timeout=timeout) as resp:  # nosec B310
            code = getattr(resp, "status", 200)
            return (200 <= code < 300, f"HTTP {code}")
    except HTTPError as exc:
        # Some public endpoints are challenge-guarded but still up.
        if exc.code in {401, 403}:
            return (True, f"HTTP {exc.code} (protected)")
        return (False, f"HTTP {exc.code}")
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

    env_files = [".env.worker.local", ".env.worker", ".env"]
    found_env = False
    for ef in env_files:
        if (ROOT / ef).exists():
            say("OK", "file", ef)
            found_env = True
            break
    if not found_env:
        say(
            "FAIL",
            "file",
            "missing base .env (checked .env.worker.local, .env.worker, .env)",
        )
        fails += 1

    remaining_critical = [
        "railway.toml",
        "core/profiles/heiwa-one-system.yaml",
        "runtime/fleets/hub/main.py",
        "runtime/fleets/hub/config.py",
        "node/cli/scripts/agents/worker_manager.py",
        "node/cli/scripts/agents/wrappers/codex_exec.sh",
        "node/cli/scripts/agents/wrappers/openclaw_exec.sh",
        "node/cli/scripts/agents/wrappers/picoclaw_exec.py",
        "node/cli/scripts/agents/wrappers/ollama_exec.py",
    ]
    for rel in remaining_critical:
        p = ROOT / rel
        if p.exists():
            say("OK", "file", rel)
        else:
            say("FAIL", "file", f"missing {rel}")
            fails += 1

    for cmd in ("codex", "openclaw", "ollama", "railway", "wrangler"):
        if cmd_ok(cmd):
            say("OK", "command", cmd)
            if cmd == "wrangler":
                # Check authentication status
                wrangler_rc = subprocess.call(
                    ["/usr/bin/env", "bash", "-lc", "wrangler whoami >/dev/null 2>&1"]
                )
                if wrangler_rc == 0:
                    say("OK", "service", "wrangler is authenticated")
                else:
                    say(
                        "WARN",
                        "service",
                        "wrangler is NOT authenticated (run `wrangler login`)",
                    )
                    warns += 1
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
        say(
            "WARN",
            "service",
            "openclaw gateway not reachable (wrapper may fallback to --local)",
        )
        warns += 1

    env_file = ROOT / ".env.worker.local"
    if not env_file.exists():
        env_file = ROOT / ".env.worker"
    if not env_file.exists():
        env_file = ROOT / ".env"
    env = read_env_file(env_file)
    if env:
        nats_url = env.get("NATS_URL", "")
        db_url = env.get("DATABASE_URL", "")
        if "railway.internal" in nats_url:
            say(
                "WARN",
                "env",
                "NATS_URL uses railway.internal; local workers usually cannot reach this directly",
            )
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
        "https://api.heiwa.ltd/health",
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
