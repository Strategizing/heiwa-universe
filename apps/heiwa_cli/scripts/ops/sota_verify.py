"""
SOTA Verify — send a task to the hub and stream the result.

Usage: python sota_verify.py "your instruction here"
"""

import asyncio
import json
import os
import sys
import uuid
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
for pkg in ["heiwa_sdk", "heiwa_protocol", "heiwa_identity"]:
    path = str(ROOT / f"packages/{pkg}")
    if path not in sys.path:
        sys.path.insert(0, path)
if str(ROOT / "apps") not in sys.path:
    sys.path.insert(0, str(ROOT / "apps"))

from heiwa_sdk.config import load_swarm_env, settings
load_swarm_env()


def render_output_text(output: str) -> str:
    text = str(output or "").strip()
    if not text:
        return ""
    start = text.find("{")
    end = text.rfind("}")
    if start != -1 and end > start:
        try:
            payload = json.loads(text[start : end + 1])
        except json.JSONDecodeError:
            return text
        payloads = payload.get("payloads")
        if isinstance(payloads, list):
            rendered = []
            for item in payloads:
                if isinstance(item, dict):
                    rendered_text = str(item.get("text") or "").strip()
                    if rendered_text:
                        rendered.append(rendered_text)
            if rendered:
                return "\n\n".join(rendered)
    return text


async def sota_verify(instruction: str):
    hub_url = os.getenv("HEIWA_HUB_URL") or getattr(settings, "HUB_BASE_URL", None) or "https://api.heiwa.ltd"
    token = os.getenv("HEIWA_AUTH_TOKEN") or getattr(settings, "HEIWA_AUTH_TOKEN", "") or ""
    task_id = f"sota-task-{uuid.uuid4().hex[:6]}"

    # Submit via HTTP
    import urllib.request
    import urllib.error

    body = json.dumps({
        "raw_text": instruction,
        "sender_id": "devon-operator",
        "source_surface": "sota-verification",
        "task_id": task_id,
    }).encode()
    headers = {"Content-Type": "application/json", "Authorization": f"Bearer {token}"}

    req = urllib.request.Request(f"{hub_url}/tasks", data=body, headers=headers, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            result = json.loads(resp.read().decode())
            route = result.get("route", {})
            print(f"🚀 Dispatched: {task_id}")
            print(f"   Route: {route.get('intent_class')} -> {route.get('target_tool')}")
    except (urllib.error.URLError, urllib.error.HTTPError, OSError) as e:
        print(f"❌ Hub unreachable: {e}")
        sys.exit(1)

    # Stream result via WebSocket
    async def poll_result(timeout: float = 60.0) -> int:
        headers = {"Authorization": f"Bearer {token}"} if token else {}
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                req = urllib.request.Request(f"{hub_url}/tasks/{task_id}", headers=headers, method="GET")
                with urllib.request.urlopen(req, timeout=10) as resp:
                    payload = json.loads(resp.read().decode())
                    status = str(payload.get("status") or payload.get("run_status") or "").upper()
                    summary = str(payload.get("summary") or payload.get("result") or payload.get("content") or "").strip()
                    if status in {"PASS", "SUCCESS", "COMPLETED"}:
                        print("\n✅ [RESULT received]")
                        print("-" * 40)
                        print(render_output_text(summary) or "No summary.")
                        print("-" * 40)
                        return 0
                    if status in {"FAIL", "ERROR", "BLOCKED_AUTH", "BLOCKED_NO_CONTENT"}:
                        print(f"\n❌ {status}: {summary or 'task failed'}")
                        return 1
            except urllib.error.HTTPError as e:
                if e.code not in {404, 202}:
                    print(f"⚠️ Poll error: HTTP {e.code}")
                    break
            except Exception:
                pass
            await asyncio.sleep(2)
        print("⚠️ Timed out waiting for Heiwa result.")
        return 1

    try:
        import websockets
    except ImportError:
        print("⚠️ websockets not installed — falling back to HTTP polling.")
        sys.exit(await poll_result())

    ws_url = hub_url.replace("https://", "wss://").replace("http://", "ws://")
    ws_url = f"{ws_url}/ws/tasks/{task_id}?token={token}"

    try:
        async with websockets.connect(ws_url, open_timeout=10) as ws:
            deadline = time.time() + 60
            while time.time() < deadline:
                try:
                    raw = await asyncio.wait_for(ws.recv(), timeout=35.0)
                except asyncio.TimeoutError:
                    continue
                event = json.loads(raw)
                status = event.get("status", "")
                if event.get("type") == "heartbeat":
                    continue
                if status in {"DELIVERED", "PASS"}:
                    print("\n✅ [RESULT received]")
                    print("-" * 40)
                    print(render_output_text(event.get("summary", "No summary.")))
                    print("-" * 40)
                    return
                elif status in {"FAIL", "BLOCKED_AUTH", "BLOCKED_NO_CONTENT"}:
                    print(f"\n❌ {status}: {event.get('message', '')}")
                    return
                else:
                    msg = event.get("message") or event.get("content") or ""
                    if msg:
                        print(f"  [{status}] {msg}")
    except Exception as e:
        print(f"⚠️ WebSocket error: {e}")
        sys.exit(await poll_result())


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: sota_verify.py <instruction>")
        sys.exit(1)
    asyncio.run(sota_verify(sys.argv[1]))
