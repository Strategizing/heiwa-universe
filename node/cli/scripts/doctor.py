#!/usr/bin/env python3
"""
Local operability doctor for Heiwa.
Runs fast checks for runtime/tooling alignment on a workstation node.
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


ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))


def status_line(level: str, label: str, detail: str) -> None:
    print(f"[{level}] {label}: {detail}")


def check_python() -> int:
    major, minor = sys.version_info[:2]
    if (major, minor) >= (3, 14):
        status_line(
            "FAIL",
            "python",
            "Python 3.14+ detected; pinned dependencies fail on 3.14. Use Python 3.13 for local venv.",
        )
        return 1
    status_line("OK", "python", f"{sys.version.split()[0]}")
    return 0


def check_commands() -> int:
    missing = []
    for cmd in ("codex", "ollama", "openclaw", "gh", "railway", "docker", "tailscale"):
        if shutil.which(cmd):
            status_line("OK", "command", cmd)
        else:
            missing.append(cmd)
            status_line("WARN", "command", f"{cmd} not found in PATH")
    return 0 if not missing else 0


def check_files() -> int:
    required = [
        "config/agents.yaml",
        "fleets/hub/main.py",
        "cli/scripts/agents/sentinel.py",
        "cli/scripts/verify_deployment.py",
        "requirements.txt",
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
        status_line("WARN", "env", ".env not found (using process environment only)")

    required = ("DATABASE_URL", "DISCORD_BOT_TOKEN", "DISCORD_GUILD_ID")
    missing = [k for k in required if k not in keys]
    if missing:
        status_line("WARN", "env", f"missing keys: {', '.join(missing)}")
    else:
        status_line("OK", "env", "required keys present")
    return 0


def check_local_model_mode() -> int:
    mode = os.getenv("HEIWA_LLM_MODE", "local_only").strip().lower()
    if mode != "local_only":
        status_line("WARN", "llm_mode", f"expected local_only, found {mode}")
        return 0
    status_line("OK", "llm_mode", "local_only")
    return 0


def check_imports() -> int:
    modules = (
        "requests",
        "yaml",
        "psycopg2",
        "nats",
        "fleets.hub.main",
        "scripts.agents.sentinel",
    )
    failures = 0
    for mod in modules:
        try:
            importlib.import_module(mod)
            status_line("OK", "import", mod)
        except Exception as exc:
            failures += 1
            status_line("FAIL", "import", f"{mod} ({type(exc).__name__}: {exc})")
    return failures


def _tcp_reachable(host: str, port: int, timeout: float = 3.0) -> bool:
    try:
        with socket.create_connection((host, port), timeout=timeout):
            return True
    except Exception:
        return False


def check_network_targets() -> int:
    failures = 0

    database_url = os.getenv("DATABASE_URL")
    if database_url:
        parsed = urlparse(database_url)
        host = parsed.hostname
        port = parsed.port or 5432
        if host and _tcp_reachable(host, port):
            status_line("OK", "network", f"DATABASE_URL reachable at {host}:{port}")
        else:
            failures += 1
            status_line("FAIL", "network", "DATABASE_URL host/port unreachable")
    else:
        status_line("WARN", "network", "DATABASE_URL not set")

    nats_url = os.getenv("NATS_URL")
    if nats_url:
        # Support common comma-separated NATS server lists.
        first = nats_url.split(",")[0].strip()
        parsed = urlparse(first)
        host = parsed.hostname
        port = parsed.port or 4222
        if host and _tcp_reachable(host, port):
            status_line("OK", "network", f"NATS_URL reachable at {host}:{port}")
        else:
            failures += 1
            status_line("FAIL", "network", "NATS_URL host/port unreachable")
    else:
        status_line("WARN", "network", "NATS_URL not set")

    return failures


def check_codex_flag() -> int:
    codex = shutil.which("codex")
    if not codex:
        return 0
    try:
        result = subprocess.run(
            [codex, "--help"],
            capture_output=True,
            text=True,
            check=False,
        )
        if "--full-auto" in result.stdout:
            status_line("OK", "codex-cli", "supports --full-auto")
            return 0
        status_line("WARN", "codex-cli", "unable to confirm --full-auto support")
        return 0
    except Exception as exc:
        status_line("WARN", "codex-cli", f"help check failed: {exc}")
        return 0


def main() -> int:
    print("--- HEIWA DOCTOR ---")
    failures = 0
    failures += check_python()
    failures += check_commands()
    failures += check_files()
    failures += check_env()
    failures += check_local_model_mode()
    failures += check_imports()
    failures += check_network_targets()
    failures += check_codex_flag()
    if failures:
        print(f"\n❌ Doctor found {failures} blocking issue(s).")
        return 1
    print("\n✅ Doctor checks passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
