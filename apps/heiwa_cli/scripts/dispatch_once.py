import asyncio
import json
import os
import sys
import uuid
import time
from pathlib import Path

# Ensure project root is on sys.path
ROOT = Path(__file__).resolve().parents[3]
for p in ["packages/heiwa_sdk", "packages/heiwa_protocol", "packages/heiwa_identity", "apps"]:
    p_path = str(ROOT / p)
    if p_path not in sys.path:
        sys.path.insert(0, p_path)

import nats
from nats.errors import TimeoutError
from heiwa_protocol.protocol import Subject

async def dispatch_once(prompt: str, node_id: str):
    try:
        nc = await nats.connect(os.getenv("NATS_URL", "nats://localhost:4222"), connect_timeout=5)
    except Exception as e:
        print(f"❌ Connection failed: {e}")
        sys.exit(1)
        
    # Subscribe to TASK_STATUS to catch the ACK
    sub = await nc.subscribe(Subject.TASK_STATUS.value)

    token = os.getenv("HEIWA_AUTH_TOKEN")
    if not token:
        try:
            from heiwa_sdk.config import settings
            token = getattr(settings, "HEIWA_AUTH_TOKEN", "")
        except ImportError:
            token = ""

    task_id = f"cli-task-{uuid.uuid4().hex[:8]}"
    
    payload = {
        "sender_id": node_id,
        "timestamp": time.time(),
        "type": Subject.CORE_REQUEST.name,
        "auth_token": token,
        "data": {
            "task_id": task_id,
            "raw_text": prompt,
            "source": "cli-dispatch",
            "intent_class": "general",
            "target_runtime": "any",
            "sender_id": node_id
        }
    }
    
    await nc.publish(Subject.CORE_REQUEST.value, json.dumps(payload).encode())
    print(f"🚀 [HEIWA] Dispatched: '{prompt}' (Task: {task_id})")

    try:
        # Wait up to 5 seconds for Spine ACK
        while True:
            msg = await sub.next_msg(timeout=5.0)
            data = json.loads(msg.data.decode()).get("data", json.loads(msg.data.decode()))
            received_task_id = data.get("task_id")
            
            # Check if this status matches our task_id, or if it's a generic auth block
            if received_task_id == task_id or (data.get("status") == "BLOCKED_AUTH" and received_task_id == "unknown"):
                # Handle extended explicit ACK contract
                accepted = data.get("accepted", data.get("status") == "ACKNOWLEDGED")
                reason = data.get("reason", data.get("message", "No reason provided."))
                
                if accepted:
                    print(f"✅ Accepted by Spine. Task ID: {received_task_id}")
                    await nc.close()
                    sys.exit(0)
                else:
                    print(f"❌ Rejected by Spine. Reason: {reason}")
                    await nc.close()
                    sys.exit(1)
    except TimeoutError:
        print("❌ Network Error: Spine did not acknowledge within 5 seconds.")
        await nc.close()
        sys.exit(1)

if __name__ == "__main__":
    if len(sys.argv) < 3:
        print("Usage: dispatch_once.py <node_id> <prompt>")
        sys.exit(1)
    node_id = sys.argv[1]
    prompt = " ".join(sys.argv[2:])
        
    asyncio.run(dispatch_once(prompt, node_id))
