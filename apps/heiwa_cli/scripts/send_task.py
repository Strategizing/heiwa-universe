import asyncio
import json
import sys
import os
import uuid
import time
from pathlib import Path

# Ensure project root is on sys.path
ROOT = Path(__file__).resolve().parents[3]
if str(ROOT / "packages/heiwa_sdk") not in sys.path:
    sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
if str(ROOT / "packages/heiwa_protocol") not in sys.path:
    sys.path.insert(0, str(ROOT / "packages/heiwa_protocol"))
if str(ROOT / "packages/heiwa_identity") not in sys.path:
    sys.path.insert(0, str(ROOT / "packages/heiwa_identity"))
if str(ROOT / "apps") not in sys.path:
    sys.path.insert(0, str(ROOT / "apps"))

import httpx
from heiwa_sdk.config import settings

async def send_command(instruction: str, node_id: str):
    task_id = f"cli-task-{uuid.uuid4().hex[:8]}"
    token = os.getenv("HEIWA_AUTH_TOKEN")
    if not token:
        token = getattr(settings, "HEIWA_AUTH_TOKEN", "")
    hub_url = str(settings.HUB_BASE_URL or "https://api.heiwa.ltd").rstrip("/")

    headers = {"Content-Type": "application/json", "Accept": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"

    payload = {
        "task_id": task_id,
        "prompt": instruction,
        "source": "cli-dispatch",
        "requested_by": node_id,
    }
    async with httpx.AsyncClient(timeout=20.0) as client:
        response = await client.post(f"{hub_url}/tasks", json=payload, headers=headers)
        response.raise_for_status()
        accepted = response.json()

    print(f"🚀 [HEIWA] Sent Task: {accepted.get('task_id', task_id)}")
    print(f"📝 Instruction: {instruction}")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: send_task.py <instruction> [node_id]")
        sys.exit(1)
    
    instruction = sys.argv[1]
    node_id = sys.argv[2] if len(sys.argv) > 2 else "cli-user"
    
    asyncio.run(send_command(instruction, node_id))
