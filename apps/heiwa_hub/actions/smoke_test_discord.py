from __future__ import annotations

import asyncio
import json
import logging
import os
import sys
import time
import uuid
from pathlib import Path
from urllib.error import HTTPError
from urllib.parse import urlencode
from urllib.request import Request, urlopen

import httpx
import websockets

# Ensure project root is on sys.path for direct script execution
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../../")))

from heiwa_sdk.config import settings
from heiwa_sdk.db import Database

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger("SmokeTestDiscord")

KPI_SECONDS = float(os.getenv("HEIWA_SMOKE_DISCORD_KPI_SECONDS", "60"))
POLL_INTERVAL_SECONDS = float(os.getenv("HEIWA_SMOKE_DISCORD_POLL_SECONDS", "2"))
DISCORD_API_BASE = "https://discord.com/api/v10"
SMOKE_PREFIX = "HEIWA_SMOKE_PROBE:"


def _resolve_auth_token() -> str:
    token = os.getenv("HEIWA_AUTH_TOKEN")
    if token:
        return token
    return getattr(settings, "HEIWA_AUTH_TOKEN", "")


def _resolve_hub_url() -> str:
    return str(settings.HUB_BASE_URL or "https://api.heiwa.ltd").rstrip("/")


def _resolve_discord_bot_token() -> str:
    token = os.getenv("DISCORD_BOT_TOKEN") or os.getenv("DISCORD_TOKEN")
    if token:
        return token
    return getattr(settings, "DISCORD_BOT_TOKEN", "")


def _resolve_discord_channel_id() -> int | None:
    explicit = str(os.getenv("HEIWA_DISCORD_SMOKE_CHANNEL_ID", "")).strip()
    if explicit:
        try:
            return int(explicit)
        except ValueError:
            logger.warning("Ignoring invalid HEIWA_DISCORD_SMOKE_CHANNEL_ID=%s", explicit)

    purpose = str(os.getenv("HEIWA_DISCORD_SMOKE_PURPOSE", "smoke-test")).strip() or "smoke-test"
    try:
        db_value = Database().get_discord_channel(purpose)
        if db_value:
            return int(db_value)
    except Exception as exc:
        logger.debug("Failed to resolve Discord smoke channel from DB: %s", exc)
    return None


def _discord_get(path: str, bot_token: str) -> object:
    req = Request(
        f"{DISCORD_API_BASE}{path}",
        headers={
            "Authorization": f"Bot {bot_token}",
            "User-Agent": "heiwa-smoke-discord/1.0",
        },
        method="GET",
    )
    with urlopen(req, timeout=10) as resp:
        return json.loads(resp.read().decode())


def _get_bot_user(bot_token: str) -> dict:
    return _discord_get("/users/@me", bot_token)


def _fetch_channel_messages(bot_token: str, channel_id: int, limit: int = 25) -> list[dict]:
    query = urlencode({"limit": limit})
    data = _discord_get(f"/channels/{channel_id}/messages?{query}", bot_token)
    return data if isinstance(data, list) else []


def _baseline_message_id(messages: list[dict]) -> int:
    if not messages:
        return 0
    return max(int(str(item.get("id", "0")) or "0") for item in messages)


def _flatten_message_text(message: dict) -> str:
    parts: list[str] = []
    content = str(message.get("content") or "")
    if content:
        parts.append(content)

    for embed in message.get("embeds", []):
        if embed.get("title"):
            parts.append(str(embed["title"]))
        if embed.get("description"):
            parts.append(str(embed["description"]))
        footer = embed.get("footer") or {}
        if footer.get("text"):
            parts.append(str(footer["text"]))
        for field in embed.get("fields", []):
            if field.get("name"):
                parts.append(str(field["name"]))
            if field.get("value"):
                parts.append(str(field["value"]))

    return "\n".join(parts)


async def _submit_task(*, hub_url: str, auth_token: str, task_id: str, probe_id: str, channel_id: int) -> dict:
    headers = {
        "Authorization": f"Bearer {auth_token}",
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "heiwa-smoke-discord/1.0",
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

    raise TimeoutError(f"Discord smoke test did not complete within {KPI_SECONDS:.2f}s")


async def _wait_for_discord_post(
    *,
    bot_token: str,
    channel_id: int,
    bot_user_id: str,
    task_id: str,
    probe_marker: str,
    baseline_id: int,
    deadline: float,
) -> dict:
    while True:
        if time.monotonic() >= deadline:
            raise TimeoutError

        messages = _fetch_channel_messages(bot_token, channel_id, limit=25)
        for message in messages:
            try:
                message_id = int(str(message.get("id", "0")) or "0")
            except ValueError:
                continue
            if message_id <= baseline_id:
                continue
            author = message.get("author") or {}
            if str(author.get("id", "")) != bot_user_id:
                continue

            flattened = _flatten_message_text(message)
            if task_id in flattened and probe_marker in flattened:
                return message

        await asyncio.sleep(POLL_INTERVAL_SECONDS)


async def main() -> int:
    bot_token = _resolve_discord_bot_token()
    if not bot_token:
        logger.info("SKIP: DISCORD_BOT_TOKEN/DISCORD_TOKEN is not configured.")
        return 0

    channel_id = _resolve_discord_channel_id()
    if not channel_id:
        logger.info("SKIP: HEIWA_DISCORD_SMOKE_CHANNEL_ID or DB purpose 'smoke-test' is not configured.")
        return 0

    auth_token = _resolve_auth_token()
    if not auth_token:
        logger.error("HEIWA_AUTH_TOKEN is required for the Discord smoke test.")
        return 1

    hub_url = _resolve_hub_url()

    try:
        bot_user = _get_bot_user(bot_token)
        bot_user_id = str(bot_user["id"])
    except HTTPError as exc:
        logger.error("Failed to authenticate with Discord API: HTTP %s", exc.code)
        return 1
    except Exception as exc:
        logger.error("Failed to authenticate with Discord API: %s", exc)
        return 1

    baseline_id = _baseline_message_id(_fetch_channel_messages(bot_token, channel_id, limit=10))

    task_id = f"smoke-discord-{uuid.uuid4().hex[:8]}"
    probe_id = uuid.uuid4().hex[:10]
    probe_marker = f"HEIWA_SMOKE_PROBE_OK:{probe_id}"
    deadline = time.monotonic() + KPI_SECONDS

    try:
        accepted = await _submit_task(
            hub_url=hub_url,
            auth_token=auth_token,
            task_id=task_id,
            probe_id=probe_id,
            channel_id=channel_id,
        )
        route = accepted.get("route", {})
        logger.info("Accepted %s via %s/%s", task_id, route.get("intent_class", "unknown"), route.get("target_tool", "unknown"))

        statuses, result = await _wait_for_terminal_result(hub_url=hub_url, auth_token=auth_token, task_id=task_id)
        if not any(status in {"ACKNOWLEDGED", "DISPATCHED_PLAN", "DISPATCHED_FALLBACK"} for status in statuses):
            raise RuntimeError(f"Discord smoke task never showed orchestrator progress: {statuses}")

        summary = str(result.get("summary", ""))
        terminal = str(result.get("status") or result.get("run_status") or "").upper()
        if terminal not in {"PASS", "DELIVERED"}:
            raise RuntimeError(f"Discord smoke result was not successful: {result}")
        if probe_marker not in summary:
            raise RuntimeError(f"Discord smoke result missing probe marker {probe_marker!r}")

        message = await _wait_for_discord_post(
            bot_token=bot_token,
            channel_id=channel_id,
            bot_user_id=bot_user_id,
            task_id=task_id,
            probe_marker=probe_marker,
            baseline_id=baseline_id,
            deadline=deadline,
        )
        logger.info("✅ Discord post observed in channel %s via message %s.", channel_id, message.get("id"))
        return 0
    except TimeoutError:
        logger.error("❌ TIMEOUT: Discord smoke test did not complete within %.2fs.", KPI_SECONDS)
        return 1
    except RuntimeError as exc:
        logger.error("❌ DISCORD SMOKE FAILURE: %s", exc)
        return 1


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
