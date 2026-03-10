import asyncio
import json
import os
import sys
import uuid
import time
from pathlib import Path
from dotenv import load_dotenv

# Setup paths
ROOT = Path('/Users/dmcgregsauce/heiwa')
load_dotenv(ROOT / '.env')

sys.path.insert(0, str(ROOT / 'packages/heiwa_sdk'))
sys.path.insert(0, str(ROOT / 'packages/heiwa_protocol'))
sys.path.insert(0, str(ROOT / 'packages/heiwa_identity'))
sys.path.insert(0, str(ROOT))

import nats
from heiwa_protocol.protocol import Subject, Payload

async def sota_verify(instruction: str):
    nats_url = os.getenv("NATS_URL", "nats://localhost:4222")
    try:
        nc = await nats.connect(nats_url)
        print(f"🔗 Connected to NATS Swarm: {nats_url}")
    except Exception as e:
        print(f"❌ Connection failed: {e}")
        return

    task_id = f"sota-task-{uuid.uuid4().hex[:6]}"
    
    async def message_handler(msg):
        data = json.loads(msg.data.decode())
        inner = data.get("data", {})
        
        if msg.subject == Subject.LOG_THOUGHT.value:
            if inner.get("task_id") == task_id:
                agent = inner.get("agent", "unknown")
                content = inner.get("content", "")
                print(f"[🧠 {agent}]: {content}", end="", flush=True)
                if not inner.get("stream"):
                    print()
        
        elif msg.subject == Subject.TASK_EXEC_RESULT.value:
            if inner.get("task_id") == task_id:
                print("\n\n✅ [RESULT received]")
                print("-" * 40)
                print(inner.get("summary", "No summary provided."))
                print("-" * 40)
                stop_event.set()

    stop_event = asyncio.Event()
    await nc.subscribe(Subject.LOG_THOUGHT.value, cb=message_handler)
    await nc.subscribe(Subject.TASK_EXEC_RESULT.value, cb=message_handler)

    token = os.getenv("HEIWA_AUTH_TOKEN")
    if not token:
        from heiwa_sdk.config import settings
        token = getattr(settings, "HEIWA_AUTH_TOKEN", "")

    payload = {
        Payload.SENDER_ID: "devon-operator",
        Payload.TIMESTAMP: time.time(),
        Payload.TYPE: Subject.CORE_REQUEST.name,
        "auth_token": token,
        Payload.DATA: {
            "task_id": task_id,
            "raw_text": instruction,
            "source": "sota-verification",
            "intent_class": "operate",
            "target_runtime": "any",
            "sender_id": "devon-operator"
        }
    }
    
    await nc.publish(Subject.CORE_REQUEST.value, json.dumps(payload).encode())
    print(f"🚀 [DEVON] Prompting Heiwa: {instruction}")
    
    try:
        await asyncio.wait_for(stop_event.wait(), timeout=60)
    except asyncio.TimeoutError:
        print("\n\n⚠️ Timeout waiting for Heiwa result.")

    await nc.drain()
    await nc.close()

if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(1)
    asyncio.run(sota_verify(sys.argv[1]))
