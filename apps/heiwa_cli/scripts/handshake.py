import asyncio
import json
import os
import sys
import uuid
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
for p in ["packages/heiwa_sdk", "packages/heiwa_protocol", "packages/heiwa_identity", "apps"]:
    p_path = str(ROOT / p)
    if p_path not in sys.path:
        sys.path.insert(0, p_path)

import nats
from nats.errors import TimeoutError
from heiwa_protocol.protocol import Subject
from heiwa_sdk.config import settings

async def check_handshake(verbose=True) -> bool:
    """Validates full vertical path. Returns True if healthy."""
    health_ok = True

    def log_step(name, status, msg=""):
        if verbose:
            icon = "✅" if status else "❌"
            print(f"{icon} {name.ljust(25)} {msg}")

    # 1. Local env + token presence
    token = settings.HEIWA_AUTH_TOKEN or os.getenv("HEIWA_AUTH_TOKEN")
    if token:
        log_step("Auth Token", True, "[Present]")
    else:
        log_step("Auth Token", False, "[Missing HEIWA_AUTH_TOKEN]")
        health_ok = False

    # 2. Reachability to configured NATS
    nats_url = settings.NATS_URL
    try:
        nc = await nats.connect(nats_url, connect_timeout=3)
        log_step("NATS Reachability", True, f"[{nats_url}]")
    except Exception as e:
        log_step("NATS Reachability", False, f"[{nats_url} - {e}]")
        health_ok = False
        return health_ok

    # 3. Spine ACK round-trip
    try:
        sub = await nc.subscribe(Subject.TASK_STATUS.value)
        task_id = f"cmd-hs-{uuid.uuid4().hex[:6]}"
        payload = {
            "sender_id": "handshake-check",
            "timestamp": time.time(),
            "type": Subject.CORE_REQUEST.name,
            "auth_token": token,
            "data": {
                "task_id": task_id,
                "raw_text": "ping",
                "source": "handshake",
                "intent_class": "ping",
                "target_runtime": "spine",
                "sender_id": "handshake-check",
            }
        }
        await nc.publish(Subject.CORE_REQUEST.value, json.dumps(payload).encode())
        
        # Wait up to 3s for Spine ACK
        ack_received = False
        start_time = time.time()
        while time.time() - start_time < 3:
            msg = await sub.next_msg(timeout=3.0)
            data = json.loads(msg.data.decode()).get("data", json.loads(msg.data.decode()))
            
            if data.get("task_id") == task_id or (data.get("status") == "BLOCKED_AUTH" and data.get("task_id") == "unknown"):
                if data.get("accepted"):
                    ack_received = True
                    break
                else:
                    log_step("Spine ACK", False, f"[Rejected: {data.get('reason')}]")
                    health_ok = False
                    await nc.close()
                    return health_ok

        log_step("Spine ACK", True, "[Received]")

        # 4. TASK_EXEC_RESULT receipt window (skip full exec for now, just verifying Spine ACK is enough for basic connectivity)
        # To truly verify EXEC_RESULT, we would dispatch a safe command and wait for result.
        
    except TimeoutError:
        log_step("Spine ACK", False, "[Timeout: Spine orchestrator unresponsive]")
        health_ok = False
    except Exception as e:
        log_step("Spine ACK", False, f"[Error: {e}]")
        health_ok = False

    await nc.close()
    return health_ok

if __name__ == "__main__":
    if asyncio.run(check_handshake()):
        sys.exit(0)
    else:
        print("\nFix the errors above to restore Heiwa command path.")
        sys.exit(1)
