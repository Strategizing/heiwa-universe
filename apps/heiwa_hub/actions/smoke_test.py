from __future__ import annotations

import asyncio
import json
import logging
import os
import sys
import time
import uuid
from pathlib import Path

import httpx
import websockets

# Ensure project root is on sys.path for direct script execution
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../../")))

from heiwa_sdk.config import settings

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger("SmokeTest")

KPI_SECONDS = float(os.getenv("HEIWA_SMOKE_KPI_SECONDS", "30"))
SMOKE_PREFIX = "HEIWA_SMOKE_PROBE:"


def _resolve_auth_token() -> str:
    token = os.getenv("HEIWA_AUTH_TOKEN")
    if token:
        return token
    return getattr(settings, "HEIWA_AUTH_TOKEN", "")


def _resolve_hub_url() -> str:
    return str(settings.HUB_BASE_URL or "https://api.heiwa.ltd").rstrip("/")


async def _submit_task(*, hub_url: str, auth_token: str, task_id: str, probe_id: str) -> dict:
    headers = {
        "Authorization": f"Bearer {auth_token}",
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "heiwa-smoke/1.0",
    }
    payload = {
        "task_id": task_id,
        "raw_text": (
            "Verify the Heiwa runtime service scope with a safe smoke probe and report the result. "
            f"{SMOKE_PREFIX}{probe_id}"
        ),
        "sender_id": "discord-smoke-bridge",
        "source_surface": "discord",
    }
    async with httpx.AsyncClient(timeout=20.0) as client:
        response = await client.post(f"{hub_url}/tasks", json=payload, headers=headers)
        response.raise_for_status()
        return response.json()


async def _wait_for_terminal_result(*, hub_url: str, auth_token: str, task_id: str) -> tuple[list[str], dict]:
    ws_url = hub_url.replace("https://", "wss://").replace("http://", "ws://")
    ws_url = f"{ws_url}/ws/tasks/{task_id}?token={auth_token}"
    deadline = time.monotonic() + KPI_SECONDS
    seen_statuses: list[str] = []
    last_event: dict = {}

    async with websockets.connect(ws_url, open_timeout=10) as ws:
        while time.monotonic() < deadline:
            remaining = max(1.0, deadline - time.monotonic())
            raw = await asyncio.wait_for(ws.recv(), timeout=remaining)
            event = json.loads(raw)
            status = str(event.get("status") or event.get("run_status") or "").upper()
            if status:
                seen_statuses.append(status)
                last_event = event
                if status in {"DELIVERED", "PASS", "FAIL", "BLOCKED_AUTH", "BLOCKED_NO_CONTENT"}:
                    return seen_statuses, event

    raise TimeoutError(f"Smoke test did not complete within {KPI_SECONDS:.2f}s")


async def main() -> int:
    auth_token = _resolve_auth_token()
    if not auth_token:
        logger.error("HEIWA_AUTH_TOKEN is required for the smoke test.")
        return 1

    hub_url = _resolve_hub_url()
    task_id = f"smoke-e2e-{uuid.uuid4().hex[:8]}"
    probe_id = uuid.uuid4().hex[:10]
    started = time.monotonic()

    try:
        accepted = await _submit_task(hub_url=hub_url, auth_token=auth_token, task_id=task_id, probe_id=probe_id)
        route = accepted.get("route", {})
        logger.info("Accepted %s via %s/%s", task_id, route.get("intent_class", "unknown"), route.get("target_tool", "unknown"))

        statuses, result = await _wait_for_terminal_result(hub_url=hub_url, auth_token=auth_token, task_id=task_id)
        summary = str(result.get("summary", ""))
        expected_marker = f"HEIWA_SMOKE_PROBE_OK:{probe_id}"

        if not any(status in {"ACKNOWLEDGED", "DISPATCHED_PLAN", "DISPATCHED_FALLBACK"} for status in statuses):
            raise RuntimeError(f"Smoke task never showed orchestrator progress: {statuses}")

        terminal = str(result.get("status") or result.get("run_status") or "").upper()
        if terminal not in {"PASS", "DELIVERED"}:
            raise RuntimeError(f"Smoke result was not successful: {result}")
        if expected_marker not in summary:
            raise RuntimeError(f"Smoke result missing probe marker {expected_marker!r}: {summary[:500]}")

        elapsed = time.monotonic() - started
        if elapsed > KPI_SECONDS:
            raise RuntimeError(f"Smoke test exceeded KPI: {elapsed:.2f}s > {KPI_SECONDS:.2f}s")

        logger.info("✅ Smoke test completed in %.2fs.", elapsed)
        return 0
    except Exception as exc:
        logger.error("❌ SMOKE FAILURE: %s", exc)
        return 1


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
