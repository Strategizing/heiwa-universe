import asyncio
import json
import sys
import os
import uuid
import time
from pathlib import Path

# Ensure project root is on sys.path
ROOT = Path(__file__).resolve().parents[2]
RUNTIME_ROOT = ROOT / "runtime"
if str(RUNTIME_ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
    sys.path.insert(0, str(ROOT / "packages"))
    sys.path.insert(0, str(ROOT / "apps"))

import nats
from heiwa_protocol.protocol import Subject, Payload

async def send_command(instruction: str, node_id: str):
    nats_url = os.getenv("NATS_URL", "nats://localhost:4222")
    try:
        nc = await nats.connect(nats_url)
    except Exception as e:
        print(f"❌ Connection failed: {e}")
        return

    task_id = f"cli-task-{uuid.uuid4().hex[:8]}"
    
    # Wrap in Protocol
    payload = {
        Payload.SENDER_ID: node_id,
        Payload.TIMESTAMP: time.time(),
        Payload.TYPE: Subject.CORE_REQUEST.name,
        Payload.DATA: {
            "task_id": task_id,
            "raw_text": instruction,
            "source": "cli-dispatch",
            "intent_class": "general",
            "target_runtime": "any"
        }
    }
    
    # Send
    await nc.publish(Subject.CORE_REQUEST.value, json.dumps(payload).encode())
    print(f"🚀 [HEIWA] Sent Task: {task_id}")
    print(f"📝 Instruction: {instruction}")
    
    await nc.drain()
    await nc.close()

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: send_task.py <instruction> [node_id]")
        sys.exit(1)
    
    instruction = sys.argv[1]
    node_id = sys.argv[2] if len(sys.argv) > 2 else "cli-user"
    
    asyncio.run(send_command(instruction, node_id))