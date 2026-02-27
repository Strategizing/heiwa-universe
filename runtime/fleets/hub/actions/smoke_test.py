import asyncio
import json
import logging
import os
import sys
import time
from typing import Any

import nats
from nats.errors import TimeoutError

# Ensure project root is on sys.path for direct script execution
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../../")))

from fleets.hub.protocol import Subject

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger("SmokeTest")

async def main():
    url = os.getenv("NATS_URL", "nats://127.0.0.1:4222")
    logger.info(f"Connecting to NATS at {url}...")

    try:
        nc = await nats.connect(url, connect_timeout=5)
    except Exception as e:
        logger.error(f"Failed to connect to NATS: {e}")
        sys.exit(1)

    logger.info("Connected to NATS.")

    task_id = f"smoke-test-{int(time.time())}"

    # We will subscribe to TASK_STATUS to catch the ACK
    sub = await nc.subscribe(Subject.TASK_STATUS.value)

    # Create the payload to send to Spine
    payload = {
        "sender_id": "heiwa-smoke-tester",
        "timestamp": time.time(),
        "type": Subject.CORE_REQUEST.value,
        "data": {
            "task_id": task_id,
            "intent_class": "test",
            "instruction": "Smoke test execution",
            "response_channel_id": "smoke",
        }
    }

    # Send the request
    logger.info(f"Publishing smoke test envelope {task_id} to {Subject.CORE_REQUEST.value}")
    await nc.publish(Subject.CORE_REQUEST.value, json.dumps(payload).encode())

    # Wait for the response
    logger.info("Waiting for ACK from Spine...")
    try:
        # Give Spine 5 seconds to reply
        msg = await sub.next_msg(timeout=5.0)
        data = json.loads(msg.data.decode())

        status_data = data.get("data", data)
        received_task_id = status_data.get("task_id")
        status = status_data.get("status")

        if received_task_id == task_id and status == "ACKNOWLEDGED":
            logger.info("✅ SUCCESS: Received ACKNOWLEDGED from Spine.")
            await nc.close()
            sys.exit(0)
        else:
            logger.warning(f"⚠️ Received unexpected status message: {status_data}")
            await nc.close()
            sys.exit(1)

    except TimeoutError:
        logger.error("❌ TIMEOUT: Did not receive ACK from Spine within 5 seconds.")
        await nc.close()
        sys.exit(1)

if __name__ == "__main__":
    asyncio.run(main())
