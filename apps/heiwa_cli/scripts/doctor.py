#!/usr/bin/env python3
"""
Heiwa operator doctor.

Runs fast checks for the current HTTP/WebSocket hub ingress, local operator
environment, and streaming prerequisites.
"""

from __future__ import annotations

import json
import os
import shutil
import sys
from pathlib import Path
from urllib import error as urllib_error
from urllib import request as urllib_request


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
        key, value = line.split("=", 1)
        data[key.strip()] = value.strip().strip('"').strip("'")
    return data


def _load_env_keys() -> tuple[dict[str, str], list[str]]:
    env_path = ROOT / ".env"
    worker_env_path = ROOT / ".env.worker.local"
    file_env: dict[str, str] = {}
    loaded: list[str] = []
    if env_path.exists():
        file_env.update(_read_env_file(env_path))
        loaded.append(str(env_path))
    if worker_env_path.exists():
        file_env.update(_read_env_file(worker_env_path))
        loaded.append(str(worker_env_path))
    return file_env, loaded


def _hub_url(file_env: dict[str, str]) -> str:
    return (
        os.getenv("HEIWA_HUB_URL")
        or file_env.get("HEIWA_HUB_URL")
        or os.getenv("HEIWA_HUB_BASE_URL")
        or file_env.get("HEIWA_HUB_BASE_URL")
        or "https://api.heiwa.ltd"
    )


def _auth_token(file_env: dict[str, str]) -> str:
    return os.getenv("HEIWA_AUTH_TOKEN") or file_env.get("HEIWA_AUTH_TOKEN") or ""


def _fetch_json(url: str, headers: dict[str, str] | None = None, timeout: float = 5.0) -> tuple[dict | None, str | None]:
    req_headers = {
        "Accept": "application/json",
        "User-Agent": "HeiwaCLI/2.1",
    }
    req_headers.update(headers or {})
    req = urllib_request.Request(url, headers=req_headers, method="GET")
    try:
        with urllib_request.urlopen(req, timeout=timeout) as resp:
            return json.loads(resp.read().decode()), None
    except urllib_error.HTTPError as exc:
        detail = exc.read().decode(errors="ignore").strip()
        return None, f"HTTP {exc.code}{(': ' + detail[:120]) if detail else ''}"
    except Exception as exc:
        return None, str(exc)


def check_python() -> int:
    status_line("OK", "python", sys.version.split()[0])
    return 0


def check_commands() -> int:
    for cmd in ("git", "railway", "ollama", "openclaw", "claude", "codex", "gemini"):
        if shutil.which(cmd):
            status_line("OK", "command", cmd)
        else:
            status_line("WARN", "command", f"{cmd} not found in PATH")
    return 0


def check_files() -> int:
    required = [
        "apps/heiwa_cli/heiwa",
        "apps/heiwa_cli/scripts/dispatch_once.py",
        "apps/heiwa_cli/scripts/terminal_chat.py",
        "apps/heiwa_hub/mcp_server.py",
    ]
    failures = 0
    for rel in required:
        path = ROOT / rel
        if path.exists():
            status_line("OK", "file", rel)
        else:
            failures += 1
            status_line("FAIL", "file", f"missing {rel}")
    return failures


def check_env(file_env: dict[str, str], env_files: list[str]) -> int:
    if env_files:
        status_line("OK", "env", f"loaded key names from {', '.join(env_files)}")
    else:
        status_line("WARN", "env", "no local env files found; relying on process env only")

    token = _auth_token(file_env)
    hub_url = _hub_url(file_env)

    failures = 0
    if token:
        status_line("OK", "auth", "HEIWA_AUTH_TOKEN present")
    else:
        failures += 1
        status_line("FAIL", "auth", "HEIWA_AUTH_TOKEN missing")

    status_line("OK", "hub", hub_url)
    return failures


def check_python_modules() -> int:
    failures = 0
    for mod in ("httpx", "websockets", "rich", "prompt_toolkit"):
        try:
            __import__(mod)
            status_line("OK", "python-module", mod)
        except Exception:
            level = "FAIL" if mod in {"prompt_toolkit"} else "WARN"
            if level == "FAIL":
                failures += 1
            status_line(level, "python-module", f"{mod} unavailable")
    return failures


def check_hub_health(file_env: dict[str, str]) -> int:
    data, err = _fetch_json(f"{_hub_url(file_env)}/health", timeout=8.0)
    if not data:
        status_line("FAIL", "hub-health", err or "unreachable")
        return 1
    detail = (
        f"status={data.get('status')} "
        f"backend={data.get('state_backend', 'unknown')} "
        f"transport={data.get('gateway_transport', 'unknown')}"
    )
    status_line("OK", "hub-health", detail)
    return 0


def check_route_probe(file_env: dict[str, str]) -> int:
    token = _auth_token(file_env)
    headers = {
        "Content-Type": "application/json",
        "Authorization": f"Bearer {token}",
        "Accept": "application/json",
        "User-Agent": "HeiwaCLI/2.1",
    }
    body = json.dumps(
        {
            "raw_text": "doctor route probe",
            "sender_id": "doctor",
            "source_surface": "cli",
        }
    ).encode()
    req = urllib_request.Request(f"{_hub_url(file_env)}/tasks", data=body, headers=headers, method="POST")
    try:
        with urllib_request.urlopen(req, timeout=8.0) as resp:
            payload = json.loads(resp.read().decode())
    except urllib_error.HTTPError as exc:
        detail = exc.read().decode(errors="ignore").strip()
        status_line("FAIL", "route-probe", f"HTTP {exc.code}{(': ' + detail[:120]) if detail else ''}")
        return 1
    except Exception as exc:
        status_line("FAIL", "route-probe", str(exc))
        return 1

    route = payload.get("route", {})
    detail = f"intent={route.get('intent_class', 'unknown')} tool={route.get('target_tool', 'unknown')}"
    status_line("OK", "route-probe", detail)
    return 0


def main() -> int:
    print("--- HEIWA OPERATOR DOCTOR ---")
    status_line("INFO", "root", str(ROOT))
    file_env, env_files = _load_env_keys()

    failures = sum(
        [
            check_python(),
            check_commands(),
            check_files(),
            check_env(file_env, env_files),
            check_python_modules(),
            check_hub_health(file_env),
            check_route_probe(file_env),
        ]
    )

    print("---")
    if failures:
        print(f"\n❌ Doctor found {failures} blocking issue(s).")
        return 1
    print("\n✅ Operator ingress looks healthy.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
