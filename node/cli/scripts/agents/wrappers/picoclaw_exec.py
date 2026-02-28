#!/usr/bin/env python3
"""
cli/scripts/agents/wrappers/picoclaw_exec.py

PicoClaw wrapper for worker_manager.
Input can be:
1) Plain text: interpreted as a query for default command.
2) JSON object:
   {
     "command": "search",
     "args": ["heiwa architecture"]
   }
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path


def _read_payload() -> str:
    if len(sys.argv) > 1:
        return " ".join(sys.argv[1:]).strip()
    return sys.stdin.read().strip()


def _log_dir(root: Path) -> tuple[Path, str]:
    out = root / "runtime" / "logs" / "picoclaw"
    out.mkdir(parents=True, exist_ok=True)
    run_id = f"{datetime.now(timezone.utc).strftime('%Y%m%d_%H%M%S')}_{os.getpid()}"
    return out, run_id


def _parse_command(payload: str) -> tuple[str, list[str]]:
    default_cmd = os.getenv("PICOCLAW_DEFAULT_COMMAND", "search").strip() or "search"
    if not payload:
        return default_cmd, []

    try:
        data = json.loads(payload)
    except json.JSONDecodeError:
        return default_cmd, [payload]

    if not isinstance(data, dict):
        return default_cmd, [payload]

    command = str(data.get("command", default_cmd)).strip() or default_cmd
    args = data.get("args", [])
    if not isinstance(args, list):
        args = [str(args)]
    return command, [str(item) for item in args]


def main() -> int:
    root = Path(os.environ.get("HEIWA_WORKSPACE_ROOT", ".")).resolve()
    log_dir, run_id = _log_dir(root)
    payload = _read_payload()

    payload_file = log_dir / f"{run_id}.payload.txt"
    payload_file.write_text(payload, encoding="utf-8")
    log_file = log_dir / f"{run_id}.jsonl"

    binary = os.getenv("PICOCLAW_BIN", "picoclaw")
    resolved = shutil.which(binary) if "/" not in binary else binary
    if not resolved:
        err = f"picoclaw binary not found (PICOCLAW_BIN={binary})"
        log_file.write_text(json.dumps({"event": "ERROR", "error": err}) + "\n", encoding="utf-8")
        print(f"[ERR] {err}", file=sys.stderr)
        return 2

    command, args = _parse_command(payload)
    timeout_sec = int(os.getenv("PICOCLAW_TIMEOUT", "180"))
    
    cmd = [resolved, command]
    
    # Inject model override if present
    model_override = os.getenv("PICOCLAW_MODEL")
    if model_override:
        cmd.extend(["--model", model_override])
        
    cmd.extend(args)

    with log_file.open("a", encoding="utf-8") as f:
        f.write(json.dumps({"event": "START", "run_id": run_id, "cmd": cmd}) + "\n")

    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout_sec,
            check=False,
        )
    except subprocess.TimeoutExpired:
        with log_file.open("a", encoding="utf-8") as f:
            f.write(json.dumps({"event": "ERROR", "error": "TIMEOUT"}) + "\n")
        print("[ERR] picoclaw timed out", file=sys.stderr)
        return 3
    except Exception as exc:  # noqa: BLE001
        with log_file.open("a", encoding="utf-8") as f:
            f.write(json.dumps({"event": "ERROR", "error": str(exc)}) + "\n")
        print(f"[ERR] picoclaw execution failed: {exc}", file=sys.stderr)
        return 4

    with log_file.open("a", encoding="utf-8") as f:
        f.write(
            json.dumps(
                {
                    "event": "END",
                    "returncode": result.returncode,
                    "stdout_bytes": len(result.stdout or ""),
                    "stderr_bytes": len(result.stderr or ""),
                }
            )
            + "\n"
        )

    if result.stdout:
        print(result.stdout.strip())
    if result.stderr:
        print(result.stderr.strip(), file=sys.stderr)

    return int(result.returncode)


if __name__ == "__main__":
    raise SystemExit(main())
