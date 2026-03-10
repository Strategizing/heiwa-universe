#!/usr/bin/env python3
# cli/scripts/agents/wrappers/ollama_exec.py
"""
Isolated wrapper for Ollama local inference.
- Reads config from agents.yaml
- Full I/O logging
- Timeout + retry logic
- Never called directly by agents
"""
from __future__ import annotations

import json
import os
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Dict, Optional

import requests
import yaml


def load_config() -> Dict:
    root = Path(os.environ.get("HEIWA_WORKSPACE_ROOT", ".")).resolve()
    cfg_path = root / "config" / "agents.yaml"
    return yaml.safe_load(cfg_path.read_text(encoding="utf-8"))


def setup_logging(root: Path) -> tuple[Path, str]:
    log_dir = root / "runtime" / "logs" / "ollama"
    log_dir.mkdir(parents=True, exist_ok=True)
    run_id = f"{datetime.now(timezone.utc).strftime('%Y%m%d_%H%M%S')}_{os.getpid()}"
    return log_dir, run_id


def log_event(log_file: Path, event: Dict) -> None:
    with log_file.open("a", encoding="utf-8") as f:
        f.write(json.dumps(event, default=str) + "\n")


def call_ollama(
    base_url: str,
    model: str,
    prompt: str,
    timeout: int,
    temperature: float = 0.2,
    num_ctx: int = 8192,
) -> tuple[str, Optional[str]]:
    """Call Ollama API. Returns (response_text, error_or_none)."""
    url = f"{base_url.rstrip('/')}/api/generate"
    payload = {
        "model": model,
        "prompt": prompt,
        "stream": False,
        "options": {
            "temperature": temperature,
            "num_ctx": num_ctx,
        },
    }
    try:
        resp = requests.post(url, json=payload, timeout=timeout)
        resp.raise_for_status()
        data = resp.json()
        return data.get("response", ""), None
    except requests.Timeout:
        return "", "TIMEOUT"
    except requests.RequestException as e:
        return "", str(e)


def main() -> int:
    root = Path(os.environ.get("HEIWA_WORKSPACE_ROOT", ".")).resolve()
    cfg = load_config()
    prov = cfg["providers"]["ollama"]
    budgets = cfg["budgets"]

    log_dir, run_id = setup_logging(root)
    log_file = log_dir / f"{run_id}.jsonl"
    payload_file = log_dir / f"{run_id}.payload.txt"

    # Read prompt from stdin or args
    if len(sys.argv) > 1:
        prompt = " ".join(sys.argv[1:])
    else:
        prompt = sys.stdin.read()

    # Budget check
    max_chars = int(budgets["max_prompt_chars"])
    if len(prompt) > max_chars:
        log_event(log_file, {"event": "REJECTED", "reason": "prompt_too_large", "chars": len(prompt)})
        print(f"[ERR] Prompt exceeds budget: {len(prompt)} > {max_chars}", file=sys.stderr)
        return 1

    # Save payload
    payload_file.write_text(prompt, encoding="utf-8")

    log_event(log_file, {
        "event": "START",
        "run_id": run_id,
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "model": prov["default_model"],
        "prompt_bytes": len(prompt),
    })

    base_url = prov["base_url"]
    model = os.environ.get("HEIWA_OLLAMA_MODEL") or prov["default_model"]
    fallback = prov.get("fallback_model")
    timeout = int(prov["timeout_seconds"])
    gen = prov.get("generation", {})
    temperature = float(gen.get("temperature", 0.2))
    num_ctx = int(gen.get("num_ctx", 8192))

    # Attempt primary model
    response, err = call_ollama(base_url, model, prompt, timeout, temperature, num_ctx)

    # Fallback on failure
    if err and fallback:
        log_event(log_file, {"event": "FALLBACK", "from": model, "to": fallback, "error": err})
        model = fallback
        response, err = call_ollama(base_url, model, prompt, timeout, temperature, num_ctx)

    if err:
        log_event(log_file, {"event": "ERROR", "model": model, "error": err})
        print(f"[ERR] Ollama failed: {err}", file=sys.stderr)
        return 2

    log_event(log_file, {
        "event": "SUCCESS",
        "model": model,
        "response_bytes": len(response),
    })

    # Output response
    print(response)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())