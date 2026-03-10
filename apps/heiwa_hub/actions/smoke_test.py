import asyncio
import json
import logging
import os
import sys
import time
import uuid

import nats
from nats.errors import TimeoutError

# Ensure project root is on sys.path for direct script execution
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../../")))

from heiwa_protocol.protocol import Subject

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger("SmokeTest")

KPI_SECONDS = float(os.getenv("HEIWA_SMOKE_KPI_SECONDS", "30"))
SMOKE_PREFIX = "HEIWA_SMOKE_PROBE:"


def _task_payload(task_id: str, probe_id: str, token: str) -> dict:
    return {
        "sender_id": "discord-smoke-bridge",
        "timestamp": time.time(),
        "type": Subject.CORE_REQUEST.value,
        "auth_token": token,
        "data": {
            "task_id": task_id,
            "raw_text": (
                "Verify the Heiwa runtime service scope with a safe smoke probe and report the result. "
                f"{SMOKE_PREFIX}{probe_id}"
            ),
            "source": "discord",
            "intent_class": "audit",
            "response_channel_id": "discord-smoke",
            "sender_id": "discord-smoke-bridge",
        },
    }


async def _wait_for_status(sub, task_id: str, expected: str, deadline: float) -> dict:
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError
        msg = await sub.next_msg(timeout=remaining)
        data = json.loads(msg.data.decode()).get("data", {})
        if data.get("task_id") != task_id:
            continue
        status = data.get("status")
        if status == expected:
            return data
        if status in {"BLOCKED_AUTH", "BLOCKED_NO_CONTENT", "FAIL"}:
            raise RuntimeError(f"Smoke test aborted with status {status}: {data.get('message')}")


async def _wait_for_result(sub, task_id: str, deadline: float) -> dict:
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError
        msg = await sub.next_msg(timeout=remaining)
        data = json.loads(msg.data.decode()).get("data", {})
        if data.get("task_id") == task_id:
            return data


async def main():
    url = os.getenv("NATS_URL", "nats://127.0.0.1:4222")
    logger.info(f"Connecting to NATS at {url}...")

    try:
        nc = await nats.connect(url, connect_timeout=5)
    except Exception as e:
        logger.error(f"Failed to connect to NATS: {e}")
        sys.exit(1)

    logger.info("Connected to NATS.")

    task_id = f"smoke-e2e-{uuid.uuid4().hex[:8]}"
    probe_id = uuid.uuid4().hex[:10]
    started = time.monotonic()
    deadline = started + KPI_SECONDS

    status_sub = await nc.subscribe(Subject.TASK_STATUS.value)
    result_sub = await nc.subscribe(Subject.TASK_EXEC_RESULT.value)

    token = os.getenv("HEIWA_AUTH_TOKEN")
    if not token:
        try:
            from heiwa_sdk.config import settings
            token = getattr(settings, "HEIWA_AUTH_TOKEN", "")
        except ImportError:
            token = ""

    payload = _task_payload(task_id=task_id, probe_id=probe_id, token=token)

    # Send the request
    logger.info(f"Publishing smoke test envelope {task_id} to {Subject.CORE_REQUEST.value}")
    await nc.publish(Subject.CORE_REQUEST.value, json.dumps(payload).encode())

    try:
        logger.info("Waiting for ACKNOWLEDGED...")
        await _wait_for_status(status_sub, task_id, "ACKNOWLEDGED", deadline)
        logger.info("✅ ACKNOWLEDGED received.")

        logger.info("Waiting for DISPATCHED_PLAN...")
        await _wait_for_status(status_sub, task_id, "DISPATCHED_PLAN", deadline)
        logger.info("✅ DISPATCHED_PLAN received.")

        logger.info("Waiting for TASK_EXEC_RESULT...")
        result = await _wait_for_result(result_sub, task_id, deadline)
        if result.get("status") != "PASS":
            raise RuntimeError(f"Smoke result was not PASS: {result}")

        summary = str(result.get("summary", ""))
        expected_marker = f"HEIWA_SMOKE_PROBE_OK:{probe_id}"
        if expected_marker not in summary:
            raise RuntimeError(f"Smoke result missing probe marker {expected_marker!r}: {summary[:500]}")
        logger.info("✅ PASS result received with probe marker.")

        logger.info("Waiting for DELIVERED...")
        await _wait_for_status(status_sub, task_id, "DELIVERED", deadline)
        elapsed = time.monotonic() - started
        if elapsed > KPI_SECONDS:
            raise RuntimeError(f"Smoke test exceeded KPI: {elapsed:.2f}s > {KPI_SECONDS:.2f}s")
        logger.info("✅ DELIVERED received in %.2fs.", elapsed)
        await nc.close()
        sys.exit(0)

    except TimeoutError:
        logger.error("❌ TIMEOUT: Smoke test did not complete within %.2fs.", KPI_SECONDS)
        await nc.close()
        sys.exit(1)
    except RuntimeError as exc:
        logger.error("❌ SMOKE FAILURE: %s", exc)
        await nc.close()
        sys.exit(1)

if __name__ == "__main__":
    asyncio.run(main())
