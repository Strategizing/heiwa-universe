#!/usr/bin/env python3
"""
CLI probe utility for the cli-platform-expert skill.

Purpose:
- Check whether common provider CLIs are installed
- Capture version information
- Optionally run non-interactive auth/context checks

The script avoids interactive login flows and uses short timeouts.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any


ANSI_RE = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")


def strip_ansi(text: str) -> str:
    return ANSI_RE.sub("", text or "")


def trim_text(text: str, limit: int = 800) -> str:
    text = strip_ansi(text).strip()
    if len(text) <= limit:
        return text
    return text[: limit - 15].rstrip() + "... [truncated]"


@dataclass(frozen=True)
class ProviderProfile:
    key: str
    aliases: tuple[str, ...]
    version_cmds: tuple[tuple[str, ...], ...]
    auth_cmds: tuple[tuple[str, ...], ...] = ()
    install_hint: str | None = None
    notes: str | None = None


PROFILES: dict[str, ProviderProfile] = {
    "gh": ProviderProfile(
        key="gh",
        aliases=("gh", "github", "github-cli"),
        version_cmds=(("gh", "--version"),),
        auth_cmds=(("gh", "auth", "status"),),
        install_hint="Install GitHub CLI (gh) and run `gh auth login` if needed.",
    ),
    "wrangler": ProviderProfile(
        key="wrangler",
        aliases=("wrangler", "cloudflare", "cloudflare-wrangler"),
        version_cmds=(("wrangler", "--version"), ("wrangler", "version")),
        auth_cmds=(("wrangler", "whoami"),),
        install_hint="Install Cloudflare Wrangler and run `wrangler login` if needed.",
        notes="Confirm exact subcommand syntax with `wrangler --help` because it changes across versions.",
    ),
    "railway": ProviderProfile(
        key="railway",
        aliases=("railway", "railway-cli"),
        version_cmds=(("railway", "--version"), ("railway", "version")),
        auth_cmds=(("railway", "whoami"),),
        install_hint="Install Railway CLI and run `railway login` if needed.",
    ),
    "openclaw": ProviderProfile(
        key="openclaw",
        aliases=("openclaw",),
        version_cmds=(("openclaw", "--version"), ("openclaw", "version")),
        auth_cmds=(),
        notes="Heiwa policy may block OpenClaw enablement; check local docs before operational changes.",
    ),
    "picoclaw": ProviderProfile(
        key="picoclaw",
        aliases=("picoclaw",),
        version_cmds=(("picoclaw", "--version"), ("picoclaw", "version")),
        auth_cmds=(),
        notes="PicoClaw is often embedded in role-specific worker flows rather than direct user auth flows.",
    ),
}


ALIAS_TO_KEY: dict[str, str] = {}
for _key, _profile in PROFILES.items():
    for _alias in _profile.aliases:
        ALIAS_TO_KEY[_alias] = _key


def resolve_provider(token: str) -> ProviderProfile | None:
    token = token.strip().lower()
    if not token:
        return None
    key = ALIAS_TO_KEY.get(token, token)
    return PROFILES.get(key)


def run_command(argv: tuple[str, ...], timeout: float) -> dict[str, Any]:
    env = os.environ.copy()
    env.setdefault("NO_COLOR", "1")
    env.setdefault("CLICOLOR", "0")
    env.setdefault("CI", "1")
    try:
        proc = subprocess.run(
            argv,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=timeout,
            env=env,
            check=False,
        )
        return {
            "ok": proc.returncode == 0,
            "exit_code": proc.returncode,
            "stdout": trim_text(proc.stdout),
            "stderr": trim_text(proc.stderr),
        }
    except FileNotFoundError:
        return {"ok": False, "exit_code": None, "stdout": "", "stderr": "command not found"}
    except subprocess.TimeoutExpired as exc:
        out = exc.stdout if isinstance(exc.stdout, str) else ""
        err = exc.stderr if isinstance(exc.stderr, str) else ""
        return {
            "ok": False,
            "exit_code": None,
            "stdout": trim_text(out),
            "stderr": trim_text(err) or f"timed out after {timeout}s",
            "timed_out": True,
        }
    except Exception as exc:  # pragma: no cover - defensive
        return {"ok": False, "exit_code": None, "stdout": "", "stderr": str(exc)}


def first_success(profile: ProviderProfile, cmds: tuple[tuple[str, ...], ...], timeout: float) -> dict[str, Any] | None:
    last_result: dict[str, Any] | None = None
    for cmd in cmds:
        result = run_command(cmd, timeout=timeout)
        result["command"] = list(cmd)
        last_result = result
        if result.get("ok"):
            return result
        if result.get("stdout"):
            return result
    return last_result


def probe_profile(profile: ProviderProfile, check_auth: bool, timeout: float) -> dict[str, Any]:
    executable = profile.aliases[0]
    path = None
    for alias in profile.aliases:
        candidate = shutil.which(alias)
        if candidate:
            executable = alias
            path = candidate
            break

    result: dict[str, Any] = {
        "provider": profile.key,
        "aliases": list(profile.aliases),
        "executable": executable,
        "found": bool(path),
        "path": path,
    }
    if profile.install_hint:
        result["install_hint"] = profile.install_hint
    if profile.notes:
        result["notes"] = profile.notes

    if not path:
        return result

    version_data = first_success(profile, profile.version_cmds, timeout)
    result["version_check"] = version_data

    if check_auth and profile.auth_cmds:
        auth_data = first_success(profile, profile.auth_cmds, timeout)
        result["auth_check"] = auth_data

    return result


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Probe provider CLIs for availability, version, and optional auth state.")
    parser.add_argument(
        "providers",
        nargs="*",
        help="Provider/tool names (examples: gh, wrangler, railway, openclaw, picoclaw). Defaults to common set.",
    )
    parser.add_argument(
        "--check-auth",
        action="store_true",
        help="Run non-interactive auth/status commands where configured.",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=4.0,
        help="Per-command timeout in seconds (default: 4.0).",
    )
    parser.add_argument(
        "--json-indent",
        type=int,
        default=2,
        help="JSON indent level (default: 2; use 0 for compact).",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()

    requested = args.providers or ["gh", "wrangler", "railway", "openclaw", "picoclaw"]
    profiles: list[ProviderProfile] = []
    unknown: list[str] = []
    seen: set[str] = set()

    for token in requested:
        profile = resolve_provider(token)
        if not profile:
            unknown.append(token)
            continue
        if profile.key in seen:
            continue
        seen.add(profile.key)
        profiles.append(profile)

    payload: dict[str, Any] = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "check_auth": bool(args.check_auth),
        "timeout_seconds": args.timeout,
        "results": [probe_profile(profile, check_auth=args.check_auth, timeout=args.timeout) for profile in profiles],
    }
    if unknown:
        payload["unknown_providers"] = unknown
        payload["known_providers"] = sorted(PROFILES.keys())

    indent = None if args.json_indent == 0 else args.json_indent
    json.dump(payload, sys.stdout, indent=indent, sort_keys=False)
    sys.stdout.write("\n")
    return 0 if not unknown else 1


if __name__ == "__main__":
    raise SystemExit(main())
