#!/usr/bin/env python3
# cli/scripts/agents/cli.py
"""
CLI Injector: Drop tasks into the queue to trigger the Sentinel.
Simulates Discord/Railway events for local smoke testing.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import time
import uuid
from pathlib import Path


def inject(payload: str, complexity: int = 1, source: str = "cli_injector") -> None:
    root = Path(os.environ.get("HEIWA_WORKSPACE_ROOT", ".")).resolve()
    queue_dir = root / "runtime" / "queue" / "pending"
    queue_dir.mkdir(parents=True, exist_ok=True)
    
    task_id = str(uuid.uuid4())[:8]
    data = {
        "id": task_id,
        "payload": payload,
        "source": source,
        "complexity": complexity,
        "created_at": time.time(),
    }
    
    path = queue_dir / f"task_{task_id}.json"
    path.write_text(json.dumps(data, indent=2), encoding="utf-8")
    
    print(f"✅ Task injected: {path}")
    print(f"🆔 ID: {task_id}")
    print(f"⚙️  Complexity: {complexity}")
    print(f"📋 Payload: {payload[:80]}...")
    print()
    print("Watch for Sentinel pickup in 'runtime/logs/sentinel.log'")


def main() -> int:
    parser = argparse.ArgumentParser(description="Inject tasks into the Sentinel queue")
    parser.add_argument("payload", help="Task payload/prompt")
    parser.add_argument("-c", "--complexity", type=int, default=1, choices=[1, 2, 3, 4, 5],
                        help="Complexity score (1-3=Ollama, 4-5=Codex)")
    parser.add_argument("-s", "--source", default="cli_injector",
                        help="Source identifier")
    
    args = parser.parse_args()
    inject(args.payload, args.complexity, args.source)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())