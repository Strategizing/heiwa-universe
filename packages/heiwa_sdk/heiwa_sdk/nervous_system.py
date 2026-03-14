# libs/heiwa_sdk/nervous_system.py
from __future__ import annotations

import logging
import os
from dataclasses import dataclass
from typing import Any

import httpx

from .config import settings

logger = logging.getLogger("NervousSystem")


@dataclass(slots=True)
class PublishAck:
    seq: int = 0
    task_id: str = ""


class HeiwaNervousSystem:
    """
    Compatibility shim for older Heiwa code that expected a "nervous system".

    The runtime transport is now HTTP/WebSocket via the hub control plane, not NATS.
    """

    def __init__(self, hub_url: str | None = None):
        self.hub_url = str(hub_url or settings.HUB_BASE_URL or "https://api.heiwa.ltd").rstrip("/")
        self.auth_token = os.getenv("HEIWA_AUTH_TOKEN") or getattr(settings, "HEIWA_AUTH_TOKEN", "") or ""
        self.connected = False

    async def connect(self):
        self.connected = True
        logger.info("Compatibility nervous system bound to hub ingress at %s", self.hub_url)

    async def disconnect(self):
        self.connected = False

    async def publish_directive(self, subject: str, data: dict[str, Any]):
        if not self.connected:
            raise ConnectionError("Compatibility nervous system not connected. Call connect() first.")

        payload = dict(data or {})
        if subject == "heiwa.moltbook.logs":
            logger.info("Legacy moltbook log forwarded locally: %s", payload.get("content", ""))
            return PublishAck(seq=0)

        prompt = str(
            payload.get("raw_text")
            or payload.get("instruction")
            or payload.get("content")
            or ""
        ).strip()
        if not prompt:
            raise ValueError(f"Legacy directive bridge requires raw_text/instruction/content for subject {subject!r}")

        headers = {"Content-Type": "application/json", "Accept": "application/json"}
        if self.auth_token:
            headers["Authorization"] = f"Bearer {self.auth_token}"

        body = {
            "prompt": prompt,
            "source": "legacy-nervous-system",
        }
        async with httpx.AsyncClient(timeout=20.0) as client:
            response = await client.post(f"{self.hub_url}/tasks", json=body, headers=headers)
            response.raise_for_status()
            accepted = response.json()
        return PublishAck(seq=0, task_id=str(accepted.get("task_id") or ""))

    async def subscribe_worker(self, subject: str, callback):
        raise RuntimeError(
            "Legacy worker subscriptions were removed with NATS. "
            "Use the authenticated worker websocket at /ws/worker instead."
        )
