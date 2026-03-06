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

import nats
from nats.errors import TimeoutError

# Ensure project root is on sys.path for direct script execution
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../../")))

from heiwa_protocol.protocol import Subject
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
    try:
        from heiwa_sdk.config import settings

        return getattr(settings, "HEIWA_AUTH_TOKEN", "")
    except ImportError:
        return ""


def _resolve_discord_bot_token() -> str:
    token = os.getenv("DISCORD_BOT_TOKEN") or os.getenv("DISCORD_TOKEN")
    if token:
        return token
    try:
        from heiwa_sdk.config import settings

        return getattr(settings, "DISCORD_BOT_TOKEN", "")
    except ImportError:
        return ""


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


def _task_payload(task_id: str, probe_id: str, token: str, channel_id: int) -> dict:
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
            "response_channel_id": channel_id,
            "response_thread_id": None,
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
            raise RuntimeError(f"Discord smoke aborted with status {status}: {data.get('message')}")


async def _wait_for_result(sub, task_id: str, deadline: float) -> dict:
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError
        msg = await sub.next_msg(timeout=remaining)
        data = json.loads(msg.data.decode()).get("data", {})
        if data.get("task_id") == task_id:
            return data


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

    url = os.getenv("NATS_URL", "nats://127.0.0.1:4222")
    logger.info("Connecting to NATS at %s...", url)
    try:
        nc = await nats.connect(url, connect_timeout=5)
    except Exception as exc:
        logger.error("Failed to connect to NATS: %s", exc)
        return 1

    task_id = f"smoke-discord-{uuid.uuid4().hex[:8]}"
    probe_id = uuid.uuid4().hex[:10]
    probe_marker = f"HEIWA_SMOKE_PROBE_OK:{probe_id}"
    deadline = time.monotonic() + KPI_SECONDS

    status_sub = await nc.subscribe(Subject.TASK_STATUS.value)
    result_sub = await nc.subscribe(Subject.TASK_EXEC_RESULT.value)

    payload = _task_payload(task_id=task_id, probe_id=probe_id, token=auth_token, channel_id=channel_id)
    logger.info("Publishing Discord smoke envelope %s to %s", task_id, Subject.CORE_REQUEST.value)
    await nc.publish(Subject.CORE_REQUEST.value, json.dumps(payload).encode())

    try:
        await _wait_for_status(status_sub, task_id, "ACKNOWLEDGED", deadline)
        logger.info("✅ ACKNOWLEDGED received.")

        await _wait_for_status(status_sub, task_id, "DISPATCHED_PLAN", deadline)
        logger.info("✅ DISPATCHED_PLAN received.")

        result = await _wait_for_result(result_sub, task_id, deadline)
        if result.get("status") != "PASS":
            raise RuntimeError(f"Discord smoke result was not PASS: {result}")
        if probe_marker not in str(result.get("summary", "")):
            raise RuntimeError(f"Discord smoke result missing probe marker {probe_marker!r}")
        logger.info("✅ PASS result received with probe marker.")

        await _wait_for_status(status_sub, task_id, "DELIVERED", deadline)
        logger.info("✅ DELIVERED received on the bus.")

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
    finally:
        await nc.close()


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
